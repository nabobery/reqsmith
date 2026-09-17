use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use extism::{Manifest, Plugin, Wasm};
use sha2::{Digest, Sha256};

use super::config::{
    ALLOW_PROJECT_PLUGINS_VAR, ConfigSource, DiscoveredConfig, PluginEntry, PluginsConfig,
    discover_configs_in, looks_like_secret_name, malformed_sha256_error, normalized_sha256,
    project_plugins_allowed_from,
};
use super::errors::PluginError;
use super::host_fns::{HostContext, HostData};
use super::models::PluginCapability;
use crate::core::redaction::RedactionPolicy;

/// A loaded and ready-to-call plugin instance.
pub struct LoadedPlugin {
    pub entry: PluginEntry,
    /// Configuration file that supplied this definition.
    pub config_path: PathBuf,
    /// Mutex because `Plugin::call` takes `&mut self`.
    plugin: Arc<Mutex<Plugin>>,
    host_context: HostContext,
    timeout_ms: u64,
}

impl LoadedPlugin {
    /// Lock the plugin, recovering from a mutex poisoned by an earlier
    /// panicking host-function call instead of panicking again — a poisoned
    /// lock says a host call unwound, not that the plugin instance is unusable.
    /// Release builds use `panic = "abort"`, so this recovery path only ever
    /// matters in debug/test builds, where a panic unwinds instead of aborting.
    fn lock(plugin: &Mutex<Plugin>) -> MutexGuard<'_, Plugin> {
        plugin.lock().unwrap_or_else(PoisonError::into_inner)
    }

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
            let mut guard = Self::lock(&plugin);
            host_context.set_env_values(allowed_env)?;
            guard.call::<&str, String>(&func_name, &input).map_err(|e| {
                let msg = e.to_string();
                // Extism 1.30 reports an epoch-deadline interrupt as exactly
                // `Error::msg("timeout")`; anything else is the plugin's own
                // failure and must not be mislabelled as a timeout.
                if msg == "timeout" {
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
        Self::lock(&self.plugin).function_exists(name)
    }
}

/// Registry of all discovered and loaded plugins.
pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
    blocked_project_config: Option<PathBuf>,
    load_errors: Vec<PluginLoadError>,
    /// Per-config discovery failures: a `plugins.toml` that failed to
    /// read, parse, or validate, keyed by its path rather than a plugin name
    /// since no entry was ever parsed out of it.
    config_errors: Vec<(PathBuf, PluginError)>,
}

/// Failure associated with one concrete plugin definition.
#[derive(Debug)]
pub struct PluginLoadError {
    pub config_path: PathBuf,
    pub name: String,
    pub error: PluginError,
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
            .field("blocked_project_config", &self.blocked_project_config)
            .field("load_error_count", &self.load_errors.len())
            .field("config_error_count", &self.config_errors.len())
            .finish()
    }
}

impl PluginRegistry {
    fn empty() -> Self {
        Self {
            plugins: Vec::new(),
            blocked_project_config: None,
            load_errors: Vec::new(),
            config_errors: Vec::new(),
        }
    }

    /// Discover plugins from every `plugins.toml` reqsmith knows about and
    /// load them.
    ///
    /// A PROJECT-LOCAL config (`$cwd/.reqsmith/plugins.toml`) is skipped
    /// unless the user opted in via `REQSMITH_ALLOW_PROJECT_PLUGINS` — see C1
    /// in `docs/plugins-security.md` — but skipping it never suppresses the
    /// user-global config, which is loaded either way. The real environment is
    /// read exactly once, here; the gating logic lives in
    /// `discover_and_load_with` so it can be exercised in tests without
    /// touching process environment state.
    pub fn discover_and_load(cwd: &Path) -> Self {
        let project_plugins_allowed =
            project_plugins_allowed_from(std::env::var(ALLOW_PROJECT_PLUGINS_VAR).ok().as_deref());
        Self::discover_and_load_with(cwd, dirs::config_dir().as_deref(), project_plugins_allowed)
    }

