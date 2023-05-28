use clap::Parser;
use torrent_client::args::Args;
use torrent_client::client::Client;

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let client = Client::new();

    if let Err(err) = client.download(&args.torrent_file).await {
        eprintln!("Error: {:?}", err);
        std::process::exit(-1)
    }
}
