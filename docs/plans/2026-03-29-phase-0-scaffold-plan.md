# Phase 0: Scaffold and Application Shell — Implementation Plan

Version: 1.0
Date: March 29, 2026
Status: Draft
Related documents: [hurl PRD](2026-03-29-hurl-prd.md), [Technical Spec](../specs/2026-03-29-hurl-phased-technical-spec.md)

## Context

hurl is a terminal-native API client built in Rust with Ratatui. The project is green-field — only `LICENSE` and `docs/` exist. Phase 0 establishes the application shell, terminal lifecycle, and architecture skeleton so later phases add behavior rather than restructure the app. This plan also adds a justfile for development ergonomics.

## Goal

Optimize for architecture shape, not features. Deliver a multi-pane TUI shell with focus management, panic-safe terminal restoration, structured logging, CLI parsing, and a clean internal module structure that later phases build on.

## Deliverables

At the end of Phase 0:
- App opens instantly into a stable three-pane shell (collections | request editor | response viewer)
- User can cycle focus among panes with Tab/Shift+Tab
- Resize events reflow layout correctly
- Quit path restores terminal reliably (normal exit and panic)
- Bottom help bar communicates essential controls
- `just ci` passes (format check + lint + test)
- Structured logging writes to file (not terminal)

## Out of Scope

Real file discovery, HTTP execution, request editing persistence, response rendering, environment support, assertions.

## Technology Stack

| Concern | Crate | Version |
|---------|-------|---------|
| TUI framework | `ratatui` | 0.30 |
| Terminal backend | `crossterm` | 0.29 (features: event-stream) |
| Error reports | `color-eyre` | 0.6 |
| Domain errors | `thiserror` | 2 |
| Async runtime | `tokio` | 1 (features: full) |
| Stream utilities | `futures` | 0.3 |
| CLI parsing | `clap` | 4 (features: derive) |
| Logging facade | `tracing` | 0.1 |
| Log output | `tracing-subscriber` | 0.3 (features: env-filter) |
| Serialization | `serde` | 1 (features: derive) |
| YAML | `serde_yaml` | 0.9 |
| Platform dirs | `dirs` | 6 |

## Project Structure

```
hurl/
├── Cargo.toml
├── .gitignore
├── rustfmt.toml
├── clippy.toml
├── justfile
├── LICENSE
├── docs/
│   ├── plans/
│   └── specs/
└── src/
    ├── main.rs                  # Entry point, module declarations
    ├── action.rs                # Action enum, FocusTarget enum
    ├── app.rs                   # App struct, event loop, layout, focus
    ├── cli.rs                   # Clap CLI parser
    ├── config.rs                # Config struct (tick/frame rate)
    ├── errors.rs                # HurlError via thiserror
    ├── logging.rs               # Tracing init (file output)
    ├── tui.rs                   # Terminal lifecycle, panic hook
    └── components/
        ├── mod.rs               # Component trait, EventResult enum
        ├── collections.rs       # Collection explorer (placeholder)
        ├── request_editor.rs    # Request editor (placeholder)
        ├── response_viewer.rs   # Response viewer (placeholder)
        └── status_bar.rs        # Help/status bar (functional)
```

## Implementation Steps

### Step 1: Project Root Files

**Files:** `Cargo.toml`, `.gitignore`, `rustfmt.toml`, `clippy.toml`, `justfile`

**Justfile recipes:**

| Recipe | Purpose |
|--------|---------|
| `default` | Lists all recipes |
| `build` | Debug build |
| `release` | Release build with LTO |
| `run *ARGS` | Run with args passthrough |
| `test` | Run all tests |
| `fmt` | Format code |
| `fmt-check` | CI format check |
| `lint` | Clippy with `-D warnings` |
| `fix` | Auto-fix lint + format |
| `ci` | fmt-check → lint → test |
| `clean` | Remove build artifacts |
| `doc` | Generate and open docs |
| `watch` | Watch mode (requires cargo-watch) |

**Gate:** `cargo check` succeeds.

### Step 2: Leaf Modules

**`src/action.rs`:**
- `enum Action` — `Tick`, `Render`, `Quit`, `FocusNext`, `FocusPrev`, `Resize(u16, u16)`, `Error(String)`
- `enum FocusTarget` — `Collections`, `RequestEditor`, `ResponseViewer`
- `FocusTarget::next()` / `prev()` cycling methods

**`src/errors.rs`:**
- `enum HurlError` — `Terminal(String)`, `Io(#[from] io::Error)`, `Config(String)`

**Gate:** Unit tests pass for focus cycling.

### Step 3: Infrastructure Modules

**`src/config.rs`:**
- `struct Config` with `tick_rate: Duration` (250ms), `frame_rate: Duration` (~16ms)
- `Config::from_cli()` applies CLI overrides
- `Default` impl with hardcoded values

**`src/logging.rs`:**
- Writes to `{data_local_dir}/hurl/logs/hurl.log`
- Falls back to temp/app-local log directories when the primary path is unavailable so logging does not block TUI startup
- `tracing_subscriber` with `EnvFilter`, `with_ansi(false)`, file appender
- Logs to file (not stderr) — TUI owns the terminal

