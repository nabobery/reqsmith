# Phase 4: Extensibility Platform — Implementation Plan

Version: 1.0
Date: March 29, 2026
Status: Draft
Related: [hurl PRD](2026-03-29-hurl-prd.md), [Technical Spec](../specs/2026-03-29-hurl-phased-technical-spec.md), [Phase 3 Plan](2026-03-29-phase-3-power-user-features-plan.md)

## Context

Phases 0–3 are complete. The codebase has ~9,200 lines of Rust across 36 source files with 100+ tests. hurl is a working interactive TUI + headless CLI API client with collection discovery, request editing, async HTTP execution with cancellation, response viewing, layered environment resolution, assertions, response diffing, JSON tree viewer, and YAML persistence. Phase 4 adds an extensibility layer — pre/post request hooks, custom auth providers, and variable providers — via a sandboxed WASM plugin runtime, while keeping the core request loop stable and lightweight for users who don't need plugins.

## Goal

At the end of Phase 4, hurl supports:
- Pre-request mutation hooks (modify method, URL, headers, params, body before execution)
- Post-response inspection hooks (inspect/annotate response after execution)
- Custom auth providers (e.g. AWS SigV4, OAuth token refresh)
- Explicit per-request auth plugin selection via request metadata
- Variable providers (inject ephemeral tokens into `{{var}}` resolution)
- All via sandboxed WASM plugins using Extism 1.7
- Feature-gated behind `plugins` — zero cost for users who don't need it

## Out of Scope

Unsandboxed native plugin loading, plugin marketplace, multi-user collaboration, GUI plugin management, built-in advanced auth UX beyond plugin selection, scripting engine, GraphQL/gRPC support.

---

## Technology Decision: Extism 1.7

**Why Extism over raw Wasmtime:** Extism wraps wasmtime and adds plugin framework abstractions — `host_fn!` macro, simple input/output model, built-in sandboxing, timeouts, memory limits. Raw wasmtime with WIT/Component Model would require weeks of additional boilerplate for the same result.

**Key properties:**
- `Plugin::call` is synchronous → wrap in `tokio::task::spawn_blocking`
- Communication is JSON-based (serde_json already a dependency)
- No FS/network access by default — plugins only interact through host functions
- Timeout + memory limits configurable per-plugin via Manifest
- Feature-gate behind `plugins = ["extism"]` to avoid ~20MB wasmtime compile dep

---

## New Dependencies

| Crate | Version | Purpose | Conditional |
|-------|---------|---------|-------------|
| `extism` | 1.7 | WASM plugin host (wraps wasmtime) | `optional = true`, behind `plugins` feature |
| `toml` | 0.8 | Parse `plugins.toml` config | `optional = true`, behind `plugins` feature |

---

## New Module Structure

```
src/plugins/
  mod.rs            — Feature-gated module root, re-exports public API
  models.rs         — HookContext, HookResult, ResponseContext, AuthRequest/Result, VariableRequest/Result
  config.rs         — PluginsConfig, PluginEntry TOML parsing
  errors.rs         — PluginError enum (thiserror)
  registry.rs       — PluginRegistry: discovery, loading, lifecycle
  host_fns.rs       — Host functions: provide_env_var, plugin_log, read_config
  hooks.rs          — Hook orchestration: run_pre_request, run_post_response, run_authenticate, provide_variable
```

All files in `src/plugins/` are `#[cfg(feature = "plugins")]`.

---

## Implementation Steps

### Step 1: Plugin Data Models and Config

**New files:** `src/plugins/mod.rs`, `src/plugins/models.rs`, `src/plugins/config.rs`, `src/plugins/errors.rs`
**Modify:** `Cargo.toml`, `src/main.rs`

Add `plugins` feature and optional deps to `Cargo.toml`:

```toml
[features]
default = []
plugins = ["dep:extism", "dep:toml"]

[dependencies]
extism = { version = "1.7", optional = true }
toml = { version = "0.8", optional = true }
```

Add `#[cfg(feature = "plugins")] mod plugins;` to `src/main.rs`.

