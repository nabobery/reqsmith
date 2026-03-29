# hurl/core

**Business Logic Layer** — domain models, assertions, diffing, storage, validation, HTTP execution.

## MODULE MAP

| Module | Responsibility | Key Pattern |
|--------|----------------|-------------|
| `models.rs` | Domain types | `HttpMethod`, `RequestDocument`, `ResponseArtifact`, `StoredRun`, `Assertion*` |
| `repository.rs` | File I/O | Discover `.hurl.yml` via `walkdir`, atomic writes via `.tmp` rename |
| `interpolation.rs` | Template engine | `{{var}}` resolution, returns `Vec<String>` of unresolved vars |
| `execution.rs` | HTTP client | `reqwest::Client`, `CancellationToken` support, 10MB body cap |
| `assertions.rs` | Response assertions | `evaluate_assertions()` with JSONPath, status, timing checks |
| `diffing.rs` | Response diffing | `diff_responses()` using `similar::TextDiff`, JSON normalization |
| `storage.rs` | Run persistence | Save/load `StoredRun` to `.hurl/runs/`, body downloads to `.hurl/downloads/` |
| `validation.rs` | Document validation | Schema + interpolation completeness + assertion syntax checks |
| `formatter.rs` | Request formatting | YAML formatting for editor display |
| `environment.rs` | Variable resolution | OS env → .env → .env.{name} → hurl_envs.yml → --var |

## DATA FLOW

```
RequestDocument → interpolate_document → execute_request → ResponseArtifact
                  (resolve {{vars}})      (reqwest + cancel)
                         ↓
              evaluate_assertions(response) → AssertionReport
                         ↓
              store: save_run(StoredRun) → .hurl/runs/{name}_{ts}.json
```

## ASSERTION TYPES

| Assertion | Wire Format | Evaluation |
|-----------|-------------|------------|
| `ExpectStatus(code)` | `expect_status: 200` | `response.status_code == code` |
| `ExpectTimeUnder(ms)` | `expect_time_under: 500ms` | `response.duration_ms <= ms` |
| `ExpectBodyPath { path, operator, expected }` | `expect_body_path:` | JSONPath query + operator comparison |

## DIFFING

- Compares `ResponseArtifact` pairs (status, headers, body)
- JSON bodies normalized: keys sorted, pretty-printed before diff
- Uses `similar::TextDiff` for line-level body diffs
- Output: `DiffArtifact` with `status_diff`, `header_diffs`, `body_diff`

## STORAGE LAYOUT

```
.hurl/
├── runs/           # JSON snapshots: {sanitized_name}_{timestamp}.json
└── downloads/      # Binary bodies: response_{timestamp}.bin
```

## CONVENTIONS

- **Serde**: `RequestDocument` serializes to YAML, `file_path` is `#[serde(skip)]` runtime-only
- **Errors**: `ExecutionError` enum for network/interpolation/cancel/invalid
- **Async**: All execution functions are `async`, use `tokio::select!` for cancellation
- **Body limit**: Hard cap at 10MB (`MAX_BODY_SIZE`), truncates with marker
- **Dead code**: `#[allow(dead_code)]` on modules reserved for future CLI steps

## FILE DISCOVERY

- Pattern: `*.hurl.yml` or `*.hurl.yaml`
- Ignores: `.git`, `target`, `node_modules`, `.hurl`, `dist`, `build`, `.next`, `__pycache__`, `vendor`
- Tree structure: `CollectionNode` with `Directory` or `RequestFile` kind

## QUICK START

- **Add an assertion type**: Add variant to `Assertion` enum, update `AssertionWire`, handle in `evaluate_one()`
- **Add a diff field**: Add to `DiffArtifact`, update `diff_*` function, output in `print_diff_*`
- **Add storage**: Use `next_available_path()` for collision-safe filenames
- **Test validation**: Use `validate_document(doc, env)` with `ValidationReport`
