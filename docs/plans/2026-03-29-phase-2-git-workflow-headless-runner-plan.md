# Phase 2: Git Workflow and Headless Runner — Implementation Plan

## Context

Phase 1 delivered a usable interactive TUI: collection discovery, request editing, async HTTP execution with cancellation, response viewing, `.env` loading, and YAML persistence. Phase 2 turns hurl into a dual-mode tool — any request file used in the TUI becomes runnable headlessly in CI/scripts with environment resolution, validation, formatting, and predictable exit codes. The spec's key constraint: **TUI and CLI must share the same execution core**.

---

## Implementation Steps

### Step 1: CLI Restructuring (no behavioral change)

**Files:** `src/cli.rs` (rewrite), `src/main.rs` (modify), `src/config.rs` (modify)

Convert flat `Cli` struct to subcommand-based CLI using clap's derive API. `hurl` with no subcommand = TUI (backward compatible).

```rust
#[derive(Parser)]
#[command(name = "hurl", version, about = "Terminal-native API client")]
pub struct Cli {
    #[arg(short, long, global = true)]
    pub debug: bool,
    #[arg(long)]
    pub tick_rate: Option<u64>,
    #[arg(long)]
    pub frame_rate: Option<u64>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    Run { file: PathBuf, #[arg(short, long)] env: Option<String>,
           #[arg(long = "var", value_parser = parse_key_value)] vars: Vec<(String, String)>,
           #[arg(short, long, default_value = "human")] output: OutputMode,
           #[arg(short, long)] quiet: bool },
    Fmt  { files: Vec<PathBuf>, #[arg(long)] check: bool },
    Validate { files: Vec<PathBuf>, #[arg(short, long)] env: Option<String>,
               #[arg(short, long, default_value = "human")] output: OutputMode },
    List { #[arg(default_value = ".")] path: PathBuf,
           #[arg(short, long, default_value = "human")] output: OutputMode },
}

#[derive(ValueEnum, Clone, Default)]
pub enum OutputMode { #[default] Human, Json }
```

- `main.rs`: match on `cli.command` — `None` or missing → launch TUI; `Some(cmd)` → dispatch to command handler
- `config.rs`: adapts to accept optional TUI timing overrides from the CLI path
- Preserve `--tick-rate`/`--frame-rate` for TUI launches so Phase 1 behavior stays intact; parse them at the top level and only apply them when no subcommand is present

**Tests:** Parse each subcommand, default (None) behavior, `--var key=value` parsing, invalid args rejected.
**Gate:** `just ci` passes, `cargo run` still launches TUI.

### Step 2: New Data Models

**File:** `src/core/models.rs` (extend)

Add these types:

```rust
// Environment resolution
pub enum VarSource { OsEnv, DotEnv, DotEnvNamed(String), HurlEnvsYml(String), CliOverride }
pub struct EnvironmentSet { pub values: HashMap<String, String>, pub sources: HashMap<String, VarSource> }

// Validation
pub enum ValidationSeverity { Error, Warning }
pub struct ValidationDiagnostic { pub severity, pub message: String, pub field: Option<String> }
pub struct ValidationReport { pub diagnostics: Vec<ValidationDiagnostic> }

// Execution result
pub struct RunResult { pub request_name: String, pub request_file: PathBuf,
    pub response: Option<ResponseArtifact>, pub error: Option<String>,
    pub exit_code: ExitCode, pub cancelled: bool }
pub enum ExitCode { Success=0, InternalError=1, ValidationFailure=2, NetworkFailure=3,
    AssertionFailure=4, Interrupted=130 }
```

**Tests:** Serde roundtrip where applicable, `ValidationReport::has_errors()`, `ExitCode` conversion.
**Gate:** `just ci` passes.

### Step 3: Layered Environment Resolution

**New file:** `src/core/environment.rs`
**Modify:** `src/infra/env_loader.rs` (add `load_named_env`), `src/core/mod.rs`

Precedence (highest wins):
1. CLI `--var` overrides
2. Named env from `hurl_envs.yml`
3. `.env.<name>` file
4. `.env` file
5. OS environment (fallback for unresolved vars only)

```rust
pub fn resolve_environment(
    cwd: &Path,
    env_name: Option<&str>,
    cli_vars: &[(String, String)],
) -> Result<EnvironmentSet, EnvironmentError>
```

`hurl_envs.yml` schema:
```yaml
environments:
  staging:
    base_url: https://staging.example.com
    token: staging-token
```

Serde model: `HurlEnvsFile { environments: BTreeMap<String, BTreeMap<String, String>> }`

**Key decisions:**
- `hurl_envs.yml` missing = no error. `--env staging` but no `staging` key = error.
- OS env is **fallback only** — not eagerly loaded. After merging layers 1-4, check `std::env::var()` for any remaining unresolved `{{var}}` references.
- `load_named_env(cwd, name)` reads `.env.<name>` via dotenvy, returns empty map if missing.
- Headless resolution is request-relative: `.env`, `.env.<name>`, and `hurl_envs.yml` are loaded from the request file's parent directory, falling back to process cwd only for unsaved/in-memory requests.

