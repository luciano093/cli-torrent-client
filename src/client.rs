use std::net::TcpStream;
use std::sync::{Mutex, Arc};

use threadpool::ThreadPool;

use crate::{metainfo, torrent};
use crate::torrent::Torrent;

#[derive(Debug)]
pub enum Error {
    MetaInfoError(metainfo::Error),
    TorrentError(torrent::Error)
}

impl From<metainfo::Error> for Error {
    fn from(value: metainfo::Error) -> Self {
        Self::MetaInfoError(value)
    }
}

impl From<torrent::Error> for Error {
    fn from(value: torrent::Error) -> Self {
        Self::TorrentError(value)
    }
}

pub struct Client {
    thread_pool: ThreadPool
}

impl Client {
    pub fn new() -> Self {
        let thread_pool = ThreadPool::new(4);

        Client { 
            thread_pool
        }
    }

    /// `torrent_file` may be passed as a magnet link or path to file
    pub fn download(&self, torrent: &str) -> Result<(), Error> {
        let mut torrent = Torrent::new(torrent)?;

        let task = move || {
            torrent.download();
        };

        self.thread_pool.execute(task);
        self.thread_pool.join();

        Ok(())
    }
}