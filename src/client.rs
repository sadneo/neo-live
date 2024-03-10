use std::io::{self, BufRead, Write};
use std::net::TcpStream;

fn main() -> std::io::Result<()> {
    let ip_addr = "127.0.0.1";
    let port = 3249;

    let mut stream = TcpStream::connect((ip_addr, port))?;
    let mut stdin = io::stdin().lock();

    loop {
        let mut buffer = String::new();
        stdin.read_line(&mut buffer)?;
        let bytes = buffer.as_bytes();
        let length = bytes.len().to_be_bytes();

        stream.write_all(&length)?;
        stream.write_all(bytes)?;
    }
}