Plugin data models in `src/plugins/models.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    PreRequest,
    PostResponse,
    Authenticate,
    ProvideVariable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub body: Option<String>,
    pub env_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookResult {
    pub method: Option<String>,       // None = unchanged
    pub url: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub params: Option<Vec<(String, String)>>,
    pub body: Option<String>,
    pub error: Option<String>,        // If set, abort with this message
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseContext {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body_text: Option<String>,
    pub duration_ms: u128,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub auth_type: String,
    pub config: HashMap<String, String>,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResult {
    pub headers: Vec<(String, String)>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableRequest {
    pub name: String,
    pub config: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableResult {
    pub value: Option<String>,
    pub error: Option<String>,
}
```

Conversions: `HookContext::from_document(&RequestDocument, &EnvironmentSet)` and `HookResult::apply_to(&self, &RequestDocument) -> RequestDocument`. Same pattern for `ResponseContext ↔ ResponseArtifact`.

Plugin config in `src/plugins/config.rs` — parse `.hurl/plugins.toml`:

```toml
api_version = 1
timeout_ms = 5000
memory_limit_pages = 256

[[plugin]]
name = "aws-sigv4"
path = ".hurl/plugins/aws-sigv4.wasm"
enabled = true
capabilities = ["authenticate"]
[plugin.config]
region = "us-east-1"
```

Config struct:

```rust
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
```

Discovery order: `$CWD/.hurl/plugins.toml` → `$XDG_CONFIG_HOME/hurl/plugins.toml`.
Relative plugin paths resolve from the discovered `plugins.toml` directory, and `~/...` paths expand to the user's home directory.

Plugin errors in `src/plugins/errors.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("Plugin '{name}' failed: {message}")]
    ExecutionFailed { name: String, message: String },

    #[error("Plugin '{name}' timed out after {timeout_ms}ms")]
    Timeout { name: String, timeout_ms: u64 },

    #[error("Plugin manifest error: {0}")]
    ManifestError(String),

    #[error("Plugin WASM load failed for '{name}': {message}")]
    LoadFailed { name: String, message: String },

    #[error("Unsupported plugin API version {version} (supported: {supported})")]
    UnsupportedApiVersion { version: u32, supported: u32 },

    #[error("Plugin '{name}' lacks required capability: {capability}")]
    MissingCapability { name: String, capability: String },
}
```

Add to `src/errors.rs`:
```rust
#[cfg(feature = "plugins")]
#[error("Plugin error: {0}")]
Plugin(String),
```

**Tests:** TOML parsing (valid, missing fields, invalid capabilities), JSON serde round-trip for all hook types, HookContext/HookResult conversion to/from RequestDocument.

**Gate:** `cargo test` and `cargo test --features plugins` both pass.

### Step 2: Plugin Registry and Extism Loading

**New files:** `src/plugins/registry.rs`, `src/plugins/host_fns.rs`

`PluginRegistry` owns all loaded plugin instances:

```rust
pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
    config: PluginsConfig,
}

struct LoadedPlugin {
    entry: PluginEntry,
    plugin: std::sync::Arc<std::sync::Mutex<extism::Plugin>>,
    host_context: HostContext,
    timeout_ms: u64,
}

impl PluginRegistry {
    pub fn discover_and_load(cwd: &Path) -> Result<Self, PluginError>;
    pub fn pre_request_plugins(&self) -> impl Iterator<Item = &LoadedPlugin>;
    pub fn post_response_plugins(&self) -> impl Iterator<Item = &LoadedPlugin>;
    pub fn auth_plugins(&self) -> impl Iterator<Item = &LoadedPlugin>;
    pub fn variable_plugins(&self) -> impl Iterator<Item = &LoadedPlugin>;
    pub fn plugin_names(&self) -> Vec<&str>;
    pub fn is_empty(&self) -> bool;
}
```

Discovery and loading:
1. Search for `plugins.toml` in discovery order
2. Parse config
3. Validate `api_version == 1`
4. For each enabled plugin entry: resolve the configured path relative to the discovered config file, expand `~/...`, load WASM via `extism::Manifest` with timeout + memory limits, register host functions, validate required exports via `plugin.function_exists()`
5. Return registry

Host functions in `src/plugins/host_fns.rs`:

