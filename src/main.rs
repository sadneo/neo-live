use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::RwLock;
use tokio::task;

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

const CHANNEL_SIZE: usize = 5;

#[derive(Serialize, Deserialize, Debug)]
struct TextUpdate {
    text: String,
}

impl TextUpdate {
    fn encode(self) -> Option<Vec<u8>> {
        let mut buf = Vec::new();
        let payload = match rmp_serde::to_vec(&self) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("msgpack encode error: {e}");
                return None;
            }
        };

        buf.clear();
        let len = (payload.len() as u32).to_be_bytes();
        buf.extend_from_slice(&len);
        buf.extend_from_slice(&payload);

        Some(buf)
    }
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

struct FrameReader<T> {
    reader: BufReader<T>,
}

impl<R: tokio::io::AsyncRead + Unpin> FrameReader<R> {
    fn new(reader: R) -> FrameReader<R> {
        let reader = BufReader::new(reader);
        Self { reader }
    }
    async fn read_loop(mut self, sender: Sender<Vec<u8>>) {
        let mut len_buf = [0u8; 4];
        loop {
            if self.reader.read_exact(&mut len_buf).await.is_err() {
                eprintln!("Connection closed");
                break;
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            eprintln!("{}", len);

            let mut buf = vec![0u8; len];
            if self.reader.read_exact(&mut buf).await.is_err() {
                eprintln!("Connection closed");
                break;
            }
            if sender.send(buf).await.is_err() {
                eprintln!("Channel closed");
                break;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { host_mode } => serve(host_mode, cli.port).await,
        Command::Connect { address } => {
            connect(
                Ipv4Addr::from_str(&address).expect("Expected address"),
                cli.port,
            )
            .await
        }
    }
}

async fn serve(host_mode: HostMode, port: u16) {
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

    tokio::task::spawn(async move {
        let listener = TcpListener::bind(write_socket)
            .await
            .expect("Failed to create Listener");
        loop {
            match listener.accept().await {
                Ok((stream, address)) => {
                    let mut clients_guard = clients_ref.write().await;
                    clients_guard.push(stream);
                    eprintln!("Client added at {}", address);
                }
                Err(e) => {
                    eprintln!("listener: {:?}", e);
                    break;
                }
            }
        }
    });

    let stdin = io::stdin();
    let reader = FrameReader::new(stdin);
    let (tx, mut rx) = mpsc::channel(CHANNEL_SIZE);

    tokio::task::spawn(async move { reader.read_loop(tx) });

    loop {
        let Some(msg_bytes) = rx.recv().await else {
            eprintln!("Something went wrong");
            break;
        };
        let message: TextUpdate = rmp_serde::from_slice(&msg_bytes).unwrap();
        let framed_msg = message.encode().unwrap();

        // write to all streams, retain ones that don't error
        let mut clients_guard = clients.write().await;
        let mut i = 0;
        while i < clients_guard.len() {
            let stream = &mut clients_guard[i];
            match stream.write_all(&framed_msg).await {
                Ok(_) => i += 1,
                Err(_) => {
                    clients_guard.remove(i);
                }
            }
        }
    }
}

async fn connect(remote: Ipv4Addr, port: u16) {
    let read_socket = SocketAddrV4::new(remote, port);
    let stream = TcpStream::connect(read_socket).await.unwrap();
    let reader = FrameReader::new(stream);
    let mut stdout = io::stdout();
    let (tx, mut rx) = mpsc::channel(CHANNEL_SIZE);

    task::spawn(async move { reader.read_loop(tx) });

    loop {
        let Some(msg_bytes) = rx.recv().await else {
            eprintln!("Something went wrong");
            break;
        };
        let message: TextUpdate = rmp_serde::from_slice(&msg_bytes).unwrap();
        eprintln!("message: {:?}", message);

        let framed_msg = message.encode().unwrap();
        stdout.write_all(&framed_msg).await.expect("IO error");
    }
}
