use std::net::SocketAddrV4;

use log::{error, info, trace};
use tokio::io::{self, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self};
use tokio::task;

use yrs::updates::decoder::Decode;
use yrs::{GetString, ReadTxn, Text, Transact, Update};

use crate::protocol::{self, FrameReader, MessageKind, PluginUpdate, SyncMessage};

const CHANNEL_SIZE: usize = 5;

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
    trace!("Spawned server stream read loop");

    // write stdin updates to stream
    let (stdin_tx, mut stdin_rx) = mpsc::channel(CHANNEL_SIZE);
    let stdin_reader = FrameReader::new(input);
    task::spawn(async move { stdin_reader.read_loop(stdin_tx).await });
    trace!("Spawned plugin stream read loop");

    let doc = yrs::Doc::new();
    {
        let handshake_msg = SyncMessage::new(MessageKind::InitialSync, "".to_owned(), Vec::new());
        let framed = protocol::encode_frame(&handshake_msg).unwrap();
        write_half.write_all(&framed).await.unwrap();
        write_half.flush().await.unwrap();
        info!("Sent InitialSync to server");
    }

    // Wait for server's initial sync response before accepting any plugin updates
    let sync_response = stream_rx.recv().await.expect("Server disconnected");
    let msg: SyncMessage = rmp_serde::from_slice(&sync_response).unwrap();
    if msg.is_update() && !msg.payload.is_empty() {
        let update = Update::decode_v1(&msg.payload).unwrap();
        let mut txn = doc.transact_mut();
        let _ = txn.apply_update(update);
    }
    if msg.is_update() {
        let text = doc.get_or_insert_text(msg.buffer.as_str());
        let txn = doc.transact();
        let text_content = text.get_string(&txn);

        let plugin_update = PluginUpdate::new(0, 0, msg.buffer, text_content);
        let framed = protocol::encode_frame(&plugin_update).unwrap();
        output.write_all(&framed).await.unwrap();
        output.flush().await.unwrap();
        trace!("Sent PluginUpdate to stdout");
    }
    info!("Received initial sync from server");

    loop {
        tokio::select! {
            // server -> stdout
            val = stream_rx.recv() => {
                match val {
                    Some(msg_bytes) => {
                        let msg: SyncMessage = match rmp_serde::from_slice(&msg_bytes) {
                            Ok(u) => u,
                            Err(e) => {
                                error!("Failed to deserialize SyncMessage: {}", e);
                                continue;
                            }
                        };

                        if !msg.is_update() {
                            trace!("Ignoring non-update message from server");
                            continue;
                        }

                        let buffer_name = msg.buffer.clone();
                        let update_data = msg.payload;

                        if !update_data.is_empty() {
                            let update = Update::decode_v1(&update_data).unwrap();
                            let mut txn = doc.transact_mut();
                            let _ = txn.apply_update(update);
                        }

                        let text = doc.get_or_insert_text(buffer_name.as_str());
                        let txn = doc.transact();
                        let text_content = text.get_string(&txn);

                        let plugin_update = PluginUpdate::new(0, 0, buffer_name, text_content);
                        let framed = protocol::encode_frame(&plugin_update).unwrap();
                        output.write_all(&framed).await.unwrap();
                        output.flush().await.unwrap();
                        trace!("Sent PluginUpdate to stdout");
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
                        let plugin_update: PluginUpdate = match rmp_serde::from_slice(&msg_bytes) {
                            Ok(u) => u,
                            Err(e) => {
                                error!("Failed to deserialize PluginUpdate: {}", e);
                                continue;
                            }
                        };

                        let buffer_name = plugin_update.buffer().clone();
                        let text_content = plugin_update.text().clone();

                        {
                            let text = doc.get_or_insert_text(buffer_name.as_str());
                            let len = {
                                let txn = doc.transact();
                                text.len(&txn)
                            };
                            let mut txn = doc.transact_mut();
                            text.remove_range(&mut txn, 0, len as u32);
                            text.insert(&mut txn, 0, &text_content);
                        }

                        let update = {
                            let txn = doc.transact();
                            txn.encode_diff_v1(&yrs::StateVector::default())
                        };

                        let msg = SyncMessage::new(MessageKind::Update, buffer_name, update);
                        let framed = protocol::encode_frame(&msg).unwrap();
                        write_half.write_all(&framed).await.unwrap();
                        write_half.flush().await.unwrap();
                        trace!("Sent update to server");
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
