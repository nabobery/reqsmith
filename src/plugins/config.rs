use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::errors::PluginError;
use super::models::PluginCapability;
use crate::core::redaction::RedactionPolicy;

/// Current supported plugin API version.
pub const CURRENT_API_VERSION: u32 = 1;

/// Top-level structure of `.reqsmith/plugins.toml`.
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
    /// Optional SHA-256 hex digest of the WASM file. When set, the digest of
    /// the on-disk file is verified before the plugin is loaded; a mismatch
    /// aborts the load with `PluginError::IntegrityMismatch`. See
    /// `docs/plugins-security.md` for the full trust model.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Environment variable names this plugin is allowed to read via
    /// `provide_env_var`. Defaults to empty, meaning the plugin receives NO
    /// environment values (least privilege / default-deny). Names not
    /// present here are silently withheld even if requested by the plugin.
    #[serde(default)]
    pub env_allowlist: Vec<String>,
}

/// Which of the two search locations a `plugins.toml` was found at.
///
/// This distinction drives the C1 consent gate: `ProjectLocal` configs can
/// ship inside a cloned repository and therefore describe untrusted WASM,
/// while `UserGlobal` configs are placed by the user themselves and are
/// trusted as today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// `$cwd/.reqsmith/plugins.toml` — untrusted, may come from a cloned repo.
    ProjectLocal,
    /// `$XDG_CONFIG_HOME/reqsmith/plugins.toml` — trusted, placed by the user.
    UserGlobal,
}

#[derive(Debug, Clone)]
pub struct DiscoveredConfig {
    pub path: PathBuf,
    pub config: PluginsConfig,
    pub source: ConfigSource,
}

/// Result of discovering `plugins.toml` candidates: the configs that parsed
/// and validated, plus the per-candidate failures. Kept separate (rather than
/// a single `Result`) so one bad config never hides the others — see
/// "Per-entry failure isolation" in `docs/plugins-security.md`.
#[derive(Debug, Default)]
pub struct DiscoveredConfigs {
    pub configs: Vec<DiscoveredConfig>,
    pub errors: Vec<(PathBuf, PluginError)>,
}

fn default_timeout() -> u64 {
    5000
}

fn default_memory() -> u32 {
    256
}

/// Upper bound on `timeout_ms`. A larger wall-clock budget stops being a
/// resource guard and just hangs the request pipeline.
pub const MAX_TIMEOUT_MS: u64 = 30_000;

/// Upper bound on `memory_limit_pages` (1024 * 64KiB = 64MiB).
pub const MAX_MEMORY_LIMIT_PAGES: u32 = 1024;

fn default_true() -> bool {
    true
}

