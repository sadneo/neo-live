use tokio::io::{AsyncReadExt, BufReader};
use tokio::sync::mpsc::Sender;

use log::{error, trace};
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};

#[repr(u8)]
#[derive(Serialize_repr, Deserialize_repr, Debug, PartialEq, Copy, Clone)]
pub enum MessageKind {
    InitialSync = 1,
    Update = 2,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct SyncMessage {
    pub kind: MessageKind,
    pub buffer: String,
    pub payload: Vec<u8>,
}

impl SyncMessage {
    pub fn new(kind: MessageKind, buffer: String, payload: Vec<u8>) -> Self {
        Self {
            kind,
            buffer,
            payload,
        }
    }

    pub fn is_initial_sync(&self) -> bool {
        self.kind == MessageKind::InitialSync
    }

    pub fn is_update(&self) -> bool {
        self.kind == MessageKind::Update
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
    pub fn new(cursor_row: u32, cursor_col: u32, buffer: String, text: String) -> Self {
        PluginUpdate {
            cursor_row,
            cursor_col,
            buffer,
            text,
        }
    }

    pub fn from_text(text: String) -> Self {
        PluginUpdate {
            cursor_row: 0,
            cursor_col: 0,
            buffer: "".to_owned(),
            text,
        }
    }

    pub fn cursor_row(&self) -> u32 {
        self.cursor_row
    }

    pub fn cursor_col(&self) -> u32 {
        self.cursor_col
    }

    pub fn buffer(&self) -> &String {
        &self.buffer
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
        loop {
            let Some(buf) = self.read_one().await else {
                break;
            };
            if sender.send(buf).await.is_err() {
                error!("Channel closed");
                break;
            }
        }
        error!("Read loop broke")
    }
    pub async fn read_one(&mut self) -> Option<Vec<u8>> {
        let mut len_buf = [0u8; 4];
        if self.reader.read_exact(&mut len_buf).await.is_err() {
            error!("Connection closed");
            return None;
        }
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut buf = vec![0u8; len];
        if self.reader.read_exact(&mut buf).await.is_err() {
            error!("Connection closed");
            return None;
        }

        Some(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, DuplexStream};

    fn decode_len_prefix(buf: &[u8]) -> u32 {
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&buf[..4]);
        u32::from_be_bytes(len_bytes)
    }

    async fn write_frame(stream: &mut DuplexStream, payload: &[u8]) {
        let len = (payload.len() as u32).to_be_bytes();
        stream.write_all(&len).await.unwrap();
        stream.write_all(payload).await.unwrap();
    }

    #[test]
    fn sync_message_kind_helpers_work() {
        let msg = SyncMessage::new(MessageKind::InitialSync, "main.rs".to_owned(), Vec::new());
        assert!(msg.is_initial_sync());
        assert!(!msg.is_update());

        let msg = SyncMessage::new(MessageKind::Update, "main.rs".to_owned(), Vec::new());
        assert!(msg.is_update());
        assert!(!msg.is_initial_sync());
    }

    #[test]
    fn plugin_update_accessors_work() {
        let update = PluginUpdate::new(10, 22, "src/lib.rs".to_owned(), "hello".to_owned());
        assert_eq!(update.cursor_row(), 10);
        assert_eq!(update.cursor_col(), 22);
        assert_eq!(update.buffer(), "src/lib.rs");
        assert_eq!(update.text(), "hello");
    }

    #[test]
    fn encode_frame_prefix_matches_payload_len() {
        let msg = SyncMessage::new(MessageKind::Update, "buffer".to_owned(), vec![1, 2, 3]);
        let framed = encode_frame(&msg).expect("frame");
        let len = decode_len_prefix(&framed) as usize;
        assert_eq!(len, framed.len() - 4);
        assert!(len > 0);
    }

    #[tokio::test]
    async fn frame_reader_reads_single_frame() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let mut frame_reader = FrameReader::new(reader);

        let payload = vec![9, 8, 7, 6, 5];
        write_frame(&mut writer, &payload).await;

        let read = frame_reader.read_one().await.expect("frame");
        assert_eq!(read, payload);
    }

    #[tokio::test]
    async fn frame_reader_reads_multiple_frames() {
        let (mut writer, reader) = tokio::io::duplex(128);
        let mut frame_reader = FrameReader::new(reader);

        let payload_a = vec![1, 2, 3];
        let payload_b = vec![4, 5, 6, 7];
        write_frame(&mut writer, &payload_a).await;
        write_frame(&mut writer, &payload_b).await;

        let read_a = frame_reader.read_one().await.expect("frame a");
        let read_b = frame_reader.read_one().await.expect("frame b");

        assert_eq!(read_a, payload_a);
        assert_eq!(read_b, payload_b);
    }
}
