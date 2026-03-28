use clap::Parser;

/// Terminal-native API client.
#[derive(Parser, Debug)]
#[command(name = "hurl", version, about)]
pub struct Cli {
    /// Enable debug logging
    #[arg(short, long)]
    pub debug: bool,

    /// Override tick rate in milliseconds
    #[arg(long)]
    pub tick_rate: Option<u64>,

    /// Override frame rate in milliseconds
    #[arg(long)]
    pub frame_rate: Option<u64>,
}
