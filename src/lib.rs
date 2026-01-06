use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;

use tokio::io::{self, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::RwLock;
use tokio::task;

use log::{debug, error, info, trace};
use serde::{Deserialize, Serialize};

const CHANNEL_SIZE: usize = 5;

#[derive(Debug)]
struct IncomingMessage {
    from: SocketAddrV4,
    content: Vec<u8>,
}

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
        let payload = match rmp_serde::to_vec_named(&self) {
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

pub struct FrameReader<T> {
    reader: BufReader<T>,
}

impl<R: tokio::io::AsyncRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> FrameReader<R> {
        let reader = BufReader::new(reader);
        Self { reader }
    }
    pub async fn read_loop(mut self, sender: Sender<Vec<u8>>) {
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
    pub async fn next_frame(&mut self) -> Option<Vec<u8>> {
        let mut len_buf = [0u8; 4];
        trace!("stream waiting on reading data...");
        if self.reader.read_exact(&mut len_buf).await.is_err() {
            error!("Connection closed");
            return None;
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        trace!("FrameReader unpacked {} bytes", len);

        let mut buf = vec![0u8; len];
        if self.reader.read_exact(&mut buf).await.is_err() {
            error!("Connection closed");
            return None;
        }

        Some(buf)
    }
}

#[derive(Clone)]
struct ClientPool {
    clients: Arc<RwLock<Vec<OwnedWriteHalf>>>,
}

impl ClientPool {
    fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(Vec::new())),
        }
    }

    async fn add(&self, stream: OwnedWriteHalf, addr: &SocketAddrV4) {
        info!("Client added at {}", addr);
        self.clients.write().await.push(stream);
    }

    // writes data to all clients, removing any that error out.
    async fn broadcast(&self, data: &[u8], ignore: &SocketAddrV4) {
        let mut clients = self.clients.write().await;

        // retain only clients that successfully accept the write
        let mut i = 0;
        trace!("Broadcasting update");
        while i < clients.len() {
            let client = &clients[i];
            let addr_string = client
                .peer_addr()
                .map(|s| format!("{}", s))
                .unwrap_or("<bad peer_addr>".to_owned());

            trace!("Handling client {}", addr_string);
            if let Ok(SocketAddr::V4(addr)) = client.peer_addr() {
                trace!("Judging whether to skip addr {}, ignore {}", addr, ignore);
                if &addr == ignore {
                    trace!("Skipping {}", addr);
                    i += 1;
                    continue;
                }
            }

            match clients[i].write_all(data).await {
                Ok(_) => {
                    trace!("Broadcasted to client with peer_addr {}", addr_string,);
                    i += 1;
                }
                Err(_) => {
                    // client disconnected or error, remove them
                    info!("Client at {} disconnected", addr_string);
                    clients.swap_remove(i);
                }
            }
            // clients[i].flush().await.unwrap();
        }
        trace!("Broadcast done");

        for client in &mut *clients {
            client.flush().await.unwrap();
        }
    }
}

async fn run_listener(addr: SocketAddrV4, pool: ClientPool, tx: Sender<IncomingMessage>) {
    let listener = TcpListener::bind(addr)
        .await
        .expect("Failed to bind listener");

    loop {
        let tx_ref = tx.clone();
        match listener.accept().await {
            Ok((stream, SocketAddr::V4(address))) => {
                info!("{} connected to server", address);
                let (read_half, write_half) = stream.into_split();

                // add the stream to the pool for broadcasting
                pool.add(write_half, &address).await;

                // when the stream sends messages, add "from" address so when it gets broadcasted
                // it doesn't get sent back to the same guy
                let mut reader = FrameReader::new(read_half);
                tokio::spawn(async move {
                    while let Some(msg) = reader.next_frame().await {
                        trace!("Transmitting message");
                        let msg = IncomingMessage {
                            from: address,
                            content: msg,
                        };
                        tx_ref.send(msg).await.unwrap();
                    }
                });
            }
            Ok((_, SocketAddr::V6(addr))) => {
                error!("i don't wanna think about ipv6 yet {}", addr)
            }
            Err(e) => {
                error!("Listener error: {:?}", e);
                break;
            }
        }
    }
}

