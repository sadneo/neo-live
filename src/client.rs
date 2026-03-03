use std::collections::HashMap;
use std::net::SocketAddrV4;
use std::sync::Arc;

use log::{error, info, trace};
use tokio::io::{self, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex, Notify, RwLock};
use tokio::task;

use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};

use crate::protocol::{self, FrameReader, MessageKind, PluginUpdate, SyncMessage};

const CHANNEL_SIZE: usize = 5;

#[derive(Clone)]
struct BufferState {
    synced: bool,
    syncing: bool,
    last_state_vector: Vec<u8>,
    notify: Arc<Notify>,
}

struct ClientContext<W>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    doc: Arc<Doc>,
    buffers: Arc<RwLock<HashMap<String, BufferState>>>,
    write: Arc<Mutex<W>>,
}

impl<W> Clone for ClientContext<W>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            doc: Arc::clone(&self.doc),
            buffers: Arc::clone(&self.buffers),
            write: Arc::clone(&self.write),
        }
    }
}

impl<W> ClientContext<W>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    fn new(write: W) -> Self {
        Self {
            doc: Arc::new(Doc::new()),
            buffers: Arc::new(RwLock::new(HashMap::new())),
            write: Arc::new(Mutex::new(write)),
        }
    }

    async fn send_message(&self, msg: &SyncMessage) -> Result<(), ()> {
        let framed = match protocol::encode_frame(msg) {
            Some(framed) => framed,
            None => {
                error!("Failed to encode frame");
                return Err(());
            }
        };

        let mut writer = self.write.lock().await;
        if let Err(e) = writer.write_all(&framed).await {
            error!("Failed to write to server: {}", e);
            return Err(());
        }
        if let Err(e) = writer.flush().await {
            error!("Failed to flush to server: {}", e);
            return Err(());
        }
        Ok(())
    }

    async fn ensure_buffer_synced(&self, buffer: &str) -> Result<(), ()> {
        let notify;
        let should_send;
        {
            let mut buffers = self.buffers.write().await;
            let entry = buffers.entry(buffer.to_owned()).or_insert_with(|| BufferState {
                synced: false,
                syncing: false,
                last_state_vector: Vec::new(),
                notify: Arc::new(Notify::new()),
            });

            if entry.synced {
                return Ok(());
            }

            notify = entry.notify.clone();
            if entry.syncing {
                should_send = false;
            } else {
                entry.syncing = true;
                should_send = true;
            }
        }

        if should_send {
            let msg = SyncMessage::new(MessageKind::InitialSync, buffer.to_owned(), Vec::new());
            self.send_message(&msg).await?;
            info!("Sent InitialSync to server for buffer {}", buffer);
        }

        notify.notified().await;
        Ok(())
    }

    async fn handle_server_message<WOut>(
        &self,
        msg: SyncMessage,
        output: &mut WOut,
    ) -> Result<(), ()>
    where
        WOut: tokio::io::AsyncWrite + Unpin,
    {
        if !msg.is_update() {
            trace!("Ignoring non-update message from server");
            return Ok(());
        }

        let buffer_name = msg.buffer;
        let update_data = msg.payload;

        if !update_data.is_empty() {
            let update = match Update::decode_v1(&update_data) {
                Ok(update) => update,
                Err(e) => {
                    error!("Failed to decode update from server: {}", e);
                    return Ok(());
                }
            };
            let mut txn = self.doc.transact_mut();
            let _ = txn.apply_update(update);
        }

        let (text_content, state_vector) = {
            let text = self.doc.get_or_insert_text(buffer_name.as_str());
            let txn = self.doc.transact();
            (text.get_string(&txn), txn.state_vector().encode_v1())
        };

        {
            let mut buffers = self.buffers.write().await;
            let entry = buffers.entry(buffer_name.clone()).or_insert_with(|| BufferState {
                synced: false,
                syncing: false,
                last_state_vector: Vec::new(),
                notify: Arc::new(Notify::new()),
            });
            entry.synced = true;
            entry.syncing = false;
            entry.last_state_vector = state_vector;
            entry.notify.notify_waiters();
        }

        let plugin_update = PluginUpdate::new(0, 0, buffer_name, text_content);
        let framed = match protocol::encode_frame(&plugin_update) {
            Some(framed) => framed,
            None => {
                error!("Failed to encode plugin update");
                return Ok(());
            }
        };

        if let Err(e) = output.write_all(&framed).await {
            error!("Failed to write PluginUpdate to stdout: {}", e);
            return Err(());
        }
        if let Err(e) = output.flush().await {
            error!("Failed to flush PluginUpdate to stdout: {}", e);
            return Err(());
        }

        trace!("Sent PluginUpdate to stdout");
        Ok(())
    }

    async fn handle_plugin_update(&self, plugin_update: PluginUpdate) -> Result<(), ()> {
        let buffer_name = plugin_update.buffer().clone();
        if buffer_name.is_empty() {
            error!("PluginUpdate missing buffer name");
            return Ok(());
        }

        self.ensure_buffer_synced(&buffer_name).await?;

        let text_content = plugin_update.text().clone();
        {
            let text = self.doc.get_or_insert_text(buffer_name.as_str());
            let len = {
                let txn = self.doc.transact();
                text.len(&txn)
            };
            let mut txn = self.doc.transact_mut();
            text.remove_range(&mut txn, 0, len);
            text.insert(&mut txn, 0, &text_content);
        }

        let last_state_vector = {
            let buffers = self.buffers.read().await;
            buffers
                .get(&buffer_name)
                .map(|state| state.last_state_vector.clone())
                .unwrap_or_default()
        };

        let update = {
            let txn = self.doc.transact();
            if last_state_vector.is_empty() {
                txn.encode_diff_v1(&StateVector::default())
            } else {
                match StateVector::decode_v1(&last_state_vector) {
                    Ok(state_vector) => txn.encode_diff_v1(&state_vector),
                    Err(e) => {
                        error!("Failed to decode state vector: {}", e);
                        txn.encode_diff_v1(&StateVector::default())
                    }
                }
            }
        };

        {
            let mut buffers = self.buffers.write().await;
            if let Some(state) = buffers.get_mut(&buffer_name) {
                let txn = self.doc.transact();
                state.last_state_vector = txn.state_vector().encode_v1();
            }
        }

        if update.is_empty() {
            trace!("Skipping empty update for buffer {}", buffer_name);
            return Ok(());
        }

        let msg = SyncMessage::new(MessageKind::Update, buffer_name, update);
        self.send_message(&msg).await?;
        trace!("Sent update to server");
        Ok(())
    }
}

