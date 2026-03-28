# hurl/core

**Business Logic Layer** — domain models, file persistence, interpolation, HTTP execution.

## MODULE MAP

| Module | Responsibility | Key Pattern |
|--------|----------------|-------------|
| `models.rs` | Domain types | `HttpMethod`, `RequestDocument`, `ResponseArtifact`, `CollectionNode` |
| `repository.rs` | File I/O | Discover `.hurl.yml` via `walkdir`, atomic writes via `.tmp` rename |
| `interpolation.rs` | Template engine | `{{var}}` resolution, returns `Vec<String>` of unresolved vars |
| `execution.rs` | HTTP client | `reqwest::Client`, `CancellationToken` support, 10MB body cap |

## DATA FLOW

```
RequestDocument → interpolate_document → execute_request → ResponseArtifact
                  (resolve {{vars}})      (reqwest + cancel)
```

## CONVENTIONS

- **Serde**: `RequestDocument` serializes to YAML, `file_path` is `#[serde(skip)]` runtime-only
- **Errors**: `ExecutionError` enum for network/interpolation/cancel/invalid
- **Async**: All execution functions are `async`, use `tokio::select!` for cancellation
- **Body limit**: Hard cap at 10MB (`MAX_BODY_SIZE`), truncates with marker

## FILE DISCOVERY

- Pattern: `*.hurl.yml` or `*.hurl.yaml`
- Ignores: `.git`, `target`, `node_modules`, `.hurl`, `dist`, `build`, `.next`, `__pycache__`, `vendor`
- Tree structure: `CollectionNode` with `Directory` or `RequestFile` kind

## QUICK START

- **Add a model**: Derive `Debug, Clone, PartialEq, Eq, Serialize, Deserialize` on struct
- **Add interpolation field**: Handled automatically by `interpolate_document`
- **Test with cancellation**: Use `CancellationToken::new()` + `token.cancel()` before call
