use std::collections::HashSet;
use std::fmt::Write;
use std::path::Path;
use std::process::ExitCode;

use crate::plugins::config::{DiscoveredConfig, PluginEntry, discover_config};
use crate::plugins::registry::PluginRegistry;
use crate::plugins::registry::resolve_wasm_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginListStatus {
    Loaded,
    Disabled,
    MissingWasm,
    Configured,
    LoadFailed,
}

impl PluginListStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::Disabled => "disabled",
            Self::MissingWasm => "missing_wasm",
            Self::Configured => "configured",
            Self::LoadFailed => "load_failed",
        }
    }
}

fn plugin_status(
    entry: &PluginEntry,
    discovered: &DiscoveredConfig,
    cwd: &Path,
    loaded_names: Option<&HashSet<String>>,
) -> PluginListStatus {
    if !entry.enabled {
        return PluginListStatus::Disabled;
    }

    let wasm_path = resolve_wasm_path(discovered, cwd, &entry.path);
    if !wasm_path.is_file() {
        return PluginListStatus::MissingWasm;
    }

    match loaded_names {
        Some(loaded) if loaded.contains(&entry.name) => PluginListStatus::Loaded,
        Some(_) => PluginListStatus::LoadFailed,
        None => PluginListStatus::Configured,
    }
}

fn render_plugin_list(
    discovered: &DiscoveredConfig,
    cwd: &Path,
    loaded_names: Option<&HashSet<String>>,
) -> String {
    let mut output = String::new();
    let caps_header = "CAPABILITIES";
    writeln!(
        output,
        "{:<20} {:<12} {:<14} {caps_header}",
        "NAME", "ENABLED", "STATUS"
    )
    .unwrap();
    writeln!(output, "{}", "-".repeat(80)).unwrap();

    for entry in &discovered.config.plugin {
        let caps: Vec<&str> = entry
            .capabilities
            .iter()
            .map(|c| match c {
                crate::plugins::models::PluginCapability::PreRequest => "pre_request",
                crate::plugins::models::PluginCapability::PostResponse => "post_response",
                crate::plugins::models::PluginCapability::Authenticate => "authenticate",
                crate::plugins::models::PluginCapability::ProvideVariable => "provide_variable",
            })
            .collect();
        let status = plugin_status(entry, discovered, cwd, loaded_names);
        writeln!(
            output,
            "{:<20} {:<12} {:<14} {}",
            entry.name,
            if entry.enabled { "yes" } else { "no" },
            status.label(),
            caps.join(", ")
        )
        .unwrap();
    }

    output
}

/// Execute `hurl plugin list`.
pub fn execute_list() -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_default();
    let discovered = match discover_config(&cwd) {
        Ok(Some(config)) => config,
        Ok(None) => {
            println!("No plugins found.");
            println!();
            println!("To add plugins, create .hurl/plugins.toml in your project.");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("Error reading plugins config: {e}");
            return ExitCode::from(1);
        }
    };
    let registry = PluginRegistry::discover_and_load(&cwd);
    let load_error = registry.as_ref().err().map(|error| error.to_string());
    let loaded_names = registry.ok().map(|registry| {
        registry
            .plugin_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<HashSet<_>>()
    });

    print!(
        "{}",
        render_plugin_list(&discovered, &cwd, loaded_names.as_ref())
    );

    if let Some(error) = load_error {
        eprintln!("Warning: one or more plugins could not be loaded: {error}");
    }

    ExitCode::SUCCESS
}

