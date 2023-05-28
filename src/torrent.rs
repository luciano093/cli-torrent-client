use std::{net::SocketAddr, collections::{HashSet, HashMap}, io::{stdout, Write, Seek}, fmt::Display, fs::OpenOptions, sync::{Arc, RwLock, mpsc}};

use bit_vec::BitVec;
use tokio::net::TcpStream;
use crate::{metainfo::{self, MetaInfo, FileMode}, tracker::{Tracker, self, TrackerRequest, Peers}, peer::{Peer, self, Message, WriteMessage}};

static BLOCK_SIZE: u32 = 16384;

#[derive(Debug)]
pub enum Error {
    MetaInfoError(metainfo::Error),
    TrackerError(tracker::Error),
    PeerError(peer::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MetaInfoError(err) => write!(f, "{}", err),
            Self::TrackerError(err) => write!(f, "{}", err),
            Self::PeerError(err) => write!(f, "{}", err),
        }
    }
}

impl std::error::Error for Error { }

impl From<metainfo::Error> for Error {
    fn from(value: metainfo::Error) -> Self {
        Self::MetaInfoError(value)
    }
}

impl From<tracker::Error> for Error {
    fn from(value: tracker::Error) -> Self {
        Self::TrackerError(value)
    }
}

impl From<peer::Error> for Error {
    fn from(value: peer::Error) -> Self {
        Self::PeerError(value)
    }
}

struct DownloadingPiece {
    piece: Option<u32>,
    offset: u32,
    currently_downloading: Arc<RwLock<HashSet<u32>>>,
}

impl DownloadingPiece {
    pub fn new(currently_downloading: Arc<RwLock<HashSet<u32>>>) -> Self {
        Self { piece: None, offset: 0, currently_downloading }
    }
}

impl Drop for DownloadingPiece {
    fn drop(&mut self) {
        if let Some(piece) = self.piece {
            println!("dropping {}", piece);
            self.currently_downloading.write().unwrap().remove(&piece);
        }
    }
}

pub struct Torrent {
    peer_id: [u8; 20],
    metainfo: MetaInfo,
    tracker: Tracker,
    connected_peers: Arc<RwLock<HashSet<SocketAddr>>>,
    file_bitfield: Arc<RwLock<BitVec>>,
    pieces_currently_downloading: Arc<RwLock<HashSet<u32>>>,
}

impl Torrent {
    /// Creates a new torrent and connects to the first tracker given by the metainfo
    pub async fn new(torrent: &str) -> Result<Self, Error> {
        let metainfo = MetaInfo::try_from(torrent)?;

        // calculate how many bytes the torrent needs to download
        // TODO: increase limit (around 3GB right now)
        let left = match metainfo.info().mode() {
            FileMode::SingleFile { length, .. } => {

                *length as u128
            }
            FileMode::MultipleFiles { files } => {
                let mut length = 0u128; // about 3GB max

                for file in files {
                    length += file.lenght() as u128;
                }

                length
            }
        };

        let peer_id_str = "-aa-aaaaaaaaaaaaaaaa".as_bytes();
        let mut peer_id = [0u8; 20];
        for (i, char) in peer_id_str.iter().enumerate() {
            peer_id[i] = *char;
        }

        let request = TrackerRequest::new(
            *metainfo.info_hash(),
            peer_id,
            6881,
            0,
            0,
            left,
            true,
            false
        );

        let tracker = Tracker::connect(metainfo.announce(), &request).await?;

        let file_bitfield = Arc::new(RwLock::new(BitVec::from_elem(metainfo.info().pieces().len(), false)));

        // let file = Arc::new(AtomicFile::new(file, bitfield, metainfo.info().piece_length()));

        Ok(Torrent {
            peer_id,
            metainfo,
            tracker,
            connected_peers: Arc::new(RwLock::new(HashSet::new())),
            file_bitfield,
            pieces_currently_downloading: Arc::new(RwLock::new(HashSet::new())),
        })
    }

