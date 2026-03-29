use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::errors::PluginError;
use super::models::PluginCapability;

/// Current supported plugin API version.
pub const CURRENT_API_VERSION: u32 = 1;

/// Top-level structure of `.hurl/plugins.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginsConfig {
    pub api_version: u32,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_memory")]
    pub memory_limit_pages: u32,
    #[serde(default)]
    pub plugin: Vec<PluginEntry>,
}

/// A single plugin entry within the config.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginEntry {
    pub name: String,
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub capabilities: Vec<PluginCapability>,
    #[serde(default)]
    pub config: HashMap<String, toml::Value>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredConfig {
    pub path: PathBuf,
    pub config: PluginsConfig,
}

fn default_timeout() -> u64 {
    5000
}

fn default_memory() -> u32 {
    256
}

fn default_true() -> bool {
    true
}

impl PluginsConfig {
    /// Validate the config against the current API version.
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.api_version != CURRENT_API_VERSION {
            return Err(PluginError::UnsupportedApiVersion {
                version: self.api_version,
                supported: CURRENT_API_VERSION,
            });
        }
        for entry in &self.plugin {
            if entry.capabilities.is_empty() {
                return Err(PluginError::ManifestError(format!(
                    "Plugin '{}' declares no capabilities",
                    entry.name
                )));
            }
        }
        Ok(())
    }

    /// Return only enabled plugin entries.
    pub fn enabled_plugins(&self) -> impl Iterator<Item = &PluginEntry> {
        self.plugin.iter().filter(|p| p.enabled)
    }
}

impl PluginEntry {
    /// Flatten the TOML config values into a `HashMap<String, String>`.
    pub fn flat_config(&self) -> HashMap<String, String> {
        self.config
            .iter()
            .map(|(k, v)| {
                let val = match v {
                    toml::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), val)
            })
            .collect()
    }
}

