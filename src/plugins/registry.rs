use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use extism::{Manifest, Plugin, Wasm};

use super::config::{DiscoveredConfig, PluginEntry, PluginsConfig, discover_config};
use super::errors::PluginError;
use super::host_fns::{HostContext, HostData};
use super::models::PluginCapability;

/// A loaded and ready-to-call plugin instance.
#[allow(dead_code)]
pub struct LoadedPlugin {
    pub entry: PluginEntry,
    /// Mutex because `Plugin::call` takes `&mut self`.
    plugin: Arc<Mutex<Plugin>>,
    host_context: HostContext,
    timeout_ms: u64,
}

impl LoadedPlugin {
    /// Call an exported function by name with JSON input, returning JSON output.
    pub async fn call(
        &self,
        func_name: &str,
        input: String,
        env_values: HashMap<String, String>,
    ) -> Result<String, PluginError> {
        let plugin = Arc::clone(&self.plugin);
        let host_context = self.host_context.clone();
        let plugin_name = self.entry.name.clone();
        let timeout_ms = self.timeout_ms;
        let func_name = func_name.to_string();

        tokio::task::spawn_blocking(move || {
            let mut guard = plugin.lock().unwrap();
            host_context.set_env_values(env_values);
            guard.call::<&str, String>(&func_name, &input).map_err(|e| {
                let msg = e.to_string();
                if msg.contains("timeout") || msg.contains("deadline") {
                    PluginError::Timeout {
                        name: plugin_name.clone(),
                        timeout_ms,
                    }
                } else {
                    PluginError::ExecutionFailed {
                        name: plugin_name.clone(),
                        message: msg,
                    }
                }
            })
        })
        .await
        .map_err(|e| PluginError::ExecutionFailed {
            name: self.entry.name.clone(),
            message: e.to_string(),
        })?
    }

    /// Check if this plugin exports the given function.
    pub fn has_function(&self, name: &str) -> bool {
        self.plugin.lock().unwrap().function_exists(name)
    }
}