    pub async fn download(&mut self) {
        let mut file_len = 0;

        let (sender, reciever) = mpsc::channel::<WriteMessage>();

        println!("pieces: {}, piece length: {}", self.file_bitfield.read().unwrap().len(), self.metainfo.info().piece_length());
        if let FileMode::SingleFile { length, .. } = self.metainfo.info().mode() {
            println!("file len: {}", length);
            file_len = *length;
        }

        let num_of_pieces = self.metainfo.info().pieces().len();

        let last_piece_length = get_last_piece_length(file_len as usize, self.metainfo.info().pieces().len(), self.metainfo.info().piece_length() as usize);

        let mut file = OpenOptions::new()
            .read(false)
            .write(true)
            .create(true)
            .open(self.metainfo.info().name())
            .unwrap();

        let bitfield = Arc::clone(&self.file_bitfield);

        let piece_length = self.metainfo.info().piece_length();

        std::thread::spawn(move || {
            let mut pieces = HashMap::<u32, BitVec>::new();

            for i in 0..num_of_pieces {
                let block_num = if i != num_of_pieces - 1 {
                    (piece_length + BLOCK_SIZE - 1) / BLOCK_SIZE // rounds up
                } else {
                    (last_piece_length + BLOCK_SIZE - 1) / BLOCK_SIZE // rounds up
                };
                
                pieces.insert(i as u32, BitVec::from_elem(block_num as usize, false));
            }

            for write_message in reciever {
                // write to file
                let offset = (write_message.index() as u64 * piece_length as u64) + write_message.begin() as u64;

                file.seek(std::io::SeekFrom::Start(offset)).unwrap();

                file.write_all(write_message.block()).unwrap();
                
                if !pieces.contains_key(&write_message.index()) {
                    println!("test");
                }

                let block_index = (write_message.begin() as u64 / BLOCK_SIZE as u64) as usize;
                pieces.get_mut(&write_message.index()).unwrap().set(block_index, true);

                if pieces[&write_message.index()].all() {
                    file.flush().unwrap();
                    file.sync_all().unwrap();
                    println!("piece {} completed", write_message.index());
                    bitfield.write().unwrap().set(write_message.index() as usize, true);
                }
            }
        });

        'main: loop {
            if self.file_bitfield.read().unwrap().all() {
                println!("Download finished");
                break;
            }

            self.tracker.announce().await.unwrap();

            // handle each peer deparately in its own thread
            match self.tracker.response().unwrap().peers() {
                Peers::Binary(peers) => {
                    for &addr in peers.iter() {
                        if self.file_bitfield.read().unwrap().all() {
                            println!("Download finished");
                            break 'main;
                        }

                        // skip if peer is already connected
                        if self.connected_peers.read().unwrap().contains(&addr) {
                            continue;
                        }

                        let connected_peers = Arc::clone(&self.connected_peers);
                        let info_hash = *self.info_hash();
                        let peer_id = self.peer_id;
                        let piece_length = self.metainfo.info().piece_length();
                        let file_bitfield = Arc::clone(&self.file_bitfield);
                        let currently_downloading = Arc::clone(&self.pieces_currently_downloading);
                        let sender = mpsc::Sender::clone(&sender);

                        let connection = async move {
                            match handle_peer(addr, info_hash, peer_id, piece_length, last_piece_length, file_bitfield, currently_downloading, sender).await {
                                Ok(()) => (),
                                Err(Error::PeerError(peer::Error::IoError(_))) => (),
                                Err(err) => {
                                    stdout().lock().write_all(format!("{}\n", err).as_bytes()).unwrap();
                                    stdout().lock().flush().unwrap();
                                },
                            };

                            connected_peers.write().unwrap().remove(&addr);
                        };

                        //connection();

                        self.connected_peers.write().unwrap().insert(addr);
                        tokio::spawn(connection);
                    }
                },
                Peers::Dictionary(_peers) => {
                    todo!()
                },
            };
        }

        // send "completed" event to tracker
    }

    pub const fn metainfo(&self) -> &MetaInfo {
        &self.metainfo
    }

    pub const fn info_hash(&self) -> &[u8; 20] {
        self.metainfo.info_hash()
    }
}

