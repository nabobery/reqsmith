use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use extism::{Manifest, Plugin, Wasm};
use sha2::{Digest, Sha256};

use super::config::{
    ALLOW_PROJECT_PLUGINS_VAR, ConfigSource, DiscoveredConfig, PluginEntry, PluginsConfig,
    discover_config, project_plugins_allowed_from,
};
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
        // SECURITY (C3, least privilege): only pass through env values whose
        // key is explicitly listed in this plugin's `env_allowlist`. A
        // default-empty allowlist means the plugin receives NO environment
        // values at all via `provide_env_var`, even if the caller supplied
        // a full environment map (which may contain secrets/tokens).
        let allowed_env = filter_env(&self.entry.env_allowlist, &env_values);

        let plugin = Arc::clone(&self.plugin);
        let host_context = self.host_context.clone();
        let plugin_name = self.entry.name.clone();
        let timeout_ms = self.timeout_ms;
        let func_name = func_name.to_string();

        tokio::task::spawn_blocking(move || {
            let mut guard = plugin.lock().unwrap();
            host_context.set_env_values(allowed_env);
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
    /// Build an empty registry (no config found, or a gated config that was
    /// refused).
    fn empty() -> Self {
        Self {
            plugins: Vec::new(),
            config: PluginsConfig {
                api_version: 1,
                timeout_ms: 5000,
                memory_limit_pages: 256,
                plugin: Vec::new(),
            },
        }
    }

    /// Discover plugins from `.reqsmith/plugins.toml` and load them.
    ///
    /// Returns an empty registry if no config file is found, or if a
    /// PROJECT-LOCAL config (`$cwd/.reqsmith/plugins.toml`) is found but the
    /// user has not opted in via `REQSMITH_ALLOW_PROJECT_PLUGINS` — see C1 in
    /// `docs/plugins-security.md`. The real environment is read exactly
    /// once, here, at the public entry point; the actual gating logic lives
    /// in `discover_and_load_with` so it can be exercised in tests without
    /// touching process environment state.
    pub fn discover_and_load(cwd: &Path) -> Result<Self, PluginError> {
        let project_plugins_allowed =
            project_plugins_allowed_from(std::env::var(ALLOW_PROJECT_PLUGINS_VAR).ok().as_deref());
        Self::discover_and_load_with(cwd, project_plugins_allowed)
    }

    /// Same as `discover_and_load`, but takes the already-resolved C1
    /// consent decision as a parameter instead of reading it from the
    /// environment. This is the testable core: tests can pass `true`/`false`
    /// directly and avoid any `std::env::set_var` races.
    fn discover_and_load_with(
        cwd: &Path,
        project_plugins_allowed: bool,
    ) -> Result<Self, PluginError> {
        let discovered = match discover_config(cwd)? {
            Some(c) => c,
            None => return Ok(Self::empty()),
        };

        // C1 CONSENT GATE: a project-local `plugins.toml` can ship inside a
        // cloned/untrusted repository. Unlike the user-global config (which
        // the user placed themselves), we refuse to instantiate any WASM
        // from it unless the user has explicitly opted in for this run.
        if gate_project_local(discovered.source, project_plugins_allowed) {
            tracing::warn!(
                path = %discovered.path.display(),
                "Project-local plugin config found but not loaded: set {}=1 to allow \
                 running WASM plugins shipped inside this project (only do this for \
                 repositories you trust). See docs/plugins-security.md.",
                ALLOW_PROJECT_PLUGINS_VAR,
            );
            return Ok(Self::empty());
        }

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

            // C2 INTEGRITY PINNING: when the entry declares a `sha256`, the
            // on-disk WASM bytes must match before we ever instantiate it.
            let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| PluginError::LoadFailed {
                name: entry.name.clone(),
                message: format!("failed to read {}: {e}", wasm_path.display()),
            })?;
            if let Some(expected) = &entry.sha256 {
                verify_sha256(&entry.name, &wasm_bytes, expected)?;
            }

            let host_context = HostContext::new(HostData {
                env_values: HashMap::new(), // populated at call time, and filtered by env_allowlist (C3)
                plugin_config: entry.flat_config(),
            });

            // C2 (no TOCTOU): instantiate from the exact bytes we just read
            // and hashed, NOT by handing Extism the path to re-open. If the
            // manifest pointed back at `wasm_path`, an attacker could swap the
            // file between our `verify_sha256` and Extism's open, and the
            // bytes executed would differ from the bytes we verified. Feeding
            // the in-memory buffer closes that window.
            let manifest = build_manifest(wasm_bytes, &config);

            let host_functions = host_context.build_functions();

            // The trailing `true` enables the WASI *guest* runtime only
            // (stdio/clock/etc. inside the sandbox) — it does NOT map any
            // host directories or hosts, and must stay paired with the
            // absence of `allowed_paths`/`allowed_hosts` above.
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

/// Build the Extism [`Manifest`] for a plugin from its **already-read,
/// already-verified** WASM bytes.
///
/// SECURITY: this takes owned `wasm_bytes` and uses [`Wasm::data`] (in-memory)
/// rather than [`Wasm::file`] on purpose — see the call site in
/// `discover_and_load_with`. The manifest intentionally sets NO `allowed_paths`
/// and NO `allowed_hosts` (C4 sandbox boundary), so the plugin has no host
/// filesystem or outbound-network access; its only surface is the host
/// functions registered separately. `timeout_ms`/`memory_limit_pages` remain
/// the resource-exhaustion guard (Extism's Rust SDK exposes no simple
/// fuel/instruction-metering knob here).
fn build_manifest(wasm_bytes: Vec<u8>, config: &PluginsConfig) -> Manifest {
    Manifest::new([Wasm::data(wasm_bytes)])
        .with_timeout(std::time::Duration::from_millis(config.timeout_ms))
        .with_memory_max(config.memory_limit_pages)
}

/// Should the C1 consent gate refuse to load WASM for this discovered
/// config? Only `ConfigSource::ProjectLocal` is ever gated, and only when
/// the user hasn't opted in; `ConfigSource::UserGlobal` is always trusted
/// regardless of the opt-in flag. Pure function of already-resolved inputs
/// so both branches (including "user-global is never gated") are directly
/// unit-testable without touching real environment/filesystem state.
fn gate_project_local(source: ConfigSource, project_plugins_allowed: bool) -> bool {
    source == ConfigSource::ProjectLocal && !project_plugins_allowed
}

/// Lowercase hex-encode a byte slice (e.g. a SHA-256 digest), without
/// pulling in a dedicated hex crate.
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
    }
    out
}

