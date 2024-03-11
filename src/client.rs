use clap::Parser;
use std::io::{self, BufRead, Write};
use std::net::TcpStream;

#[derive(Parser, Debug)]
struct Cli {
    /// The IP address to listen on. Defaults to "127.0.0.1"
    #[arg(short, long)]
    address: Option<String>,

    /// The port to listen on. Defaults to "3249"
    #[arg(short, long)]
    port: Option<u16>,
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let ip_addr = match &cli.address {
        Some(address) => address,
        None => "127.0.0.1",
    };
    let port = cli.port.unwrap_or(3249);

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
