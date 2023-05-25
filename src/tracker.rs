use std::net::{TcpStream, Ipv4Addr, Ipv6Addr, SocketAddr, IpAddr};
use std::io::{self, Write, Cursor, BufReader, Read};
use std::str::from_utf8;
use std::time::Duration;

use url::Url;

use crate::bencode::{FromBencode, self, Bedecode, Type, FromBencodeType};


#[derive(Debug)]
pub enum Error {
    IoError(io::Error),
    ParseError(url::ParseError),
    DecodingError(bencode::Error),
    MissingInterval,
    MissingComplete,
    MissingIncomplete,
    MissingPeers,
    MissingPeerId,
    MissingPeerIp,
    MissingPeerPort,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(err) => write!(f, "{}", err),
            Self::ParseError(err) => write!(f, "{}", err),
            Self::DecodingError(_err) => todo!(),
            _ => todo!(),
        }
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::IoError(value)
    }
}

impl From<url::ParseError> for Error {
    fn from(value: url::ParseError) -> Self {
        Self::ParseError(value)
    }
}

impl From<bencode::Error> for Error {
    fn from(value: bencode::Error) -> Self {
        Self::DecodingError(value)
    }
}

impl std::error::Error for Error { }


pub enum IpType {
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
}

pub enum Event {
    Started,
    Stopped,
    Completed,
}

pub struct TrackerRequest {
    info_hash: [u8; 20],
    peer_id: [u8; 20],
    port: u16,
    uploaded: u128,
    downloaded: u128,
    left: u128,
    compact: bool, // to implement
    no_peer_id: bool, // ignored if compact is enabled
    event: Option<Event>,
    ip: Option<SocketAddr>, // only needed if client sends requests from another ip
    numwant: Option<u16>, // number of peers client wants to recieve, default is 50
    key: Option<u32>, // random number used to identify multiple instances of a client
    trackerid: Option<String>, // only needed if a previous announce contained one
}

impl TrackerRequest {
    pub fn new(info_hash: [u8; 20], peer_id: [u8; 20], port: u16, uploaded: u128, downloaded: u128, left: u128, compact: bool, no_peer_id: bool) -> Self {
        Self {
            info_hash,
            peer_id,
            port,
            uploaded,
            downloaded,
            left,
            compact,
            no_peer_id,
            event: None,
            ip: None,
            numwant: None,
            key: None,
            trackerid: None
        }
    }
}

impl TrackerRequest {
    pub fn create_request(&self, path: &str, host: &str) -> Vec<u8> {   
       let info_hash: String = url::form_urlencoded::byte_serialize(&self.info_hash).collect();
       let peer_id: String = url::form_urlencoded::byte_serialize(&self.peer_id).collect();

       let mut request = Vec::new();
       let mut cursor = Cursor::new(&mut request);

       write!(
            cursor,
            "GET {}?info_hash={}&peer_id={}&port={}&uploaded={}&downloaded={}&left={}&compact={}",
            path, info_hash, peer_id, self.port, self.uploaded, self.downloaded, self.left, self.compact as u8
        ).unwrap();

        if self.no_peer_id {
            write!(cursor, "&no_peer_id=1").unwrap()
        }

        if let Some(event) = &self.event {
            let event = match event {
                Event::Started => "started",
                Event::Stopped => "stopped",
                Event::Completed => "completed",
            };

            write!(cursor, "&event={}", event).unwrap();
        }

        if let Some(ip) = &self.ip {
            write!(cursor, "&ip={}", ip).unwrap()
        }

        if let Some(numwant) = &self.numwant {
            write!(cursor, "&numwant={}", numwant).unwrap()
        }

        if let Some(key) = &self.key {
            write!(cursor, "&key={}", key).unwrap()
        }

        if let Some(trackerid) = &self.trackerid {
            write!(cursor, "&trackerid={}", trackerid).unwrap()
        }

        write!(cursor, " HTTP/1.1\r\nHost: {}\r\nAccept: */*\r\n\r\n", host).unwrap();

       request
    }
}

#[derive(Debug)]
pub enum Peers {
    Binary(Vec<SocketAddr>),
    Dictionary(Vec<(SocketAddr, String)>),
}

impl FromBencodeType for Peers {
    type Error = Error;
    fn from_bencode_type(value: &Type) -> Result<Self, Self::Error> where Self: Sized {

        // parse binary model
        match value.try_into_byte_string() {
            Ok((bytes, _)) => {
                let mut vec = Vec::new();

                for addr_bytes in bytes.chunks(6) {
                    let ip = Ipv4Addr::new(addr_bytes[0], addr_bytes[1], addr_bytes[2], addr_bytes[3]);
                    let port = u16::from_be_bytes([addr_bytes[4], addr_bytes[5]]);
                    let addr = SocketAddr::new(IpAddr::V4(ip), port);
                    vec.push(addr);
                }

                return Ok(Self::Binary(vec));
            }
            Err(_) => (),
        }

        // parse dictionary model

        let mut vec = Vec::new();

        for dict in value.try_into_list()?.0.iter() {
            let mut peer_id = None;
            let mut ip = None;
            let mut port = None;

            let mut iter = dict.try_into_dict()?.0.iter();

            while let Some((name, value)) = iter.next() {
                let name = name.try_into_byte_string().unwrap().0;

                match (name, value) {
                    (b"peer id", Type::String(bytes, _)) => {
                        peer_id = Some(String::from_utf8(bytes.to_vec()).unwrap());
                    }
                    (b"ip", Type::String(bytes, _)) => {
                        let string = String::from_utf8(bytes.to_vec()).unwrap();
                        ip = Some(string.parse().unwrap());
                    }
                    (b"port", Type::Integer(int, _)) => {
                        port = Some(int.parse().unwrap());
                    }
                    _ => (),
                }
            }

            let peer_id = peer_id.ok_or(Error::MissingPeerId)?;
            let ip = ip.ok_or(Error::MissingPeerIp)?;
            let port = port.ok_or(Error::MissingPeerPort)?;

            let addr = SocketAddr::new(ip, port);

            vec.push((addr, peer_id));
        }

        Ok(Self::Dictionary(vec))
    }
}

