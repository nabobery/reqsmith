# reqsmith

[![CI](https://github.com/nabobery/reqsmith/actions/workflows/ci.yml/badge.svg)](https://github.com/nabobery/reqsmith/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Terminal-native API client** — a local-first, Git-friendly alternative to Postman and Insomnia for developers who live in the terminal.

reqsmith sits between `curl` and GUI API clients: it gives you a keyboard-driven TUI for composing and inspecting HTTP requests while keeping all request definitions as plain YAML files that are readable, diffable, and committable to Git.

![reqsmith CLI demo](https://raw.githubusercontent.com/nabobery/reqsmith/main/docs/demo.gif)

New to reqsmith? See [`docs/getting-started.md`](docs/getting-started.md) for a runnable first-run walkthrough.

## Tech Stack

| Concern        | Technology                                                                                                                              |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Language       | Rust (Edition 2024, default build MSRV 1.88)                                                                                            |
| TUI            | ratatui 0.30 + crossterm 0.29                                                                                                           |
| Async runtime  | Tokio (full features)                                                                                                                   |
| HTTP client    | reqwest 0.13 (rustls, HTTP/2, streaming)                                                                                                |
| Serialization  | serde + serde_yaml                                                                                                                      |
| JSON querying  | serde_json + serde_json_path (JSONPath)                                                                                                 |
| Diffing        | similar 2                                                                                                                               |
| CLI parsing    | clap 4 (derive API)                                                                                                                     |
| Error handling | color-eyre + thiserror                                                                                                                  |
| Logging        | tracing + tracing-subscriber + tracing-appender                                                                                         |
| Env loading    | dotenvy                                                                                                                                 |
| Plugin host    | extism 1.30 (optional, `plugins` feature flag — requires **Rust >= 1.91**; see [Build with plugin support](#build-with-plugin-support)) |

## Features

- **Interactive TUI** — three-pane layout (collections, request editor, response viewer) driven entirely by keyboard
- **Headless CLI** — run any saved request without launching the TUI; suitable for CI pipelines
- **YAML request files** — requests stored as `.req.yml` files; human-readable and Git-diff friendly
- **Directory-based collections** — auto-discovers all `.req.yml` files in your project tree on startup
- **Environment variable support** — load variables from `.env`, `.env.<name>`, `reqsmith_envs.yml`, or OS environment
- **Template interpolation** — use `{{variable_name}}` placeholders in URLs, headers, params, and bodies
- **Assertion engine** — declare expected status codes, response times, and JSONPath body assertions in request files
- **Response diffing** — compare two stored run snapshots to detect regressions
- **Collapsible JSON tree viewer** — navigate large JSON responses without leaving the terminal
- **Plugin system** (optional) — extend with WebAssembly plugins via Extism for custom auth and hooks
- **Run history** — persist run results to `.reqsmith/runs/` for later diffing and auditing

## Installation

### Prerequisites

- Rust 1.88 or later for the default (plugin-free) build
- Rust **1.91 or later** if you want the optional `plugins` feature (it
  pulls in Extism/Wasmtime, which need a newer toolchain than the crate's
  own MSRV)
- (Optional) [`just`](https://github.com/casey/just) for development commands

### Install from a release

Prebuilt archives (`reqsmith-<tag>-<target>.tar.gz`, or `.zip` on Windows) are
attached to every [release](https://github.com/nabobery/reqsmith/releases).
Verify one before extracting it:

```bash
sha256sum --ignore-missing -c SHA256SUMS
gh attestation verify reqsmith-<tag>-<target>.tar.gz --repo nabobery/reqsmith
```

See [docs/getting-started.md](docs/getting-started.md#install-from-a-release)
for the full list of targets.

### Build from source

```bash
git clone https://github.com/nabobery/reqsmith
cd reqsmith
cargo build --release --locked
cp target/release/reqsmith ~/.local/bin/reqsmith
```

### Build with plugin support

Requires Rust >= 1.91 (see [Prerequisites](#prerequisites) above):

```bash
cargo build --release --locked --features plugins
```

## Usage

### TUI mode

```bash
reqsmith
reqsmith --tick-rate 100 --frame-rate 33
reqsmith --debug
```

### TUI keyboard shortcuts

| Key                  | Action                                                                                 |
| -------------------- | -------------------------------------------------------------------------------------- |
| `Tab` / `Shift+Tab`  | Cycle focus between panes                                                              |
| `j`/`Down`, `k`/`Up` | Navigate list items                                                                    |
| `Enter`              | Collections: open request / expand directory. Request editor: enter insert (edit) mode |
| `i`                  | Enter insert (edit) mode in the request editor                                         |
| `Ctrl+R`             | Send request                                                                           |
| `Ctrl+S`             | Save request                                                                           |
| `w`                  | Save binary response body (response viewer)                                            |
| `Esc`                | Cancel an in-flight request, or leave insert/edit mode                                 |
| `q` / `Ctrl+C`       | Quit                                                                                   |

### CLI subcommands

```bash
# Run a saved request
reqsmith run requests/create_user.req.yml
reqsmith run requests/create_user.req.yml --env staging
reqsmith run requests/create_user.req.yml --var token=abc123 --save
# Reject private/loopback targets (opt-in SSRF guard; off by default)
reqsmith run requests/create_user.req.yml --deny-private-networks
# Machine-readable JSON output (status, headers, body, assertions, etc.)
reqsmith run requests/create_user.req.yml --output json

# Format request files
reqsmith fmt requests/create_user.req.yml
reqsmith fmt . --check

# Validate request files
reqsmith validate requests/

# List discovered request files
reqsmith list

# Diff two stored run snapshots
reqsmith diff .reqsmith/runs/baseline.json .reqsmith/runs/candidate.json

# Manage plugins (requires plugins feature)
reqsmith plugin list
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

`enabled` on a header/param row defaults to `true` and can be omitted; set it
to `false` to keep a row in the file without sending it. Request files are
discovered by the `.req.yml` (or `.req.yaml`) suffix.

## Environment Variables

Resolution precedence (highest wins):

1. `--var key=value` CLI overrides
2. Named environment in `reqsmith_envs.yml` (selected with `--env <name>`)
3. `.env.<name>` file
4. `.env` file
5. OS environment variables

### `reqsmith_envs.yml` example

Named environments live under a top-level `environments:` key:

```yaml
environments:
  dev:
    base_url: http://localhost:8080
    token: dev-token

  staging:
    base_url: https://staging.example.com
    token: staging-token
```

## Exit Codes

| Code  | Meaning                             |
| ----- | ----------------------------------- |
| `0`   | Success                             |
| `1`   | Internal or configuration error     |
| `2`   | Validation or interpolation failure |
| `3`   | Network or transport failure        |
| `4`   | Assertion failure                   |
| `130` | Interrupted (`Ctrl+C`)              |

## Storage Layout

| Path                     | Contents                                                  |
| ------------------------ | --------------------------------------------------------- |
| `.reqsmith/runs/`        | Stored run snapshots, one `*.json` file per saved run     |
| `.reqsmith/downloads/`   | Binary response bodies                                    |
| `.reqsmith/plugins.toml` | Project-local plugin config, merged with your user-global `reqsmith/plugins.toml` under `$XDG_CONFIG_HOME` on Linux or `~/Library/Application Support` on macOS (see [Security](#security)) |

## Security

reqsmith's threat model, redaction contract, and plugin trust boundaries are
documented in detail in [`docs/security-model.md`](docs/security-model.md)
and [`docs/plugins-security.md`](docs/plugins-security.md). The short
version:

- **Response header redaction.** Sensitive _response_ headers (by default:
  `authorization`, `proxy-authorization`, `cookie`, `set-cookie`,
  `x-api-key`, `api-key`, `x-auth-token`, `x-amz-security-token`,
  `www-authenticate`, `authentication`, plus a growing list of CSRF/HMAC and
  config-style key names — matched case-insensitively; see
  [`docs/security-model.md`](docs/security-model.md) for the full set) are
  replaced with `<redacted:sha256:XXXXXXXX>` (eight hex characters of the
  real value's SHA-256 digest, so a rotated secret still shows as *changed*
  when two saved runs are diffed) before they're printed, diffed, or written
  to a `.reqsmith/runs/*.json` snapshot. Extend the set with a comma-separated
  `REQSMITH_REDACT_HEADERS` environment variable, e.g.
  `REQSMITH_REDACT_HEADERS=x-internal-token,x-tenant-secret`. This contract is
  deliberately header-name based: request files and response bodies are not
  rewritten, and a plugin's `post_response` hook can rename a sensitive
  header before this check ever sees it (see
  [`docs/plugins-security.md`](docs/plugins-security.md)). Do not hardcode
  credentials in request files or return secrets in response bodies that you
  plan to print or save.
- **Project-local plugins are opt-in.** reqsmith loads and merges both
  `./.reqsmith/plugins.toml` (project-local) and your user-global
  `reqsmith/plugins.toml` — `$XDG_CONFIG_HOME/reqsmith/plugins.toml` on Linux
  (default `~/.config/reqsmith/plugins.toml`), or `~/Library/Application
  Support/reqsmith/plugins.toml` on macOS — into one registry, but only
  instantiates WASM from the project-local file if you explicitly set
  `REQSMITH_ALLOW_PROJECT_PLUGINS=1` (or `true`/`yes`) for that invocation —
  this stops a cloned repository from silently running WASM the moment you
  run `reqsmith` in it. User-global plugins are trusted as before and always
  load. Set the variable per-invocation, never from a shell profile or CI's
  global env — an always-on export opts every repo you `cd` into.
- **Opt-in private-network guard.** `reqsmith run --deny-private-networks`
  (or `REQSMITH_DENY_PRIVATE_NETWORKS=1`, the only way to enable it for the
  TUI) rejects requests to unspecified, loopback, "this network" (`0.0.0.0/8`),
  RFC1918, link-local, CGNAT, multicast, reserved, IPv6 unique-local, and
  IPv4-mapped/tunneled
  equivalents of those addresses (a basic SSRF guard), and applies the same
  check to every redirect hop, not just the initial URL; it fails closed on
  any private-resolving or unresolvable host and ignores proxy environment
  variables while active. Off by default so localhost/LAN targets keep
  working. See the documented DNS-rebinding limitation in
  [`docs/security-model.md`](docs/security-model.md).
- **Atomic, symlink-refusing writes.** Run snapshots, downloads, and
  `reqsmith fmt` rewrites go through one atomic writer that never follows a
  symlinked destination and reserves snapshot filenames race-free.
- **Reporting a vulnerability**: see [`SECURITY.md`](SECURITY.md).

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
just build      # Build in debug mode
just release    # Build in release mode
just run        # Run the application
just test       # Run all tests (same two invocations as the `test` CI job)
just fmt        # Format code
just fmt-check  # Check formatting without modifying
just lint       # Run clippy lints (same two invocations as the `clippy` CI job)
just fix        # Auto-fix clippy and format issues
just ci         # Fast pre-push gate: fmt-check + lint + test
just audit      # Advisory scan (requires cargo-audit)
just deny       # License/advisory/source policy (requires cargo-deny)
just secrets    # Secret scan (requires gitleaks)
just ci-full    # Everything CI runs: ci + audit + deny + secrets
just doc        # Generate and open documentation
just clean      # Remove build artifacts
just watch      # Watch for changes and rebuild (requires cargo-watch)
```

> `just lint`, `just fix`, `just ci`, and `just ci-full` build `--all-features`
> (the `plugins` feature), so they require Rust >= 1.91. On an older
> toolchain, run the default-feature commands directly (e.g. `cargo clippy
> --all-targets --locked -- -D warnings`); the MSRV-1.88 guarantee covers the
> default build only. `just ci-full` additionally needs `cargo-audit`,
> `cargo-deny`, and `gitleaks` installed.

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the full contribution workflow.

## License

MIT — see [`LICENSE`](LICENSE).

## Community

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to propose changes and run the project's checks
- [`SECURITY.md`](SECURITY.md) — how to report a vulnerability
- [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) — community expectations
- [`SUPPORT.md`](SUPPORT.md) — where to get help