```rust
host_fn!(pub provide_env_var(user_data: HostData; key: String) -> String {
    let env = &user_data.env_values;
    Ok(env.get(&key).cloned().unwrap_or_default())
});

host_fn!(pub plugin_log(user_data: HostData; message: String) -> String {
    tracing::info!(source = "plugin", "{message}");
    Ok(String::new())
});

host_fn!(pub read_config(user_data: HostData; key: String) -> String {
    let config = &user_data.plugin_config;
    Ok(config.get(&key).map(|v| v.to_string()).unwrap_or_default())
});
```

`HostData.env_values` is refreshed immediately before each plugin invocation so `provide_env_var` reads the current run's resolved environment instead of a stale snapshot from startup.

Extism Manifest per-plugin:
- `timeout`: from `config.timeout_ms`
- `memory.max_pages`: from `config.memory_limit_pages`
- No `allowed_hosts` (no network)
- No `allowed_paths` (no filesystem)

**Tests:** Load valid plugin, reject missing WASM, reject unsupported API version, validate export checking, discovery with no config returns empty registry.

**Gate:** `cargo test --features plugins` passes.

### Step 3: Hook Orchestration

**New file:** `src/plugins/hooks.rs`

Four public async functions:

```rust
pub async fn run_pre_request(
    registry: &PluginRegistry,
    ctx: HookContext,
    env_values: &HashMap<String, String>,
) -> HookContext
pub async fn run_post_response(
    registry: &PluginRegistry,
    ctx: ResponseContext,
    env_values: &HashMap<String, String>,
) -> ResponseContext
pub async fn run_authenticate(
    registry: &PluginRegistry,
    req: AuthRequest,
    env_values: &HashMap<String, String>,
) -> Option<AuthResult>
pub async fn provide_variable(
    registry: &PluginRegistry,
    name: &str,
    env_values: &HashMap<String, String>,
) -> Option<String>
```

Pattern for each:
1. Filter plugins by capability
2. Serialize input to JSON
3. `tokio::task::spawn_blocking` → lock Mutex → `plugin.call::<&str, String>(func_name, &input)`
4. Deserialize output
5. On error: `tracing::warn!`, skip plugin, continue (failure isolation)
6. For pre_request/post_response: chain results (each plugin's output → next's input)
7. For authenticate/provide_variable: first successful result wins, except explicit auth selection routes to the named plugin only

**Tests:** Mock plugin behavior with test WASM fixtures, error propagation, chaining, timeout handling.

**Gate:** `cargo test --features plugins` passes.

### Step 4: Runner Integration

**Modify:** `src/core/runner.rs`, `src/core/models.rs`

Add to `RunOptions`:
```rust
#[cfg(feature = "plugins")]
pub plugin_registry: Option<std::sync::Arc<crate::plugins::PluginRegistry>>,
```

Add `VarSource::Plugin` to `VarSource` enum (feature-gated).

The augmented `run_request` pipeline:

```
1.  Resolve environment                     (existing)
2.  Apply OS env fallback                   (existing)
2.5 Variable provider hook                  (NEW — resolve missing vars via plugins)
3.  Optional pre-flight validation          (existing)
3.5 Pre-request mutation hook               (NEW — plugins modify request)
3.6 Authentication hook                     (NEW — selected plugin adds auth headers)
4.  Execute HTTP request                    (existing)
4.5 Post-response inspection hook           (NEW — plugins inspect/annotate response)
5.  Evaluate assertions                     (existing)
```

All new steps wrapped in `#[cfg(feature = "plugins")]` and no-op when `plugin_registry` is `None`.

**Variable provider integration** (between steps 2 and 3):
```rust
#[cfg(feature = "plugins")]
if let Some(registry) = &options.plugin_registry {
    let still_missing: Vec<String> = referenced_vars.iter()
        .filter(|v| !env.values.contains_key(*v))
        .cloned()
        .collect();
    for var_name in still_missing {
        if let Some(value) = plugins::hooks::provide_variable(registry, &var_name, &env.values).await {
            env.values.insert(var_name.clone(), value);
            env.sources.insert(var_name, VarSource::Plugin);
        }
    }
}
```

**Pre-request hook** (between steps 3 and 4):
```rust
#[cfg(feature = "plugins")]
let doc = if let Some(registry) = &options.plugin_registry {
    let ctx = plugins::models::HookContext::from_document(&doc, &env);
    let result = plugins::hooks::run_pre_request(registry, ctx, &env.values).await;
    result.apply_to_document(&doc)
} else {
    doc.clone()
};
```