**Tests:** Each layer in isolation, merge precedence (higher overrides lower), missing files graceful, OS env fallback, named env not found error.
**Gate:** `just ci` passes.

### Step 4: Shared Runner

**New file:** `src/core/runner.rs`
**Modify:** `src/core/mod.rs`, `src/app.rs`

```rust
pub struct RunOptions {
    pub env_name: Option<String>,
    pub cli_vars: Vec<(String, String)>,
    pub validate_before_run: bool,
    pub cwd: PathBuf,
}

pub async fn run_request(
    client: &reqwest::Client,
    doc: &RequestDocument,
    options: &RunOptions,
    cancel: CancellationToken,
) -> RunResult
```

Pipeline: resolve env (request-relative) → validate (optional) → execute with a reusable client → map result to `RunResult` with proper `ExitCode`.

**TUI refactor:** `app.rs` `SendRequest` handler calls `runner::run_request()` instead of directly calling `execution::execute_request()`. Pass the existing pooled `reqwest::Client` plus the user-controlled `cancel_token`. This keeps connection reuse intact while ensuring env resolution is consistent between TUI and CLI.

**Tests:** Validation failure → `ExitCode::ValidationFailure`, network error → `ExitCode::NetworkFailure`, cancellation → `ExitCode::Interrupted` plus structured `cancelled` state, request-relative env lookup works for nested files.
**Gate:** `just ci` passes, TUI still works identically.

### Step 5: Validation Pipeline

**New file:** `src/core/validation.rs`
**Modify:** `src/core/mod.rs`

```rust
pub fn validate_document(doc: &RequestDocument, env: Option<&EnvironmentSet>) -> ValidationReport
```

Checks:
1. **Schema:** name non-empty, url non-empty, method valid, header keys non-empty
2. **URL structure:** after interpolation, parse as a real URL; warn only when the URL is merely missing a scheme, error for malformed absolute URLs
3. **Interpolation completeness:** all `{{var}}` resolvable; missing = error
4. **Header validity:** names are valid HTTP header names
5. **Body consistency:** warn if body present for GET/HEAD/OPTIONS

**Tests:** Each check type, clean doc passes, multiple errors collected.
**Gate:** `just ci` passes.

### Step 6: Formatter

**New file:** `src/core/formatter.rs`
**Modify:** `src/core/mod.rs`

Canonical YAML output with deterministic key ordering:
1. `name` 2. `method` 3. `url` 4. `headers` (sorted by key) 5. `params` (sorted by key) 6. `body`

Empty collections omitted. Uses `serde_yaml::Value::Mapping` with controlled insertion order for full control over output.

```rust
pub fn format_document(doc: &RequestDocument) -> String
pub fn is_formatted(path: &Path) -> Result<bool>
pub fn format_file(path: &Path) -> Result<bool>  // returns true if changed
```

**Tests:** Round-trip stability (format twice = same), canonical key order, sorted headers/params, empty collections omitted.
**Gate:** `just ci` passes.

### Step 7: Output Formatting

**New file:** `src/output.rs`
**Modify:** `src/main.rs`

```rust
pub fn print_run_result(result: &RunResult, mode: &OutputMode, quiet: bool)
pub fn print_validation_report(report: &ValidationReport, path: &Path, mode: &OutputMode)
pub fn print_list(nodes: &[CollectionNode], mode: &OutputMode)
```

- **Human mode:** Status line (`200 OK (145ms)`), headers, body excerpt. Errors to stderr.
- **JSON mode:** Structured JSON to stdout (`{ "status": 200, "duration_ms": 145, ... }`)
- **Quiet mode:** Suppress success output; errors still go to stderr.

**Tests:** Human output contains status code, JSON output parses as valid JSON, quiet suppresses success.
**Gate:** `just ci` passes.

### Step 8: Command Handlers

**New files:** `src/commands/mod.rs`, `src/commands/run.rs`, `src/commands/fmt.rs`, `src/commands/validate.rs`, `src/commands/list.rs`
**Modify:** `src/main.rs`

Each command handler is a standalone async function wired from `main.rs`:

- **`run`:** Load file → build reusable client → `run_request()` → `print_run_result()` → exit code. Install `tokio::signal::ctrl_c()` to cancel the token for graceful SIGINT handling.
- **`fmt`:** Discover or use explicit files → `format_file()` each → report changes. With no file arguments, discover `.hurl.yml` files from the current directory. `--check` mode: exit 1 if any unformatted.
- **`validate`:** Load files → resolve env per request file location → `validate_document()` → `print_validation_report()` → exit 2 if errors. With no file arguments, discover `.hurl.yml` files from the current directory.
- **`list`:** `discover_requests()` → `print_list()`.

**Tests:** Integration tests using `std::process::Command` or `assert_cmd` to run the binary.
**Gate:** `just ci` passes.

### Step 9: Error Handling and Exit Codes

**Modify:** `src/errors.rs`

Add variants: `Environment(String)`, `Validation(String)`, `Formatter(String)`