/// Normalize a declared `sha256`: trim, drop an optional `sha256:` prefix,
/// lowercase. `None` unless the result is exactly 64 hex characters.
pub(crate) fn normalized_sha256(declared: &str) -> Option<String> {
    let value = declared.trim();
    let value = value.strip_prefix("sha256:").unwrap_or(value).trim();
    (value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

/// The error a malformed `sha256` produces, shared by `validate` and the
/// load-time integrity check so both report the format rule identically.
pub(crate) fn malformed_sha256_error(name: &str) -> PluginError {
    PluginError::ManifestError(format!(
        "Plugin '{name}' has a malformed sha256: expected 64 hex characters, \
         optionally prefixed with 'sha256:'"
    ))
}

/// Case-insensitive heuristic for whether an env-var-style name (e.g.
/// `AWS_SECRET_ACCESS_KEY`) looks like it holds a secret. `RedactionPolicy::
/// is_sensitive` alone misses these: it matches HTTP *header* names exactly
/// (`authorization`, `x-api-key`, ...), which env var names never equal.
/// Falls back to `policy` so anything it already flags stays flagged.
pub fn looks_like_secret_name(name: &str, policy: &RedactionPolicy) -> bool {
    const TOKENS: &[&str] = &[
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "PRIVATE_KEY",
        "SESSION",
        "API_KEY",
    ];
    let upper = name.to_ascii_uppercase();
    TOKENS.iter().any(|token| upper.contains(token))
        || upper.ends_with("_KEY")
        || policy.is_sensitive(name)
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
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS {
            return Err(PluginError::ManifestError(format!(
                "timeout_ms must be between 1 and {MAX_TIMEOUT_MS} (got {})",
                self.timeout_ms
            )));
        }
        if self.memory_limit_pages == 0 || self.memory_limit_pages > MAX_MEMORY_LIMIT_PAGES {
            return Err(PluginError::ManifestError(format!(
                "memory_limit_pages must be between 1 and {MAX_MEMORY_LIMIT_PAGES} (got {})",
                self.memory_limit_pages
            )));
        }
        for entry in &self.plugin {
            if entry.capabilities.is_empty() {
                return Err(PluginError::ManifestError(format!(
                    "Plugin '{}' declares no capabilities",
                    entry.name
                )));
            }
            if let Some(declared) = &entry.sha256
                && normalized_sha256(declared).is_none()
            {
                return Err(malformed_sha256_error(&entry.name));
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

/// Environment variable that opts in to loading PROJECT-LOCAL plugin
/// configs (`$cwd/.reqsmith/plugins.toml`). See `docs/plugins-security.md`.
pub const ALLOW_PROJECT_PLUGINS_VAR: &str = "REQSMITH_ALLOW_PROJECT_PLUGINS";

/// Pure decision function for the C1 consent gate: does the given value of
/// `REQSMITH_ALLOW_PROJECT_PLUGINS` opt in to loading project-local plugins?
///
/// Accepts `1`, `true`, or `yes`, case-insensitively (surrounding whitespace
/// tolerated). Anything else — including an unset/absent variable — is
/// treated as "not opted in". Kept as a pure function of `Option<&str>` so
/// it can be unit-tested without touching real process environment state
/// (which would race across parallel tests).
pub fn project_plugins_allowed_from(var: Option<&str>) -> bool {
    matches!(
        var.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

/// The `plugins.toml` locations reqsmith consults, in precedence order:
/// project-local first, then user-global.
pub fn config_candidates(cwd: &Path, config_dir: Option<&Path>) -> Vec<(PathBuf, ConfigSource)> {
    let mut candidates = vec![(
        cwd.join(".reqsmith").join("plugins.toml"),
        ConfigSource::ProjectLocal,
    )];
    if let Some(dir) = config_dir {
        candidates.push((
            dir.join("reqsmith").join("plugins.toml"),
            ConfigSource::UserGlobal,
        ));
    }
    candidates
}

/// Discover and parse **every** `plugins.toml` that exists, project-local
/// first. Both locations are returned: a project-local config must not be
/// able to suppress the user's own global plugins just by existing. This
/// function does NOT apply the project-local consent gate (C1) — callers that
/// go on to load/execute WASM are responsible for checking
/// `DiscoveredConfig::source` and gating accordingly.
///
/// Discovery is fault-tolerant **per candidate**: an unreadable,
/// malformed, or otherwise invalid `plugins.toml` at one location is recorded
/// in `errors` and does not stop the other location(s) — typically the
/// user-global config at `$XDG_CONFIG_HOME/reqsmith/plugins.toml` (on macOS,
/// `~/Library/Application Support/reqsmith/plugins.toml`) — from being read.
/// A malicious or broken project-local config must not be able to make the
/// user's own trusted plugins disappear just by existing.
pub fn discover_configs(cwd: &Path) -> DiscoveredConfigs {
    discover_configs_in(cwd, dirs::config_dir().as_deref())
}

/// Same as [`discover_configs`], with the user-global config directory
/// injected so tests need not touch `XDG_CONFIG_HOME`.
pub(crate) fn discover_configs_in(cwd: &Path, config_dir: Option<&Path>) -> DiscoveredConfigs {
    let mut configs = Vec::new();
    let mut errors = Vec::new();

    for (candidate, source) in config_candidates(cwd, config_dir) {
        if !candidate.is_file() {
            continue;
        }
        let content = match std::fs::read_to_string(&candidate) {
            Ok(content) => content,
            Err(e) => {
                errors.push((
                    candidate.clone(),
                    PluginError::ManifestError(format!(
                        "Failed to read {}: {e}",
                        candidate.display()
                    )),
                ));
                continue;
            }
        };
        let config: PluginsConfig = match toml::from_str(&content) {
            Ok(config) => config,
            Err(e) => {
                errors.push((
                    candidate.clone(),
                    PluginError::ManifestError(format!(
                        "Failed to parse {}: {e}",
                        candidate.display()
                    )),
                ));
                continue;
            }
        };
        if let Err(e) = config.validate() {
            errors.push((candidate.clone(), e));
            continue;
        }
        configs.push(DiscoveredConfig {
            path: candidate,
            config,
            source,
        });
    }

    DiscoveredConfigs { configs, errors }
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
                sha256: None,
                env_allowlist: vec![],
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
                    sha256: None,
                    env_allowlist: vec![],
                },
                PluginEntry {
                    name: "off".into(),
                    path: "off.wasm".into(),
                    enabled: false,
                    capabilities: vec![PluginCapability::PreRequest],
                    config: HashMap::new(),
                    sha256: None,
                    env_allowlist: vec![],
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
            sha256: None,
            env_allowlist: vec![],
        };
        let flat = entry.flat_config();
        assert_eq!(flat.get("region").unwrap(), "us-east-1");
        assert_eq!(flat.get("retry").unwrap(), "3");
    }

    #[test]
    fn discover_configs_returns_empty_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = discover_configs_in(tmp.path(), None);
        assert!(result.configs.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn discover_configs_finds_project_local() {
        let tmp = tempfile::tempdir().unwrap();
        let reqsmith_dir = tmp.path().join(".reqsmith");
        std::fs::create_dir_all(&reqsmith_dir).unwrap();
        std::fs::write(
            reqsmith_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "test"
path = "test.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();

        let discovered = discover_configs_in(tmp.path(), None);
        assert!(discovered.errors.is_empty());
        assert_eq!(discovered.configs.len(), 1);
        assert_eq!(
            discovered.configs[0].path,
            reqsmith_dir.join("plugins.toml")
        );
        assert_eq!(discovered.configs[0].config.plugin.len(), 1);
        assert_eq!(discovered.configs[0].config.plugin[0].name, "test");
        assert_eq!(discovered.configs[0].source, ConfigSource::ProjectLocal);
    }

    #[test]
    fn discover_configs_records_bad_toml_as_a_per_candidate_error() {
        let tmp = tempfile::tempdir().unwrap();
        let reqsmith_dir = tmp.path().join(".reqsmith");
        std::fs::create_dir_all(&reqsmith_dir).unwrap();
        std::fs::write(reqsmith_dir.join("plugins.toml"), "this is not toml {{{{").unwrap();

        let discovered = discover_configs_in(tmp.path(), None);
        assert!(discovered.configs.is_empty());
        assert_eq!(discovered.errors.len(), 1);
        assert_eq!(discovered.errors[0].0, reqsmith_dir.join("plugins.toml"));
        assert!(discovered.errors[0].1.to_string().contains("parse"));
    }

    #[test]
    fn discover_configs_returns_both_locations_project_local_first() {
        let project = tempfile::tempdir().unwrap();
        let config_home = tempfile::tempdir().unwrap();

        let project_dir = project.path().join(".reqsmith");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "from-project"
path = "p.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();

        let global_dir = config_home.path().join("reqsmith");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::write(
            global_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "from-global"
path = "g.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();

        let discovered = discover_configs_in(project.path(), Some(config_home.path()));
        assert!(discovered.errors.is_empty());
        assert_eq!(discovered.configs.len(), 2);
        assert_eq!(discovered.configs[0].source, ConfigSource::ProjectLocal);
        assert_eq!(discovered.configs[0].config.plugin[0].name, "from-project");
        assert_eq!(discovered.configs[1].source, ConfigSource::UserGlobal);
        assert_eq!(discovered.configs[1].config.plugin[0].name, "from-global");
    }

    // ------------------------------------------------------------------
    // per-candidate discovery fault tolerance
    // ------------------------------------------------------------------

    #[test]
    fn invalid_project_local_does_not_hide_a_valid_user_global_config() {
        let project = tempfile::tempdir().unwrap();
        let config_home = tempfile::tempdir().unwrap();

        let project_dir = project.path().join(".reqsmith");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join("plugins.toml"), "not valid toml {{{{").unwrap();

        let global_dir = config_home.path().join("reqsmith");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::write(
            global_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "from-global"
path = "g.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();

        let discovered = discover_configs_in(project.path(), Some(config_home.path()));

        assert_eq!(discovered.configs.len(), 1);
        assert_eq!(discovered.configs[0].source, ConfigSource::UserGlobal);
        assert_eq!(discovered.configs[0].config.plugin[0].name, "from-global");

        assert_eq!(discovered.errors.len(), 1);
        assert_eq!(discovered.errors[0].0, project_dir.join("plugins.toml"));
    }

    #[test]
    fn invalid_api_version_is_a_per_candidate_error_not_a_hard_abort() {
        let tmp = tempfile::tempdir().unwrap();
        let reqsmith_dir = tmp.path().join(".reqsmith");
        std::fs::create_dir_all(&reqsmith_dir).unwrap();
        std::fs::write(
            reqsmith_dir.join("plugins.toml"),
            r#"
api_version = 99
[[plugin]]
name = "test"
path = "test.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();

        let discovered = discover_configs_in(tmp.path(), None);
        assert!(discovered.configs.is_empty());
        assert_eq!(discovered.errors.len(), 1);
        assert!(discovered.errors[0].1.to_string().contains("99"));
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

    #[test]
    fn sha256_and_env_allowlist_default_to_absent_and_empty() {
        let toml_str = r#"
api_version = 1

[[plugin]]
name = "test"
path = "test.wasm"
capabilities = ["pre_request"]
"#;
        let config: PluginsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.plugin[0].sha256, None);
        assert!(config.plugin[0].env_allowlist.is_empty());
    }

    #[test]
    fn sha256_and_env_allowlist_parse_when_present() {
        let toml_str = r#"
api_version = 1

[[plugin]]
name = "test"
path = "test.wasm"
capabilities = ["pre_request"]
sha256 = "deadbeef"
env_allowlist = ["API_TOKEN", "OTHER"]
"#;
        let config: PluginsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.plugin[0].sha256.as_deref(), Some("deadbeef"));
        assert_eq!(
            config.plugin[0].env_allowlist,
            vec!["API_TOKEN".to_string(), "OTHER".to_string()]
        );
    }

    // ------------------------------------------------------------------
    // C1: project-local consent gate
    // ------------------------------------------------------------------

    #[test]
    fn project_plugins_allowed_from_accepts_truthy_values_case_insensitively() {
        assert!(project_plugins_allowed_from(Some("1")));
        assert!(project_plugins_allowed_from(Some("true")));
        assert!(project_plugins_allowed_from(Some("TRUE")));
        assert!(project_plugins_allowed_from(Some("True")));
        assert!(project_plugins_allowed_from(Some("yes")));
        assert!(project_plugins_allowed_from(Some("YES")));
        assert!(project_plugins_allowed_from(Some(" 1 ")));
    }

    #[test]
    fn project_plugins_allowed_from_rejects_absent_or_other_values() {
        assert!(!project_plugins_allowed_from(None));
        assert!(!project_plugins_allowed_from(Some("")));
        assert!(!project_plugins_allowed_from(Some("0")));
        assert!(!project_plugins_allowed_from(Some("false")));
        assert!(!project_plugins_allowed_from(Some("no")));
        assert!(!project_plugins_allowed_from(Some("maybe")));
    }

    #[test]
    fn config_candidates_are_ordered_project_local_then_user_global() {
        let candidates = config_candidates(Path::new("/proj"), Some(Path::new("/home/.config")));
        assert_eq!(
            candidates,
            vec![
                (
                    PathBuf::from("/proj/.reqsmith/plugins.toml"),
                    ConfigSource::ProjectLocal
                ),
                (
                    PathBuf::from("/home/.config/reqsmith/plugins.toml"),
                    ConfigSource::UserGlobal
                ),
            ]
        );
    }

    #[test]
    fn config_candidates_omit_user_global_without_a_config_dir() {
        let candidates = config_candidates(Path::new("/proj"), None);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].1, ConfigSource::ProjectLocal);
    }

    // ------------------------------------------------------------------
    // env-var-name secret heuristic
    // ------------------------------------------------------------------

    #[test]
    fn looks_like_secret_name_flags_common_secret_style_env_vars() {
        let policy = RedactionPolicy::default();
        for name in [
            "AWS_SECRET_ACCESS_KEY",
            "GITHUB_TOKEN",
            "MY_API_KEY",
            "DB_PASSWORD",
            "DB_PASSWD",
            "GCP_CREDENTIALS_JSON",
            "TLS_PRIVATE_KEY",
            "SESSION_ID",
        ] {
            assert!(looks_like_secret_name(name, &policy), "{name}");
        }
    }

    #[test]
    fn looks_like_secret_name_does_not_flag_ordinary_names() {
        let policy = RedactionPolicy::default();
        for name in ["REGION", "SERVICE", "HOST", "PORT"] {
            assert!(!looks_like_secret_name(name, &policy), "{name}");
        }
    }

    #[test]
    fn looks_like_secret_name_falls_back_to_the_redaction_policy() {
        // Not caught by the token/substring heuristic, but an exact header
        // name the redaction policy already treats as sensitive.
        let policy = RedactionPolicy::default();
        assert!(looks_like_secret_name("authorization", &policy));
    }

    // ------------------------------------------------------------------
    // Resource-limit validation
    // ------------------------------------------------------------------

    fn config_with_limits(timeout_ms: u64, memory_limit_pages: u32) -> PluginsConfig {
        PluginsConfig {
            api_version: 1,
            timeout_ms,
            memory_limit_pages,
            plugin: vec![],
        }
    }

    #[test]
    fn validate_rejects_out_of_range_timeout() {
        for timeout in [0, MAX_TIMEOUT_MS + 1] {
            let err = config_with_limits(timeout, 256).validate().unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("timeout_ms"), "{msg}");
            assert!(msg.contains(&MAX_TIMEOUT_MS.to_string()), "{msg}");
        }
        assert!(config_with_limits(MAX_TIMEOUT_MS, 256).validate().is_ok());
    }

    #[test]
    fn validate_rejects_out_of_range_memory_limit() {
        for pages in [0, MAX_MEMORY_LIMIT_PAGES + 1] {
            let err = config_with_limits(5000, pages).validate().unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("memory_limit_pages"), "{msg}");
            assert!(msg.contains(&MAX_MEMORY_LIMIT_PAGES.to_string()), "{msg}");
        }
        assert!(
            config_with_limits(5000, MAX_MEMORY_LIMIT_PAGES)
                .validate()
                .is_ok()
        );
    }

    // ------------------------------------------------------------------
    // Digest format
    // ------------------------------------------------------------------

    #[test]
    fn normalized_sha256_accepts_prefix_whitespace_and_uppercase() {
        let digest = "a".repeat(64);
        assert_eq!(normalized_sha256(&digest).as_deref(), Some(digest.as_str()));
        assert_eq!(
            normalized_sha256(&format!("  sha256:{}  ", digest.to_uppercase())).as_deref(),
            Some(digest.as_str())
        );
    }

    #[test]
    fn normalized_sha256_rejects_wrong_length_or_non_hex() {
        assert!(normalized_sha256("deadbeef").is_none());
        assert!(normalized_sha256(&"z".repeat(64)).is_none());
        assert!(normalized_sha256(&"a".repeat(65)).is_none());
        assert!(normalized_sha256("").is_none());
    }

    #[test]
    fn validate_rejects_malformed_sha256() {
        let mut config = config_with_limits(5000, 256);
        config.plugin.push(PluginEntry {
            name: "pinned".into(),
            path: "p.wasm".into(),
            enabled: true,
            capabilities: vec![PluginCapability::PreRequest],
            config: HashMap::new(),
            sha256: Some("deadbeef".into()),
            env_allowlist: vec![],
        });
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("malformed sha256"));

        config.plugin[0].sha256 = Some(format!("sha256:{}", "a".repeat(64)));
        assert!(config.validate().is_ok());
    }
}
