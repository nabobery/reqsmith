# hurl

**Terminal-native API client** — local-first, Git-friendly alternative to Postman/Insomnia.

**Updated:** 2026-03-29
**Commit:** 662e9da
**Branch:** main

## STACK

| Layer | Technology |
|-------|------------|
| Language | Rust 2024 edition (MSRV 1.85) |
| TUI | ratatui 0.30 + crossterm 0.29 |
| Async | tokio (full features) |
| HTTP | reqwest (planned) |
| Errors | color-eyre + thiserror |
| Logging | tracing + tracing-subscriber |
| CLI | clap 4 (derive) |
| Storage | serde + serde_yaml |

## STRUCTURE

```
hurl/
├── src/              # Core application (8 modules)
│   ├── main.rs       # Entry point, wires CLI → Config → TUI → App
│   ├── app.rs        # Root model: event loop, focus, rendering (275 lines, heavy tests)
│   ├── tui.rs        # Terminal lifecycle: raw mode, alt screen, panic hook
│   ├── action.rs     # Action/FocusTarget enums — shared vocabulary
│   ├── cli.rs        # clap Parser: --debug, --tick-rate, --frame-rate
│   ├── config.rs     # Config from CLI + defaults (250ms tick, 16ms frame)
│   ├── errors.rs     # HurlError enum (Terminal, Io, Config)
│   ├── logging.rs    # File-based tracing with fallback directories
│   └── components/   # Pane implementations
├── docs/
│   ├── specs/        # Technical specification
│   └── plans/        # Project plans
├── Cargo.toml        # Dependencies + release profile (LTO, strip, panic=abort)
├── justfile          # Dev commands: build, test, lint, ci, watch
├── rustfmt.toml      # max_width=100, use_field_init_shorthand
└── clippy.toml       # Empty (default lints)
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Terminal setup/teardown | `src/tui.rs` | EnterGuard pattern, panic hook |
| Event loop + rendering | `src/app.rs` | tokio::select! for render/tick/events |
| Keyboard handling | `src/app.rs:86-104` | Global keys → focused component delegation |
| Focus cycling | `src/action.rs` | FocusTarget::next/prev with 3 panes |
| Component trait | `src/components/mod.rs` | handle_key + render + focus/blur hooks |
| Collections pane | `src/components/collections.rs` | j/k navigation, vim-style |
| Logging setup | `src/logging.rs` | Fallback dirs: data_local_dir → temp → .hurl/logs |

## CONVENTIONS

- **Edition 2024** — use `use ...` not `use crate::...` where path resolution allows
- **Field init shorthand** — `Self { config }` not `Self { config: config }`
- **100 char line width** — enforced by rustfmt.toml
- **Component pattern** — all panes implement `Component` trait
- **Action enum** — state transitions go through `Action`, not direct mutations
- **Test-first** — every module has inline `#[cfg(test)] mod tests`
- **Focus-based input** — only focused pane receives key events

## ANTI-PATTERNS (THIS PROJECT)

- `#[allow(dead_code)]` used intentionally — variants reserved for future steps (Step 4+, 5, 6, 8+)
- No `@ts-ignore` / `as any` equivalents — Rust type safety enforced
- No blocking in event loop — use `tokio::spawn` or `spawn_blocking`
- Never restore terminal manually — `Tui::reset()` is single cleanup path
- No `panic!` in production code — use `color_eyre::Result` for error propagation
- `unwrap()` only in tests — production code uses `?` or explicit error handling

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

- Log file fallback order matters: data_local_dir → temp → .hurl/logs (see logging.rs tests)
- Panic hook installed in `Tui::enter()` — terminal always restores on crash
- `render` and `tick` intervals are independent (frame_rate vs tick_rate)
