use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

pub fn serve(sock_addr: (&str, u16)) -> std::io::Result<()> {
    let listener = TcpListener::bind(sock_addr).unwrap();

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

pub fn client(sock_addr: (&str, u16)) -> std::io::Result<()> {
    use std::io::{self, BufRead};

    let mut stream = TcpStream::connect(sock_addr)?;
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
