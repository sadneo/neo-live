use std::net::SocketAddrV4;

use log::{error, trace};
use tokio::io::{self, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self};
use tokio::task;

use crate::protocol::{self, FrameReader, PluginUpdate, TextUpdate};

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
                        let text_update: TextUpdate = rmp_serde::from_slice(&msg_bytes).unwrap();
                        let plugin_update = PluginUpdate::from_text(text_update.text().to_owned());

                        trace!("server -> stdout {:?}", plugin_update);
                        let framed = protocol::encode_frame(&plugin_update).unwrap();
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
                        let plugin_update: PluginUpdate = rmp_serde::from_slice(&msg_bytes).unwrap();
                        let text_update: TextUpdate = TextUpdate::new(plugin_update.text().to_owned());

                        trace!("stdin -> server {:?}", text_update);
                        let framed = protocol::encode_frame(&text_update).unwrap();
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