/// Discover and parse the plugins config from the filesystem.
///
/// Search order:
/// 1. `$cwd/.hurl/plugins.toml`
/// 2. `$XDG_CONFIG_HOME/hurl/plugins.toml` (via `dirs::config_dir`)
///
/// Returns `None` if no config file is found.
pub fn discover_config(cwd: &Path) -> Result<Option<DiscoveredConfig>, PluginError> {
    let candidates = [
        Some(cwd.join(".hurl").join("plugins.toml")),
        dirs::config_dir().map(|d| d.join("hurl").join("plugins.toml")),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.is_file() {
            let content = std::fs::read_to_string(&candidate).map_err(|e| {
                PluginError::ManifestError(format!("Failed to read {}: {e}", candidate.display()))
            })?;
            let config: PluginsConfig = toml::from_str(&content).map_err(|e| {
                PluginError::ManifestError(format!("Failed to parse {}: {e}", candidate.display()))
            })?;
            config.validate()?;
            return Ok(Some(DiscoveredConfig {
                path: candidate,
                config,
            }));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_config() {
        let toml_str = r#"
api_version = 1
timeout_ms = 3000
memory_limit_pages = 128

[[plugin]]
name = "auth-sigv4"
path = "plugins/auth.wasm"
enabled = true
capabilities = ["authenticate"]

[plugin.config]
region = "us-east-1"
service = "execute-api"

[[plugin]]
name = "vars"
path = "plugins/vars.wasm"
capabilities = ["provide_variable"]
"#;
        let config: PluginsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.api_version, 1);
        assert_eq!(config.timeout_ms, 3000);
        assert_eq!(config.memory_limit_pages, 128);
        assert_eq!(config.plugin.len(), 2);

        let first = &config.plugin[0];
        assert_eq!(first.name, "auth-sigv4");
        assert!(first.enabled);
        assert_eq!(first.capabilities, vec![PluginCapability::Authenticate]);
        assert_eq!(
            first.config.get("region").unwrap().as_str().unwrap(),
            "us-east-1"
        );

        let second = &config.plugin[1];
        assert_eq!(second.name, "vars");
        assert!(second.enabled); // default
        assert_eq!(second.capabilities, vec![PluginCapability::ProvideVariable]);
    }

    #[test]
    fn parse_defaults_applied() {
        let toml_str = r#"
api_version = 1

[[plugin]]
name = "test"
path = "test.wasm"
capabilities = ["pre_request"]
"#;
        let config: PluginsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.timeout_ms, 5000);
        assert_eq!(config.memory_limit_pages, 256);
        assert!(config.plugin[0].enabled);
    }

    #[test]
    fn validate_rejects_wrong_api_version() {
        let config = PluginsConfig {
            api_version: 99,
            timeout_ms: 5000,
            memory_limit_pages: 256,
            plugin: vec![],
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("99"));
    }

    #[test]
    fn validate_rejects_no_capabilities() {
        let config = PluginsConfig {
            api_version: 1,
            timeout_ms: 5000,
            memory_limit_pages: 256,
            plugin: vec![PluginEntry {
                name: "bad".into(),
                path: "bad.wasm".into(),
                enabled: true,
                capabilities: vec![],
                config: HashMap::new(),
            }],
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no capabilities"));
    }

    #[test]
    fn enabled_plugins_filters_disabled() {
        let config = PluginsConfig {
            api_version: 1,
            timeout_ms: 5000,
            memory_limit_pages: 256,
            plugin: vec![
                PluginEntry {
                    name: "on".into(),
                    path: "on.wasm".into(),
                    enabled: true,
                    capabilities: vec![PluginCapability::PreRequest],
                    config: HashMap::new(),
                },
                PluginEntry {
                    name: "off".into(),
                    path: "off.wasm".into(),
                    enabled: false,
                    capabilities: vec![PluginCapability::PreRequest],
                    config: HashMap::new(),
                },
            ],
        };
        let names: Vec<_> = config.enabled_plugins().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["on"]);
    }

    #[test]
    fn flat_config_converts_values() {
        let entry = PluginEntry {
            name: "test".into(),
            path: "test.wasm".into(),
            enabled: true,
            capabilities: vec![PluginCapability::Authenticate],
            config: HashMap::from([
                ("region".into(), toml::Value::String("us-east-1".into())),
                ("retry".into(), toml::Value::Integer(3)),
            ]),
        };
        let flat = entry.flat_config();
        assert_eq!(flat.get("region").unwrap(), "us-east-1");
        assert_eq!(flat.get("retry").unwrap(), "3");
    }

    #[test]
    fn discover_config_returns_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = discover_config(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn discover_config_finds_project_local() {
        let tmp = tempfile::tempdir().unwrap();
        let hurl_dir = tmp.path().join(".hurl");
        std::fs::create_dir_all(&hurl_dir).unwrap();
        std::fs::write(
            hurl_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "test"
path = "test.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();

        let discovered = discover_config(tmp.path()).unwrap().unwrap();
        assert_eq!(discovered.path, hurl_dir.join("plugins.toml"));
        assert_eq!(discovered.config.plugin.len(), 1);
        assert_eq!(discovered.config.plugin[0].name, "test");
    }

    #[test]
    fn discover_config_rejects_bad_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let hurl_dir = tmp.path().join(".hurl");
        std::fs::create_dir_all(&hurl_dir).unwrap();
        std::fs::write(hurl_dir.join("plugins.toml"), "this is not toml {{{{").unwrap();

        let err = discover_config(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("parse"));
    }

    #[test]
    fn multiple_capabilities_parse() {
        let toml_str = r#"
api_version = 1

[[plugin]]
name = "multi"
path = "multi.wasm"
capabilities = ["pre_request", "post_response", "authenticate", "provide_variable"]
"#;
        let config: PluginsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.plugin[0].capabilities.len(), 4);
    }
}