#[derive(Debug)]
pub struct TrackerResponse {
    warning_message: Option<String>,
    interval: u32,
    min_interval: Option<u32>,
    tracker_id: Option<String>,
    complete: Option<u32>,
    incomplete: Option<u32>,
    peers: Peers,
}

impl TrackerResponse {
    pub fn warning_message(&self) -> &str {
        self.warning_message.as_ref().unwrap()
    }

    pub fn interval(&self) -> u32 {
        self.interval
    }

    pub const fn min_interval(&self) -> Option<u32> {
        self.min_interval
    }

    pub const fn tracker_id(&self) -> Option<&String> {
        self.tracker_id.as_ref()
    }

    pub fn complete(&self) -> Option<u32> {
        self.complete
    }

    pub fn incomplete(&self) -> Option<u32> {
        self.incomplete
    }

    pub fn peers(&self) -> &Peers {
        &self.peers
    }
}

impl FromBencode for TrackerResponse {
    type Error = Error;

    fn from_bencode(bytes: &[u8]) -> Result<Self, Self::Error> where Self: Sized {
        let mut begin = 0;

        // find where dictionary begins
        for (i, pair) in bytes.windows(2).enumerate() {
            if pair[0] == '\n' as u8 && pair[1] == 'd' as u8 {
                begin = i + 1;
            }
        }

        // decode dictionary
        let map = bytes[begin..].try_into_dict()?.0;

        let mut warning_message = None;
        let mut interval = None;
        let mut min_interval = None;
        let mut tracker_id = None;
        let mut complete = None;
        let mut incomplete = None;
        let mut peers = None;

        let mut iter = map.iter();

        while let Some((name, value)) = iter.next() {
            let name = name.try_into_byte_string()?.0;

            match (name, value) {
                (b"warning message", Type::String(bytes, _)) => {
                    warning_message = Some(from_utf8(bytes).unwrap().to_string());
                }
                (b"interval", Type::Integer(int, _)) => {
                    interval = Some(int.parse().unwrap());
                }
                (b"min interval", Type::Integer(int, _)) => {
                    min_interval = Some(int.parse().unwrap());
                }
                (b"tracker id", Type::String(bytes, _)) => {
                    tracker_id = Some(from_utf8(bytes).unwrap().to_string());
                }
                (b"complete", Type::Integer(int, _)) => {
                    complete = Some(int.parse().unwrap());
                }
                (b"incomplete", Type::Integer(int, _)) => {
                    incomplete = Some(int.parse().unwrap());
                }
                (b"peers", value) => {
                    peers = Some(Peers::from_bencode_type(value)?);
                }
                _ => (),
            }
        }

        let interval = interval.ok_or(Error::MissingInterval)?;
        let peers = peers.ok_or(Error::MissingPeers)?;

        if true {
            Ok(TrackerResponse { warning_message, interval, min_interval, tracker_id, complete, incomplete, peers })
        } else {
            todo!()
        }
    }
}

pub struct Tracker {
    stream: TcpStream,
    response: Option<TrackerResponse>,
    request: Vec<u8>,
}

impl Tracker {
    pub fn connect(url: &str, request: &TrackerRequest) -> Result<Tracker, Error> {
        // connects to tracker
        let url = Url::parse(url)?;

        let tracker_address = url.socket_addrs(|| None)?[0];

        let stream = TcpStream::connect(tracker_address)?;

        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();

        // creates request
        let host = &format!("{}:{}", url.host_str().unwrap(), url.port_or_known_default().unwrap());
        println!("host: {}", host);
        let request = request.create_request(url.path(), host);

        Ok(Tracker { stream, response: None, request })
    }

    pub fn announce(&mut self) -> Result<(), Error> {
        loop {
            // writes request
            match self.stream.write_all(&self.request) {
                Err(err) if err.kind() == io::ErrorKind::ConnectionReset || err.kind() == io::ErrorKind::ConnectionAborted || err.kind() == io::ErrorKind::PermissionDenied => {
                    println!("reconnecting");
                    self.stream = TcpStream::connect(self.stream.peer_addr().unwrap())?;
                    continue;
                }
                Err(err) => return Err(err.into()),
                _ => (),
            };

            // reads response
            let result = BufReader::new(&self.stream).bytes().collect::<Result<Vec<u8>, std::io::Error>>();

            match result {
                Ok(response) => if response.len() != 0 {
                    self.response = match TrackerResponse::from_bencode(&response) {
                        Ok(response) => Some(response),
                        Err(err) => {
                            println!("error: {:?}", err);
                            todo!()
                        },
                    };

                    break;
                },
                Err(err) if err.kind() == io::ErrorKind::ConnectionReset => {
                    println!("reconnecting");
                    self.stream = TcpStream::connect(self.stream.peer_addr().unwrap())?;
                    continue;
                }
                Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                    continue;
                }
                Err(err) => {
                    eprintln!("{:?}", err);
                    continue;
                }
            }
        }

        Ok(())
    }

    pub const fn stream(&self) -> &TcpStream {
        &self.stream
    }

    pub const fn response(&self) -> Option<&TrackerResponse> {
        self.response.as_ref()
    }
}

/// gives totally random peer id following no convention 
pub fn random_peer_id() -> [u8; 20] {
    rand::random()
}