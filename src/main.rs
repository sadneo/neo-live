use clap::{Parser, Subcommand};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::path::PathBuf;
use std::str::FromStr;
use std::fs;
use std::sync::mpsc::{self, Receiver};
use std::thread;

mod watchers;

#[derive(Parser, Debug)]
#[command(version, author, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// IPv4 Address
    #[arg(short, long, default_value = "127.0.0.1")]
    address: String,

    /// Port to use
    #[arg(short, long, default_value = "3248")]
    write_port: u16,

    /// Port to use
    #[arg(short, long, default_value = "3249")]
    port: u16,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Serve file to a port
    Serve {
        /// file to serve
        path: PathBuf,
    },
    /// Connect to server at port
    Connect {
        /// file to write to
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let path = std::path::PathBuf::from("test.txt").canonicalize().unwrap();
    let (file_update_sender, file_update_receiver) = mpsc::channel::<String>();
    thread::spawn(move || {
        watchers::watch(&path, file_update_sender);
    });

    let addr = Ipv4Addr::from_str(&cli.address).unwrap();
    let socket = SocketAddrV4::new(addr, cli.port);
    let write_socket = SocketAddrV4::new(addr, cli.write_port);

    match cli.command {
        Command::Serve { path } => todo!("path: {:?}", path),
        Command::Connect { path } => connect(path, file_update_receiver, socket, write_socket),
    }
}

fn connect(output: PathBuf, file_updates: Receiver<String>, read_socket: SocketAddrV4, write_socket: SocketAddrV4) {
    thread::spawn(move || {
        let mut stream = TcpStream::connect(read_socket).unwrap();
        let mut buffer = String::new();
        loop {
            stream.read_to_string(&mut buffer).unwrap();
            fs::write(&output, &buffer).unwrap();
            buffer.clear();
        }
    });
    thread::spawn(move || {
        let mut stream = TcpStream::connect(write_socket).unwrap();
        for update in file_updates {
            stream.write_all(update.as_bytes()).unwrap();
        }
    });
}
