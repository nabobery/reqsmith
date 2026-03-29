use std::path::PathBuf;
use std::process::ExitCode;

use tokio_util::sync::CancellationToken;

use crate::cli::OutputMode;
use crate::core::models;
use crate::core::repository;
use crate::core::runner::{self, RunOptions};
use crate::output;

pub async fn execute(
    file: PathBuf,
    env: Option<String>,
    vars: Vec<(String, String)>,
    output_mode: OutputMode,
    quiet: bool,
    save: bool,
) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_default();

    let doc = match repository::load_request(&file) {
        Ok(doc) => doc,
        Err(e) => {
            eprintln!("Failed to load {}: {e}", file.display());
            return ExitCode::from(models::ExitCode::InternalError as u8);
        }
    };

    #[cfg(feature = "plugins")]
    let plugin_registry = match crate::plugins::registry::PluginRegistry::discover_and_load(&cwd) {
        Ok(r) if !r.is_empty() => {
            eprintln!("Loaded {} plugin(s)", r.len());
            Some(std::sync::Arc::new(r))
        }
        Ok(_) => None,
        Err(e) => {
            eprintln!("Warning: plugin loading failed: {e}");
            None
        }
    };

    let options = RunOptions {
        env_name: env,
        cli_vars: vars,
        validate_before_run: true,
        cwd,
        #[cfg(feature = "plugins")]
        plugin_registry,
    };
    let client = crate::infra::http_client::build_client();

    // Set up cancellation via Ctrl+C.
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel_clone.cancel();
        }
    });

    let result = runner::run_request(&client, &doc, &options, cancel).await;
    let exit_code = result.exit_code;

    output::print_run_result(&result, &output_mode, quiet);

    if save {
        if let Some(stored) = models::StoredRun::from_run_result(&result) {
            let save_dir = std::env::current_dir().unwrap_or_default();
            match crate::core::storage::save_run(&stored, &save_dir) {
                Ok(path) => eprintln!("Run saved to {}", path.display()),
                Err(e) => eprintln!("Failed to save run: {e}"),
            }
        } else {
            eprintln!("No response to save (request may have failed)");
        }
    }

    ExitCode::from(exit_code)
}