/// Execute `hurl plugin info <name>`.
pub fn execute_info(name: String) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_default();
    let discovered = match discover_config(&cwd) {
        Ok(Some(config)) => config,
        Ok(None) => {
            eprintln!("No plugins found.");
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("Error reading plugins config: {e}");
            return ExitCode::from(1);
        }
    };
    let registry = PluginRegistry::discover_and_load(&cwd);
    let loaded_names = registry.as_ref().ok().map(|registry| {
        registry
            .plugin_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<HashSet<_>>()
    });
    let load_error = registry.err().map(|error| error.to_string());

    let entry = discovered.config.plugin.iter().find(|e| e.name == name);
    match entry {
        Some(entry) => {
            let wasm_path = resolve_wasm_path(&discovered, &cwd, &entry.path);
            let status = plugin_status(entry, &discovered, &cwd, loaded_names.as_ref());
            println!("Name:         {}", entry.name);
            println!("Path:         {}", wasm_path.display());
            println!("Enabled:      {}", entry.enabled);
            println!("Status:       {}", status.label());
            println!(
                "Capabilities: {}",
                entry
                    .capabilities
                    .iter()
                    .map(|c| format!("{c:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if !entry.config.is_empty() {
                println!("Config:");
                for (k, v) in &entry.config {
                    println!("  {k} = {v}");
                }
            }
            if let Ok(meta) = std::fs::metadata(&wasm_path) {
                let size_kb = meta.len() / 1024;
                println!("WASM size:    {size_kb} KB");
            }
            if let Some(error) = load_error {
                println!("Load note:    {error}");
            }
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("Plugin '{name}' not found.");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::plugins::config::PluginsConfig;
    use crate::plugins::models::PluginCapability;

    use super::*;

    fn discovered_config(root: &Path) -> DiscoveredConfig {
        DiscoveredConfig {
            path: root.join(".hurl/plugins.toml"),
            config: PluginsConfig {
                api_version: 1,
                timeout_ms: 5000,
                memory_limit_pages: 256,
                plugin: vec![],
            },
        }
    }

    fn plugin_entry(name: &str, path: &str, enabled: bool) -> PluginEntry {
        PluginEntry {
            name: name.into(),
            path: PathBuf::from(path),
            enabled,
            capabilities: vec![PluginCapability::PreRequest],
            config: HashMap::new(),
        }
    }

    #[test]
    fn plugin_status_reports_disabled_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("disabled", "plugins/test.wasm", false);

        assert_eq!(
            plugin_status(&entry, &discovered, tmp.path(), None),
            PluginListStatus::Disabled
        );
    }

    #[test]
    fn plugin_status_reports_missing_wasm() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("missing", "plugins/test.wasm", true);

        assert_eq!(
            plugin_status(&entry, &discovered, tmp.path(), None),
            PluginListStatus::MissingWasm
        );
    }

    #[test]
    fn plugin_status_reports_loaded_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let hurl_dir = tmp.path().join(".hurl/plugins");
        std::fs::create_dir_all(&hurl_dir).unwrap();
        std::fs::write(hurl_dir.join("loaded.wasm"), b"wasm").unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("loaded", "plugins/loaded.wasm", true);
        let loaded_names = HashSet::from([String::from("loaded")]);

        assert_eq!(
            plugin_status(&entry, &discovered, tmp.path(), Some(&loaded_names)),
            PluginListStatus::Loaded
        );
    }

    #[test]
    fn plugin_status_reports_load_failure_for_present_but_unloaded_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let hurl_dir = tmp.path().join(".hurl/plugins");
        std::fs::create_dir_all(&hurl_dir).unwrap();
        std::fs::write(hurl_dir.join("broken.wasm"), b"wasm").unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("broken", "plugins/broken.wasm", true);
        let loaded_names = HashSet::new();

        assert_eq!(
            plugin_status(&entry, &discovered, tmp.path(), Some(&loaded_names)),
            PluginListStatus::LoadFailed
        );
    }

    #[test]
    fn render_plugin_list_includes_status_column() {
        let tmp = tempfile::tempdir().unwrap();
        let hurl_dir = tmp.path().join(".hurl/plugins");
        std::fs::create_dir_all(&hurl_dir).unwrap();
        std::fs::write(hurl_dir.join("loaded.wasm"), b"wasm").unwrap();

        let mut discovered = discovered_config(tmp.path());
        discovered.config.plugin = vec![
            plugin_entry("loaded", "plugins/loaded.wasm", true),
            plugin_entry("disabled", "plugins/missing.wasm", false),
        ];
        let loaded_names = HashSet::from([String::from("loaded")]);

        let rendered = render_plugin_list(&discovered, tmp.path(), Some(&loaded_names));

        assert!(rendered.contains("STATUS"));
        assert!(rendered.contains("loaded"));
        assert!(rendered.contains("disabled"));
    }
}
