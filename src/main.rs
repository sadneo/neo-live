use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use clap::{Parser, Subcommand};

mod watchers;

#[derive(Parser, Debug)]
#[command(version, author, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    file: PathBuf,

    /// IPv4 Address
    #[arg(short, long, default_value = "127.0.0.1")]
    address: String,

    /// Port to use
    #[arg(short, long, default_value = "3249")]
    port: u16,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Serve file to a port
    Serve,
    /// Become client to a served port
    Client,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    // let ip_addr = &cli.address;
    // let port = cli.port;

    let path = std::path::PathBuf::from("test.txt").canonicalize().unwrap();
    let (sx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        watchers::watch(&path, sx);
    });

    for event in rx {
        println!("{event}");
    }
}
