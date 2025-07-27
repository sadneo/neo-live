use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use std::thread;
use std::sync::{Arc, RwLock};
use std::io::{Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};

use tokio::net::TcpStream as TokioTcpStream;
use tokio::io::BufReader as TokioBufReader;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};

use clap::{Parser, Subcommand, ValueEnum};
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rmp_serde::{encode};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct TextUpdate {
    text: String,
}

#[derive(ValueEnum, Clone, Debug)]
enum HostMode {
    Local, // 127.0.0.1
    Lan,   // Local IP like 192.168.x.x
    All,   // 0.0.0.0
}

#[derive(Parser, Debug)]
#[command(version, author, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Port to use
    #[arg(short, long, default_value = "3248")]
    port: u16,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Serve file to a socket
    Serve {
        /// File to serve
        path: PathBuf,

        // currently defaults to HostMode::Local for debugging
        /// Interface to bind to
        #[arg(long, value_enum, default_value_t = HostMode::Local)]
        host_mode: HostMode,
    },
    /// Connect to server at socket
    Connect {
        /// Remote IPv4 address to connect to
        #[arg(short, long, default_value = "127.0.0.1")]
        address: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { path, host_mode } => serve(path, host_mode, cli.port).await,
        Command::Connect { address } => {
            connect(
                Ipv4Addr::from_str(&address).expect("Expected address"),
                cli.port,
            )
            .await
        }
    }
}

async fn serve(file_path: PathBuf, host_mode: HostMode, port: u16) {
    let address = match host_mode {
        HostMode::Local => Ipv4Addr::new(127, 0, 0, 1),
        HostMode::All => Ipv4Addr::new(0, 0, 0, 0),
        HostMode::Lan => local_ip_address::local_ip()
            .unwrap()
            .to_string()
            .parse()
            .unwrap(),
    };
    let write_socket = SocketAddrV4::new(address, port);

    let clients: Arc<RwLock<Vec<TcpStream>>> = Arc::new(RwLock::new(Vec::new()));
    let clients_ref = clients.clone();
    let file_path_clone = file_path.clone();

    thread::spawn(move || {
        let listener = TcpListener::bind(write_socket).expect("Failed to create listener");
        for stream in listener.incoming().flatten() {
            let mut clients_guard = clients.write().unwrap();
            clients_guard.push(stream);
            eprintln!("Client added");
        }
    });

    // TODO: instead of watcher, listen to stdin instead and send the the entire buffer through msgpack
    // instead of a notify Event
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher: RecommendedWatcher =
        Watcher::new(tx, Config::default()).expect("Failed to create watcher");

    watcher
        .watch(&file_path_clone, RecursiveMode::NonRecursive)
        .expect("Failed to watch file");

    let mut buf = Vec::new();
    loop {
        if let Ok(event) = rx.recv() {
            if let EventKind::Modify(_) = event.unwrap().kind {
                eprintln!("modify");
                let Ok(contents) = fs::read_to_string(&file_path) else {
                    continue;
                };
                let message = TextUpdate { text: contents };

                let payload = match encode::to_vec(&message) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("msgpack encode error: {e}");
                        continue;
                    }
                };

                buf.clear();
                let len = (payload.len() as u32).to_be_bytes();
                buf.extend_from_slice(&len);
                buf.extend_from_slice(&payload);

                let mut guard = clients_ref.write().unwrap();
                guard.retain_mut(|stream| stream.write_all(&buf).is_ok());
            }
        }
    }
}

async fn connect(remote: Ipv4Addr, port: u16) {
    let read_socket = SocketAddrV4::new(remote, port);
    let stream = TokioTcpStream::connect(read_socket).await.unwrap();
    let mut reader = TokioBufReader::new(stream);
    let mut stdout = io::stdout();

    let mut len_buf = [0u8; 4];
    loop {
        if reader.read_exact(&mut len_buf).await.is_err() {
            eprintln!("Connection closed");
            break;
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        eprintln!("{}", len);

        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).await.unwrap();
        let message = rmp_serde::from_slice::<TextUpdate>(&buf).unwrap();
        eprintln!("message: {:?}", message);
        
        stdout.write_all(&buf).await.expect("IO error");
    }
}
