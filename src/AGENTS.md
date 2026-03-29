# hurl/src

**Core Logic + TUI Engine** — Elm-inspired state management in Rust 2024.

## MODULE MAP

| Module | Responsibility | Key Pattern |
|--------|----------------|-------------|
| `main.rs` | Entry point | CLI → Config → TUI → App wiring |
| `app.rs` | Root model | `tokio::select!` loop (render/tick/events) |
| `tui.rs` | Terminal lifecycle | `EnterGuard` pattern + panic hook |
| `action.rs` | State vocabulary | `Action` + `FocusTarget` enums |
| `cli.rs` | Argument parsing | `clap` Parser (tick/frame rates) |
| `config.rs` | App settings | CLI overrides + 250ms/16ms defaults |
| `errors.rs` | Error handling | `HurlError` (Terminal, Io, Config, ...) |
| `logging.rs` | Tracing | Fallback: data_local → temp → .hurl/logs |
| `output.rs` | Output formatting | Print results in JSON/YAML/text modes |
| `components/` | UI panes | See `components/AGENTS.md` |
| `core/` | Business logic | See `core/AGENTS.md` |
| `commands/` | CLI subcommands | See `commands/AGENTS.md` |
| `infra/` | Infrastructure | See `infra/AGENTS.md` |

## ROUTING & STATE

- **Input Flow**: `Tui` (crossterm) → `App` (event loop) → `Component` (focused pane).
- **State Transitions**: Components return `Option<Action>`. `App` processes actions.
- **Focus Management**: `FocusTarget` in `action.rs` defines cycling order.
- **Rendering**: `App::draw` delegates to focused + background components.

## CONVENTIONS

- **Edition 2024**: Use `use ...` for local path resolution.
- **Field Shorthand**: `Self { config }` preferred.
- **Testing**: Every module MUST have inline `#[cfg(test)] mod tests`.
- **Async**: No blocking in event loop. Use `tokio::spawn` for side effects.

## QUICK START

- **Add a Pane**: Implement `Component` trait in `components/`, add to `FocusTarget`.
- **New Action**: Add variant to `Action` enum, handle in `App::update`.
- **Debug Logs**: Check `logging.rs` for fallback directory logic.
