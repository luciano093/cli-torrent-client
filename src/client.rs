use crate::{metainfo, torrent};
use crate::torrent::Torrent;

#[derive(Debug)]
pub enum Error {
    MetaInfoError(metainfo::Error),
    TorrentError(torrent::Error),
    JoinError(tokio::task::JoinError),
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

impl From<tokio::task::JoinError> for Error {
    fn from(value: tokio::task::JoinError) -> Self {
        Self::JoinError(value)
    }
}

pub struct Client { }

impl Client {
    pub const fn new() -> Self {
        Client { }
    }

    /// `torrent_file` may be passed as a magnet link or path to file
    pub async fn download(&self, torrent: &str) -> Result<(), Error> {
        let torrent = torrent.to_string();

        tokio::spawn(async move {
            let mut torrent = Torrent::new(&torrent).await?;
            torrent.download().await;

            Ok(())
        }).await?
    }
}

impl Default for Client {
    fn default() -> Self {
        Client::new()
    }
}