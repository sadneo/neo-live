use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use log::info;

use neo_live::TextUpdate;
use neo_live::serve;

fn init_logger() {
    let _ = env_logger::builder()
        .is_test(true)
        .try_init();
}

#[tokio::test]
async fn serve_basic() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32480);
    let (mut client, server) = tokio::io::duplex(64);
    tokio::task::spawn(serve(addr, server));

    // we use the wait statements for the network io
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let mut conn = TcpStream::connect(addr).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let orig_decoded = TextUpdate::new("testing! robux robux robux".to_owned());
    let orig_encoded = orig_decoded.encode().unwrap();
    client.write_all(&orig_encoded).await.unwrap();

    let mut buf = [0; 4096];
    let n = conn.read(&mut buf).await.unwrap();
    info!("read {} bytes", n);
    let decoded = rmp_serde::from_slice(&buf[4..n]).unwrap();

    assert_eq!(orig_decoded, decoded);
}

#[tokio::test]
async fn serve_broadcast() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32481);
    let (mut client, server) = tokio::io::duplex(64);
    tokio::task::spawn(serve(addr, server));

    // connect a bunch of streams to test broadcast
    const STREAM_COUNT: usize = 50;
    let mut connections = vec![];

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    for _ in 0..STREAM_COUNT {
        let conn = TcpStream::connect(addr).await.unwrap();
        connections.push(conn);
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // send the data
    let orig_decoded = TextUpdate::new("testing! robux robux robux".to_owned());
    let orig_encoded = orig_decoded.encode().unwrap();
    client.write_all(&orig_encoded).await.unwrap();

    // read from all the streams
    let mut buf = [0; 4096];
    for i in 0..STREAM_COUNT {
        let conn = &mut connections[i];
        let n = conn.read(&mut buf).await.unwrap();
        let decoded = rmp_serde::from_slice(&buf[4..n]).unwrap();
        assert_eq!(orig_decoded, decoded);
    }
}

#[tokio::test]
async fn serve_drop() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32482);
    let (mut client, server) = tokio::io::duplex(64);
    tokio::task::spawn(serve(addr, server));

    // connections
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let conn1 = TcpStream::connect(addr).await.unwrap();
    let mut conn2 = TcpStream::connect(addr).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    conn1
        .set_linger(Some(std::time::Duration::from_secs(0)))
        .unwrap();
    drop(conn1);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let msg_a = TextUpdate::new("bait".to_owned());
    client.write_all(&msg_a.encode().unwrap()).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let msg_b = TextUpdate::new("survivor".to_owned());
    client.write_all(&msg_b.encode().unwrap()).await.unwrap();

    // check
    let mut buf = vec![0; 4096];

    let n = conn2.read(&mut buf).await.unwrap();
    let decoded_a: TextUpdate = rmp_serde::from_slice(&buf[4..n]).unwrap();
    assert_eq!(decoded_a.text(), "bait");

    let n = conn2.read(&mut buf).await.unwrap();
    let decoded_b: TextUpdate = rmp_serde::from_slice(&buf[4..n]).unwrap();
    assert_eq!(decoded_b.text(), "survivor");
}

#[tokio::test]
async fn serve_fragment() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32483);
    let (mut client, server) = tokio::io::duplex(64);
    tokio::task::spawn(serve(addr, server));

    // we use the wait statements for the network io
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let mut conn = TcpStream::connect(addr).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let orig_decoded = TextUpdate::new("testing! robux robux robux".to_owned());
    let orig_encoded = orig_decoded.encode().unwrap();
    info!("sent {} bytes", orig_encoded.len());
    client.write_all(&orig_encoded[0..8]).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    client.write_all(&orig_encoded[8..]).await.unwrap();

    let mut buf = [0; 4096];
    let n = conn.read(&mut buf).await.unwrap();
    info!("read {} bytes", n);
    let decoded = rmp_serde::from_slice(&buf[4..n]).unwrap();

    assert_eq!(orig_decoded, decoded);
}

#[tokio::test]
async fn serve_garbage() {
    init_logger();
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 32484);
    let (mut client, server) = tokio::io::duplex(64);
    tokio::task::spawn(serve(addr, server));

    // we use the wait statements for the network io
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let mut conn = TcpStream::connect(addr).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // don't be alarmed if you see an error here about deserialization, that's what we want to see
    {
        let mut write_buf = vec![];
        let payload = b"this is not valid msgpack";
        let len = (payload.len() as u32).to_be_bytes();
        write_buf.extend_from_slice(&len);
        write_buf.extend_from_slice(payload);
        client.write_all(&write_buf).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let orig_decoded = TextUpdate::new("testing! robux robux robux".to_owned());
    let orig_encoded = orig_decoded.encode().unwrap();
    info!("sent {} bytes", orig_encoded.len());
    client.write_all(&orig_encoded).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut buf = [0; 4096];
    let n = conn.read(&mut buf).await.unwrap();
    info!("read {} bytes", n);
    let decoded: TextUpdate = rmp_serde::from_slice(&buf[4..n]).unwrap();
    assert_eq!(orig_decoded, decoded);
}
