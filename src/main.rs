use clap::Parser;
use torrent_client2::args::Args;
use torrent_client2::client::Client;

fn main() {
    let args = Args::parse();

    let client = Client::new();

    match client.download(&args.torrent_file) {
        Ok(_) => (),
        Err(err) => {
            eprintln!("Error: {:?}", err);
            std::process::exit(-1);
        }
    }
}