// server acts as a relay to send buffer contents
// later will relay CRDT operations instead
// later check whether the messages are valid so it doesn't relay junk
//
// uses TcpListener to add streams to ClientPool
// both reads and writes to streams
pub async fn serve(addr: SocketAddrV4) {
    let pool = ClientPool::new();

    // listen for connections
    // also set up input reading from clients here
    let pool_ref = pool.clone();
    let (tx, mut rx) = mpsc::channel(CHANNEL_SIZE);

    tokio::task::spawn(async move {
        debug!("starting listener with address {}", addr);
        run_listener(addr, pool_ref, tx).await;
    });
    trace!("created listeners");

    while let Some(msg) = rx.recv().await {
        let Ok(message) = rmp_serde::from_slice::<TextUpdate>(&msg.content) else {
            error!("Failed to deserialize message");
            continue;
        };
        info!("received message: {}", message.text);

        let Some(framed_msg) = message.encode() else {
            error!("Failed to encode message");
            continue;
        };

        pool.broadcast(&framed_msg, &msg.from).await;
    }
    error!("done with err {:?}", rx.recv().await);
}

// client takes the updated contents from the buffer and sends them to relay
// later will calculate CRDT operations and send them
// later will group edits together to save compute
//
// TcpStream to server to read and write buffer updates
// stdout to write updated contents to plugin
// stdin to read changes from plugin
pub async fn connect(read_socket: SocketAddrV4) {
    run_client(read_socket, io::stdin(), io::stdout()).await;
}

pub async fn run_client<R, W>(read_socket: SocketAddrV4, input: R, mut output: W)
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Unpin,
{
    // define reader for stream, stdin, stdout
    let (read_half, mut write_half) = TcpStream::connect(read_socket)
        .await
        .expect("Failed to connect to remote")
        .into_split();

    // read from stream for broadcasted updates
    let (stream_tx, mut stream_rx) = mpsc::channel(CHANNEL_SIZE);
    let stream_reader = FrameReader::new(read_half);
    task::spawn(async move { stream_reader.read_loop(stream_tx).await });
    trace!("Spawned stream read loop");

    // write stdin updates to stream
    let (stdin_tx, mut stdin_rx) = mpsc::channel(CHANNEL_SIZE);
    let stdin_reader = FrameReader::new(input);
    task::spawn(async move { stdin_reader.read_loop(stdin_tx).await });
    trace!("Spawned stream read loop");

    // write broadcasted updates to stdout
    loop {
        // use select here
        tokio::select! {
            // server -> stdout
            val = stream_rx.recv() => {
                match val {
                    Some(msg_bytes) => {
                        let message: TextUpdate = rmp_serde::from_slice(&msg_bytes).unwrap();
                        trace!("server -> stdout {:?}", message);
                        let framed = message.encode().unwrap();
                        output.write_all(&framed).await.unwrap();
                        output.flush().await.unwrap();
                    }
                    None => {
                        error!("Server disconnected");
                        break;
                    }
                }
            }

            // stdin -> server
            val = stdin_rx.recv() => {
                match val {
                    Some(msg_bytes) => {
                        // deserialize and serialize for later
                        // also ensures that plugin isn't sending garbage to the server
                        let message: TextUpdate = rmp_serde::from_slice(&msg_bytes).unwrap();
                        trace!("stdin -> server {:?}", message);
                        let framed = message.encode().unwrap();
                        write_half.write_all(&framed).await.unwrap();
                        write_half.flush().await.unwrap();
                    }
                    None => {
                        error!("Stdin closed");
                        break;
                    }
                }
            }
        }
    }
}
