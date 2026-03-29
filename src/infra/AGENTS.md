# hurl/infra

**Infrastructure Layer** — HTTP client, environment loading, external integrations.

## MODULE MAP

| Module | Responsibility | Key Pattern |
|--------|----------------|-------------|
| `env_loader.rs` | `.env` parsing | `dotenvy` with error tolerance (warns, doesn't fail) |
| `http_client.rs` | HTTP client builder | `reqwest::Client` with 30s timeout, 10 redirects |

## ENVIRONMENT LOADING

- `load_env(cwd)` → loads `.env` from directory
- `load_named_env(cwd, name)` → loads `.env.{name}` (e.g., `.env.staging`)
- Errors logged via `tracing::warn`, never panic
- Quoted values handled by `dotenvy`

## HTTP CLIENT DEFAULTS

- Timeout: 30 seconds
- Redirects: limited to 10
- User-Agent: `hurl/{version}`
- TLS: `rustls` (no OpenSSL dependency)

## CONVENTIONS

- **Tolerance**: Env loader never fails — returns empty map on error
- **Version**: Uses `env!("CARGO_PKG_VERSION")` for User-Agent
- **Reusability**: Build client once, clone across tasks
