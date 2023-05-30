use std::net::SocketAddr;
use std::collections::HashSet;
use std::io::{stdout, Write};
use std::fmt::Display;
use std::sync::Arc;

use bit_vec::BitVec;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{RwLock, mpsc};
use url::Url;

use crate::metainfo::{self, MetaInfo, FileMode};
use crate::tracker::{Tracker, self, TrackerRequest, Peers};
use crate::peer::{Peer, self, Message, WriteMessage};

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
    available_pieces: Arc<RwLock<HashSet<u32>>>,
    file_bitfield: Arc<RwLock<BitVec>>,
}

impl DownloadingPiece {
    pub fn new(available_pieces: Arc<RwLock<HashSet<u32>>>, file_bitfield: Arc<RwLock<BitVec>>) -> Self {
        Self { piece: None, offset: 0, available_pieces, file_bitfield }
    }
}

impl Drop for DownloadingPiece {
    fn drop(&mut self) {
        if let Some(piece) = self.piece {
            println!("dropping {}", piece);

            let available_pieces = Arc::clone(&self.available_pieces);
            let file_bitfield = Arc::clone(&self.file_bitfield);

            tokio::spawn(async move {
                if file_bitfield.read().await.get(piece as usize).is_none() {
                    available_pieces.write().await.insert(piece);
                }
            });
            
        }
    }
}

pub struct Torrent {
    peer_id: [u8; 20],
    metainfo: MetaInfo,
    connected_peers: Arc<RwLock<HashSet<SocketAddr>>>,
    file_bitfield: Arc<RwLock<BitVec>>,
    available_pieces: Arc<RwLock<HashSet<u32>>>,
}

