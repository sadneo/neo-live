use std::io::{Read, Write};
use std::net::TcpListener;

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:62639").unwrap();
    
    let (mut stream, addr) = listener.accept()?;
    println!("{:?}\n{:?}", stream, addr);

    loop {
        let mut buffer = [0; 16];
        stream.read_exact(&mut buffer)?;
        println!("{:?}", buffer);

        stream.write_all(&buffer)?;

        let message = String::from_utf8(buffer.to_vec()).unwrap();
        println!("{}", message);
    }
}
