pub enum TorrentType {
    MagnetLink(String),
    InfoHash(String),
    Base32InfoHash(String),
    TorrentFile(String),
    TorrentFileUrl(String),
}

impl TryFrom<&str> for TorrentType {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if is_magnet_link(value) {
            Ok(Self::MagnetLink(value.to_string()))
        } else if is_torrent_file(value) {
            Ok(Self::TorrentFile(value.to_string()))
        } else if is_torrent_file_url(value) {
            Ok(Self::TorrentFileUrl(value.to_string()))
        } else if is_info_hash(value) {
            Ok(Self::InfoHash(value.to_string()))
        } else if is_base32_info_hash(value) {
            Ok(Self::Base32InfoHash(value.to_string()))
        }
        else {
            Err(())
        }
    }
}

fn is_magnet_link(value: &str) -> bool {
    value.starts_with("magnet:?xt=urn:btih:")
}

fn is_info_hash(value: &str) -> bool {
    // info hash are 20-byte SHA-1 hash represented as a hex string
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_torrent_file(value: &str) -> bool {
    let path = std::path::Path::new(value);
    path.is_file() && path.extension().map_or(false, |extension| extension == "torrent")
}

fn is_torrent_file_url(_value: &str) -> bool {
    todo!()
}

fn is_base32_info_hash(_value: &str) -> bool {
    // 32-byte base32 encoded representation of info hash
    todo!()
}