impl Torrent {
    /// Creates a new torrent and connects to the first tracker given by the metainfo
    pub async fn new(torrent: &str) -> Result<Torrent, Error> {
        let metainfo = MetaInfo::try_from(torrent)?;

        // todo move this into download function
        // calculate how many bytes the torrent needs to download
        // TODO: increase limit (around 3GB right now)
        let _length = match metainfo.info().mode() {
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

        let file_bitfield = Arc::new(RwLock::new(BitVec::from_elem(metainfo.info().pieces().len(), false)));

        let mut available_pieces = HashSet::new();

        for i in 0..(metainfo.info().pieces().len() as u32) {
            available_pieces.insert(i);
        }

        Ok(Torrent {
            peer_id,
            metainfo,
            connected_peers: Arc::new(RwLock::new(HashSet::new())),
            file_bitfield,
            available_pieces: Arc::new(RwLock::new(available_pieces)),
        })
    }

    pub async fn download(&mut self) {
        let mut file_len = 0;

        if let FileMode::SingleFile { length, .. } = self.metainfo.info().mode() {
            println!("file len: {}", length);
            file_len = *length;
        }

        let request = TrackerRequest::new(
            *self.metainfo.info_hash(),
            self.peer_id,
            6881,
            0,
            0,
            file_len.into(),
            true,
            false
        );

        let url = Url::parse(self.metainfo.announce()).unwrap();
        let tracker_address = url.socket_addrs(|| None).unwrap()[0];
        let mut tracker_stream = TcpStream::connect(tracker_address).await.unwrap();

        let mut tracker = Tracker::new(&mut tracker_stream, &url, &request).await.unwrap();

        let (sender, mut reciever) = mpsc::channel::<WriteMessage>(1000);

        println!("pieces: {}, piece length: {}", self.metainfo.info().pieces().len(), self.metainfo.info().piece_length());
        

        let num_of_pieces = self.metainfo.info().pieces().len();

        let last_piece_length = get_last_piece_length(file_len as usize, self.metainfo.info().pieces().len(), self.metainfo.info().piece_length() as usize);

        let mut file = OpenOptions::new()
            .read(false)
            .write(true)
            .create(true)
            .open(self.metainfo.info().name())
            .await
            .unwrap();

        let bitfield = Arc::clone(&self.file_bitfield);

        let piece_length = self.metainfo.info().piece_length();

        tokio::spawn(async move {
            let block_num = (piece_length + BLOCK_SIZE - 1) / BLOCK_SIZE; // rounds up
            let last_block_num = (last_piece_length + BLOCK_SIZE - 1) / BLOCK_SIZE; // rounds up

            let mut received_blocks = vec![BitVec::from_elem(block_num as usize, false); num_of_pieces - 1];
            received_blocks.push(BitVec::from_elem(last_block_num as usize, false));

            let mut pieces = vec![Vec::new(); num_of_pieces];

            while let Some(write_message) = reciever.recv().await {
                let piece_buffer = pieces.get_mut(write_message.index() as usize).unwrap();

                // allocates needed size for slice copy
                let begin = write_message.begin() as usize;
                if piece_buffer.len() < begin + write_message.block().len() {
                    piece_buffer.resize(begin + write_message.block().len(), 0);
                }
                piece_buffer[begin..begin + write_message.block().len()].copy_from_slice(write_message.block());

                let block_index = (write_message.begin() as u64 / BLOCK_SIZE as u64) as usize;
                received_blocks.get_mut(write_message.index() as usize).unwrap().set(block_index, true);

                if received_blocks[write_message.index() as usize].all() {
                    println!("piece {} completed", write_message.index());
                    bitfield.write().await.set(write_message.index() as usize, true);

                    // write to file
                    let offset = write_message.index() as u64 * piece_length as u64;

                    file.seek(std::io::SeekFrom::Start(offset)).await.unwrap();
                    file.write_all(&pieces[write_message.index() as usize]).await.unwrap();
                }
            }
        });

        'main: loop {
            if self.file_bitfield.read().await.all() {
                println!("Download finished");
                break;
            }

            // todo handle errors
            if let Err(_err) = tracker.announce().await {
                continue;
            }

            // handle each peer deparately in its own thread
            match tracker.response().unwrap().peers() {
                Peers::Binary(peers) => {
                    for &addr in peers.iter() {
                        if self.file_bitfield.read().await.all() {
                            println!("Download finished");
                            break 'main;
                        }

                        // skip if peer is already connected
                        if self.connected_peers.read().await.contains(&addr) {
                            continue;
                        }

                        let connected_peers = Arc::clone(&self.connected_peers);
                        let info_hash = *self.info_hash();
                        let peer_id = self.peer_id;
                        let piece_length = self.metainfo.info().piece_length();
                        let file_bitfield = Arc::clone(&self.file_bitfield);
                        let available_pieces = Arc::clone(&self.available_pieces);
                        let sender = mpsc::Sender::clone(&sender);

                        let connection = async move {
                            match handle_peer(addr, info_hash, peer_id, piece_length, last_piece_length, file_bitfield, available_pieces, sender).await {
                                Ok(()) => (),
                                Err(Error::PeerError(peer::Error::IoError(_))) => (),
                                Err(err) => {
                                    let mut stdout = stdout().lock();
                                    stdout.write_all(format!("{}\n", err).as_bytes()).unwrap();
                                    stdout.flush().unwrap();
                                },
                            };

                            connected_peers.write().await.remove(&addr);
                        };

                        self.connected_peers.write().await.insert(addr);
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

async fn handle_peer(address: SocketAddr, info_hash: [u8; 20], peer_id: [u8; 20], piece_length: u32, last_piece_length: u32, file_bitfield: Arc<RwLock<BitVec>>, available_pieces: Arc<RwLock<HashSet<u32>>>, sender: mpsc::Sender<WriteMessage>) -> Result<(), Error> {
    // connects and sends handshake
    let pieces = available_pieces.read().await.len();

    let mut stream = match TcpStream::connect(address).await {
        Ok(stream) => stream,
        Err(err) => return Err(peer::Error::IoError(err).into()),
    };

    let mut peer = Peer::new(&mut stream, pieces).await?;

    let mut downloading_piece = DownloadingPiece::new(Arc::clone(&available_pieces), Arc::clone(&file_bitfield));

    let _peer_handshake = peer.handshake(info_hash, peer_id).await?;

    loop {
        // possibly makes all slow when not handling stuck peers
        let message = peer.read_message().await?;
        // println!("piece: {:?}, offset: {:?}, message: {}", downloading_piece.piece, downloading_piece.offset, message);

        match message {
            Message::KeepAlive => {
                // closes connection if peer has no piece the file needs
                if is_there_next_piece(&peer, &available_pieces).await {
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
                    if let Some(next_piece) = get_next_piece(&peer, &available_pieces).await {
                        downloading_piece.piece = Some(next_piece);

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

                if !peer.am_interested() && is_there_next_piece(&peer, &available_pieces).await {
                    peer.send_interested().await?;
                }
            }
            Message::Bitfield(bitfield) => {
                peer.update_bitfield(bitfield);

                if !peer.am_interested() && is_there_next_piece(&peer, &available_pieces).await {
                    peer.send_interested().await?;
                }
            }
            Message::Request { index, begin, length } => (), // peer.send_piece(index, begin, length)?,
            Message::Piece { index, begin, block } => {
                sender.send(WriteMessage::new(index, begin, &block)).await.unwrap();

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
                    if let Some(next_piece) = get_next_piece(&peer, &available_pieces).await {
                        downloading_piece.piece = Some(next_piece);
  
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

/// removes piece from `available_pieces set` if found
async fn get_next_piece(peer: &Peer<'_>, available_pieces: &RwLock<HashSet<u32>>) -> Option<u32> {
    let mut available_pieces = available_pieces.write().await;

    for (piece, exists) in peer.bitfield().iter().enumerate() {
        let piece = piece as u32;
        if exists && available_pieces.get(&piece).is_some() {
            if piece == 396 {
                std::process::exit(0);
            }

            // Remove the piece from the available pieces and return it.
            available_pieces.remove(&piece);
            return Some(piece);
        }
    }

    // If no pieces meet the above conditions, return None.
    None
}

async fn is_there_next_piece(peer: &Peer<'_>, available_pieces: &RwLock<HashSet<u32>>) -> bool {
    let available_pieces = available_pieces.read().await;

    for &piece in available_pieces.iter() {
        if peer.bitfield().get(piece as usize).is_some() {
            return true;
        }
    }

    false
}

fn get_last_piece_length(file_length: usize, pieces: usize, piece_length: usize) -> u32 {
    let length_without_last_piece = piece_length * (pieces - 1);
    (file_length - length_without_last_piece) as u32
}