async fn handle_peer(address: SocketAddr, info_hash: [u8; 20], peer_id: [u8; 20], piece_length: u32, last_piece_length: u32, file_bitfield: Arc<RwLock<BitVec>>, currently_downloading: Arc<RwLock<HashSet<u32>>>, sender: mpsc::Sender<WriteMessage>) -> Result<(), Error> {
    // connects and sends handshake
    let pieces = file_bitfield.read().unwrap().len();

    let mut stream = match TcpStream::connect(address).await {
        Ok(stream) => stream,
        Err(err) => return Err(peer::Error::IoError(err).into()),
    };

    let mut peer = Peer::new(&mut stream, pieces).await?;

    let mut downloading_piece = DownloadingPiece::new(Arc::clone(&currently_downloading));

    let _peer_handshake = peer.handshake(info_hash, peer_id).await?;

    loop {
        let message = peer.read_message().await?;

        match message {
            Message::KeepAlive => {
                // closes connection if peer has no piece the file needs
                if get_next_piece(&peer, &file_bitfield, &currently_downloading).is_none() {
                    return Ok(());
                }
            },
            Message::Choke => {
                peer.set_is_choking(true);
            }
            Message::Unchoke => {
                // redundant message
                if !peer.is_choking() {
                    continue;
                }

                peer.set_is_choking(false);

                if downloading_piece.piece.is_none() {
                    if let Some(next_piece) = get_next_piece(&peer, &file_bitfield, &currently_downloading) {
                        downloading_piece.piece = Some(next_piece);
                        currently_downloading.write().unwrap().insert(next_piece);

                        peer.send_request(next_piece, downloading_piece.offset, BLOCK_SIZE).await?;
                    } else {
                        // no more pieces needed
                        return Ok(());
                    };
                } else {
                    let remaining_piece_size = if downloading_piece.piece.unwrap() as usize == pieces - 1 {
                        last_piece_length - downloading_piece.offset
                    } else {
                        piece_length - downloading_piece.offset
                    };

                    // sends request for smaller block size if needed
                    if remaining_piece_size < BLOCK_SIZE {
                        peer.send_request(downloading_piece.piece.unwrap(), downloading_piece.offset, remaining_piece_size).await?;
                    } else {
                        peer.send_request(downloading_piece.piece.unwrap(), downloading_piece.offset, BLOCK_SIZE).await?;
                    }
                }
            }
            Message::Interested => {
                // todo
            }
            Message::NotInterested => (),
            Message::Have(piece_index) => {
                peer.update_piece(piece_index as usize);

                if !peer.am_interested() && get_next_piece(&peer, &file_bitfield, &currently_downloading).is_some() {
                    peer.send_interested().await?;
                }
            }
            Message::Bitfield(bitfield) => {
                peer.update_bitfield(bitfield);

                if !peer.am_interested() && get_next_piece(&peer, &file_bitfield, &currently_downloading).is_some() {
                    peer.send_interested().await?;
                }
            }
            Message::Request { index, begin, length } => (), // peer.send_piece(index, begin, length)?,
            Message::Piece { index, begin, block } => {
                sender.send(WriteMessage::new(index, begin, &block)).unwrap();

                downloading_piece.offset += block.len() as u32;

                let remaining_piece_size = if index as usize == pieces - 1 {
                    last_piece_length - downloading_piece.offset
                } else {
                    piece_length - downloading_piece.offset
                };

                if remaining_piece_size == 0 {
                    // Reset the offset to zero for the next piece
                    downloading_piece.offset = 0;

                    // Request the next piece
                    if let Some(next_piece) = get_next_piece(&peer, &file_bitfield, &currently_downloading) {
                        downloading_piece.piece = Some(next_piece);

                        currently_downloading.write().unwrap().insert(next_piece);
  
                        peer.send_request(next_piece, 0, BLOCK_SIZE).await?;
                    } else {
                        // no more pieces needed
                        return Ok(());
                    };
                }

                // Check if the remaining size is less than the block size
                else if remaining_piece_size < BLOCK_SIZE {
                    // request a smaller block to finish the piece
                    peer.send_request(downloading_piece.piece.unwrap(), downloading_piece.offset, remaining_piece_size).await?;
                } else {
                    // Otherwise, request the next block as usual
                    peer.send_request(downloading_piece.piece.unwrap(), downloading_piece.offset, BLOCK_SIZE).await?;
                }
            }
            Message::Cancel { index, begin, length } => (), // todo (cancels previouslly requested piece)
            _ => (),
        }
    }
}

fn get_next_piece(peer: &Peer, file_bitfield: &RwLock<BitVec>, currently_downloading: &RwLock<HashSet<u32>>) -> Option<u32> {
    let file_bitfield = file_bitfield.read().unwrap();
    let currently_downloading = currently_downloading.read().unwrap();

    // Iterate over each piece in the file's bitfield.
    for (index, exists) in file_bitfield.iter().enumerate() {
        let piece_index = index as u32;

        // If the piece is not in the file, is in the peer's bitfield, and it's not currently being downloaded,
        // return it as the next piece to be downloaded.
        if !exists && peer.bitfield().get(index).unwrap() && !currently_downloading.contains(&piece_index) {
            return Some(piece_index);
        }
    }

    // If no pieces meet the above conditions, return None.
    None
}

fn get_last_piece_length(file_length: usize, pieces: usize, piece_length: usize) -> u32 {
    let length_without_last_piece = piece_length * (pieces - 1);
    (file_length - length_without_last_piece) as u32
}