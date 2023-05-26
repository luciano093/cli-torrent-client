use std::{fs, fmt};
use std::path::PathBuf;
use std::io::Read;
use std::str::from_utf8;

use chrono::NaiveDateTime;
use sha1::{Sha1, Digest};

use crate::bencode::{self, FromBencode, Bedecode, Type, FromBencodeType};
use crate::input::TorrentType;

#[derive(Debug)]
pub enum Error {
    MissingInfo,
    MissingPieceLength,
    MissingPieces,
    MissingName,
    MissingMd5Sum,
    MalformedTimestamp,
    MissingLength,
    MissingPath,
    DecodingError(bencode::Error)
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedTimestamp => write!(f, "Error: timestamp has the wrong format"),
            _ => todo!(),
        }
    }
}

impl std::error::Error for Error { }

impl From<bencode::Error> for Error {
    fn from(value: bencode::Error) -> Self {
        Self::DecodingError(value)
    }
}

#[derive(Debug)]
pub struct CreationDate(NaiveDateTime);

/// Represents a file of a multi-file info dictionary
#[derive(Debug)]
pub struct File {
    length: u32,
    md5sum: Option<[u8; 16]>,
    path: PathBuf,
}

impl File {
    pub const fn lenght(&self) -> u32 {
        self.length
    }

    pub const fn md5sum(&self) -> Option<&[u8; 16]> {
        self.md5sum.as_ref()
    }

    pub const fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl FromBencodeType for File {
    type Error = Error;

    fn from_bencode_type(value: &Type) -> Result<Self, Self::Error> where Self: Sized {
        let dict = value.try_into_dict()?.0;

        let mut length = None;
        let mut md5sum = None;
        let mut path = None;

        let mut iter = dict.iter();

        while let Some((name, value)) = iter.next() {
            let name = name.try_into_byte_string().unwrap().0;

            match (name, value) {
                (b"length", Type::Integer(int, _)) => {
                    length = Some(int.parse().unwrap())
                }
                (b"md5sum", Type::String(bytes, _)) => {
                    let mut arr = [0u8; 16];

                    for i in 0..16 {
                        arr[i] = bytes[i];
                    }

                    md5sum = Some(arr);
                }
                (b"path", Type::List(list, _)) => {
                    let mut path_buf = PathBuf::new();

                    for elem in list {
                        let elem = from_utf8(elem.try_into_byte_string()?.0).unwrap();
                        path_buf.push(format!("/{}", elem));
                    }
                    
                    path = Some(path_buf)
                }
                _ => todo!(),
            }
        }

        let length = length.ok_or(Error::MissingLength)?;
        let path = path.ok_or(Error::MissingPath)?;

        Ok(File {
            length,
            md5sum,
            path
        })
    }
}

pub struct Info {
    piece_length: u32,
    pieces: Vec<[u8; 20]>,
    private: Option<bool>,
    name: String,
    mode: FileMode,
}

impl fmt::Debug for Info {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "piece_length: {}, pieces: Vec<[{}; 20]>, private: {:?}, name: {}, mode: {:?}", self.piece_length, self.pieces.len(), self.private, self.name, self.mode)
    }
}

impl Info {
    pub const fn piece_length(&self) -> u32 {
        self.piece_length
    }

    pub const fn pieces(&self) -> &Vec<[u8; 20]> {
        &self.pieces
    }

    pub const fn private(&self) -> &Option<bool> {
        &self.private
    }

    pub const fn name(&self) -> &String {
        &self.name
    }

    pub const fn mode(&self) -> &FileMode {
        &self.mode
    }
}

impl FromBencodeType for Info {
    type Error = Error;

    fn from_bencode_type(value: &Type) -> Result<Self, Self::Error> where Self: Sized {
        let mut info_dic = value.try_into_dict()?.0.iter();

        let mut piece_length = None;
        let mut pieces = None;
        let mut private = None;
        let mut name = None;
        let mut length = None;
        let mut md5sum = None;
        let mut files = None;

        while let Some((field_name, value)) = info_dic.next() {
            let field_name = field_name.try_into_byte_string()?.0;

            match (field_name, value) {
                (b"piece length", Type::Integer(int, _)) => {
                    piece_length = Some(int.parse().unwrap());
                }
                (b"pieces", Type::String(bytes, _)) => {
                    let mut vec = Vec::new();

                    for sha1 in bytes.chunks(20) {
                        let mut sha1_arr = [0u8; 20];

                        for i in 0..20 {
                            sha1_arr[i] = sha1[i];
                        }

                        vec.push(sha1_arr);
                    }

                    pieces = Some(vec);
                }
                (b"private", Type::Integer(int, _)) => {
                    let int: u32 = int.parse().unwrap();

                    private = if let 0 = int {
                        Some(false)
                    } else {
                        Some(true)
                    }
                }
                (b"name", Type::String(bytes, _)) => {
                    name = Some(from_utf8(bytes).unwrap().to_string());
                }
                (b"length", Type::Integer(int, _)) => {
                    length = Some(int.parse().unwrap());
                }
                (b"md5sum", Type::String(bytes, _)) => {
                    let mut arr = [0u8; 16];

                    for i in 0..16 {
                        arr[i] = bytes[i];
                    }

                    md5sum = Some(arr);
                }
                (b"files", Type::List(list, _)) => {
                    let mut vec = Vec::new();

                    for file in list {
                        vec.push(File::from_bencode_type(file)?);
                    }

                    files = Some(vec);
                }
                _ => todo!(),
            }
        }

        let piece_length = piece_length.ok_or(Error::MissingPieceLength)?;
        let pieces = pieces.ok_or(Error::MissingPieces)?;
        let name = name.ok_or(Error::MissingName)?;

        let mode = if files.is_some() {
            let files = files.unwrap();

            FileMode::MultipleFiles { files }
        } else {
            let length = length.ok_or(Error::MissingLength)?;

            FileMode::SingleFile { length, md5sum }
        };

        Ok(Info {
            piece_length,
            pieces,
            private,
            name,
            mode
        })
    }
}