**Auth hook** (between pre-request and execute):
```rust
#[cfg(feature = "plugins")]
if let Some(registry) = &options.plugin_registry {
    let auth_req = plugins::models::AuthRequest::from_document(&doc);
    if let Some(auth_result) =
        plugins::hooks::run_authenticate(registry, auth_req, &env.values).await
    {
        for (key, value) in auth_result.headers {
            doc.headers.push(KeyValueField { key, value, enabled: true });
        }
    }
}
```

`RequestDocument` gains an optional `auth_plugin: Option<String>` field. When present, `run_request` treats it as an explicit opt-in to that named authenticate-capable plugin and fails validation if the plugin is unavailable or returns no auth headers.

**Post-response hook** (between execute and assertions):
```rust
#[cfg(feature = "plugins")]
let artifact = if let Some(registry) = &options.plugin_registry {
    let resp_ctx = plugins::models::ResponseContext::from_artifact(&artifact);
    let mutated = plugins::hooks::run_post_response(registry, resp_ctx, &env.values).await;
    mutated.apply_to_artifact(artifact)
} else {
    artifact
};
```

Post-response mutation must keep `ResponseArtifact` internally coherent by updating derived metadata such as content type, content length, binary/text flags, and stored body bytes when the hook rewrites the body text.

**Tests:** Runner with no plugins (unchanged behavior), runner with mock plugin registry.

**Gate:** `cargo test` (no features) passes identically. `cargo test --features plugins` passes.

### Step 5: App and TUI Integration

**Modify:** `src/app.rs`, `src/action.rs`, `src/components/status_bar.rs`

Add to `Action` enum:
```rust
#[cfg(feature = "plugins")]
PluginsLoaded(usize),
#[cfg(feature = "plugins")]
PluginError(String),
```

Add to `App` struct:
```rust
#[cfg(feature = "plugins")]
plugin_registry: Option<std::sync::Arc<plugins::PluginRegistry>>,
```

In `App::new()`:
```rust
#[cfg(feature = "plugins")]
let plugin_registry = match plugins::PluginRegistry::discover_and_load(&cwd) {
    Ok(r) if !r.is_empty() => {
        tracing::info!("Loaded {} plugin(s)", r.plugin_names().len());
        Some(std::sync::Arc::new(r))
    }
    Ok(_) => None,
    Err(e) => {
        tracing::warn!("Plugin loading failed: {e}");
        None
    }
};
```

In `SendRequest` handler: pass `plugin_registry.clone()` into `RunOptions`.

Status bar: show plugin count when loaded (dim text).

**Tests:** App initializes correctly. Status bar renders plugin count.

**Gate:** `cargo test` and `cargo test --features plugins` both pass. TUI still works.

### Step 6: CLI Plugin Commands

**New file:** `src/commands/plugin.rs`
**Modify:** `src/cli.rs`, `src/commands/mod.rs`, `src/main.rs`

Add `Plugin` subcommand (feature-gated):
```rust
#[cfg(feature = "plugins")]
/// Manage plugins
Plugin {
    #[command(subcommand)]
    action: PluginAction,
},

#[cfg(feature = "plugins")]
#[derive(Subcommand, Debug)]
pub enum PluginAction {
    /// List discovered plugins and their status
    List,
    /// Show detailed info about a specific plugin
    Info { name: String },
}
```

Implementation:
- `hurl plugin list` — discover plugins, print table: name, capabilities, enabled, path
- `hurl plugin info <name>` — config, WASM path, file size, API version

**Tests:** CLI parsing for plugin subcommands.

**Gate:** `cargo test --features plugins` passes.

### Step 7: CLI Run Integration

**Modify:** `src/commands/run.rs`

Load `PluginRegistry` in the `execute()` function and pass to `RunOptions`:

```rust
#[cfg(feature = "plugins")]
let plugin_registry = match plugins::PluginRegistry::discover_and_load(&cwd) {
    Ok(r) if !r.is_empty() => Some(std::sync::Arc::new(r)),
    Ok(_) => None,
    Err(e) => { eprintln!("Warning: plugin loading failed: {e}"); None }
};
```

**Tests:** CLI run works with and without plugins feature.

**Gate:** `just ci` passes.

---

## Dependency Graph