/// C2 integrity check: verify that `bytes` hashes (SHA-256, hex) to
/// `expected` (case-insensitive comparison). Pure function so it can be
/// unit-tested against plain byte slices without needing a real WASM file
/// or plugin instance.
fn verify_sha256(name: &str, bytes: &[u8], expected: &str) -> Result<(), PluginError> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex_encode(&hasher.finalize());
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(PluginError::IntegrityMismatch {
            name: name.to_string(),
            expected: expected.to_string(),
            actual,
        })
    }
}

/// Filter `all` down to only the entries whose key appears (case-sensitive)
/// in `allow`. Pure helper backing the C3 env-var least-privilege gate, kept
/// free of any WASM/host-function machinery so it can be unit-tested
/// directly without a live plugin instance.
fn filter_env(allow: &[String], all: &HashMap<String, String>) -> HashMap<String, String> {
    all.iter()
        .filter(|(key, _)| allow.iter().any(|allowed_key| allowed_key == *key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
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
        let reqsmith_dir = tmp.path().join(".reqsmith");
        std::fs::create_dir_all(&reqsmith_dir).unwrap();
        std::fs::write(
            reqsmith_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "ghost"
path = "nonexistent.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();

        // Bypass the C1 gate directly (this test is about the missing-wasm
        // skip logic, not about the gate itself — see the `c1_*` tests below
        // for that).
        let registry = PluginRegistry::discover_and_load_with(tmp.path(), true).unwrap();
        assert!(registry.is_empty());
    }

    #[test]
    fn discover_rejects_invalid_api_version() {
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

        let err = PluginRegistry::discover_and_load(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("99"));
    }

    #[test]
    fn resolve_wasm_path_uses_config_directory_for_relative_paths() {
        let discovered = DiscoveredConfig {
            path: PathBuf::from("/tmp/project/.reqsmith/plugins.toml"),
            config: PluginsConfig {
                api_version: 1,
                timeout_ms: 5000,
                memory_limit_pages: 256,
                plugin: vec![],
            },
            source: ConfigSource::ProjectLocal,
        };

        let resolved = resolve_wasm_path(
            &discovered,
            Path::new("/tmp/project"),
            Path::new("plugins/a.wasm"),
        );

        assert_eq!(
            resolved,
            PathBuf::from("/tmp/project/.reqsmith/plugins/a.wasm")
        );
    }

    #[test]
    fn resolve_wasm_path_expands_home_directory() {
        let home = dirs::home_dir().unwrap();
        let discovered = DiscoveredConfig {
            path: home.join(".config/reqsmith/plugins.toml"),
            config: PluginsConfig {
                api_version: 1,
                timeout_ms: 5000,
                memory_limit_pages: 256,
                plugin: vec![],
            },
            source: ConfigSource::UserGlobal,
        };

        let resolved = resolve_wasm_path(
            &discovered,
            Path::new("/tmp"),
            Path::new("~/plugins/a.wasm"),
        );

        assert_eq!(resolved, home.join("plugins/a.wasm"));
    }

    // ------------------------------------------------------------------
    // C1: project-local consent gate
    // ------------------------------------------------------------------

    #[test]
    fn c1_gate_project_local_decision_table() {
        assert!(gate_project_local(ConfigSource::ProjectLocal, false));
        assert!(!gate_project_local(ConfigSource::ProjectLocal, true));
        // User-global is never gated, regardless of the opt-in flag.
        assert!(!gate_project_local(ConfigSource::UserGlobal, false));
        assert!(!gate_project_local(ConfigSource::UserGlobal, true));
    }

    #[test]
    fn c1_project_local_not_loaded_without_opt_in() {
        let tmp = tempfile::tempdir().unwrap();
        let reqsmith_dir = tmp.path().join(".reqsmith");
        std::fs::create_dir_all(&reqsmith_dir).unwrap();
        std::fs::write(
            reqsmith_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "untrusted"
path = "untrusted.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();
        // A real (if bogus) WASM file exists, so that if the gate were ever
        // bypassed we would be able to tell (see the opted-in test below,
        // which turns this same setup into a load attempt/error).
        std::fs::write(reqsmith_dir.join("untrusted.wasm"), b"not actually wasm").unwrap();

        let registry = PluginRegistry::discover_and_load_with(tmp.path(), false).unwrap();
        assert!(
            registry.is_empty(),
            "project-local config must not load without REQSMITH_ALLOW_PROJECT_PLUGINS opt-in"
        );
    }

    #[test]
    fn c1_project_local_attempts_load_when_opted_in() {
        let tmp = tempfile::tempdir().unwrap();
        let reqsmith_dir = tmp.path().join(".reqsmith");
        std::fs::create_dir_all(&reqsmith_dir).unwrap();
        std::fs::write(
            reqsmith_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "untrusted"
path = "untrusted.wasm"
capabilities = ["pre_request"]
"#,
        )
        .unwrap();
        std::fs::write(reqsmith_dir.join("untrusted.wasm"), b"not actually wasm").unwrap();

        // Opted in: the gate no longer short-circuits, so we actually reach
        // the WASM loading step and get a load error back from Extism
        // (the bytes aren't valid WASM) — proving that the earlier empty
        // registry in the not-opted-in test was caused by the consent gate
        // and not, say, the file being missing.
        let err = PluginRegistry::discover_and_load_with(tmp.path(), true).unwrap_err();
        assert!(matches!(err, PluginError::LoadFailed { .. }));
    }

    // ------------------------------------------------------------------
    // C2: WASM integrity pinning
    // ------------------------------------------------------------------

    #[test]
    fn c2_manifest_is_built_from_in_memory_bytes_not_a_reopened_path() {
        // Regression guard for the pin TOCTOU: the manifest handed to Extism
        // must carry the exact bytes we verified (`Wasm::Data`), never a file
        // path Extism would re-open after our hash check (`Wasm::File`).
        let config = PluginsConfig {
            api_version: 1,
            timeout_ms: 5000,
            memory_limit_pages: 256,
            plugin: vec![],
        };
        let bytes = b"verified wasm bytes".to_vec();
        let manifest = build_manifest(bytes.clone(), &config);

        assert_eq!(manifest.wasm.len(), 1);
        match &manifest.wasm[0] {
            extism::Wasm::Data { data, .. } => assert_eq!(data, &bytes),
            other => panic!("expected in-memory Wasm::Data, got {other:?}"),
        }
    }

    #[test]
    fn c2_verify_sha256_rejects_wrong_digest() {
        let bytes = b"hello wasm bytes";
        let err = verify_sha256("plugin", bytes, "deadbeef").unwrap_err();
        match err {
            PluginError::IntegrityMismatch { name, expected, .. } => {
                assert_eq!(name, "plugin");
                assert_eq!(expected, "deadbeef");
            }
            other => panic!("expected IntegrityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn c2_verify_sha256_accepts_correct_digest_case_insensitively() {
        let bytes = b"hello wasm bytes";
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hex_encode(&hasher.finalize());

        assert!(verify_sha256("plugin", bytes, &digest).is_ok());
        assert!(verify_sha256("plugin", bytes, &digest.to_uppercase()).is_ok());
    }

    #[test]
    fn c2_discover_and_load_rejects_mismatched_sha256() {
        let tmp = tempfile::tempdir().unwrap();
        let reqsmith_dir = tmp.path().join(".reqsmith");
        std::fs::create_dir_all(&reqsmith_dir).unwrap();
        std::fs::write(
            reqsmith_dir.join("plugins.toml"),
            r#"
api_version = 1
[[plugin]]
name = "pinned"
path = "pinned.wasm"
capabilities = ["pre_request"]
sha256 = "deadbeef"
"#,
        )
        .unwrap();
        std::fs::write(reqsmith_dir.join("pinned.wasm"), b"some wasm bytes").unwrap();

        // Use `true` to bypass the C1 gate (irrelevant to this test) and
        // reach the integrity check.
        let err = PluginRegistry::discover_and_load_with(tmp.path(), true).unwrap_err();
        assert!(matches!(err, PluginError::IntegrityMismatch { .. }));
    }

    #[test]
    fn c2_discover_and_load_accepts_correct_sha256_and_proceeds_past_integrity_check() {
        let tmp = tempfile::tempdir().unwrap();
        let reqsmith_dir = tmp.path().join(".reqsmith");
        std::fs::create_dir_all(&reqsmith_dir).unwrap();
        let wasm_bytes = b"some wasm bytes";
        let mut hasher = Sha256::new();
        hasher.update(wasm_bytes);
        let digest = hex_encode(&hasher.finalize());
        std::fs::write(
            reqsmith_dir.join("plugins.toml"),
            format!(
                r#"
api_version = 1
[[plugin]]
name = "pinned"
path = "pinned.wasm"
capabilities = ["pre_request"]
sha256 = "{digest}"
"#
            ),
        )
        .unwrap();
        std::fs::write(reqsmith_dir.join("pinned.wasm"), wasm_bytes).unwrap();

        let err = PluginRegistry::discover_and_load_with(tmp.path(), true).unwrap_err();
        // The digest matches, so the integrity check passes; the bytes
        // still aren't valid WASM, so we expect Extism's own load error
        // rather than an IntegrityMismatch — proving the integrity check
        // itself let a correctly-pinned file through.
        assert!(matches!(err, PluginError::LoadFailed { .. }));
    }

    // ------------------------------------------------------------------
    // C3: env-var least privilege
    // ------------------------------------------------------------------

    #[test]
    fn c3_filter_env_default_empty_allowlist_yields_nothing() {
        let all = HashMap::from([("API_TOKEN".to_string(), "secret".to_string())]);
        let filtered = filter_env(&[], &all);
        assert!(filtered.is_empty());
    }

    #[test]
    fn c3_filter_env_only_passes_allowlisted_keys() {
        let all = HashMap::from([
            ("API_TOKEN".to_string(), "secret".to_string()),
            ("OTHER_SECRET".to_string(), "nope".to_string()),
        ]);
        let allow = vec!["API_TOKEN".to_string()];
        let filtered = filter_env(&allow, &all);
        assert_eq!(filtered.len(), 1);
        assert_eq!(
            filtered.get("API_TOKEN").map(String::as_str),
            Some("secret")
        );
        assert!(!filtered.contains_key("OTHER_SECRET"));
    }

    #[test]
    fn c3_filter_env_is_case_sensitive() {
        let all = HashMap::from([("api_token".to_string(), "secret".to_string())]);
        let allow = vec!["API_TOKEN".to_string()];
        let filtered = filter_env(&allow, &all);
        assert!(filtered.is_empty());
    }
}
