use std::io::{self, Write};
use std::net::TcpStream;

fn main() -> std::io::Result<()> {
    let mut stream = TcpStream::connect("127.0.0.1:62639")?;
    let stdin = io::stdin();
    println!("{:?}", stream);

    loop {
        let mut buffer = String::new();
        stdin.read_line(&mut buffer)?;
        stream.write_all(buffer.as_bytes())?;
    }
    // when you first connect, the server sends the file
    // the file is created temporarily in a specified directory with configuration from the client
    // you edit the file with your editor of choice
    // when the connection is closed, the file is deleted
}
