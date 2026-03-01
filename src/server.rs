use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;

use log::{debug, error, info, trace};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::RwLock;

use crate::protocol::{self, FrameReader, TextUpdate};

const CHANNEL_SIZE: usize = 5;

#[derive(Debug)]
struct IncomingMessage {
    from: SocketAddrV4,
    content: Vec<u8>,
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
        info!("received message: {}", message.text());

        let Some(framed_msg) = protocol::encode_frame(&message) else {
            error!("Failed to encode message");
            continue;
        };

        pool.broadcast(&framed_msg, &msg.from).await;
    }
    error!("done with err {:?}", rx.recv().await);
}
