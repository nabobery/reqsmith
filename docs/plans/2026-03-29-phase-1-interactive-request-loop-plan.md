# Phase 1: Interactive Request Loop — Implementation Plan

Version: 1.0
Date: March 29, 2026
Status: Draft
Related: [hurl PRD](2026-03-29-hurl-prd.md), [Technical Spec](../specs/2026-03-29-hurl-phased-technical-spec.md), [Phase 0 Plan](2026-03-29-phase-0-scaffold-plan.md)

## Context

Phase 0 delivered a stable three-pane TUI shell with focus management, panic-safe terminal restoration, structured logging, and a clean module structure. Phase 1 turns this shell into a usable local API client by implementing the first end-to-end value loop: discover requests from disk, edit them, execute them asynchronously, and inspect responses — all within the TUI.

## Goal

At the end of Phase 1, hurl is genuinely usable for interactive local API testing. A user can open hurl in a project directory, see discovered `.hurl.yml` files, select one, edit its fields, send it, and inspect the response. Everything persists to readable YAML.

## Out of Scope

Headless `hurl run`, rich assertions engine, response diffing, scripting/hooks, plugins, persistent run history, `hurl_envs.yml`, `.env.<name>` multi-environment support (Phase 2).

---

## New Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `reqwest` | 0.12 | HTTP client (`json`, `rustls-tls` features) |
| `walkdir` | 2 | Recursive `.hurl.yml` file discovery |
| `tui-textarea` | 0.7 | Multi-line body editor + inline text inputs |
| `dotenvy` | 0.15 | `.env` file loading |
| `serde_json` | 1 | JSON pretty-printing for response bodies |
| `tokio-util` | 0.7 | `CancellationToken` for request cancellation |

## New Module Structure

```
src/
  core/
    mod.rs               # pub mod declarations
    models.rs            # RequestDocument, HttpMethod, KeyValueField, ResponseArtifact, CollectionNode
    repository.rs        # discover_requests(), load_request(), save_request()
    execution.rs         # execute_request() with async + cancellation
    interpolation.rs     # {{variable}} substitution
  infra/
    mod.rs               # pub mod declarations
    http_client.rs       # Reqwest Client builder
    env_loader.rs        # .env file loading via dotenvy
```

## New Action Variants

```rust
// In src/action.rs — additions to Action enum:
CollectionsDiscovered(Vec<CollectionNode>),
SelectRequest(PathBuf),
RequestLoaded(RequestDocument),
SaveRequest,
RequestSaved(PathBuf),
SendRequest,
CancelRequest,
RequestCompleted(Box<ResponseArtifact>),
RequestFailed(String),
RequestCancelled,
StatusMessage(String),
```

## Async Communication

Add `tokio::sync::mpsc::UnboundedSender<Action>` / `UnboundedReceiver<Action>` to App. Background tasks (HTTP execution, file discovery, file save) send results through this channel. The event loop adds a fourth branch to `tokio::select!`:

```rust
Some(action) = action_rx.recv() => { Some(action) }
```

---

## Implementation Steps

### Step 1: Core Data Models
**New:** `src/core/mod.rs`, `src/core/models.rs`
**Modify:** `src/main.rs` (add `mod core;`)

Define foundational types:
- `HttpMethod` — GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS with `#[serde(rename_all = "UPPERCASE")]`
- `KeyValueField` — `{ key, value, enabled }` with `#[serde(default)]` on enabled
- `RequestDocument` — `{ name, method, url, headers, params, body, #[serde(skip)] file_path }`
- `ResponseArtifact` — `{ status_code, http_version, headers, content_type, content_length, duration_ms, body_text }`
- `CollectionNode` — `{ name, path, kind (Directory|RequestFile), children, depth }`

Use `skip_serializing_if = "Vec::is_empty"` and `skip_serializing_if = "Option::is_none"` for clean YAML output.

**Tests:** Serde round-trip, default values, skip_serializing_if behavior.
**Gate:** `just ci` passes.

### Step 2: Repository — Discovery and Persistence
**New:** `src/core/repository.rs`
**Depends on:** Step 1