**`src/cli.rs`:**
- `struct Cli` — `debug: bool`, `tick_rate: Option<u64>`, `frame_rate: Option<u64>`

**`src/tui.rs`:**
- `struct Tui` wrapping `Terminal<CrosstermBackend<Stdout>>`
- `enter()` — raw mode, alternate screen, panic hook, hide cursor
- `enter()` must rollback terminal state if setup fails part-way through
- `exit()` — reset terminal, show cursor
- Panic hook captures original hook, calls `reset()` before forwarding

**Gate:** `cargo build` succeeds.

### Step 4: Components

**`src/components/mod.rs`:**
- `enum EventResult` — `Consumed`, `Ignored`, `Action(Action)`
- `trait Component` — `handle_key()`, `render(frame, area, focused)`, `focus()`, `blur()`

**`src/components/collections.rs`:**
- Placeholder items: `["GET /users", "POST /users", "GET /users/:id"]`
- `j`/`k` navigation → `Consumed`; else → `Ignored`
- `List` widget, border cyan when focused

**`src/components/request_editor.rs`:**
- Placeholder: "Select a request to edit"
- All keys → `Ignored`

**`src/components/response_viewer.rs`:**
- Placeholder: "Send a request to see the response"
- All keys → `Ignored`

**`src/components/status_bar.rs`:**
- Render function (not a Component): mode + focus name + key hints

**Gate:** `cargo build` succeeds.

### Step 5: Application Core

**`src/app.rs`:**
- `struct App` — owns Config, FocusTarget, should_quit, all three panes
- Event loop uses separate render and tick intervals so `frame_rate` and `tick_rate` are both live configuration
- Configure Tokio intervals with `MissedTickBehavior::Skip` to avoid catch-up bursts after stalls
- Key routing: global keys first (`q`/`Ctrl+C`→Quit, `Tab`→FocusNext, `BackTab`→FocusPrev), then delegate to focused component
- Filter release key events before routing so Windows does not double-handle input
- Layout: vertical split (main + status bar 2 lines), main horizontal (30% collections + 70% right), right vertical (50/50 request + response)

```
+------------------------------------------+
| Collections  |  Request Editor           |
| (30%)        |  (70%, top 50%)           |
|              |---------------------------|
|              |  Response Viewer           |
|              |  (70%, bottom 50%)         |
+------------------------------------------+
| NORMAL | Collections   Tab q Ctrl+C      |
+------------------------------------------+
```

**Gate:** Unit tests pass for key handling + state transitions.

### Step 6: Entry Point

**`src/main.rs`:**
- Flow: `color_eyre::install()` → parse CLI → build config → init logging → create Tui → create App → `tui.enter()` → `app.run()` → `tui.exit()`

**Gate:** `cargo run` launches TUI, `q` exits cleanly.

### Step 7: Verification

- `just ci` passes
- Manual verification of all acceptance criteria (see below)

## Key Technical Notes

1. **Crossterm Shift+Tab**: Reports as `KeyCode::BackTab`, not Tab with shift modifier
2. **Cross-platform key handling**: Ignore `KeyEventKind::Release` events so Windows does not emit duplicate actions for a single keystroke
3. **Panic hook ordering**: `color_eyre::install()` BEFORE `tui.enter()` — terminal hook wraps color-eyre's hook
4. **Logging**: Must write to file, not stderr — TUI owns the terminal, but file logging should degrade gracefully if the preferred directory is unavailable
5. **Event loop borrows**: Map events to `Option<Action>` outside `select!` to avoid borrow-checker issues
6. **Render cadence**: Tokio `interval()` ticks immediately on first poll; use that behavior intentionally for first paint, then keep render and tick timers independent
7. **Edition 2024**: Requires Rust 1.85+; set `rust-version` in Cargo.toml

## Testing Strategy

### Unit Tests (in-module `#[cfg(test)]`)

| Module | Tests |
|--------|-------|
| `action.rs` | Focus cycling (next, prev, round-trip) |
| `config.rs` | Default values, CLI overrides |
| `app.rs` | Key→Action mapping (q→Quit, Tab→FocusNext, BackTab→FocusPrev, Ctrl+C→Quit), state transitions (Quit sets flag, FocusNext/Prev cycles focus) |
| `components/collections.rs` | j/k→Consumed, unknown→Ignored |

### Smoke Tests (`tests/smoke.rs`)

- `App::new()` doesn't panic
- `Config::default()` produces valid values

### Manual Verification Checklist

- [ ] `cargo run` starts app with three panes and status bar
- [ ] Tab cycles focus (border color changes)
- [ ] Shift+Tab cycles in reverse
- [ ] j/k navigates in collections pane
- [ ] q exits cleanly, terminal restored
- [ ] Ctrl+C exits cleanly, terminal restored
- [ ] Terminal resize reflows layout
- [ ] Forced panic restores terminal
- [ ] `just ci` passes (fmt-check + lint + test)

## Acceptance Criteria (from Technical Spec)

- App starts and exits cleanly from supported terminals
- Terminal state is restored on normal exit and panic
- No pane input causes focus corruption or crash
- The shell is structured so later feature work does not require relocating core modules

## Phase 0 Exit Gate

- Shell architecture is stable enough that feature work can land without structural churn
- Terminal lifecycle is reliable
- Focus and layout model are proven
