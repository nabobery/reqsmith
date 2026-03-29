# hurl/plugins

**WASM Plugin System** — Extism-based extensibility with pre/post hooks, auth, and variable providers.

**Feature gated:** `plugins` feature flag required.

## MODULE MAP

| Module | Responsibility | Key Pattern |
|--------|----------------|-------------|
| `config.rs` | Plugin discovery | Parse `plugins.toml`, validate API version, resolve WASM paths |
| `models.rs` | Hook types | `HookContext`, `HookResult`, `AuthRequest/Result`, `VariableRequest/Result` |
| `registry.rs` | Plugin loading | `PluginRegistry::discover_and_load()`, capability-based lookup |
| `hooks.rs` | Hook execution | Run hooks in order, chain mutations, log errors and skip |
| `host_fns.rs` | Host functions | `provide_env_var`, `plugin_log`, `read_config` for WASM plugins |
| `errors.rs` | Error types | `PluginError` enum for load/timeout/manifest/execution failures |

## PLUGIN CAPABILITIES

| Capability | Hook Function | When Called |
|------------|---------------|-------------|
| `PreRequest` | `pre_request` | Before HTTP execution, may mutate request |
| `PostResponse` | `post_response` | After HTTP execution, may mutate response |
| `Authenticate` | `authenticate` | Add auth headers to request |
| `ProvideVariable` | `provide_variable` | Resolve missing `{{variables}}` |

## HOOK EXECUTION FLOW

```
Runner
  ↓
[plugins] provide_variable → fill missing {{vars}}
  ↓
[plugins] pre_request → mutate request (method/url/headers/body)
  ↓
[plugins] authenticate → add auth headers
  ↓
HTTP execution
  ↓
[plugins] post_response → inspect/mutate response
```

## CONFIGURATION

Search order for `plugins.toml`:
1. `$CWD/.hurl/plugins.toml` (project-local)
2. `$XDG_CONFIG_HOME/hurl/plugins.toml` (user-global)

```toml
api_version = 1
timeout_ms = 5000          # default: 5000
memory_limit_pages = 256   # default: 256

[[plugin]]
name = "auth-sigv4"
path = "plugins/auth.wasm"
enabled = true
capabilities = ["authenticate"]

[plugin.config]
region = "us-east-1"
service = "execute-api"
```

## HOST FUNCTIONS

Plugins can call these host functions:
- `provide_env_var(key) → String` — read environment variable
- `plugin_log(msg)` — emit tracing::info log
- `read_config(key) → String` — read plugin's config value

## DATA TYPES (JSON wire format)

All hook inputs/outputs use JSON serialization.

| Type | Input Fields | Output Fields |
|------|--------------|---------------|
| `HookContext` | method, url, headers, params, body, env_name | — |
| `HookResult` | — | method?, url?, headers?, params?, body?, error? |
| `ResponseContext` | status_code, headers, body_text, duration_ms, content_type | — |
| `AuthRequest` | auth_type, config, method, url, headers, body | — |
| `AuthResult` | — | headers, error? |
| `VariableRequest` | name, config | — |
| `VariableResult` | — | value?, error? |

## CONVENTIONS

- **Feature gate**: All plugin code uses `#[cfg(feature = "plugins")]` in runner
- **Error tolerance**: Hook failures logged via `tracing::warn`, never panic
- **Mutation chaining**: Each plugin's output becomes next plugin's input
- **Capability filter**: Only plugins with matching capability are called
- **Timeout**: Default 5000ms, configurable in `plugins.toml`
- **Memory limit**: Default 256 pages, configurable in `plugins.toml`
- **Dead code**: `#[allow(dead_code)]` on `PluginRegistry` fields reserved for future use

## FILE DISCOVERY

- Config: `.hurl/plugins.toml` or `$XDG_CONFIG_HOME/hurl/plugins.toml`
- WASM files: relative paths resolved from config directory, `~/` expanded
- API version: Must match `CURRENT_API_VERSION` (currently 1)

## QUICK START

- **Add a plugin**: Create WASM module, add entry to `plugins.toml`
- **Add capability**: Implement required export function, declare in `capabilities`
- **Debug hooks**: Check `tracing::info` logs with source = "plugin"
- **Test hooks**: Use `PluginRegistry::discover_and_load()` with tempdir
