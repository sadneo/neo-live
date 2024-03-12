use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task;

pub async fn serve(sock_addr: (&str, u16)) -> std::io::Result<()> {
    let listener = TcpListener::bind(sock_addr).await.unwrap();

    loop {
        let (stream, _) = listener.accept().await?;
        task::spawn(serve_stream(stream));
    }
}

async fn serve_stream(mut stream: TcpStream) -> std::io::Result<()> {
    loop {
        let size = stream.read_u64().await?;
        let mut buffer = vec![0; size as usize];
        stream.read_exact(&mut buffer).await?;

        stream.write_all(&buffer).await?;

        let message = String::from_utf8(buffer.to_vec()).unwrap();
        print!("{}", message);
    }
}

pub async fn client(sock_addr: (&str, u16)) -> std::io::Result<()> {
    use std::io::{self, BufRead};

    let mut stream = TcpStream::connect(sock_addr).await?;
    let mut stdin = io::stdin().lock();

    loop {
        let mut buffer = String::new();
        stdin.read_line(&mut buffer)?;
        let bytes = buffer.as_bytes();
        let length = bytes.len().to_be_bytes();

        stream.write_all(&length).await?;
        stream.write_all(bytes).await?;
    }
}
