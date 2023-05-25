use clap::Parser;

#[derive(Debug, Parser)]
pub struct Args {
    #[arg()]
    pub torrent_file: String,
}