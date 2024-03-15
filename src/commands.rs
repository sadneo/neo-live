use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
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
    let (rstream, wstream) = TcpStream::connect(sock_addr).await?.into_split();

    task::spawn(handle_client(io::stdin(), wstream));
    task::spawn(handle_client(rstream, io::stdout()));
    Ok(())
}

async fn handle_client<Reader, Writer>(mut rstream: Reader, mut wstream: Writer) -> std::io::Result<()>
where
    Reader: AsyncReadExt + Unpin,
    Writer: AsyncWriteExt + Unpin,
{
    loop {
        let size = rstream.read_u64().await?;
        let mut buffer = vec![0; size as usize];
        rstream.read_exact(&mut buffer).await?;

        wstream.write_all(&size.to_be_bytes()).await?;
        wstream.write_all(&buffer).await?;
    }
}
