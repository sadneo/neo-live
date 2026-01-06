use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use log::{info, trace};
use neo_live::serve;
use neo_live::TextUpdate;

fn init_logger() {
    let _ = env_logger::builder().is_test(true).try_init();
}

// Helper to reliably read one protocol frame from a stream
// Replaces the flaky `conn.read(&mut buf)`
async fn read_frame(stream: &mut TcpStream) -> TextUpdate {
    let mut len_buf = [0u8; 4];

    trace!("reading header");
    stream
        .read_exact(&mut len_buf)
        .await
        .expect("Failed to read header");
    let len = u32::from_be_bytes(len_buf) as usize;

    trace!("reading payload");
    let mut payload = vec![0u8; len];
    stream
        .read_exact(&mut payload)
        .await
        .expect("Failed to read payload");

    rmp_serde::from_slice(&payload).expect("Deserialization failed")
}

#[tokio::test]
async fn serve_basic() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32480);

    // Start server (no longer takes a reader arg)
    tokio::task::spawn(serve(addr));
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Connect Receiver (Client B)
    let mut client_b = TcpStream::connect(addr).await.unwrap();

    // 2. Connect Sender (Client A)
    let mut client_a = TcpStream::connect(addr).await.unwrap();

    // 3. Sender sends message
    let update = TextUpdate::new("testing! robux".to_owned());
    let encoded = update.encode().unwrap();
    client_a.write_all(&encoded).await.unwrap();
    client_a.flush().await.unwrap();

    // 4. Receiver should get it
    let decoded = read_frame(&mut client_b).await;
    assert_eq!(update, decoded);
}

#[tokio::test]
async fn serve_broadcast() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32481);
    tokio::task::spawn(serve(addr));
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Setup Sender
    let mut sender = TcpStream::connect(addr).await.unwrap();

    // 2. Setup 50 Listeners
    const STREAM_COUNT: usize = 50;
    let mut listeners = vec![];
    for _ in 0..STREAM_COUNT {
        listeners.push(TcpStream::connect(addr).await.unwrap());
    }

    // Allow server to accept them all
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 3. Send Data
    let update = TextUpdate::new("broadcast test".to_owned());
    let encoded = update.encode().unwrap();
    sender.write_all(&encoded).await.unwrap();

    // 4. Verify all listeners got it
    for listener in listeners.iter_mut() {
        let decoded = read_frame(listener).await;
        assert_eq!(update, decoded);
    }

    // 5. CRITICAL: Verify Sender did NOT get it (No Echo)
    // We try to read with a short timeout. It should timeout (Err).
    let result = timeout(Duration::from_millis(100), read_frame(&mut sender)).await;
    assert!(
        result.is_err(),
        "Sender received its own message! (Echo bug)"
    );
}

#[tokio::test]
async fn serve_drop() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32482);
    tokio::task::spawn(serve(addr));
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Connect Client A (will drop)
    let client_a = TcpStream::connect(addr).await.unwrap();

    // 2. Connect Client B (Listener)
    let mut client_b = TcpStream::connect(addr).await.unwrap();

    // 3. Connect Client C (Sender)
    let mut client_c = TcpStream::connect(addr).await.unwrap();

    // 4. Drop Client A explicitly
    drop(client_a);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 5. Send message 1
    let msg1 = TextUpdate::new("bait".to_owned());
    client_c.write_all(&msg1.encode().unwrap()).await.unwrap();

    // 6. Verify B gets it (Server shouldn't crash just because A is gone)
    let decoded = read_frame(&mut client_b).await;
    assert_eq!(decoded.text(), "bait");
}

#[tokio::test]
async fn serve_fragment() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32483);
    tokio::task::spawn(serve(addr));
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Connect Receiver
    let mut receiver = TcpStream::connect(addr).await.unwrap();

    // 2. Connect Sender
    let mut sender = TcpStream::connect(addr).await.unwrap();

    let update = TextUpdate::new("fragmented".to_owned());
    let encoded = update.encode().unwrap();

    // 3. Send Partial Data (Header + partial payload)
    info!("sent partial bytes");
    sender.write_all(&encoded[0..6]).await.unwrap(); // Header(4) + "fr"(2)
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 4. Send Rest
    sender.write_all(&encoded[6..]).await.unwrap();

    // 5. Receiver should reconstruct it perfectly
    let decoded = read_frame(&mut receiver).await;
    assert_eq!(update, decoded);
}

#[tokio::test]
async fn serve_garbage() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32484);
    tokio::task::spawn(serve(addr));
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut receiver = TcpStream::connect(addr).await.unwrap();
    let mut sender = TcpStream::connect(addr).await.unwrap();

    // 1. Send Garbage
    {
        let mut bad_buf = vec![];
        let payload = b"this is not valid msgpack";
        // Header says length is correct, but payload is garbage
        let len = (payload.len() as u32).to_be_bytes();
        bad_buf.extend_from_slice(&len);
        bad_buf.extend_from_slice(payload);

        sender.write_all(&bad_buf).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 2. Send Valid Data (on same connection)
    // Note: Depending on implementation, server might drop the connection on error.
    // If your server drops clients on error, you'd need to reconnect `sender` here.
    // Assuming your loop logs error and continues:

    let update = TextUpdate::new("real data".to_owned());
    let encoded = update.encode().unwrap();
    sender.write_all(&encoded).await.unwrap();

    // 3. Receiver should get the valid data (skipping the garbage)
    let decoded = read_frame(&mut receiver).await;
    assert_eq!(update, decoded);
}
