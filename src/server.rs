use clap::Parser;
use std::io::{Read, Write};
use std::net::TcpListener;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// IPv4 Address
    #[arg(short, long)]
    address: Option<String>,

    /// Port to use
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

    let listener = TcpListener::bind((ip_addr, port)).unwrap();

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };

        loop {
            let mut buffer = [0; 8];
            stream.read_exact(&mut buffer)?;
            let mut data_buffer = vec![0; usize::from_be_bytes(buffer)];
            stream.read_exact(&mut data_buffer)?;

            stream.write_all(&data_buffer)?;

            let message = String::from_utf8(data_buffer.to_vec()).unwrap();
            print!("{}", message);
        }
    }
    Ok(())
}
