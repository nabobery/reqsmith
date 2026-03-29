# Phase 3: Power User Features — Implementation Plan

Version: 1.0
Date: March 29, 2026
Status: Draft
Related: [hurl PRD](2026-03-29-hurl-prd.md), [Technical Spec](../specs/2026-03-29-hurl-phased-technical-spec.md), [Phase 2 Plan](2026-03-29-phase-2-git-workflow-headless-runner-plan.md)

## Context

Phases 0-2 are complete. The codebase has ~6,570 lines of Rust across 30 source files with 100+ tests. hurl is a working interactive TUI + headless CLI API client with collection discovery, request editing, async HTTP execution with cancellation, response viewing, layered environment resolution, validation, formatting, and YAML persistence. Phase 3 adds assertions, response diffing, and richer JSON exploration — the features that turn hurl from a manual testing tool into a structured regression-checking tool.

## Goal

At the end of Phase 3, hurl supports response assertions (status, latency, body-path), response diffing between runs, a collapsible JSON tree viewer, and improved binary/large-body handling — all surfaced in both TUI and CLI modes.

## Out of Scope

Full scripting engine, auth plugin catalog, remote team sync, WASM plugin host, pre-request/post-response hooks, GraphQL/gRPC support.

---

## New Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde_json_path` | 0.7 | RFC 9535 JSONPath for body-path assertions |
| `similar` | 2 | Text diff engine (line-level, patience algorithm) |

No `tui-tree-widget` — build a custom JSON tree using the existing `FlatNode` pattern from `CollectionsPane` (proven in codebase, consistent with project conventions).

---

## Implementation Steps

### Step 1: Assertion & Diff Domain Types

**Modify:** `src/core/models.rs`

