use tokio::io::{AsyncReadExt, BufReader};
use tokio::sync::mpsc::Sender;

use log::{error, trace};
use serde::{Deserialize, Serialize};

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
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct PluginUpdate {
    cursor_row: u32,
    cursor_col: u32,
    buffer: String,
    text: String,
}

impl PluginUpdate {
    pub fn from_text(text: String) -> Self {
        PluginUpdate {
            cursor_row: 0,
            cursor_col: 0,
            buffer: "".to_owned(),
            text,
        }
    }

    pub fn text(&self) -> &String {
        &self.text
    }
}

pub fn encode_frame(obj: &impl Serialize) -> Option<Vec<u8>> {
    let payload = match rmp_serde::to_vec_named(obj) {
        Ok(p) => p,
        Err(e) => {
            error!("msgpack encode error: {e}");
            return None;
        }
    };

    let mut buf = Vec::with_capacity(4 + payload.len());
    let len = (payload.len() as u32).to_be_bytes();
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&payload);

    Some(buf)
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
