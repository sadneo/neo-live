use std::fs::OpenOptions;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(version, author, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Port to use
    #[arg(short, long, default_value = "3248")]
    port: u16,

    /// Log output (stderr, file)
    #[arg(long, default_value = "stderr")]
    log_output: String,

    /// Log level
    #[arg(long, default_value = "trace")]
    log_level: String,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Serve file to a socket
    Serve {
        // currently defaults to HostMode::Local for debugging
        /// Interface to bind to
        #[arg(long, value_enum, default_value_t = HostMode::Local)]
        host_mode: HostMode,
    },
    /// Connect to server at socket
    Connect {
        /// Remote IPv4 address to connect to
        #[arg(short, long, default_value = "127.0.0.1")]
        address: String,
    },
}

#[derive(ValueEnum, Clone, Debug)]
pub enum HostMode {
    Local, // 127.0.0.1
    Lan,   // Local IP like 192.168.x.x
    All,   // 0.0.0.0
}

fn resolve_address(host_mode: HostMode, port: u16) -> SocketAddrV4 {
    let address = match host_mode {
        HostMode::Local => Ipv4Addr::new(127, 0, 0, 1),
        HostMode::All => Ipv4Addr::new(0, 0, 0, 0),
        HostMode::Lan => local_ip_address::local_ip()
            .expect("Failed to get local IP")
            .to_string()
            .parse()
            .expect("Failed to parse local IP"),
    };
    SocketAddrV4::new(address, port)
}

fn init_logging(log_output: String, log_level: String) {
    use log::LevelFilter;

    let log_level = match &*log_level.to_ascii_lowercase() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Off,
    };

    let mut builder = env_logger::Builder::from_default_env();
    let mut builder = builder.filter_level(log_level);

    match &*log_output {
        "file" => {
            let log_file = OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/neo-live.log")
                .expect("Could not create log file");

            builder = builder.target(env_logger::Target::Pipe(Box::new(log_file)))
        }
        "stderr" => (),
        _ => return,
    };

    builder.init();
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_logging(cli.log_output, cli.log_level);

    match cli.command {
        Command::Serve { host_mode } => {
            let addr = resolve_address(host_mode, cli.port);
            neo_live::serve(addr).await
        }
        Command::Connect { address } => {
            neo_live::connect(SocketAddrV4::new(
                Ipv4Addr::from_str(&address).expect("Expected address"),
                cli.port,
            ))
            .await
        }
    }
}
