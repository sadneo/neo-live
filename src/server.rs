use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;

use log::{debug, error, info, trace};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::RwLock;

use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

use crate::protocol::{self, FrameReader, MessageKind, SyncMessage};

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

    async fn add(&self, stream: OwnedWriteHalf) {
        self.clients.write().await.push(stream);
    }

    // writes data to all clients, removing any that error out.
    async fn broadcast(&self, data: &[u8], ignore: &SocketAddrV4) {
        let mut clients = self.clients.write().await;

        // retain only clients that successfully accept the write
        let mut i = 0;
        while i < clients.len() {
            let client = &clients[i];
            let addr_string = client
                .peer_addr()
                .map(|s| format!("{}", s))
                .unwrap_or("<bad peer_addr>".to_owned());

            trace!("Handling client {}", addr_string);
            if let Ok(SocketAddr::V4(addr)) = client.peer_addr() {
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
        }

        for client in &mut *clients {
            client.flush().await.unwrap();
        }
    }

    async fn send_to(&self, data: &[u8], addr: &SocketAddrV4) {
        let mut clients = self.clients.write().await;

        for client in &mut *clients {
            if let Ok(SocketAddr::V4(client_addr)) = client.peer_addr() {
                if &client_addr == addr {
                    client.write_all(data).await.unwrap();
                    client.flush().await.unwrap();
                    break;
                }
            }
        }
    }
}

struct ServerState {
    doc: Doc,
    pool: ClientPool,
}

async fn run_listener(
    addr: SocketAddrV4,
    tx: Sender<IncomingMessage>,
    state: Arc<RwLock<ServerState>>,
) {
    let listener = TcpListener::bind(addr)
        .await
        .expect("Failed to bind listener");

    loop {
        let tx_ref = tx.clone();
        match listener.accept().await {
            Ok((stream, SocketAddr::V4(address))) => {
                info!("{} connected to server", address);
                let (read_half, write_half) = stream.into_split();

                // add stream to the pool for broadcasting
                {
                    let state = state.read().await;
                    state.pool.add(write_half).await;
                }

                // when the stream sends messages, add "from" address so when it gets broadcasted
                // it doesn't get sent back to the same guy
                let mut reader = FrameReader::new(read_half);
                tokio::spawn(async move {
                    while let Some(msg) = reader.read_one().await {
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

async fn handle_initial_sync(
    state: &Arc<RwLock<ServerState>>,
    from: &SocketAddrV4,
    msg: SyncMessage,
) {
    let buffer_name = msg.buffer.clone();

    let updates = {
        let state_guard = state.read().await;
        let _text = state_guard.doc.get_or_insert_text(buffer_name.as_str());

        if msg.payload.is_empty() {
            let sv = StateVector::default();
            state_guard.doc.transact().encode_diff_v1(&sv)
        } else if let Ok(sv) = StateVector::decode_v1(&msg.payload) {
            state_guard.doc.transact().encode_diff_v1(&sv)
        } else {
            Vec::new()
        }
    };

    let sync_response = SyncMessage::new(MessageKind::Update, buffer_name, updates);
    let framed = protocol::encode_frame(&sync_response).unwrap();

    let state = state.read().await;
    state.pool.send_to(&framed, from).await;
    info!("Sent sync response to {}", from);
}

async fn handle_update(state: &Arc<RwLock<ServerState>>, from: &SocketAddrV4, msg: SyncMessage) {
    let buffer_name = msg.buffer.clone();
    let update_data = msg.payload;

    if !update_data.is_empty() {
        let update = Update::decode_v1(&update_data).unwrap();
        let state_guard = state.read().await;
        let mut txn = state_guard.doc.transact_mut();
        let _ = txn.apply_update(update);
    }

    let framed = protocol::encode_frame(&SyncMessage::new(
        MessageKind::Update,
        buffer_name.clone(),
        update_data,
    ))
    .unwrap();

    let state = state.read().await;
    state.pool.broadcast(&framed, from).await;
    trace!("Broadcasted update for buffer {}", buffer_name);
}

// server acts as a relay to send buffer contents
// later will relay CRDT operations instead
// later check whether the messages are valid so it doesn't relay junk
//
// uses TcpListener to add streams to ClientPool
// both reads and writes to streams
pub async fn serve(addr: SocketAddrV4) {
    let doc = Doc::new();
    let pool = ClientPool::new();

    let state = Arc::new(RwLock::new(ServerState { doc, pool }));
    let state_ref = state.clone();

    // listen for connections
    // also set up input reading from clients here
    let (tx, mut rx) = mpsc::channel(CHANNEL_SIZE);

    tokio::task::spawn(async move {
        debug!("Starting listener with address {}", addr);
        run_listener(addr, tx, state_ref).await;
    });

    while let Some(incoming) = rx.recv().await {
        let Ok(msg) = rmp_serde::from_slice::<SyncMessage>(&incoming.content) else {
            error!("Failed to deserialize message");
            continue;
        };

        if msg.kind == MessageKind::InitialSync {
            debug!("Received initial sync request for buffer: {}", msg.buffer);
            handle_initial_sync(&state, &incoming.from, msg).await;
        } else if msg.kind == MessageKind::Update {
            debug!("Received update for buffer: {}", msg.buffer);
            handle_update(&state, &incoming.from, msg).await;
        } else {
            error!("Unknown message kind: {:?}", msg.kind);
        }
    }
    error!("done with err {:?}", rx.recv().await);
}
