mod action;
mod app;
mod cli;
mod commands;
mod components;
mod config;
mod core;
mod errors;
mod infra;
mod logging;
mod output;
mod tui;

use std::process::ExitCode;

use clap::Parser;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = color_eyre::install() {
        eprintln!("Failed to install error handler: {e}");
        return ExitCode::from(1);
    }

    let cli = cli::Cli::parse();

    // Hold the guard alive so logs flush on exit.
    let _log_guard = match logging::init_logging(cli.debug) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("Failed to initialize logging: {e}");
            return ExitCode::from(1);
        }
    };

    match cli.command {
        Some(cmd) => run_command(cmd).await,
        None => run_tui(cli.tick_rate, cli.frame_rate).await,
    }
}

async fn run_tui(tick_rate: Option<u64>, frame_rate: Option<u64>) -> ExitCode {
    let config = config::Config::new(tick_rate, frame_rate);

    tracing::info!("Starting hurl TUI");

    let mut tui = match tui::Tui::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to create terminal: {e}");
            return ExitCode::from(1);
        }
    };

    let mut app = app::App::new(config);

    if let Err(e) = tui.enter() {
        eprintln!("Failed to enter TUI mode: {e}");
        return ExitCode::from(1);
    }

    let result = app.run(&mut tui).await;

    if let Err(e) = tui.exit() {
        eprintln!("Failed to exit TUI mode: {e}");
        return ExitCode::from(1);
    }

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_command(cmd: cli::Command) -> ExitCode {
    match cmd {
        cli::Command::Run {
            file,
            env,
            vars,
            output,
            quiet,
        } => commands::run::execute(file, env, vars, output, quiet).await,

        cli::Command::Fmt { files, check } => commands::fmt::execute(files, check),

        cli::Command::Validate { files, env, output } => {
            commands::validate::execute(files, env, output)
        }

        cli::Command::List { path, output } => commands::list::execute(path, output),
    }
}