/// Registry of all discovered and loaded plugins.
pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
    #[allow(dead_code)]
    config: PluginsConfig,
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRegistry")
            .field("plugin_count", &self.plugins.len())
            .field(
                "names",
                &self
                    .plugins
                    .iter()
                    .map(|p| p.entry.name.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl PluginRegistry {
    /// Discover plugins from `.hurl/plugins.toml` and load them.
    ///
    /// Returns an empty registry if no config file is found.
    pub fn discover_and_load(cwd: &Path) -> Result<Self, PluginError> {
        let discovered = match discover_config(cwd)? {
            Some(c) => c,
            None => {
                return Ok(Self {
                    plugins: Vec::new(),
                    config: PluginsConfig {
                        api_version: 1,
                        timeout_ms: 5000,
                        memory_limit_pages: 256,
                        plugin: Vec::new(),
                    },
                });
            }
        };
        let config = discovered.config.clone();

        let mut plugins = Vec::new();

        for entry in config.enabled_plugins() {
            let wasm_path = resolve_wasm_path(&discovered, cwd, &entry.path);

            if !wasm_path.is_file() {
                tracing::warn!(
                    plugin = %entry.name,
                    path = %wasm_path.display(),
                    "WASM file not found, skipping plugin"
                );
                continue;
            }

            let host_context = HostContext::new(HostData {
                env_values: HashMap::new(), // populated at call time
                plugin_config: entry.flat_config(),
            });

            let manifest = Manifest::new([Wasm::file(&wasm_path)])
                .with_timeout(std::time::Duration::from_millis(config.timeout_ms))
                .with_memory_max(config.memory_limit_pages);

            let host_functions = host_context.build_functions();

            let plugin = Plugin::new(&manifest, host_functions, true).map_err(|e| {
                PluginError::LoadFailed {
                    name: entry.name.clone(),
                    message: e.to_string(),
                }
            })?;

            plugins.push(LoadedPlugin {
                entry: entry.clone(),
                plugin: Arc::new(Mutex::new(plugin)),
                host_context,
                timeout_ms: config.timeout_ms,
            });

            tracing::info!(
                plugin = %entry.name,
                capabilities = ?entry.capabilities,
                "Loaded plugin"
            );
        }

        Ok(Self { plugins, config })
    }

    /// Iterate over plugins that declare the given capability.
    pub fn plugins_with_capability(
        &self,
        capability: &PluginCapability,
    ) -> impl Iterator<Item = &LoadedPlugin> {
        self.plugins
            .iter()
            .filter(move |p| p.entry.capabilities.contains(capability))
    }

    pub fn plugin_named_with_capability(
        &self,
        name: &str,
        capability: &PluginCapability,
    ) -> Option<&LoadedPlugin> {
        self.plugins.iter().find(|plugin| {
            plugin.entry.name == name && plugin.entry.capabilities.contains(capability)
        })
    }

    /// All loaded plugin names.
    #[allow(dead_code)]
    pub fn plugin_names(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.entry.name.as_str()).collect()
    }

    /// Whether any plugins are loaded.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Number of loaded plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }
}

pub(crate) fn resolve_wasm_path(discovered: &DiscoveredConfig, cwd: &Path, path: &Path) -> PathBuf {
    if let Some(home_relative) = path.to_str().and_then(|value| value.strip_prefix("~/")) {
        return dirs::home_dir()
            .unwrap_or_else(|| cwd.to_path_buf())
            .join(home_relative);
    }

    if path.is_absolute() {
        return path.to_path_buf();
    }

    discovered.path.parent().unwrap_or(cwd).join(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn discover_returns_empty_when_no_config() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = PluginRegistry::discover_and_load(tmp.path()).unwrap();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.plugin_names().is_empty());
    }

    #[test]
    fn discover_skips_missing_wasm_files() {
        let tmp = tempfile::tempdir().unwrap();
        let hurl_dir = tmp.path().join(".hurl");
        std::fs::create_dir_all(&hurl_dir).unwrap();
        std::fs::write(
            hurl_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "ghost"
path = "nonexistent.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();

        let registry = PluginRegistry::discover_and_load(tmp.path()).unwrap();
        assert!(registry.is_empty());
    }

    #[test]
    fn discover_rejects_invalid_api_version() {
        let tmp = tempfile::tempdir().unwrap();
        let hurl_dir = tmp.path().join(".hurl");
        std::fs::create_dir_all(&hurl_dir).unwrap();
        std::fs::write(
            hurl_dir.join("plugins.toml"),
            r#"
api_version = 99
[[plugin]]
name = "test"
path = "test.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();

        let err = PluginRegistry::discover_and_load(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("99"));
    }

    #[test]
    fn resolve_wasm_path_uses_config_directory_for_relative_paths() {
        let discovered = DiscoveredConfig {
            path: PathBuf::from("/tmp/project/.hurl/plugins.toml"),
            config: PluginsConfig {
                api_version: 1,
                timeout_ms: 5000,
                memory_limit_pages: 256,
                plugin: vec![],
            },
        };

        let resolved = resolve_wasm_path(
            &discovered,
            Path::new("/tmp/project"),
            Path::new("plugins/a.wasm"),
        );

        assert_eq!(resolved, PathBuf::from("/tmp/project/.hurl/plugins/a.wasm"));
    }

    #[test]
    fn resolve_wasm_path_expands_home_directory() {
        let home = dirs::home_dir().unwrap();
        let discovered = DiscoveredConfig {
            path: home.join(".config/hurl/plugins.toml"),
            config: PluginsConfig {
                api_version: 1,
                timeout_ms: 5000,
                memory_limit_pages: 256,
                plugin: vec![],
            },
        };

        let resolved = resolve_wasm_path(
            &discovered,
            Path::new("/tmp"),
            Path::new("~/plugins/a.wasm"),
        );

        assert_eq!(resolved, home.join("plugins/a.wasm"));
    }
}