Add `assertions` field to `RequestDocument`:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub assertions: Vec<Assertion>,
```

Update `RequestDocument::new_empty()` to include `assertions: Vec::new()`.

New assertion types:

```rust
pub enum Assertion {
    ExpectStatus(u16),
    ExpectTimeUnder(u64),  // milliseconds
    ExpectBodyPath {
        path: String,       // JSONPath expression
        operator: AssertionOperator,
        expected: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionOperator { Eq, Ne, Gt, Lt, Gte, Lte, Contains, Exists }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertionResult {
    pub assertion: Assertion,
    pub passed: bool,
    pub message: String,
    pub actual_value: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AssertionReport {
    pub results: Vec<AssertionResult>,
}
```

YAML request format uses mapping-style assertions so request files stay readable and match the PRD examples:

```yaml
assertions:
  - expect_status: 201
  - expect_time_under: 500ms
  - expect_body_path:
      path: "$.user.id"
      operator: eq
      expected: 42
```

`expect_time_under` accepts either an integer millisecond value or a string of the form `<n>ms`; formatter output should normalize to the `500ms` style.

Add `AssertionReport` helpers: `all_passed()`, `failures()`, `is_empty()`.

Extend `RunResult` with `pub assertions: AssertionReport`. Update all 6 construction sites in `runner.rs`.

Add `StoredRun` type for diff persistence:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRun {
    pub request_name: String,
    pub request_file: PathBuf,
    pub timestamp: String,
    pub status_code: u16,
    pub duration_ms: u128,
    pub headers: Vec<(String, String)>,
    pub content_type: Option<String>,
    pub body_text: Option<String>,
    pub assertions: Vec<AssertionResult>,
}
```

Add `DiffArtifact`, `HeaderDiff`, `DiffLine` types for response comparison.

**Tests:** Serde round-trip for Assertion variants, RequestDocument with assertions, AssertionReport helpers, StoredRun JSON round-trip.

**Gate:** `just ci` passes.

### Step 2: Assertions Engine

**New file:** `src/core/assertions.rs`
**Modify:** `src/core/mod.rs` (add `pub mod assertions;`)

Core function:

```rust
pub fn evaluate_assertions(
    assertions: &[Assertion],
    response: &ResponseArtifact,
) -> AssertionReport
```

Individual evaluators:
- `evaluate_status(expected: u16, actual: u16) -> AssertionResult`
- `evaluate_time_under(max_ms: u64, actual_ms: u128) -> AssertionResult`
- `evaluate_body_path(path: &str, op: &AssertionOperator, expected: &Value, body: Option<&str>) -> AssertionResult`

Body-path evaluator: parse body as `serde_json::Value`, compile JSONPath via `serde_json_path::JsonPath::parse()`, query, apply operator. Non-JSON body or invalid JSONPath → failed `AssertionResult` with descriptive message (never panics/errors — total function).

Operator logic:
- `Eq`/`Ne`: `serde_json::Value` equality
- `Gt`/`Lt`/`Gte`/`Lte`: convert to `f64`
- `Contains`: actual as string contains expected as string
- `Exists`: JSONPath returns ≥1 match

**Tests (~20):** status pass/fail, time pass/fail, body-path with each operator, missing path, non-JSON body, invalid JSONPath, empty assertions, mixed pass/fail report.

**Gate:** `just ci` passes.

### Step 3: Integrate Assertions into Runner

**Modify:** `src/core/runner.rs`

In the `Ok(artifact)` arm of `run_request()`, evaluate assertions after execution:

```rust
let assertion_report = if doc.assertions.is_empty() {
    AssertionReport::default()
} else {
    assertions::evaluate_assertions(&doc.assertions, &artifact)
};
let exit_code = if assertion_report.all_passed() {
    ExitCode::Success
} else {
    ExitCode::AssertionFailure
};
```

**Modify:** `src/app.rs` — Update `SendRequest` handler to forward assertions alongside response.

**Tests:** Runner with passing/failing assertions, no-assertions case.

**Gate:** `just ci` passes. TUI still works.

### Step 4: Assertions in CLI Output

**Modify:** `src/output.rs`

Human output (after body):

```
Assertions: 2 passed, 1 failed
  ✓ expect_status: 201
  ✓ expect_time_under: 500ms (actual: 45ms)
  ✗ expect_body_path: $.user.id == 42 (actual: 99)
```

JSON output: add `"assertions"` array. Only when assertions exist.

**Tests:** Human/JSON output with assertions, empty case.

**Gate:** `just ci` passes.

### Step 5: Assertions in TUI Response Viewer

**Modify:** `src/components/response_viewer.rs`, `src/action.rs`, `src/app.rs`

- Add fourth tab: `Assertions` (key `4`)
- Change `ViewerState::Success` to hold `ResponseArtifact` + `AssertionReport`
- Change `Action::RequestCompleted(Box<ResponseArtifact>)` to `Action::RequestCompleted(Box<ResponseArtifact>, Box<AssertionReport>)`
- Render each result as colored line: green ✓ / red ✗

**Tests:** Assertions tab renders, tab 4 selects it.

**Gate:** `just ci` passes.

### Step 6: Validation Update for Assertions

**Modify:** `src/core/validation.rs`

Add `validate_assertions()` called from `validate_document()`:
- Validate JSONPath syntax at validation time
- Warn if `expect_time_under` < 1ms
- Warn if `expect_status` outside 100-599

**Tests:** Bad JSONPath caught, valid assertions pass.

**Gate:** `just ci` passes.

### Step 7: Formatter Update for Assertions

**Modify:** `src/core/formatter.rs`

Add `assertions` to canonical output after `body`. Each assertion in YAML form. Empty list omitted.

**Tests:** Idempotent format with assertions.

**Gate:** `just ci` passes.

### Step 8: Response Diffing Engine

**New file:** `src/core/diffing.rs`
**Modify:** `src/core/mod.rs`

```rust
pub fn diff_responses(
    baseline: &ResponseArtifact,
    candidate: &ResponseArtifact,
    baseline_label: &str,
    candidate_label: &str,
) -> DiffArtifact
```

- If both bodies parse as JSON: sort keys recursively, pretty-print, diff via `similar::TextDiff::from_lines()`
- Otherwise: plain text diff
- Header diff: find added, removed, changed

**Tests (~12):** Identical → empty diff, different status, header changes, JSON with reordered keys, plain text diff.

**Gate:** `just ci` passes.

### Step 9: StoredRun Persistence

**New file:** `src/core/storage.rs`
**Modify:** `src/core/mod.rs`

```rust
pub fn save_run(run: &StoredRun, dir: &Path) -> Result<PathBuf>
pub fn load_run(path: &Path) -> Result<StoredRun>
pub fn list_runs(dir: &Path, request_name: &str) -> Result<Vec<PathBuf>>
```

Stored in `.hurl/runs/` (JSON). File naming: `{name}_{timestamp}.json`, with millisecond-resolution timestamps plus collision-safe suffixing when needed. Directory auto-created.

Opt-in: CLI `--save` flag, TUI `s` key in response viewer.

**Tests:** Save/load round-trip, list runs, directory creation.

**Gate:** `just ci` passes.

### Step 10: CLI Diff Command

**New file:** `src/commands/diff.rs`
**Modify:** `src/cli.rs`, `src/commands/mod.rs`, `src/main.rs`, `src/output.rs`

```rust
Diff {
    baseline: PathBuf,
    candidate: PathBuf,
    #[arg(short, long, default_value = "human")]
    output: OutputMode,
}
```

Load both as StoredRun files → convert to ResponseArtifact → compute diff → output.

Add `print_diff_result(diff: &DiffArtifact, mode: &OutputMode)` to output.rs.

**Tests:** CLI diff produces correct output, JSON valid.

**Gate:** `just ci` passes.

### Step 11: TUI Diff Tab

**Modify:** `src/components/response_viewer.rs`, `src/app.rs`

Add `Diff` as fifth tab (key `5`, shortcut `d`). Show unified diff between current and previous response. App holds `previous_response: Option<ResponseArtifact>` — updated on each `RequestCompleted`.

Rendering: green insertions, red deletions, dim context.

**Tests:** Diff tab renders, no previous → message.

**Gate:** `just ci` passes.

### Step 12: Collapsible JSON Tree Viewer

**New file:** `src/components/json_tree.rs`
**Modify:** `src/components/mod.rs`, `src/components/response_viewer.rs`

```rust
pub struct JsonTreeNode {
    pub key: Option<String>,
    pub value_preview: String,
    pub depth: usize,
    pub is_expandable: bool,
    pub is_expanded: bool,
    pub node_type: JsonNodeType,
}
```

Build from `serde_json::Value` following `FlatNode` pattern from `CollectionsPane`. Collapse/expand with `h`/`l`. Toggle raw/tree with `t` key in Body tab (only for valid JSON). Rebuild visible nodes by borrowing the stored JSON tree instead of cloning the full payload on every expand/collapse.

**Tests:** Build tree, collapse/expand, nested objects, render.

**Gate:** `just ci` passes.

### Step 13: Binary/Large Response Handling

**Modify:** `src/core/execution.rs`, `src/core/models.rs`, `src/components/response_viewer.rs`, `src/action.rs`, `src/app.rs`

Binary detection: >5% null bytes in first 8KB sample.

Add `is_binary: bool` to `ResponseArtifact`. Binary responses: keep the captured bytes on the response artifact, set `body_text` to a descriptive message, and allow `w` to save the body to `.hurl/downloads/response_<timestamp>.bin`.

Add `Action::SaveResponseBody`. App handler writes via `spawn_blocking` and reports the saved path in the status bar.

**Tests:** Binary detection, save action.

**Gate:** `just ci` passes.

---

## Dependency Graph

```
Step 1  (models) ──────────────┐
    │                          │
Step 2  (assertions engine)    │
    │                          │
Step 3  (runner integration)   │
    │                          │
Step 4  (CLI assertion output) │
    │                          │
Step 5  (TUI assertion tab)    │
    │                          │
Step 6  (validation update)    │  Steps 8-10 can start after Step 1
    │                          │
Step 7  (formatter update)     ├── Step 8  (diffing engine)
                               │       │
                               │   Step 9  (storage)
                               │       │
                               │   Step 10 (CLI diff cmd)
                               │       │
                               │   Step 11 (TUI diff tab)
                               │
                               ├── Step 12 (JSON tree) — after Step 5
                               │
                               └── Step 13 (binary) — independent
```

**Parallel tracks:**
- **Track A** (assertions): Steps 1 → 2 → 3 → 4 → 5 → 6 → 7
- **Track B** (diffing): Steps 1 → 8 → 9 → 10 → 11
- **Track C** (JSON tree): Step 12
- **Track D** (binary): Step 13

---

## New Files (5)

| File | Purpose | ~Lines |
|------|---------|--------|
| `src/core/assertions.rs` | Assertion evaluation engine | 250 |
| `src/core/diffing.rs` | Response diff computation | 200 |
| `src/core/storage.rs` | StoredRun persistence | 150 |
| `src/commands/diff.rs` | CLI diff subcommand | 60 |
| `src/components/json_tree.rs` | Collapsible JSON tree widget | 300 |

## Modified Files (14)

| File | Changes |
|------|---------|
| `Cargo.toml` | Add `serde_json_path`, `similar` |
| `src/core/models.rs` | Add Assertion types, extend RequestDocument/RunResult/ResponseArtifact |
| `src/core/mod.rs` | Register new modules |
| `src/core/runner.rs` | Insert assertion evaluation after execution |
| `src/core/validation.rs` | Add `validate_assertions()` |
| `src/core/formatter.rs` | Add assertions to canonical format |
| `src/core/execution.rs` | Add binary detection, `is_binary` field |
| `src/output.rs` | Add assertion + diff output |
| `src/cli.rs` | Add `Diff` subcommand, `--save` flag on `Run` |
| `src/commands/mod.rs` | Add `pub mod diff;` |
| `src/commands/run.rs` | Handle `--save` flag |
| `src/action.rs` | Update `RequestCompleted`, add `SaveResponseBody` |
| `src/app.rs` | Forward assertions, hold previous_response, new actions |
| `src/components/response_viewer.rs` | Assertions + Diff tabs, tree toggle, binary |
| `src/components/mod.rs` | Add `pub mod json_tree;` |
| `src/main.rs` | Wire `Diff` command |

---

## Key Design Decisions

1. **Assertion as enum, not trait objects** — Closed set of assertion types. Exhaustive matching at compile time. Custom assertions can be added via enum variants later.
2. **AssertionReport on RunResult** — Keeps pipeline simple. Always present but defaults empty, so existing code minimally disrupted.
3. **Custom JSON tree vs. `tui-tree-widget`** — Reuses proven `FlatNode` pattern from collections. No new dependency, consistent conventions.
4. **Diff as tab, not modal** — Simpler than overlay. Consistent with existing tab navigation. Unified diff is sufficient for V1.
5. **Opt-in persistence** — `--save` flag and explicit keybinding prevent unexpected disk usage.
6. **Assertions are synchronous** — Evaluated after async HTTP execution completes on already-fetched data.
7. **Binary detection via null-byte heuristic** — Simple, fast, well-established approach.

---

## Verification

### Automated
- `just ci` passes at every step gate
- ~80 new unit tests across new modules
- Integration tests for runner with assertions
- CLI integration tests for `hurl diff`

### Manual Checklist
- [ ] Create request with assertions, `hurl run` reports pass/fail with exit code 4
- [ ] TUI shows assertions tab with colored indicators
- [ ] `hurl validate` catches invalid JSONPath in assertions
- [ ] `hurl fmt` preserves assertions in canonical output
- [ ] `hurl run --save` persists run to `.hurl/runs/`
- [ ] `hurl diff baseline.json candidate.json` shows unified diff
- [ ] TUI diff tab shows differences between current and previous response
- [ ] JSON tree viewer expands/collapses with h/l keys
- [ ] `t` toggles raw/tree mode in body tab
- [ ] Binary response shows save hint, `w` saves to disk
- [ ] Large JSON navigates without lag in tree mode
- [ ] All Phase 0-2 functionality unaffected

## Acceptance Criteria (from Tech Spec)

- Assertions are visible and reliable in both TUI and CLI
- Diffing is usable for typical JSON API regression workflows
- Large and binary response handling is operationally safe
