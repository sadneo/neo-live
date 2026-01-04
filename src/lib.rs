use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use tokio::io::{self, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::RwLock;
use tokio::task;

use log::{debug, error, info, trace};
use serde::{Deserialize, Serialize};

const CHANNEL_SIZE: usize = 5;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct TextUpdate {
    text: String,
}

impl TextUpdate {
    pub fn new(text: String) -> Self {
        TextUpdate { text }
    }

    pub fn text(&self) -> &String {
        &self.text
    }

    pub fn encode(&self) -> Option<Vec<u8>> {
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

struct FrameReader<T> {
    reader: BufReader<T>,
}

impl<R: tokio::io::AsyncRead + Unpin> FrameReader<R> {
    fn new(reader: R) -> FrameReader<R> {
        let reader = BufReader::new(reader);
        Self { reader }
    }
    async fn read_loop(mut self, sender: Sender<Vec<u8>>) {
        trace!("FrameReader read loop started");
        let mut len_buf = [0u8; 4];
        loop {
            if self.reader.read_exact(&mut len_buf).await.is_err() {
                error!("Connection closed");
                break;
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            trace!("FrameReader unpacked {} bytes", len);

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

    // writes data to all clients, removing any that error out.
    async fn broadcast(&self, data: &[u8]) {
        let mut clients = self.clients.write().await;

        // retain only clients that successfully accept the write
        let mut i = 0;
        while i < clients.len() {
            match clients[i].write_all(data).await {
                Ok(_) => i += 1,
                Err(_) => {
                    // client disconnected or error, remove them
                    let client = &clients[i];
                    info!("Client at {} disconnected", client.local_addr().unwrap());
                    clients.swap_remove(i);
                }
            }
        }
    }
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

pub async fn serve<R>(addr: SocketAddrV4, input_source: R)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
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
        info!("received message: {}", message.text);

        let Some(framed_msg) = message.encode() else {
            error!("Failed to encode message");
            continue;
        };

        pool.broadcast(&framed_msg).await;
    }
    error!("done with err {:?}", rx.recv().await);
}

pub async fn connect(remote: Ipv4Addr, port: u16) {
    let read_socket = SocketAddrV4::new(remote, port);
    let stream = TcpStream::connect(read_socket).await.unwrap();
    let reader = FrameReader::new(stream);
    let mut stdout = io::stdout();
    let (tx, mut rx) = mpsc::channel(CHANNEL_SIZE);

    trace!("Spawned read loop");
    task::spawn(async move { reader.read_loop(tx).await });

    loop {
        let Some(msg_bytes) = rx.recv().await else {
            error!("Something went wrong");
            break;
        };
        let message: TextUpdate = rmp_serde::from_slice(&msg_bytes).unwrap();
        info!("message: {:?}", message);

        let framed_msg = message.encode().unwrap();
        stdout.write_all(&framed_msg).await.expect("IO error");
    }
}