Exit code contract:
| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Internal/config error |
| 2 | Validation/interpolation failure |
| 3 | Network/transport failure |
| 4 | Assertion failure (reserved) |
| 130 | Interrupted by user (`Ctrl+C`) |

Ensure stderr vs stdout discipline: errors to stderr, data to stdout.

### Step 10: TUI Environment Integration

**Modify:** `src/app.rs`

Replace raw `env_loader::load_env()` with `environment::resolve_environment()` so TUI gets the same layered resolution (minus CLI vars). This is a small refactor since Step 4 already wired `runner::run_request()`.

**Gate:** Full manual verification + `just ci`.

---

## Dependency Graph

```
Step 1 (CLI) ──────────────────────────────────────> Step 8 (commands)
Step 2 (models) ──┬──> Step 3 (environment) ──> Step 4 (runner) ──> Step 8
                  ├──> Step 5 (validation) ──────────────────────> Step 8
                  ├──> Step 6 (formatter) ───────────────────────> Step 8
                  └──> Step 7 (output) ──────────────────────────> Step 8
Step 9 (errors) integrated throughout
Step 10 (TUI env) after Step 4
```

Steps 3, 5, 6, 7 can be parallelized after Step 2.

## New Files (10)

| File | Purpose |
|------|---------|
| `src/core/environment.rs` | Layered env resolution pipeline |
| `src/core/runner.rs` | Shared `run_request()` for TUI + CLI |
| `src/core/validation.rs` | Schema + interpolation validation |
| `src/core/formatter.rs` | Canonical YAML formatter |
| `src/output.rs` | Human/JSON/Quiet output rendering |
| `src/commands/mod.rs` | Command handler module declarations |
| `src/commands/run.rs` | `hurl run` handler |
| `src/commands/fmt.rs` | `hurl fmt` handler |
| `src/commands/validate.rs` | `hurl validate` handler |
| `src/commands/list.rs` | `hurl list` handler |

## Modified Files (9)

| File | Change |
|------|--------|
| `Cargo.toml` | Add `serde_json` (for JSON output), `assert_cmd` + `predicates` (dev-deps) |
| `src/main.rs` | Subcommand dispatch |
| `src/cli.rs` | Subcommand enum rewrite |
| `src/config.rs` | Adapt for new CLI shape |
| `src/core/models.rs` | Add `EnvironmentSet`, `VarSource`, `ValidationReport`, `RunResult`, `ExitCode` |
| `src/core/mod.rs` | Register new modules |
| `src/infra/env_loader.rs` | Add `load_named_env()` |
| `src/app.rs` | Use `runner::run_request()` + `resolve_environment()` |
| `src/errors.rs` | Add Phase 2 error variants |

## New Dependencies

| Crate | Purpose |
|-------|---------|
| `serde_json` | JSON output mode serialization |
| `assert_cmd` (dev) | CLI integration tests |
| `predicates` (dev) | Assertion helpers for CLI tests |

## Key Architectural Decisions

1. **Runner accepts `CancellationToken` plus a reusable `reqwest::Client`** — TUI passes its user-controlled token and pooled client; CLI passes a SIGINT-driven token and a per-command client
2. **OS env as fallback, not eagerly loaded** — prevents namespace pollution; only referenced `{{var}}` names fall through to `std::env::var()`
3. **`hurl_envs.yml` is optional** — missing file = no error; `--env X` with missing key X = error
4. **Headless env lookup is request-relative** — nested request files resolve their local `.env` / `.env.<name>` / `hurl_envs.yml` regardless of the caller's cwd
5. **`hurl` with no subcommand = TUI** — backward compatible; `hurl tui` is not needed as a separate subcommand
6. **Formatter uses controlled canonical ordering** — stable YAML diffs without relying on incidental serialization order

## Verification

### Automated
- `just ci` passes at every step (fmt-check + clippy + tests)
- Unit tests for each new module
- Integration tests for CLI commands via `assert_cmd`
- Environment precedence tests

### Manual Checklist
- [ ] `hurl` (no args) still launches TUI
- [ ] `hurl run requests/example.hurl.yml` executes and prints response
- [ ] `hurl run requests/example.hurl.yml --env staging` resolves env vars correctly
- [ ] `hurl run requests/example.hurl.yml --var base_url=http://localhost:8080` overrides
- [ ] `hurl run requests/example.hurl.yml --output json` produces valid JSON
- [ ] `hurl run requests/example.hurl.yml --quiet` suppresses output on success
- [ ] `hurl run` against unreachable host exits with code 3
- [ ] `hurl run` with unresolved `{{var}}` exits with code 2
- [ ] `hurl validate requests/example.hurl.yml` catches schema errors
- [ ] `hurl fmt requests/example.hurl.yml` normalizes YAML
- [ ] `hurl fmt --check` exits 1 if unformatted
- [ ] `hurl list` shows discovered .hurl.yml files
- [ ] Ctrl+C during `hurl run` cancels gracefully and returns exit code `130`
- [ ] TUI SendRequest uses same env resolution as CLI
