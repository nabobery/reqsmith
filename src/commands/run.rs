use std::process::ExitCode;

use tokio_util::sync::CancellationToken;

use crate::cli::RunArgs;
use crate::core::models;
use crate::core::repository;
use crate::core::runner::{self, RunOptions};
use crate::output;

pub async fn execute(args: RunArgs) -> ExitCode {
    let RunArgs {
        file,
        env,
        vars,
        output: output_mode,
        quiet,
        save,
        deny_private_networks,
    } = args;
    let deny_private_networks =
        deny_private_networks || crate::core::execution::deny_private_networks_from_env();
    let cwd = std::env::current_dir().unwrap_or_default();

    let doc = match repository::load_request(&file) {
        Ok(doc) => doc,
        Err(e) => {
            eprintln!("Failed to load {}: {e}", file.display());
            return ExitCode::from(models::ExitCode::InternalError as u8);
        }
    };

    #[cfg(feature = "plugins")]
    let plugin_registry = {
        let registry = crate::plugins::registry::PluginRegistry::discover_and_load(&cwd);
        for line in plugin_warnings(
            registry.blocked_project_config(),
            registry.config_errors(),
            registry.load_errors(),
        ) {
            eprintln!("{line}");
        }
        if registry.is_empty() {
            None
        } else {
            eprintln!("Loaded {} plugin(s)", registry.len());
            Some(std::sync::Arc::new(registry))
        }
    };

    let options = RunOptions {
        env_name: env,
        cli_vars: vars,
        validate_before_run: true,
        cwd,
        deny_private_networks,
        #[cfg(feature = "plugins")]
        plugin_registry,
    };
    let client = match crate::infra::http_client::build_client(deny_private_networks) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Failed to build HTTP client: {e}");
            return ExitCode::from(models::ExitCode::InternalError as u8);
        }
    };

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
            match crate::core::storage::save_run(&save_dir, stored) {
                Ok(path) => eprintln!("Run saved to {}", path.display()),
                Err(e) => eprintln!("Failed to save run: {e}"),
            }
        } else {
            eprintln!("No response to save (request may have failed)");
        }
    }

    ExitCode::from(exit_code)
}

/// One warning line per plugin config reqsmith refused to load, per config
/// that failed to read/parse/validate, and per entry that failed.
/// Rendered rather than printed so the wording is testable.
#[cfg(feature = "plugins")]
pub(crate) fn plugin_warnings(
    blocked_project_config: Option<&std::path::Path>,
    config_errors: &[(std::path::PathBuf, crate::plugins::errors::PluginError)],
    load_errors: &[crate::plugins::registry::PluginLoadError],
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(path) = blocked_project_config {
        lines.push(format!(
            "Warning: skipped project plugin config {}; set REQSMITH_ALLOW_PROJECT_PLUGINS=1 to load it",
            path.display()
        ));
    }
    lines.extend(config_errors.iter().map(|(path, error)| {
        format!(
            "Warning: plugin config {} failed to load: {error}",
            path.display()
        )
    }));
    lines.extend(load_errors.iter().map(|failure| {
        format!(
            "Warning: plugin '{}' from {} failed to load: {}",
            failure.name,
            failure.config_path.display(),
            failure.error
        )
    }));
    lines
}

#[cfg(all(test, feature = "plugins"))]
mod tests {
    use super::*;
    use crate::plugins::errors::PluginError;
    use crate::plugins::registry::PluginLoadError;
    use std::path::Path;

    #[test]
    fn plugin_warnings_are_empty_when_nothing_was_blocked_or_failed() {
        assert!(plugin_warnings(None, &[], &[]).is_empty());
    }

    #[test]
    fn plugin_warnings_name_the_blocked_config_and_the_opt_in_variable() {
        let lines = plugin_warnings(Some(Path::new("/proj/.reqsmith/plugins.toml")), &[], &[]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("/proj/.reqsmith/plugins.toml"));
        assert!(lines[0].contains("REQSMITH_ALLOW_PROJECT_PLUGINS=1"));
    }

    #[test]
    fn plugin_warnings_report_one_line_per_config_error() {
        let config_errors = vec![(
            std::path::PathBuf::from("/proj/.reqsmith/plugins.toml"),
            PluginError::ManifestError("bad toml".into()),
        )];
        let lines = plugin_warnings(None, &config_errors, &[]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("/proj/.reqsmith/plugins.toml"));
        assert!(lines[0].contains("bad toml"));
    }

    #[test]
    fn plugin_warnings_report_one_line_per_failed_entry() {
        let errors = vec![
            PluginLoadError {
                config_path: "/global/plugins.toml".into(),
                name: "auth".to_string(),
                error: PluginError::LoadFailed {
                    name: "auth".into(),
                    message: "file not found".into(),
                },
            },
            PluginLoadError {
                config_path: "/project/plugins.toml".into(),
                name: "vars".to_string(),
                error: PluginError::UnsupportedApiVersion {
                    version: 99,
                    supported: 1,
                },
            },
        ];
        let lines = plugin_warnings(None, &[], &errors);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("auth") && lines[0].contains("file not found"));
        assert!(lines[1].contains("vars") && lines[1].contains("99"));
    }
}
