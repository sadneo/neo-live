use std::net::{SocketAddrV4};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

// Import your crate (assuming crate name is 'neo_live')
use neo_live::{run_client, TextUpdate};
use neo_live::FrameReader;

#[tokio::test]
async fn test_frame_reader_parsing() {
    // Mock a TCP stream containing two messages
    let mock_data = tokio_test::io::Builder::new()
        .read(&u32::to_be_bytes(5)) // Header 1 (len 5)
        .read(b"hello") // Payload 1
        .read(&u32::to_be_bytes(3)) // Header 2 (len 3)
        .read(b"bye") // Payload 2
        .build();

    let mut reader = FrameReader::new(mock_data);

    // Assert First Frame
    let frame1 = reader.next_frame().await.unwrap();
    assert_eq!(frame1, b"hello");

    // Assert Second Frame
    let frame2 = reader.next_frame().await.unwrap();
    assert_eq!(frame2, b"bye");
}

// Helper to start a real TCP listener on a random port
async fn setup_server() -> (TcpListener, SocketAddrV4) {
    // Bind to port 0 (OS picks random port)
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("Failed to bind");
    let addr = listener.local_addr().unwrap();
    
    // Convert generic SocketAddr to SocketAddrV4
    let addr_v4 = match addr {
        std::net::SocketAddr::V4(v4) => v4,
        _ => panic!("Not IPv4"),
    };
    
    (listener, addr_v4)
}

#[tokio::test]
async fn client_receives_remote_update() {
    // 1. Setup "Server"
    let (listener, addr) = setup_server().await;

    // 2. Prepare Data (Server -> Client)
    let update = TextUpdate::new("Remote Edit".to_owned());
    let encoded = update.encode().unwrap();

    // 3. Mock IO
    // Stdin: Empty (Client user types nothing)
    // Stdout: Expects to receive the encoded message
    let mock_stdin = tokio_test::io::Builder::new()
        .wait(Duration::from_secs(1)) // Keep open
        .build();
        
    let mock_stdout = tokio_test::io::Builder::new()
        .write(&encoded) // <--- Assertion: Client writes this to stdout
        .build();

    // 4. Run Client in background
    // We pass ownership of mocks to run_client
    let client_task = tokio::spawn(async move {
        run_client(addr, mock_stdin, mock_stdout).await;
    });

    // 5. "Server" accepts connection and sends data
    let (mut server_stream, _) = listener.accept().await.unwrap();
    server_stream.write_all(&encoded).await.unwrap();

    // 6. Cleanup
    // Wait a bit for client to process
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Abort task since mocks keep it alive forever
    client_task.abort(); 
}

#[tokio::test]
async fn client_sends_local_update() {
    let (listener, addr) = setup_server().await;

    let update = TextUpdate::new("My Local Edit".to_owned());
    let encoded = update.encode().unwrap();

    // 1. Mock IO
    // Stdin: User types the message
    let mock_stdin = tokio_test::io::Builder::new()
        .read(&encoded) 
        .wait(Duration::from_secs(1))
        .build();
        
    // Stdout: Silent
    let mock_stdout = tokio_test::io::Builder::new().build();

    // 2. Run Client
    let client_task = tokio::spawn(async move {
        run_client(addr, mock_stdin, mock_stdout).await;
    });

    // 3. "Server" accepts connection
    let (mut server_stream, _) = listener.accept().await.unwrap();

    // 4. Server reads data from Client
    let mut buf = vec![0u8; encoded.len()];
    server_stream.read_exact(&mut buf).await.unwrap();
    
    assert_eq!(buf, encoded, "Server received wrong bytes from client");

    client_task.abort();
}

#[tokio::test]
async fn client_exits_on_disconnect() {
    let (listener, addr) = setup_server().await;

    let mock_stdin = tokio_test::io::Builder::new()
        .wait(Duration::from_secs(5)) // Keep stdin open
        .build();
    let mock_stdout = tokio_test::io::Builder::new().build();

    // 1. Run Client
    let client_task = tokio::spawn(async move {
        run_client(addr, mock_stdin, mock_stdout).await;
    });

    // 2. Server accepts...
    let (server_stream, _) = listener.accept().await.unwrap();
    
    // 3. ...and immediately drops connection
    drop(server_stream);

    // 4. Assert Client terminates gracefully
    // We expect the client_task to finish NATURALLY (not timeout, not abort)
    let result = timeout(Duration::from_millis(200), client_task).await;
    
    assert!(result.is_ok(), "Client hung after server disconnected");
}