Three functions:
- `discover_requests(cwd: &Path) -> Result<Vec<CollectionNode>>` — walkdir with `.filter_entry()` skipping `.git`, `target`, `node_modules`. Find `*.hurl.yml`. Build tree grouped by directory.
- `load_request(path: &Path) -> Result<RequestDocument>` — read file, `serde_yaml::from_str`, set `file_path`.
- `save_request(doc: &RequestDocument, path: &Path) -> Result<()>` — serialize to YAML, atomic write (temp file + rename).

Add error variants to `src/errors.rs`: `Repository(String)`, `YamlParse(String)`.

**Tests:** Temp dir with test files, discovery finds correct files, round-trip save/load, invalid YAML error.
**Gate:** `just ci` passes.

### Step 3: Environment Loading and Interpolation
**New:** `src/infra/mod.rs`, `src/infra/env_loader.rs`, `src/core/interpolation.rs`
**Depends on:** Step 1

**Env loader:** `dotenvy::from_path_iter()` on `CWD/.env`. Return `HashMap<String, String>`. Gracefully handle missing `.env` (empty map).

**Interpolation:** `interpolate(template: &str, vars: &HashMap<String, String>) -> Result<String, Vec<String>>` — scan for `{{name}}`, replace from map, return unresolved variable names as error. Also `interpolate_document()` that applies to URL, header values, param values, body.

**Tests:** Simple substitution, multiple vars, missing vars error, empty template passthrough.
**Gate:** `just ci` passes.

### Step 4: HTTP Client and Execution
**New:** `src/infra/http_client.rs`, `src/core/execution.rs`
**Depends on:** Steps 1, 3

**HTTP client:** `build_client() -> reqwest::Client` — 30s timeout, redirect policy (10 max), user-agent `hurl/0.1`, explicit Rustls-backed transport.

**Execution:**
```rust
pub async fn execute_request(
    client: &reqwest::Client,
    doc: &RequestDocument,
    vars: &HashMap<String, String>,
    cancel: CancellationToken,
) -> Result<ResponseArtifact, ExecutionError>
```
1. Interpolate document
2. Build reqwest::Request (method, URL + query params, headers, body)
3. `tokio::select!` between execute and `cancel.cancelled()`
4. Read response body incrementally in chunks, continuing to honor cancellation while the body is streaming
5. Stop reading once the preview cap is reached (10MB) instead of waiting for socket close
6. Extract response: status, headers, version, body text preview
7. JSON pretty-print if content-type contains `application/json` and the preview is not truncated

**Error types:** `ExecutionError { Cancelled, Network(String), Interpolation(Vec<String>), InvalidRequest(String) }`

**Tests:** Request building correctness, cancellation before and during body streaming, JSON pretty-print, body truncation without waiting for connection close.
**Gate:** `just ci` passes.

### Step 5: Action Enum + Async Channel in App
**Modify:** `src/action.rs`, `src/app.rs`, `src/components/mod.rs`
**Depends on:** Steps 1, 4

- Add all new Action variants to `src/action.rs`
- Add `is_editing(&self) -> bool` to Component trait (default `false`)
- Add to App struct: `action_tx`, `action_rx`, `http_client`, `cancel_token: Option<CancellationToken>`, `env_vars`, `request_in_flight`
- Add `action_rx.recv()` as fourth branch in `tokio::select!`
- Spawn initial collection discovery in `App::run()` start
- Implement `update()` arms for all new actions (delegating to components)
- Revise `handle_key()`: check `is_editing()` — in insert mode, only Ctrl-shortcuts and Esc are global; `q` does NOT quit
- Before `SaveRequest` / `SendRequest`, snapshot any in-progress editor cell so the saved/sent request matches the visible text

**Tests:** Action flow tests, `q` suppressed in edit mode, `SendRequest` sets in-flight flag.
**Gate:** `just ci` passes, app still launches.

### Step 6: Collections Pane — Real Discovery
**Modify:** `src/components/collections.rs`
**Depends on:** Steps 2, 5

