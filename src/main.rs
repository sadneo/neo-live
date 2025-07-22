use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::thread;

use clap::{Parser, Subcommand, ValueEnum};
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

#[derive(ValueEnum, Clone, Debug)]
enum HostMode {
    Local, // 127.0.0.1
    Lan,   // Local IP like 192.168.x.x
    All,   // 0.0.0.0
}

#[derive(Parser, Debug)]
#[command(version, author, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Port to use
    #[arg(short, long, default_value = "3248")]
    port: u16,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Serve file to a socket
    Serve {
        /// File to serve
        path: PathBuf,

        // currently defaults to HostMode::Local for debugging
        /// Interface to bind to
        #[arg(long, value_enum, default_value_t = HostMode::Local)]
        host_mode: HostMode,
    },
    /// Connect to server at socket
    Connect {
        /// File to write to
        path: PathBuf,

        /// Remote IPv4 address to connect to
        #[arg(short, long, default_value = "127.0.0.1")]
        address: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { path, host_mode } => serve(path, host_mode, cli.port),
        Command::Connect { path, address } => connect(
            path,
            Ipv4Addr::from_str(&address).expect("Expected address"),
            cli.port,
        ),
    }
}

fn serve(file_path: PathBuf, host_mode: HostMode, port: u16) {
    let address = match host_mode {
        HostMode::Local => Ipv4Addr::new(127, 0, 0, 1),
        HostMode::All => Ipv4Addr::new(0, 0, 0, 0),
        HostMode::Lan => local_ip_address::local_ip()
            .unwrap()
            .to_string()
            .parse()
            .unwrap(),
    };
    let write_socket = SocketAddrV4::new(address, port);

    let clients: Arc<RwLock<Vec<TcpStream>>> = Arc::new(RwLock::new(Vec::new()));
    let clients_ref = clients.clone();
    let file_path_clone = file_path.clone();

    thread::spawn(move || {
        let listener = TcpListener::bind(write_socket).expect("Failed to create listener");
        for stream in listener.incoming().flatten() {
            let mut clients_guard = clients.write().unwrap();
            clients_guard.push(stream);
            println!("Client added");
        }
    });

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher: RecommendedWatcher =
        Watcher::new(tx, Config::default()).expect("Failed to create watcher");

    watcher
        .watch(&file_path_clone, RecursiveMode::NonRecursive)
        .expect("Failed to watch file");

    loop {
        if let Ok(event) = rx.recv() {
            if let EventKind::Modify(_) = event.unwrap().kind {
                println!("modify");
                let Ok(contents) = fs::read(&file_path) else {
                    continue;
                };

                let mut guard = clients_ref.write().unwrap();
                guard.retain_mut(|stream| stream.write_all(&contents).is_ok());
            }
        }
    }
}

fn connect(path: PathBuf, remote: Ipv4Addr, port: u16) {
    let read_socket = SocketAddrV4::new(remote, port);

    let mut stream = TcpStream::connect(read_socket).unwrap();
    let mut buffer = [0; 500];
    loop {
        let n = stream.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        let contents = String::from_utf8_lossy(&buffer[..n]);
        println!("recv: {:?}", contents);
        fs::write(&path, buffer).unwrap();
    }
}