    /// Same as `discover_and_load`, with the user-global config directory and
    /// the C1 consent decision injected so tests need not mutate process
    /// state. `pub(crate)` so hermetic tests elsewhere in the crate (e.g.
    /// `hooks.rs`) can build a registry without touching the real config dir
    /// or `REQSMITH_ALLOW_PROJECT_PLUGINS` env var.
    pub(crate) fn discover_and_load_with(
        cwd: &Path,
        config_dir: Option<&Path>,
        project_plugins_allowed: bool,
    ) -> Self {
        let mut registry = Self::empty();
        let policy = RedactionPolicy::from_env();

        let discovery = discover_configs_in(cwd, config_dir);
        registry.config_errors = discovery.errors;

        // Load user-global configs before project-local ones so a name
        // collision resolves in the user's (trusted) favor — the project-
        // local duplicate is then the "later" definition and gets skipped.
        // Discovery order itself (project-local first) is unaffected; only
        // this local copy, used for loading, is reordered.
        let mut configs = discovery.configs;
        configs.sort_by_key(|discovered| discovered.source != ConfigSource::UserGlobal);

        let mut claimed_names: HashMap<String, PathBuf> = HashMap::new();
        for discovered in &configs {
            // C1 CONSENT GATE: a project-local `plugins.toml` can ship inside
            // a cloned/untrusted repository, so we refuse to instantiate any
            // WASM from it without an explicit opt-in — while still loading
            // the user's own global config below.
            if gate_project_local(discovered.source, project_plugins_allowed) {
                tracing::warn!(
                    path = %discovered.path.display(),
                    "Project-local plugin config found but not loaded: set {}=1 to allow \
                     running WASM plugins shipped inside this project (only do this for \
                     repositories you trust). See docs/plugins-security.md.",
                    ALLOW_PROJECT_PLUGINS_VAR,
                );
                registry.blocked_project_config = Some(discovered.path.clone());
                continue;
            }

            registry.load_config(cwd, discovered, &policy, &mut claimed_names);
        }

        registry
    }

    /// Load one discovered config. A failing entry is recorded and skipped:
    /// one unreadable or tampered module must not take the whole registry
    /// (and every other plugin the user configured) down with it.
    ///
    /// `claimed_names` tracks which config first defined each plugin name:
    /// a later definition of an already-claimed name is rejected with a
    /// load error instead of silently shadowing or double-running alongside
    /// the first one.
    fn load_config(
        &mut self,
        cwd: &Path,
        discovered: &DiscoveredConfig,
        policy: &RedactionPolicy,
        claimed_names: &mut HashMap<String, PathBuf>,
    ) {
        for entry in discovered.config.enabled_plugins() {
            if let Some(first_path) = claimed_names.get(&entry.name) {
                let error = PluginError::ManifestError(format!(
                    "plugin '{}' is already defined in {}",
                    entry.name,
                    first_path.display()
                ));
                tracing::warn!(
                    plugin = %entry.name,
                    path = %discovered.path.display(),
                    first_defined_in = %first_path.display(),
                    "Duplicate plugin name skipped"
                );
                self.load_errors.push(PluginLoadError {
                    config_path: discovered.path.clone(),
                    name: entry.name.clone(),
                    error,
                });
                continue;
            }
            claimed_names.insert(entry.name.clone(), discovered.path.clone());

            match load_entry(cwd, discovered, entry, policy) {
                Ok(Some(plugin)) => {
                    tracing::info!(
                        plugin = %entry.name,
                        capabilities = ?entry.capabilities,
                        "Loaded plugin"
                    );
                    self.plugins.push(plugin);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(plugin = %entry.name, "Plugin not loaded: {error}");
                    self.load_errors.push(PluginLoadError {
                        config_path: discovered.path.clone(),
                        name: entry.name.clone(),
                        error,
                    });
                }
            }
        }
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
    #[cfg(test)]
    pub fn plugin_names(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.entry.name.as_str()).collect()
    }

    /// Active plugin identities, including the config path that won merging.
    pub fn loaded_plugin_ids(&self) -> impl Iterator<Item = (&Path, &str)> {
        self.plugins
            .iter()
            .map(|plugin| (plugin.config_path.as_path(), plugin.entry.name.as_str()))
    }

