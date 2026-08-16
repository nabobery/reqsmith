use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[cfg(feature = "plugins")]
pub use plugin_cli::PluginAction;

/// Terminal-native API client.
#[derive(Parser, Debug)]
#[command(name = "reqsmith", version, about)]
pub struct Cli {
    /// Enable debug logging
    #[arg(short, long, global = true)]
    pub debug: bool,

    /// Override TUI tick rate in milliseconds
    #[arg(long)]
    pub tick_rate: Option<u64>,

    /// Override TUI frame rate in milliseconds
    #[arg(long)]
    pub frame_rate: Option<u64>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Execute a saved request without launching the TUI
    Run {
        /// Path to .req.yml request file
        file: PathBuf,

        /// Named environment to use (from reqsmith_envs.yml or .env.<name>)
        #[arg(short, long)]
        env: Option<String>,

        /// Override variables (key=value)
        #[arg(long = "var", value_parser = parse_key_value)]
        vars: Vec<(String, String)>,

        /// Output format
        #[arg(short, long, default_value = "human")]
        output: OutputMode,

        /// Suppress all output except errors
        #[arg(short, long)]
        quiet: bool,

        /// Persist the run result to .reqsmith/runs/ for later diffing
        #[arg(long)]
        save: bool,

        /// Reject requests to private/loopback hosts (basic SSRF guard).
        /// Off by default so localhost/LAN targets keep working.
        #[arg(long)]
        deny_private_networks: bool,
    },

    /// Format request files with canonical YAML ordering
    Fmt {
        /// Files to format (or . to discover all)
        files: Vec<PathBuf>,

        /// Check formatting without writing (exit 1 if unformatted)
        #[arg(long)]
        check: bool,
    },

    /// Validate request file schema and interpolation
    Validate {
        /// Files to validate
        files: Vec<PathBuf>,

        /// Named environment for interpolation checking
        #[arg(short, long)]
        env: Option<String>,

        /// Output format
        #[arg(short, long, default_value = "human")]
        output: OutputMode,
    },

    /// Compare two stored run files
    Diff {
        /// Baseline stored run file
        baseline: PathBuf,
        /// Candidate stored run file
        candidate: PathBuf,
        /// Output format
        #[arg(short, long, default_value = "human")]
        output: OutputMode,
    },

    /// List discovered request files in the current project
    List {
        /// Directory to search (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format
        #[arg(short, long, default_value = "human")]
        output: OutputMode,
    },

    /// Manage plugins
    #[cfg(feature = "plugins")]
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
}

#[cfg(feature = "plugins")]
mod plugin_cli {
    use clap::Subcommand;

    #[derive(Subcommand, Debug)]
    pub enum PluginAction {
        /// List discovered plugins and their status
        List,
        /// Show detailed info about a specific plugin
        Info {
            /// Plugin name
            name: String,
        },
    }
}

/// Output mode for CLI commands.
#[derive(ValueEnum, Debug, Clone, Default, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable output
    #[default]
    Human,
    /// Machine-readable JSON output
    Json,
}

fn parse_key_value(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=VALUE: no `=` found in `{s}`"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn no_subcommand_parses_to_none() {
        let cli = Cli::parse_from(["reqsmith"]);
        assert!(cli.command.is_none());
        assert!(!cli.debug);
        assert_eq!(cli.tick_rate, None);
        assert_eq!(cli.frame_rate, None);
    }

    #[test]
    fn debug_flag_is_global() {
        let cli = Cli::parse_from(["reqsmith", "--debug"]);
        assert!(cli.debug);
        assert!(cli.command.is_none());

        let cli = Cli::parse_from(["reqsmith", "--debug", "list"]);
        assert!(cli.debug);
        assert!(matches!(cli.command, Some(Command::List { .. })));
    }

    #[test]
    fn run_subcommand_parses() {
        let cli = Cli::parse_from([
            "reqsmith",
            "run",
            "requests/test.req.yml",
            "--env",
            "staging",
            "--var",
            "token=abc123",
            "--output",
            "json",
            "--quiet",
        ]);
        match cli.command {
            Some(Command::Run {
                ref file,
                ref env,
                ref vars,
                ref output,
                quiet,
                save,
                deny_private_networks,
            }) => {
                assert_eq!(file, &PathBuf::from("requests/test.req.yml"));
                assert_eq!(env.as_deref(), Some("staging"));
                assert_eq!(vars, &[("token".to_string(), "abc123".to_string())]);
                assert_eq!(output, &OutputMode::Json);
                assert!(quiet);
                assert!(!save);
                assert!(!deny_private_networks);
            }
            _ => panic!("expected Run subcommand"),
        }
    }

    #[test]
    fn run_subcommand_defaults() {
        let cli = Cli::parse_from(["reqsmith", "run", "test.req.yml"]);
        match cli.command {
            Some(Command::Run {
                ref output, quiet, ..
            }) => {
                assert_eq!(output, &OutputMode::Human);
                assert!(!quiet);
            }
            _ => panic!("expected Run subcommand"),
        }
    }

    #[test]
    fn fmt_subcommand_parses() {
        let cli = Cli::parse_from(["reqsmith", "fmt", "a.req.yml", "b.req.yml", "--check"]);
        match cli.command {
            Some(Command::Fmt { ref files, check }) => {
                assert_eq!(files.len(), 2);
                assert!(check);
            }
            _ => panic!("expected Fmt subcommand"),
        }
    }

    #[test]
    fn validate_subcommand_parses() {
        let cli = Cli::parse_from(["reqsmith", "validate", "test.req.yml", "--env", "prod"]);
        match cli.command {
            Some(Command::Validate { ref env, .. }) => {
                assert_eq!(env.as_deref(), Some("prod"));
            }
            _ => panic!("expected Validate subcommand"),
        }
    }

    #[test]
    fn list_subcommand_defaults_to_dot() {
        let cli = Cli::parse_from(["reqsmith", "list"]);
        match cli.command {
            Some(Command::List { ref path, .. }) => {
                assert_eq!(path, &PathBuf::from("."));
            }
            _ => panic!("expected List subcommand"),
        }
    }

    #[test]
    fn var_parsing_requires_equals() {
        let result = parse_key_value("no_equals");
        assert!(result.is_err());
    }

    #[test]
    fn var_parsing_handles_value_with_equals() {
        let (k, v) = parse_key_value("url=http://host:8080/path?a=1").unwrap();
        assert_eq!(k, "url");
        assert_eq!(v, "http://host:8080/path?a=1");
    }

    #[test]
    fn multiple_vars() {
        let cli = Cli::parse_from([
            "reqsmith",
            "run",
            "test.req.yml",
            "--var",
            "a=1",
            "--var",
            "b=2",
        ]);
        match cli.command {
            Some(Command::Run { ref vars, .. }) => {
                assert_eq!(vars.len(), 2);
                assert_eq!(vars[0], ("a".to_string(), "1".to_string()));
                assert_eq!(vars[1], ("b".to_string(), "2".to_string()));
            }
            _ => panic!("expected Run subcommand"),
        }
    }

    #[test]
    fn tui_timing_flags_parse_without_subcommand() {
        let cli = Cli::parse_from(["reqsmith", "--tick-rate", "100", "--frame-rate", "33"]);
        assert_eq!(cli.tick_rate, Some(100));
        assert_eq!(cli.frame_rate, Some(33));
        assert!(cli.command.is_none());
    }
}
