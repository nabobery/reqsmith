mod action;
mod app;
mod cli;
mod components;
mod config;
mod errors;
mod logging;
mod tui;

use clap::Parser;
use color_eyre::eyre::Result;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let cli = cli::Cli::parse();
    let config = config::Config::from_cli(&cli);

    // Hold the guard alive so logs flush on exit.
    let _log_guard = logging::init_logging(cli.debug)?;

    tracing::info!("Starting hurl");

    let mut tui = tui::Tui::new()?;
    let mut app = app::App::new(config);

    tui.enter()?;
    let result = app.run(&mut tui).await;
    tui.exit()?;

    result
}