```
Step 1 (models + config) ────────────┐
    │                                 │
Step 2 (registry + extism loading)    │
    │                                 │
Step 3 (hook orchestration)           │
    │                                 │
Step 4 (runner integration) ──────────┤
    │                                 │
Step 5 (app + TUI integration)       │
    │                                 │
Step 6 (CLI plugin commands) ─────── (after Step 2)
    │                                 │
Step 7 (CLI run integration) ─────── (after Step 4)
```

Steps 1 → 2 → 3 → 4 are strictly sequential.
Steps 5, 6, 7 can be done after Step 4 in any order (6 only needs Step 2).

---

## New Files (8)

| File | Purpose | ~Lines |
|------|---------|--------|
| `src/plugins/mod.rs` | Feature-gated module root | 30 |
| `src/plugins/models.rs` | Hook context/result types, conversions | 250 |
| `src/plugins/config.rs` | TOML config parsing | 120 |
| `src/plugins/errors.rs` | PluginError enum | 40 |
| `src/plugins/registry.rs` | Plugin discovery, loading, lifecycle | 200 |
| `src/plugins/host_fns.rs` | Host functions for plugin sandbox | 80 |
| `src/plugins/hooks.rs` | Hook orchestration (pre/post/auth/var) | 200 |
| `src/commands/plugin.rs` | CLI plugin subcommands | 80 |

## Modified Files (11)

| File | Changes |
|------|---------|
| `Cargo.toml` | Add `plugins` feature, `extism` + `toml` optional deps |
| `src/main.rs` | Add `#[cfg(feature = "plugins")] mod plugins;`, wire plugin CLI command |
| `src/errors.rs` | Add `Plugin(String)` variant (feature-gated) |
| `src/core/models.rs` | Add `VarSource::Plugin` (feature-gated) |
| `src/core/runner.rs` | Add plugin hook call sites in pipeline, extend `RunOptions` |
| `src/action.rs` | Add `PluginsLoaded`, `PluginError` actions (feature-gated) |
| `src/app.rs` | Add plugin registry to App, load at startup, pass to runner |
| `src/cli.rs` | Add `Plugin { List, Info }` subcommand (feature-gated) |
| `src/commands/mod.rs` | Add `pub mod plugin;` (feature-gated) |
| `src/commands/run.rs` | Load registry, pass to RunOptions |
| `src/components/status_bar.rs` | Show plugin count |

---

## Key Design Decisions

1. **Extism over raw wasmtime** — Higher-level abstractions (host_fn!, simple I/O), sufficient for hook-style plugins. Avoids weeks of WIT/Component Model boilerplate.

2. **JSON for plugin boundary** — serde_json already a dep. Negligible overhead for request/response metadata. Avoids protobuf toolchain.

3. **Feature-gated everything** — `#[cfg(feature = "plugins")]` on every plugin-related line. Non-plugin builds are byte-identical to pre-Phase-4. No wasmtime compile cost (~20MB) for users who don't need it.

4. **Hooks are advisory, not blocking** — If a pre-request hook errors, request proceeds with a warning. If post-response hook errors, original response returned. Failure isolation is the default.

5. **Mutex around Plugin instances** — wasmtime Store is !Send. Plugin calls are sequential within a request pipeline, so Mutex contention is zero in practice.

6. **Single config file** — `.hurl/plugins.toml` is simpler than per-plugin manifests. Plugin authors ship a `.wasm` file; users configure it.

7. **spawn_blocking for Extism calls** — Extism's `Plugin::call` is synchronous. Wrapping in `spawn_blocking` keeps the TUI render loop responsive.

8. **Auth as hook, not middleware** — Auth plugins receive full request context and return headers to add. Covers AWS SigV4 (needs method+url+headers+body to sign) and OAuth (needs config to refresh token).

9. **Per-call environment injection** — Host env access is refreshed immediately before each invocation so plugins see the resolved variables for the current request, not loader-time state.

10. **Config-relative path resolution** — Relative plugin paths resolve from the discovered config file and `~/...` expands to the home directory so project-local and XDG configs behave predictably.

---

## Plugin Configuration Example

