use clap::{Parser, Subcommand};
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver};
use std::thread;

mod watchers;

#[derive(Parser, Debug)]
#[command(version, author, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Port to use
    #[arg(short, long, default_value = "3248")]
    server_port: u16,

    /// Port to use
    #[arg(short, long, default_value = "3249")]
    client_port: u16,
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

        /// Remote IPv4 address to connect to
        #[arg(short, long, default_value = "127.0.0.1")]
        address: String,
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

    match cli.command {
        Command::Serve { path } => todo!("{:?}", path),
        Command::Connect { path, address } => connect(path, file_update_receiver, Ipv4Addr::from_str(&address).unwrap(), cli.server_port, cli.client_port),
    }
}

fn connect(output: PathBuf, file_updates: Receiver<String>, remote: Ipv4Addr, server_port: u16, client_port: u16) {
    let read_socket = SocketAddrV4::new(remote, client_port);
    let write_socket = SocketAddrV4::new(remote, server_port);

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
