use clap::{Parser, Subcommand};
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::thread;

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

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

    match cli.command {
        Command::Serve { path } => serve(path, cli.server_port),
        Command::Connect { path, address } => connect(
            path,
            Ipv4Addr::from_str(&address).expect("Expected address"),
            cli.server_port,
        ),
    }
}

fn serve(file_path: PathBuf, server_port: u16) {
    let address = "127.0.0.1".parse().unwrap();
    let write_socket = SocketAddrV4::new(address, server_port);
    let listener = TcpListener::bind(write_socket).expect("error when creating listener");

    let clients: Arc<RwLock<Vec<TcpStream>>> = Arc::new(RwLock::new(Vec::new()));
    let clients_ref = clients.clone();
    let file_path_clone = file_path.clone();

    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut clients_guard = clients.write().unwrap();
            println!("{:?}", stream);
            clients_guard.push(stream);
        }
    });

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher: RecommendedWatcher =
    Watcher::new(tx, Config::default()).expect("Failed to create watcher");

    watcher
        .watch(&file_path_clone, RecursiveMode::NonRecursive)
        .expect("Failed to watch file");

    loop {
        if let Ok(event) = rx.recv() {
            if let EventKind::Modify(_) = event.unwrap().kind {
                println!("modify");
                let Ok(contents) = fs::read(&file_path) else {
                    continue;
                };

                let mut guard = clients_ref.write().unwrap();
                guard.retain_mut(|stream| {
                    stream.write_all(&contents).is_ok()
                });
            }
        }
    }
}

fn connect(output: PathBuf, remote: Ipv4Addr, server_port: u16) {
    let read_socket = SocketAddrV4::new(remote, server_port);

    let mut stream = TcpStream::connect(read_socket).unwrap();
    let mut buffer = [0; 500];
    loop {
        let n = stream.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        let contents = String::from_utf8_lossy(&buffer[..n]);
        println!("recv: {:?}", contents);
        fs::write(&output, &buffer).unwrap();
    }
}
