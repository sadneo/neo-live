use std::io::{Read, Write};
use std::net::TcpListener;

fn main() -> std::io::Result<()> {
    let ip_addr = "127.0.0.1";
    let port = 3249;

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