    /// The project-local `plugins.toml` that was found but refused for lack
    /// of a `REQSMITH_ALLOW_PROJECT_PLUGINS` opt-in, if any.
    pub fn blocked_project_config(&self) -> Option<&Path> {
        self.blocked_project_config.as_deref()
    }

    /// Per-entry load failures, keyed by config path and plugin name.
    pub fn load_errors(&self) -> &[PluginLoadError] {
        &self.load_errors
    }

    /// Configuration file that supplied the active definition of `name`.
    pub fn plugin_config_path(&self, name: &str) -> Option<&Path> {
        self.plugins
            .iter()
            .find(|plugin| plugin.entry.name == name)
            .map(|plugin| plugin.config_path.as_path())
    }

    /// Per-config discovery failures: a `plugins.toml` that failed to
    /// read, parse, or validate, by its path.
    pub fn config_errors(&self) -> &[(PathBuf, PluginError)] {
        &self.config_errors
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

/// Load one plugin entry. `Ok(None)` means the module file is simply absent,
/// which is a skip rather than a failure.
fn load_entry(
    cwd: &Path,
    discovered: &DiscoveredConfig,
    entry: &PluginEntry,
    policy: &RedactionPolicy,
) -> Result<Option<LoadedPlugin>, PluginError> {
    let wasm_path = resolve_wasm_path(discovered, cwd, &entry.path)?;

    if !wasm_path.is_file() {
        tracing::warn!(
            plugin = %entry.name,
            path = %wasm_path.display(),
            "WASM file not found, skipping plugin"
        );
        return Ok(None);
    }

    // C2 INTEGRITY PINNING: when the entry declares a `sha256`, the on-disk
    // WASM bytes must match before we ever instantiate it.
    let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| PluginError::LoadFailed {
        name: entry.name.clone(),
        message: format!("failed to read {}: {e}", wasm_path.display()),
    })?;
    if let Some(expected) = &entry.sha256 {
        verify_sha256(&entry.name, &wasm_bytes, expected)?;
    }

    warn_sensitive_env_allowlist(entry, policy);

    let host_context = HostContext::new(HostData {
        env_values: HashMap::new(), // populated at call time, and filtered by env_allowlist (C3)
        plugin_config: entry.flat_config(),
    });

    // C2 (no TOCTOU): instantiate from the exact bytes we just read and
    // hashed, NOT by handing Extism the path to re-open. If the manifest
    // pointed back at `wasm_path`, an attacker could swap the file between
    // our `verify_sha256` and Extism's open, and the bytes executed would
    // differ from the bytes we verified. Feeding the in-memory buffer closes
    // that window.
    let manifest = build_manifest(wasm_bytes, &discovered.config);
    let host_functions = host_context.build_functions();

    // The trailing `true` enables the WASI *guest* runtime only (stdio/clock
    // etc. inside the sandbox) — it does NOT map any host directories or
    // hosts, and must stay paired with the absence of
    // `allowed_paths`/`allowed_hosts` in the manifest.
    let plugin =
        Plugin::new(&manifest, host_functions, true).map_err(|e| PluginError::LoadFailed {
            name: entry.name.clone(),
            message: e.to_string(),
        })?;

    Ok(Some(LoadedPlugin {
        entry: entry.clone(),
        config_path: discovered.path.clone(),
        plugin: Arc::new(Mutex::new(plugin)),
        host_context,
        timeout_ms: discovered.config.timeout_ms,
    }))
}

/// An `env_allowlist` naming something the redaction policy treats as secret
/// is legal but worth flagging: it hands that value to guest code.
fn warn_sensitive_env_allowlist(entry: &PluginEntry, policy: &RedactionPolicy) {
    for name in &entry.env_allowlist {
        if looks_like_secret_name(name, policy) {
            tracing::warn!(
                plugin = %entry.name,
                env_var = %name,
                "Plugin env_allowlist grants a sensitive-looking variable"
            );
        }
    }
}

/// Build the Extism [`Manifest`] for a plugin from its **already-read,
/// already-verified** WASM bytes.
///
/// SECURITY: this takes owned `wasm_bytes` and uses [`Wasm::data`] (in-memory)
/// rather than [`Wasm::file`] on purpose — see the call site in `load_entry`.
/// The manifest intentionally sets NO `allowed_paths` and NO `allowed_hosts`
/// (the sandbox boundary), so the plugin has no host filesystem or
/// outbound-network access; its only surface is the host functions registered
/// separately. `timeout_ms`/`memory_limit_pages` are the resource-exhaustion
/// guard: Extism does expose `PluginBuilder::with_fuel_limit`, but the
/// wall-clock timeout already bounds a spinning plugin and needs no per-plugin
/// instruction budget to tune.
fn build_manifest(wasm_bytes: Vec<u8>, config: &PluginsConfig) -> Manifest {
    Manifest::new([Wasm::data(wasm_bytes)])
        .with_timeout(std::time::Duration::from_millis(config.timeout_ms))
        .with_memory_max(config.memory_limit_pages)
}

/// Should the C1 consent gate refuse to load WASM for this discovered
/// config? Only `ConfigSource::ProjectLocal` is ever gated, and only when
/// the user hasn't opted in; `ConfigSource::UserGlobal` is always trusted
/// regardless of the opt-in flag.
fn gate_project_local(source: ConfigSource, project_plugins_allowed: bool) -> bool {
    source == ConfigSource::ProjectLocal && !project_plugins_allowed
}

/// Lowercase hex-encode a byte slice (e.g. a SHA-256 digest), without
/// pulling in a dedicated hex crate.
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a String is infallible; the formatter cannot fail here.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// C2 integrity check: verify that `bytes` hashes (SHA-256, hex) to the
/// normalized form of `expected`. A digest that isn't 64 hex characters is a
/// config format error, not a mismatch.
fn verify_sha256(name: &str, bytes: &[u8], expected: &str) -> Result<(), PluginError> {
    let expected = normalized_sha256(expected).ok_or_else(|| malformed_sha256_error(name))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex_encode(&hasher.finalize());
    if actual == expected {
        Ok(())
    } else {
        Err(PluginError::IntegrityMismatch {
            name: name.to_string(),
            expected,
            actual,
        })
    }
}

/// Filter `all` down to only the entries whose key appears (case-sensitive)
/// in `allow`. Pure helper backing the C3 env-var least-privilege gate.
fn filter_env(allow: &[String], all: &HashMap<String, String>) -> HashMap<String, String> {
    all.iter()
        .filter(|(key, _)| allow.iter().any(|allowed_key| allowed_key == *key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Resolve an entry's declared module path against the directory holding its
/// `plugins.toml`. The module must live under that directory, so absolute,
/// home-relative and `..`-escaping paths are rejected.
pub(crate) fn resolve_wasm_path(
    discovered: &DiscoveredConfig,
    cwd: &Path,
    path: &Path,
) -> Result<PathBuf, PluginError> {
    let reject = |reason: &str| {
        Err(PluginError::ManifestError(format!(
            "Plugin path '{}' {reason}; plugin modules must live under the directory \
             containing plugins.toml",
            path.display()
        )))
    };

    // On Windows, a rooted path such as `\\etc\\evil.wasm` has a root but no
    // drive prefix, so `is_absolute()` is false even though joining it can
    // discard the config directory. Reject roots and prefixes explicitly,
    // including drive-relative paths such as `C:evil.wasm`.
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return reject("is absolute or rooted");
    }
    if path.to_str().is_some_and(|v| v.starts_with("~/")) {
        return reject("is home-relative");
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return reject("contains a '..' component");
    }

    let config_dir = discovered.path.parent().unwrap_or(cwd);
    let joined = config_dir.join(path);

    // The textual checks above stop `..`/absolute/`~` escapes, but not
    // a symlink *inside* the config dir pointing somewhere else on disk — this
    // function doesn't canonicalize and `is_file()` follows symlinks. When the
    // target exists, canonicalize both sides and require the real path to
    // still be inside the real config dir.
    if joined.is_file()
        && let (Ok(real_dir), Ok(real_target)) = (config_dir.canonicalize(), joined.canonicalize())
        && !real_target.starts_with(&real_dir)
    {
        return reject("resolves (via a symlink) outside the config directory");
    }

    Ok(joined)
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
