use clap::{Parser, Subcommand};

mod commands;

#[derive(Parser, Debug)]
#[command(version, author, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// IPv4 Address
    #[arg(short, long, default_value = "127.0.0.1")]
    address: String,

    /// Port to use
    #[arg(short, long, default_value = "3249")]
    port: u16,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Serve file to a port
    Serve,
    /// Become client to a served port
    Client,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let ip_addr = &cli.address;
    let port = cli.port;

    match cli.command {
        Commands::Serve => commands::serve((ip_addr, port)).await,
        Commands::Client => commands::client((ip_addr, port)).await,
    }
}