Replace hardcoded items with a visible flat tree derived from a preserved source tree:
```rust
struct FlatNode { display_name, path, kind, depth, is_expanded }
```

Key bindings: `j`/`k` navigate, `Enter` on file emits `Action::SelectRequest(path)`, `Enter` on dir toggles expand, `h`/`l` collapse/expand, `r` refresh.

Render: indented tree with depth-based padding, directory arrows (`>` / `v`), file names without `.hurl.yml` suffix.
Implementation note: collapse/expand must be reversible without requiring a full re-discovery from disk; preserve the canonical tree in memory and rebuild the visible list from that state.

**Tests:** Tree flattening, reversible expand/collapse, Enter emits correct action.
**Gate:** `just ci` passes.

### Step 7: Request Editor Pane
**Modify:** `src/components/request_editor.rs`
**Depends on:** Steps 1, 5

Most complex component. Internal architecture:

```rust
pub struct RequestEditorPane {
    document: Option<RequestDocument>,
    original_document: Option<RequestDocument>,  // for dirty tracking
    active_tab: EditorTab,  // Url, Headers, Params, Body
    mode: EditorMode,       // Normal, Insert
    dirty: bool,

    method_index: usize,
    url_input: TextArea<'static>,
    headers: Vec<KeyValueRow>,
    header_cursor: (usize, usize),  // row, col
    params: Vec<KeyValueRow>,
    param_cursor: (usize, usize),
    body_editor: TextArea<'static>,
}
```

**Navigation (Normal mode):**
- `1`/`2`/`3`/`4` or `h`/`l`: switch tabs (URL, Headers, Params, Body)
- `j`/`k`: navigate rows in grids
- `i`/`Enter`: enter Insert mode on focused field
- `a`: add row (headers/params), `d`: delete row, `Space`: toggle enabled

**Insert mode:** All keystrokes go to the active TextArea. `Esc` returns to Normal.
**`is_editing()`:** Returns `true` when `mode == Insert`.
**Save/send behavior:** if the user triggers save or send while editing a header/param cell, the current buffer contents must be included immediately rather than waiting for `Esc`.

**Method selector:** Cycle through methods with `<`/`>` keys when URL tab is active.

**Rendering:**
- Top: `[METHOD] [url input field___________]`
- Tab bar: `URL | Headers(n) | Params(n) | Body` with active highlight
- Content: active tab content
- Title: `" Request [modified] "` when dirty

**Loading:** Populate all fields from `RequestDocument`, clone as `original_document`, reset dirty.
**Export:** `to_document() -> Option<RequestDocument>` reads all fields back.

**Tests:** Tab switching, to_document round-trip, dirty tracking, mode transitions, is_editing().
**Gate:** `just ci` passes.

### Step 8: Response Viewer Pane
**Modify:** `src/components/response_viewer.rs`
**Depends on:** Steps 1, 5

```rust
pub struct ResponseViewerPane {
    response: Option<ResponseArtifact>,
    active_tab: ResponseTab,  // Body, Headers, Summary
    scroll_offset: u16,
    loading: bool,
    error_message: Option<String>,
}
```

**States:** Empty, Loading, Success, Error.

**Tabs:**
- *Body:* Pretty-printed JSON or raw text, scrollable with `j`/`k`, `g`/`G` for top/bottom
- *Headers:* Two-column table, scrollable
- *Summary:* Status badge (colored by 2xx/3xx/4xx/5xx), duration, content-type, size

**Status code colors:** 2xx green, 3xx yellow, 4xx red, 5xx magenta.

**Tests:** State rendering, tab switching, scroll bounds.
**Gate:** `just ci` passes.

### Step 9: Status Bar Updates
**Modify:** `src/components/status_bar.rs`
**Depends on:** Steps 5-8

Update `render()` to accept mode, dirty state, loading state, status message.

- Mode badge: `NORMAL` (cyan) or `INSERT` (yellow)
- `[modified]` indicator when dirty
- `[sending...]` when request in flight
- Context-sensitive key hints based on focus + mode
- Transient status messages (e.g., "Saved to path/file.hurl.yml")

