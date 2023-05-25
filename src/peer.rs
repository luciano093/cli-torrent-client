use std::fmt::Display;
use std::net::{SocketAddr, TcpStream};
use std::io::{self, Write, Cursor, Seek, Read, BufReader};
use std::time::Duration;

use bit_vec::BitVec;

#[derive(Debug)]
pub enum Error {
    IoError(io::Error),
    NotEnoughBytes { expected: usize, actual: usize },
    InvalidMessageId(u8),
    InvalidPayloadLength { expected: usize, actual: usize },
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(err) => write!(f, "{}", err),
            Self::NotEnoughBytes { expected, actual} => 
                write!(f, "Expected {} bytes but got: {}", expected, actual),
            Self::InvalidMessageId(id) => write!(f, "Invalid message id: {}", id),
            Self::InvalidPayloadLength { expected, actual } =>
                write!(f, "Expected payload of length {} but got {}", expected, actual),
        }
    }
}

impl std::error::Error for Error { }

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::IoError(error)
    }
}

#[derive(Debug, PartialEq, PartialOrd)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request { index: u32, begin: u32, length: u32 },
    Piece { index: u32, begin: u32, block: Vec<u8> },
    Cancel { index: u32, begin: u32, length: u32 },
    Extended(Vec<u8>),
}

impl Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KeepAlive => write!(f, "Keep Alive"),
            Self::Choke => write!(f, "Choke"),
            Self::Unchoke => write!(f, "Unchoke"),
            Self::Interested => write!(f, "Interested"),
            Self::NotInterested => write!(f, "Not Interested"),
            Self::Have(piece) => write!(f, "Have {}", piece),
            Self::Bitfield(_) => write!(f, "Bitfield"),
            Self::Request { .. } => write!(f, "Request"),
            Self::Piece { index, begin, .. } => write!(f, "Piece {} offset {}", index, begin),
            Self::Cancel { .. } => write!(f, "Cancel"),
            Self::Extended(_) => write!(f, "Extended"),
        }
    }
}

impl Message {
    pub fn from_id(id: u8) -> Self {
        match id {
            0 => Self::Choke,
            1 => Self::Unchoke,
            2 => Self::Interested,
            3 => Self::NotInterested,
            _ => panic!("Message needs a payload to be created!"),
        }
    }

    pub fn from_id_and_payload(id: u8, payload: Vec<u8>) -> Result<Self, Error> {
        let len = payload.len();

        match id {
            4 => {
                if len != 4 {
                    return Err(Error::InvalidPayloadLength { expected: 4, actual: len });
                }

                let piece_index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(Self::Have(piece_index))
            },
            5 => Ok(Self::Bitfield(payload)),
            6 | 8 => {
                if payload.len() != 12 {
                    return Err(Error::InvalidPayloadLength { expected: 12, actual: payload.len() });
                }

                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let length = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);

                if id == 6 {
                    Ok(Self::Request { index, begin, length })
                } else {
                    Ok(Self::Cancel { index, begin, length })
                }
            }
            7 => {
                if payload.len() < 8 {
                    return Err(Error::InvalidPayloadLength { expected: 8, actual: payload.len() });
                }

                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let block = payload[8..].to_vec();

                Ok(Self::Piece { index, begin, block })
            },
            20 => {
                // println!("extended message not supported");
                // Err(Error::InvalidMessageId(id))
                Ok(Self::Extended(payload))
            }
            _ => Err(Error::InvalidMessageId(id)),
        }
    }
}

pub struct Peer {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    is_choking: bool,
    is_interested: bool,
    am_choking: bool,
    am_interested: bool,
    bitfield: BitVec,
}

impl Peer {
    pub fn connect(addr: SocketAddr, num_pieces: usize) -> Result<Self, Error> {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(200))?;
        let reader = BufReader::new(stream.try_clone()?);
        
        Ok(Peer {
            stream,
            reader,
            is_choking: true,
            is_interested: false,
            am_interested: false,
            am_choking: true,
            bitfield: BitVec::from_elem(num_pieces, false),
        })
    }

    pub fn handshake(&mut self, info_hash: [u8; 20], peer_id: [u8; 20]) -> Result<[u8; 68], Error> {
        // prepare handshake

        let mut cursor = Cursor::new(vec![0u8; 68]);
        cursor.seek(io::SeekFrom::Start(0))?;

        write!(cursor, "{}BitTorrent protocol00000000", 19 as char)?;

        for byte in info_hash {
            cursor.write(&[byte])?;
        }

        for byte in peer_id {
            cursor.write(&[byte])?;
        }

        // send handshake

        self.stream.write_all(cursor.get_ref())?;

        // read response handshake
        // loops until it connects or gives an error

        loop {
            let mut handshake = [0u8; 68];

            match self.reader.read(&mut handshake) {
                Ok(received) if received == 68 => break Ok(handshake), 
                Ok(received) => break Err(Error::NotEnoughBytes { expected: 68, actual: received }),
                Err(err) if err.kind() == io::ErrorKind::TimedOut => continue,
                Err(err) => break Err(err.into()),
            }
        }
    }

    pub fn read_message(&mut self) -> Result<Message, Error> {
        // read length of message
        let mut len = [0u8; 4];
        self.reader.read_exact(&mut len)?;
    
        let len = u32::from_be_bytes(len);

        // If len is 0, it's a keep-alive message
        if len == 0 {
            return Ok(Message::KeepAlive);
        }

        // Read message id
        let mut id = [0u8; 1];
        self.reader.read_exact(&mut id)?;

        let id = id[0];

        if id > 9 && id != 20 {
            return Err(Error::InvalidMessageId(id));
        }

        // Calculate payload length and read payload if present
        let payload_len = len as usize - 1;

        if payload_len > 0 {
            let mut payload = vec![0; payload_len];
            self.reader.read_exact(&mut payload)?;

            // Construct and return the message
            Ok(Message::from_id_and_payload(id, payload)?)
        } else {
            // If there's no payload, return a message with just the ID
            Ok(Message::from_id(id))
        }
    }

    pub fn bitfield(&self) -> &BitVec {
        &self.bitfield
    }

    pub fn set_is_choking(&mut self, bool: bool) {
        self.is_choking = bool;
    }

    pub const fn am_choking(&self) -> bool {
        self.am_choking
    }

    pub const fn is_choking(&self) -> bool {
        self.is_choking
    }

    pub fn send_unchoke(&mut self) -> Result<(), Error> {
        self.stream.write_all(&[0, 0, 0, 1, 1])?;
        self.am_choking = false;

        Ok(())
    }

    pub fn send_interested(&mut self) -> Result<(), Error> {
        self.stream.write_all(&[0, 0, 0, 1, 2])?;
        self.am_interested = true;

        Ok(())
    }

    pub fn send_request(&mut self, index: u32, begin: u32, length: u32) -> Result<(), Error> {
        let mut cursor = Cursor::new(vec![0, 0, 0, 13, 6]);
        cursor.seek(io::SeekFrom::End(0)).unwrap();
        cursor.write(&index.to_be_bytes())?;
        cursor.write(&begin.to_be_bytes())?;
        cursor.write(&length.to_be_bytes())?;

        self.stream.write_all(cursor.get_ref())?;

        Ok(())
    }

    pub fn update_bitfield(&mut self, bitfield: Vec<u8>) {
        self.bitfield = BitVec::from_bytes(&bitfield);
    }

    pub fn update_piece(&mut self, piece_index: usize) {
        self.bitfield.set(piece_index, true);
    }
}