`.hurl/plugins.toml`:
```toml
api_version = 1
timeout_ms = 5000
memory_limit_pages = 256

[[plugin]]
name = "aws-sigv4"
path = ".hurl/plugins/aws-sigv4.wasm"
enabled = true
capabilities = ["authenticate"]

[plugin.config]
region = "us-east-1"
service = "execute-api"

[[plugin]]
name = "vault-vars"
path = "~/.config/hurl/plugins/vault-vars.wasm"
enabled = true
capabilities = ["provide_variable"]

[plugin.config]
vault_addr = "https://vault.internal:8200"
```

Request document opting into a specific auth plugin:
```yaml
name: Signed request
method: GET
url: https://api.example.com/private
auth_plugin: aws-sigv4
```

## Plugin Author Example (PDK Side)

Plugin authors create a Rust crate targeting `wasm32-wasip1`:

```rust
use extism_pdk::*;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, FromBytes, ToBytes)]
#[encoding(Json)]
struct HookContext {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    params: Vec<(String, String)>,
    body: Option<String>,
    env_name: Option<String>,
}

#[derive(Serialize, Deserialize, FromBytes, ToBytes)]
#[encoding(Json)]
struct HookResult {
    modified_headers: Option<Vec<(String, String)>>,
    error: Option<String>,
    // ... other fields
}

#[host_fn]
extern "ExtismHost" {
    fn provide_env_var(key: &str) -> String;
    fn plugin_log(message: &str);
}

#[plugin_fn]
pub fn pre_request(ctx: HookContext) -> FnResult<HookResult> {
    let token = unsafe { provide_env_var("API_TOKEN")? };
    let mut headers = ctx.headers;
    headers.push(("Authorization".into(), format!("Bearer {token}")));
    Ok(HookResult {
        modified_headers: Some(headers),
        ..Default::default()
    })
}
```

Build: `cargo build --target wasm32-wasip1 --release`

---

## Testing Strategy

### Unit Tests (in each module)
- `models.rs`: JSON round-trip for all hook types, conversion to/from RequestDocument/ResponseArtifact
- `config.rs`: TOML parsing (valid, missing fields, invalid capabilities, defaults)
- `errors.rs`: Error message formatting
- `registry.rs`: Discovery with no config, disabled plugins, invalid paths
- `hooks.rs`: Mock plugin responses, error propagation, chaining multiple plugins, explicit auth-plugin selection

### Integration Tests (in `tests/`)
- Build a minimal test WASM plugin using `extism-pdk` that echoes modified headers
- Full pipeline: load plugin → run_request → verify plugin mutations applied
- Failure isolation: plugin that panics doesn't crash host
- Timeout: plugin that loops indefinitely gets killed at timeout_ms

### Feature-Gate Tests
- `cargo test` (no features): all existing tests pass, no plugin code compiled
- `cargo test --features plugins`: plugin tests run in addition
- `cargo clippy --all-targets --all-features`: no warnings

### Test Fixtures
- `tests/fixtures/plugins/echo-plugin.wasm` — pre-built test WASM
- `tests/fixtures/plugins/plugins.toml` — test config

---

## Verification

### Automated
- `cargo test` (no features): all existing tests pass, no plugin code compiled
- `cargo test --features plugins`: plugin unit tests + integration tests
- `cargo clippy --all-targets --all-features`: no warnings
- `cargo build` and `cargo build --features plugins`: both compile

### Manual Checklist
- [ ] `.hurl/plugins.toml` with valid plugin loads successfully
- [ ] `hurl plugin list` shows configured plugins with enabled/state/capabilities, including disabled or broken entries
- [ ] Pre-request hook modifies headers visible in response viewer
- [ ] Post-response hook logs tracing event
- [ ] Variable provider resolves `{{var}}` that has no env source
- [ ] Auth plugin adds Authorization header to request
- [ ] Plugin timeout (5s) kills runaway plugin without crashing app
- [ ] Plugin error shows warning in status bar, request proceeds
- [ ] Missing `.wasm` file logs warning, other plugins still load
- [ ] `hurl run` in CLI mode with plugins works identically to TUI
- [ ] Non-plugin build (`cargo build`) is unchanged from Phase 3
- [ ] All Phase 0–3 functionality unaffected

## Acceptance Criteria (from Tech Spec)

- Core workflows remain fast and stable without plugins enabled
- Plugin failure does not crash the app or corrupt request state
- Security boundaries are explicit and reviewable
