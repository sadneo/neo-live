use clap::{Parser, Subcommand};

mod commands;

#[derive(Parser, Debug)]
#[command(version, author, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// IPv4 Address
    #[arg(short, long)]
    address: Option<String>,

    /// Port to use
    #[arg(short, long)]
    port: Option<u16>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Serve file to a port
    Serve,
    /// Become client to a served port
    Client,
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let ip_addr = match &cli.address {
        Some(address) => address,
        None => "127.0.0.1",
    };
    let port = cli.port.unwrap_or(3249);

    match cli.command {
        Commands::Serve => commands::serve((ip_addr, port)),
        Commands::Client => commands::client((ip_addr, port)),
    }
}
