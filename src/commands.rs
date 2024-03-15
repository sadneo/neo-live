use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
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
    let (rstream, wstream) = TcpStream::connect(sock_addr).await?.into_split();

    task::spawn(client_send(wstream));
    Ok(())
}

async fn client_send(mut wstream: OwnedWriteHalf) -> std::io::Result<()> {
    let mut stdin = io::stdin();
    loop {
        let size = stdin.read_u64().await?;
        let mut buffer = vec![0; size as usize];
        stdin.read_exact(&mut buffer).await?;

        wstream.write_all(&size.to_be_bytes()).await?;
        wstream.write_all(&buffer).await?;
    }
}
