use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::RwLock;
use tokio::task;

use clap::{Parser, Subcommand, ValueEnum};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};

const CHANNEL_SIZE: usize = 5;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct TextUpdate {
    text: String,
}

impl TextUpdate {
    fn encode(&self) -> Option<Vec<u8>> {
        let mut buf = Vec::new();
        let payload = match rmp_serde::to_vec(&self) {
            Ok(p) => p,
            Err(e) => {
                error!("msgpack encode error: {e}");
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
                error!("Connection closed");
                break;
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            info!("FrameReader unpacked {} bytes", len);

            let mut buf = vec![0u8; len];
            if self.reader.read_exact(&mut buf).await.is_err() {
                error!("Connection closed");
                break;
            }
            if sender.send(buf).await.is_err() {
                error!("Channel closed");
                break;
            }
        }
        error!("Read loop broke")
    }
}

#[derive(Clone)]
struct ClientPool {
    clients: Arc<RwLock<Vec<TcpStream>>>,
}

impl ClientPool {
    fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(Vec::new())),
        }
    }

    async fn add(&self, stream: TcpStream, addr: SocketAddr) {
        info!("Client added at {}", addr);
        self.clients.write().await.push(stream);
    }

    /// Writes data to all clients, removing any that error out.
    async fn broadcast(&self, data: &[u8]) {
        let mut clients = self.clients.write().await;

        // Retain only clients that successfully accept the write
        let mut i = 0;
        while i < clients.len() {
            match clients[i].write_all(data).await {
                Ok(_) => i += 1,
                Err(_) => {
                    // Client disconnected or error, remove them
                    let client = &clients[i];
                    info!("Client at {} disconnected", client.local_addr().unwrap());
                    clients.swap_remove(i);
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    env_logger::init();

    match cli.command {
        Command::Serve { host_mode } => serve(host_mode, cli.port, io::stdin()).await,
        Command::Connect { address } => {
            connect(
                Ipv4Addr::from_str(&address).expect("Expected address"),
                cli.port,
            )
            .await
        }
    }
}

fn resolve_address(host_mode: HostMode, port: u16) -> SocketAddrV4 {
    let address = match host_mode {
        HostMode::Local => Ipv4Addr::new(127, 0, 0, 1),
        HostMode::All => Ipv4Addr::new(0, 0, 0, 0),
        HostMode::Lan => local_ip_address::local_ip()
            .expect("Failed to get local IP")
            .to_string()
            .parse()
            .expect("Failed to parse local IP"),
    };
    SocketAddrV4::new(address, port)
}

async fn run_listener(addr: SocketAddrV4, pool: ClientPool) {
    let listener = TcpListener::bind(addr)
        .await
        .expect("Failed to bind listener");

    loop {
        match listener.accept().await {
            Ok((stream, address)) => pool.add(stream, address).await,
            Err(e) => {
                error!("Listener error: {:?}", e);
                break;
            }
        }
    }
}

async fn serve<R>(host_mode: HostMode, port: u16, input_source: R)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let addr = resolve_address(host_mode, port);
    let pool = ClientPool::new();

    // listen for connections
    let pool_ref = pool.clone();
    tokio::task::spawn(async move {
        debug!("starting listener with address {}", addr);
        run_listener(addr, pool_ref).await;
    });
    debug!("created listeners");

    // separate input reading and broadcast loop
    let (tx, mut rx) = mpsc::channel(CHANNEL_SIZE);
    tokio::task::spawn(async move {
        let reader = FrameReader::new(input_source);
        reader.read_loop(tx).await
    });

    while let Some(msg_bytes) = rx.recv().await {
        let Ok(message) = rmp_serde::from_slice::<TextUpdate>(&msg_bytes) else {
            error!("Failed to deserialize message");
            continue;
        };

        let Some(framed_msg) = message.encode() else {
            error!("Failed to encode message");
            continue;
        };

        pool.broadcast(&framed_msg).await;
    }
    error!("done with err {:?}", rx.recv().await);
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
            error!("Something went wrong");
            break;
        };
        let message: TextUpdate = rmp_serde::from_slice(&msg_bytes).unwrap();
        error!("message: {:?}", message);

        let framed_msg = message.encode().unwrap();
        stdout.write_all(&framed_msg).await.expect("IO error");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_logger() {
        let _ = env_logger::builder().parse_default_env().try_init();
    }

    #[tokio::test]
    async fn serve_basic() {
        init_logger();
        let (mut client, server) = tokio::io::duplex(64);
        tokio::task::spawn(serve(HostMode::Local, 12345, server));

        // we use the wait statements for the network io
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut conn = TcpStream::connect("127.0.0.1:12345").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let orig_decoded = TextUpdate {
            text: "testing! robux robux robux".to_owned(),
        };
        let orig_encoded = orig_decoded.encode().unwrap();
        client.write_all(&orig_encoded).await.unwrap();

        let mut buf = [0; 4096];
        let n = conn.read(&mut buf).await.unwrap();
        info!("read {} bytes", n);
        let decoded = rmp_serde::from_slice(&buf[4..n]).unwrap();

        assert_eq!(orig_decoded, decoded);
    }

    #[tokio::test]
    async fn serve_broadcast() {
        init_logger();
        let (mut client, server) = tokio::io::duplex(64);
        tokio::task::spawn(serve(HostMode::Local, 12347, server));

        // connect a bunch of streams to test broadcast
        const STREAM_COUNT: usize = 50;
        let mut connections = vec![];

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        for _ in 0..STREAM_COUNT {
            let conn = TcpStream::connect("127.0.0.1:12347").await.unwrap();
            connections.push(conn);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // send the data
        let orig_decoded = TextUpdate {
            text: "testing! robux robux robux".to_owned(),
        };
        let orig_encoded = orig_decoded.encode().unwrap();
        client.write_all(&orig_encoded).await.unwrap();

        // read from all the streams
        let mut buf = [0; 4096];
        for i in 0..STREAM_COUNT {
            let conn = &mut connections[i];
            let n = conn.read(&mut buf).await.unwrap();
            let decoded = rmp_serde::from_slice(&buf[4..n]).unwrap();
            assert_eq!(orig_decoded, decoded);
        }
    }

    #[tokio::test]
    async fn serve_drop() {
        init_logger();
        let (mut client, server) = tokio::io::duplex(64);
        tokio::task::spawn(serve(HostMode::Local, 12347, server));

        // connections
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let conn1 = TcpStream::connect("127.0.0.1:12347").await.unwrap();
        let mut conn2 = TcpStream::connect("127.0.0.1:12347").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        conn1
            .set_linger(Some(std::time::Duration::from_secs(0)))
            .unwrap();
        drop(conn1);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let msg_a = TextUpdate {
            text: "bait".to_owned(),
        };
        client.write_all(&msg_a.encode().unwrap()).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let msg_b = TextUpdate {
            text: "survivor".to_owned(),
        };
        client.write_all(&msg_b.encode().unwrap()).await.unwrap();

        // check
        let mut buf = vec![0; 4096];

        let n = conn2.read(&mut buf).await.unwrap();
        let decoded_a: TextUpdate = rmp_serde::from_slice(&buf[4..n]).unwrap();
        assert_eq!(decoded_a.text, "bait");

        let n = conn2.read(&mut buf).await.unwrap();
        let decoded_b: TextUpdate = rmp_serde::from_slice(&buf[4..n]).unwrap();
        assert_eq!(decoded_b.text, "survivor");
    }

    #[tokio::test]
    async fn serve_fragment() {
        init_logger();
        let (mut client, server) = tokio::io::duplex(64);
        tokio::task::spawn(serve(HostMode::Local, 12348, server));

        // we use the wait statements for the network io
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut conn = TcpStream::connect("127.0.0.1:12348").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let orig_decoded = TextUpdate {
            text: "testing! robux robux robux".to_owned(),
        };
        let orig_encoded = orig_decoded.encode().unwrap();
        info!("sent {} bytes", orig_encoded.len());
        client.write_all(&orig_encoded[0..8]).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        client.write_all(&orig_encoded[8..]).await.unwrap();

        let mut buf = [0; 4096];
        let n = conn.read(&mut buf).await.unwrap();
        info!("read {} bytes", n);
        let decoded = rmp_serde::from_slice(&buf[4..n]).unwrap();

        assert_eq!(orig_decoded, decoded);
    }

    #[tokio::test]
    async fn serve_garbage() {
        init_logger();
        let (mut client, server) = tokio::io::duplex(64);
        tokio::task::spawn(serve(HostMode::Local, 12341, server));

        // we use the wait statements for the network io
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut conn = TcpStream::connect("127.0.0.1:12341").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // don't be alarmed if you see an error here about deserialization, that's what we want to see
        {
            let mut write_buf = vec![];
            let payload = b"this is not valid msgpack";
            let len = (payload.len() as u32).to_be_bytes();
            write_buf.extend_from_slice(&len);
            write_buf.extend_from_slice(payload);
            client.write_all(&write_buf).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        let orig_decoded = TextUpdate {
            text: "testing! robux robux robux".to_owned(),
        };
        let orig_encoded = orig_decoded.encode().unwrap();
        info!("sent {} bytes", orig_encoded.len());
        client.write_all(&orig_encoded).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut buf = [0; 4096];
        let n = conn.read(&mut buf).await.unwrap();
        info!("read {} bytes", n);
        let decoded: TextUpdate = rmp_serde::from_slice(&buf[4..n]).unwrap();
        assert_eq!(orig_decoded, decoded);
    }
}
