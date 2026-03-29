# hurl

**Terminal-native API client** — local-first, Git-friendly alternative to Postman/Insomnia.

**Updated:** 2026-03-29
**Branch:** main
**Features:** `plugins` (extism WASM)

## STACK

| Layer | Technology |
|-------|------------|
| Language | Rust 2024 edition (MSRV 1.85) |
| TUI | ratatui 0.30 + crossterm 0.29 |
| Async | tokio (full features) |
| HTTP | reqwest 0.13 (rustls, json, stream) |
| Diffing | similar 2 (text diff) |
| JSON | serde_json + serde_json_path (JSONPath) |
| Errors | color-eyre + thiserror |
| Logging | tracing + tracing-subscriber |
| CLI | clap 4 (derive) |
| Storage | serde + serde_yaml + tempfile (dev) |

## STRUCTURE

```
hurl/
├── src/
│   ├── main.rs            # Entry point, wires CLI → Config → TUI → App
│   ├── app.rs             # Root model: event loop, focus, rendering
│   ├── tui.rs             # Terminal lifecycle: raw mode, alt screen, panic hook
│   ├── action.rs          # Action/FocusTarget enums — shared vocabulary
│   ├── cli.rs             # clap Parser: --debug, --tick-rate, --frame-rate
│   ├── config.rs          # Config from CLI + defaults (250ms tick, 16ms frame)
│   ├── errors.rs          # HurlError enum (Terminal, Io, Config)
│   ├── logging.rs         # File-based tracing with fallback directories
│   ├── output.rs          # Human/JSON output for run results, diffs, validation
│   ├── commands/          # CLI subcommands (run, fmt, validate, list, diff, plugin)
│   ├── components/        # TUI panes (collections, request_editor, response_viewer, json_tree)
│   ├── core/              # Business logic (models, runner, assertions, diffing, storage, validation)
│   ├── plugins/           # WASM plugin system (extism, hooks, host functions)
│   └── infra/             # Infrastructure (http_client, env_loader)
├── docs/
│   ├── specs/             # Technical specification
│   └── plans/             # Project plans
├── Cargo.toml             # Dependencies + release profile
├── justfile               # Dev commands
├── rustfmt.toml           # max_width=100
└── clippy.toml            # Empty (default lints)
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Terminal setup/teardown | `src/tui.rs` | EnterGuard pattern, panic hook |
| Event loop + rendering | `src/app.rs` | tokio::select! for render/tick/events |
| Keyboard handling | `src/app.rs` | Global keys → focused component delegation |
| Focus cycling | `src/action.rs` | FocusTarget::next/prev with 3 panes |
| Component trait | `src/components/mod.rs` | handle_key + render + focus/blur + is_editing |
| Collections pane | `src/components/collections.rs` | j/k navigation, vim-style |
| JSON tree viewer | `src/components/json_tree.rs` | JsonTreeState with collapse/expand |
| Response viewer | `src/components/response_viewer.rs` | Renders response + JSON tree |
| Assertion evaluation | `src/core/assertions.rs` | ExpectStatus, ExpectTimeUnder, ExpectBodyPath |
| Response diffing | `src/core/diffing.rs` | Status/header/body diff with JSON normalization |
| Run storage | `src/core/storage.rs` | Save/load runs to .hurl/runs/, body downloads |
| Document validation | `src/core/validation.rs` | Schema + interpolation + assertion checks |
| CLI diff command | `src/commands/diff.rs` | Compare two stored runs |
| CLI plugin command | `src/commands/plugin.rs` | List/info for WASM plugins |
| Output formatting | `src/output.rs` | Human/JSON for run, diff, validation, list |
| Request runner | `src/core/runner.rs` | Shared execution pipeline (TUI + CLI) |
| Plugin system | `src/plugins/` | Extism WASM plugin registry, hooks, host functions |

## CONVENTIONS

- **Edition 2024** — use `use ...` not `use crate::...` where path resolution allows
- **Field init shorthand** — `Self { config }` not `Self { config: config }`
- **100 char line width** — enforced by rustfmt.toml
- **Component pattern** — all panes implement `Component` trait
- **Action enum** — state transitions go through `Action`, not direct mutations
- **Test-first** — every module has inline `#[cfg(test)] mod tests`
- **Focus-based input** — only focused pane receives key events
- **`#[allow(dead_code)]`** — used intentionally for reserved variants

## ANTI-PATTERNS (THIS PROJECT)

- No blocking in event loop — use `tokio::spawn` or `spawn_blocking`
- Never restore terminal manually — `Tui::reset()` is single cleanup path
- No `panic!` in production code — use `color_eyre::Result` for error propagation
- `unwrap()` only in tests — production code uses `?` or explicit error handling
- Don't suppress clippy warnings without documented reason
- No type suppression (`as any`, `@ts-ignore` equivalents) — Rust type safety enforced

## COMMANDS

```bash
just build       # cargo build --locked
just test        # cargo test --locked
just lint        # cargo clippy --all-targets --all-features --locked -- -D warnings
just ci          # fmt-check + lint + test
just run *ARGS   # cargo run --locked -- {{ARGS}}
just watch       # cargo watch -x 'clippy --all-targets -- -D warnings' -x test
```

## NOTES

- Log file fallback order: data_local_dir → temp → .hurl/logs
- Panic hook in `Tui::enter()` — terminal always restores on crash
- `render` and `tick` intervals are independent (frame_rate vs tick_rate)
- Run snapshots stored in `.hurl/runs/`, binary bodies in `.hurl/downloads/`
- Assertion YAML format: `expect_status`, `expect_time_under: 500ms`, `expect_body_path`
- Plugin system uses `extism` WASM — gated behind `plugins` feature flag
- Plugin config: `.hurl/plugins.toml` or `$XDG_CONFIG_HOME/hurl/plugins.toml`