**Tests:** Hint text varies by focus, mode indicator changes.
**Gate:** `just ci` passes.

### Step 10: Integration Wiring and End-to-End Polish
**Modify:** `src/app.rs`, `src/main.rs`
**Depends on:** All previous steps

Wire everything together:
1. `App::run()` init: create mpsc channel, build reqwest client, load `.env`, spawn collection discovery
2. `SendRequest` handler: extract doc from editor, interpolate, spawn execution task, set loading state
3. `SaveRequest` handler: extract doc, spawn_blocking save, send result action
4. `SelectRequest` handler: spawn_blocking load, send `RequestLoaded`
5. Key routing: `Ctrl+R` sends, `Ctrl+S` saves, `Esc` cancels in-flight request

**Edge cases:**
- No `.hurl.yml` files: show empty state in collections
- Unresolved `{{variables}}`: show error in response pane, don't send
- Network errors: display in response pane
- Large response body: truncate preview at 10MB with indicator and stop reading further chunks once the cap is reached
- Cancellation remains active while the response body is still streaming
- New (unsaved) request: Phase 1 only saves to existing paths

**Gate:** All acceptance criteria met.

---

## Dependency Graph

```
Step 1 (models) ──┬──> Step 2 (repository) ──> Step 6 (collections)
                   ├──> Step 3 (env + interpolation)
                   │         │
                   │         └──> Step 4 (execution)
                   │                   │
                   │                   └──> Step 5 (actions + async channel)
                   │                              │
                   │                              ├──> Step 7 (request editor)
                   │                              ├──> Step 8 (response viewer)
                   │                              └──> Step 9 (status bar)
                   │                                         │
                   └─────────────────────────────────────────> Step 10 (integration)
```

Steps 2, 3 can run in parallel after Step 1.
Steps 6, 7, 8, 9 can be partially parallelized after Step 5.
Step 10 is final integration.

## Files Summary

### New files (8):
- `src/core/mod.rs`, `src/core/models.rs`, `src/core/repository.rs`, `src/core/execution.rs`, `src/core/interpolation.rs`
- `src/infra/mod.rs`, `src/infra/http_client.rs`, `src/infra/env_loader.rs`

### Modified files (10):
- `Cargo.toml` — new deps
- `src/main.rs` — `mod core; mod infra;`
- `src/action.rs` — new Action variants
- `src/app.rs` — async channel, HTTP client, execution wiring, key routing
- `src/errors.rs` — new error variants
- `src/components/mod.rs` — `is_editing()` on Component trait
- `src/components/collections.rs` — real tree from discovery
- `src/components/request_editor.rs` — full editor implementation
- `src/components/response_viewer.rs` — full response display
- `src/components/status_bar.rs` — context-sensitive hints + mode display

## Verification

### Automated
- `just ci` passes at every step gate (fmt-check + clippy + tests)
- Unit tests for each new module
- Integration test for YAML round-trip (save → load → compare)

### Manual Checklist
- [ ] Launch in directory with `.hurl.yml` files — collections shows them
- [ ] Select request — editor populates all fields
- [ ] Edit URL, headers, params, body — dirty indicator appears
- [ ] `Ctrl+S` saves — dirty indicator clears, file updated on disk
- [ ] `Ctrl+R` sends request — response appears in viewer
- [ ] Cancel in-flight request with `Esc` — returns to idle
- [ ] `Tab` cycles focus between panes
- [ ] `q` quits in normal mode but NOT in insert mode
- [ ] JSON responses are pretty-printed
- [ ] Network errors display in response pane
- [ ] `{{variable}}` from `.env` are substituted
- [ ] Missing variables show clear error before send
- [ ] Terminal resize reflows correctly
- [ ] Panic restores terminal

## Acceptance Criteria (from Tech Spec)

- User can open, edit, save, and execute a request from the TUI
- UI remains responsive during request execution
- Canceling an in-flight request returns app to idle state without corruption
- JSON responses are readable (pretty-printed)
- Error states are visible and recoverable
- Saved requests round-trip cleanly through YAML
