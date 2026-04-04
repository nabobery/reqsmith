# hurl

**Terminal-native API client** — a local-first, Git-friendly alternative to Postman and Insomnia for developers who live in the terminal.

hurl (HTTP Ultra-Rapid Launcher) sits between `curl` and GUI API clients: it gives you a keyboard-driven TUI for composing and inspecting HTTP requests while keeping all request definitions as plain YAML files that are readable, diffable, and committable to Git.

## Tech Stack

| Concern | Technology |
|---|---|
| Language | Rust (Edition 2024, MSRV 1.85) |
| TUI | ratatui 0.30 + crossterm 0.29 |
| Async runtime | Tokio (full features) |
| HTTP client | reqwest 0.13 (rustls, HTTP/2, streaming) |
| Serialization | serde + serde_yaml |
| JSON querying | serde_json + serde_json_path (JSONPath) |
| Diffing | similar 2 |
| CLI parsing | clap 4 (derive API) |
| Error handling | color-eyre + thiserror |
| Logging | tracing + tracing-subscriber + tracing-appender |
| Env loading | dotenvy |
| Plugin host | extism 1.7 (optional, `plugins` feature flag) |

## Features

- **Interactive TUI** — three-pane layout (collections, request editor, response viewer) driven entirely by keyboard
- **Headless CLI** — run any saved request without launching the TUI; suitable for CI pipelines
- **YAML request files** — requests stored as `.hurl.yml` files; human-readable and Git-diff friendly
- **Directory-based collections** — auto-discovers all `.hurl.yml` files in your project tree on startup
- **Environment variable support** — load variables from `.env`, `.env.<name>`, `hurl_envs.yml`, or OS environment
- **Template interpolation** — use `{{variable_name}}` placeholders in URLs, headers, params, and bodies
- **Assertion engine** — declare expected status codes, response times, and JSONPath body assertions in request files
- **Response diffing** — compare two stored run snapshots to detect regressions
- **Collapsible JSON tree viewer** — navigate large JSON responses without leaving the terminal
- **Plugin system** (optional) — extend with WebAssembly plugins via Extism for custom auth and hooks
- **Run history** — persist run results to `.hurl/runs/` for later diffing and auditing

## Installation

### Prerequisites

- Rust 1.85 or later
- (Optional) [`just`](https://github.com/casey/just) for development commands

### Build from source

```bash
git clone https://github.com/nabobery/hurl
cd hurl
cargo build --release --locked
cp target/release/hurl ~/.local/bin/hurl
```

### Build with plugin support

```bash
cargo build --release --locked --features plugins
```

## Usage

### TUI mode

```bash
hurl
hurl --tick-rate 100 --frame-rate 33
hurl --debug
```

### TUI keyboard shortcuts

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Cycle focus between panes |
| `j` / `k` | Navigate list items |
| `Enter` or `Ctrl+R` | Send request |
| `Ctrl+S` | Save request |
| `Esc` | Cancel / leave editor mode |
| `Ctrl+C` | Cancel in-flight request or exit |
| `?` | Open help |

### CLI subcommands

```bash
# Run a saved request
hurl run requests/create_user.hurl.yml
hurl run requests/create_user.hurl.yml --env staging
hurl run requests/create_user.hurl.yml --var token=abc123 --save

# Format request files
hurl fmt requests/create_user.hurl.yml
hurl fmt . --check

# Validate request files
hurl validate requests/

# List discovered request files
hurl list

# Diff two stored run snapshots
hurl diff .hurl/runs/baseline.yml .hurl/runs/candidate.yml

# Manage plugins (requires plugins feature)
hurl plugin list
```

## Request File Format

```yaml
name: "Create New User"
method: POST
url: "{{base_url}}/api/v1/users"
headers:
  - key: Content-Type
    value: application/json
    enabled: true
  - key: Authorization
    value: "Bearer {{token}}"
    enabled: true
params:
  - key: page
    value: "{{page}}"
    enabled: true
body: |
  {
    "username": "tui_fanatic",
    "role": "admin"
  }
assertions:
  - expect_status: 201
  - expect_time_under: 500ms
  - expect_body_path:
      path: "$.user.id"
      operator: eq
      expected: 42
```

Supported methods: `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`

Assertion operators for `expect_body_path`: `eq`, `ne`, `gt`, `lt`, `gte`, `lte`, `contains`, `exists`

## Environment Variables

Resolution precedence (highest wins):
1. `--var key=value` CLI overrides
2. Named environment in `hurl_envs.yml` (selected with `--env <name>`)
3. `.env.<name>` file
4. `.env` file
5. OS environment variables

### `hurl_envs.yml` example

```yaml
dev:
  base_url: http://localhost:8080
  token: dev-token

staging:
  base_url: https://staging.example.com
  token: staging-token
```

## Exit Codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Internal or configuration error |
| `2` | Validation or interpolation failure |
| `3` | Network or transport failure |
| `4` | Assertion failure |
| `130` | Interrupted (`Ctrl+C`) |

## Storage Layout

| Path | Contents |
|---|---|
| `.hurl/runs/` | Stored run YAML snapshots |
| `.hurl/downloads/` | Binary response bodies |
| `.hurl/plugins.toml` | Plugin registry |

## Project Structure

```
src/
├── main.rs
├── app.rs               # Root model and event loop
├── tui.rs               # Terminal lifecycle
├── cli.rs               # clap CLI definitions
├── commands/            # run, fmt, validate, list, diff, plugin subcommands
├── components/          # TUI panes
├── core/                # Business logic: models, runner, assertions, diffing, storage
├── plugins/             # WASM plugin registry and hooks
└── infra/               # HTTP client, environment loader
```

## Development

```bash
just build      # cargo build --locked
just test       # cargo test --locked
just lint       # cargo clippy --all-targets -- -D warnings
just fmt        # cargo fmt
just ci         # fmt-check + lint + test
just run        # cargo run --locked
just release    # cargo build --release --locked
just doc        # cargo doc --no-deps --open
just clean      # cargo clean
```

## License

MIT