// client takes the updated contents from the buffer and sends them to relay
// later will calculate CRDT operations and send them
// later will group edits together to save compute
//
// TcpStream to server to read and write buffer updates
// stdout to write updated contents to plugin
// stdin to read changes from plugin
pub async fn connect(read_socket: SocketAddrV4) {
    let stream = match TcpStream::connect(read_socket).await {
        Ok(stream) => stream,
        Err(e) => {
            error!("Failed to connect to remote: {}", e);
            return;
        }
    };
    let (read_half, write_half) = stream.into_split();
    run_client(read_half, write_half, io::stdin(), io::stdout()).await;
}

pub async fn run_client<R, W, RH, WH>(
    read_half: RH,
    write_half: WH,
    input: R,
    output: W,
) where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    RH: tokio::io::AsyncRead + Send + Unpin + 'static,
    WH: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    // define reader for stream, stdin, stdout
    let context = ClientContext::new(write_half);

    // read from stream for broadcasted updates
    let (stream_tx, mut stream_rx) = mpsc::channel(CHANNEL_SIZE);
    let stream_reader = FrameReader::new(read_half);
    task::spawn(async move { stream_reader.read_loop(stream_tx).await });
    trace!("Spawned server stream read loop");

    // write stdin updates to stream
    let (stdin_tx, mut stdin_rx) = mpsc::channel(CHANNEL_SIZE);
    let stdin_reader = FrameReader::new(input);
    task::spawn(async move { stdin_reader.read_loop(stdin_tx).await });
    trace!("Spawned plugin stream read loop");

    let mut output = output;
    let server_context = context.clone();
    let server_task = task::spawn(async move {
        while let Some(msg_bytes) = stream_rx.recv().await {
            let msg: SyncMessage = match rmp_serde::from_slice(&msg_bytes) {
                Ok(msg) => msg,
                Err(e) => {
                    error!("Failed to deserialize SyncMessage: {}", e);
                    continue;
                }
            };

            if server_context
                .handle_server_message(msg, &mut output)
                .await
                .is_err()
            {
                break;
            }
        }
        error!("Server disconnected");
    });

    let plugin_context = context.clone();
    let plugin_task = task::spawn(async move {
        while let Some(msg_bytes) = stdin_rx.recv().await {
            let plugin_update: PluginUpdate = match rmp_serde::from_slice(&msg_bytes) {
                Ok(update) => update,
                Err(e) => {
                    error!("Failed to deserialize PluginUpdate: {}", e);
                    continue;
                }
            };

            if plugin_context.handle_plugin_update(plugin_update).await.is_err() {
                break;
            }
        }
        error!("Stdin closed");
    });

    let _ = tokio::join!(server_task, plugin_task);
}