#[derive(Debug)]
pub enum FileMode {
    MultipleFiles {
        files: Vec<File>,
    },
    SingleFile {
        length: u64,
        md5sum: Option<[u8; 16]>,
    },
}

pub struct MetaInfo {
    info_hash: [u8; 20],
    info: Info,
    announce: String,
    announce_list: Option<Vec<Vec<String>>>,
    creation_date: Option<CreationDate>,
    comment: Option<String>,
    created_by: Option<String>,
    encoding: Option<String>
}

impl fmt::Debug for MetaInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f,
            "info_hash: {:x?}, info: {:?}, announce: {}, announce_list: {:?}, creation_date: {:?}, comment: {:?}, created_by: {:?}, encoding: {:?}",
            self.info_hash, self.info, self.announce, self.announce_list, self.creation_date, self.comment, self.created_by, self.encoding
        )
    }
}

impl MetaInfo {
    pub const fn info_hash(&self) -> &[u8; 20] {
        &self.info_hash
    }

    pub const fn info(&self) -> &Info {
        &self.info
    }

    pub const fn announce(&self) -> &String {
        &self.announce
    }

    pub const fn announce_list(&self) -> Option<&Vec<Vec<String>>> {
        self.announce_list.as_ref()
    }

    pub const fn creation_date(&self) -> Option<&CreationDate> {
        self.creation_date.as_ref()
    }

    pub const fn comment(&self) -> Option<&String> {
        self.comment.as_ref()
    }

    pub const fn created_by(&self) -> Option<&String> {
        self.created_by.as_ref()
    }

    pub const fn encoding(&self) -> Option<&String> {
        self.encoding.as_ref()
    }

    fn from_file(path: &str) -> Result<MetaInfo, Error> {
        // path validity has already been checked
        let file = fs::File::open(path).unwrap();
        let bytes = file.bytes().map(|byte| byte.unwrap()).collect::<Vec<u8>>();

        let metainfo = MetaInfo::from_bencode(&bytes)?;

        Ok(metainfo)
    }
}

impl TryFrom<&str> for MetaInfo {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Error> {
        if let Ok(torrent) = TorrentType::try_from(input) {
            match torrent {
                TorrentType::MagnetLink(_magnet) => todo!(),
                TorrentType::InfoHash(_info_hash) => todo!(),
                TorrentType::TorrentFile(file) => Ok(MetaInfo::from_file(&file)?),
                TorrentType::Base32InfoHash(_b32_hash) => todo!(),
                TorrentType::TorrentFileUrl(_url) => todo!(),
            }
        } else {
            todo!()
        }
    }
}

impl bencode::FromBencode for MetaInfo {
    type Error = Error;

    fn from_bencode(bytes: &[u8]) -> Result<Self, Self::Error> where Self: Sized {
        let map = bytes.try_into_dict()?.0;

        let mut info_hash = None;
        let mut info = None;
        let mut announce = None;
        let mut announce_list = None;
        let mut creation_date = None;
        let mut comment = None;
        let mut created_by = None;
        let mut encoding = None;

        let mut iter = map.iter();

        while let Some((name, value)) = iter.next() {
            let name = name.try_into_byte_string()?.0;

            match (name, value) {
                (b"info", value) => {
                    let info_dict = value.try_into_dict()?;

                    let mut hasher = Sha1::new();
                    hasher.update(info_dict.1);

                    let sha1: [u8; 20] = hasher.finalize().into();
                    info_hash = Some(sha1);

                    info = Some(Info::from_bencode_type(value)?);
                }
                (b"announce", Type::String(bytes, _)) => {
                    announce = Some(from_utf8(bytes).unwrap().to_string());
                }
                (b"announce-list", Type::List(list2d, _)) => {
                    let mut vec2d = Vec::new();

                    for list in list2d {
                        let mut vec = Vec::new();

                        let list = list.try_into_list()?.0;

                        for str in list {
                            let str = str.try_into_byte_string()?.0;
                            vec.push(from_utf8(str).unwrap().to_string());
                        }

                        vec2d.push(vec);
                    }

                    announce_list = Some(vec2d);
                }
                (b"creation date", Type::Integer(int, _)) => {
                    let secs = int.parse().unwrap();

                    let time = CreationDate(NaiveDateTime::from_timestamp_opt(secs, 0).unwrap());
                    
                    creation_date = Some(time);
                }
                (b"comment", Type::String(bytes, _)) => {
                    comment = Some(from_utf8(bytes).unwrap().to_string());
                }
                (b"created by", Type::String(bytes, _)) => {
                    created_by = Some(from_utf8(bytes).unwrap().to_string());
                }
                (b"encoding", Type::String(bytes, _)) => {
                    encoding = Some(from_utf8(bytes).unwrap().to_string());
                }
                _ => (),
            }
        }

        let info = info.ok_or(Error::MissingInfo)?;
        let announce = announce.ok_or(Error::MissingInfo)?;
        let info_hash = info_hash.unwrap(); // should be fine as long as info is cheked before

        Ok(MetaInfo {
            info_hash,
            info,
            announce,
            announce_list, 
            creation_date,
            comment,
            created_by,
            encoding 
        })
    }
}