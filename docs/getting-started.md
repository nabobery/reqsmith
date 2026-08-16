# Getting started

A copy-pasteable first run of `reqsmith`: install it, write a request, run it,
assert on the response, and diff two runs. Every command below was verified
against the source in this repository.

## 1. Install

```bash
git clone https://github.com/nabobery/reqsmith
cd reqsmith
cargo build --release --locked
cp target/release/reqsmith ~/.local/bin/reqsmith   # or add target/release to PATH
```

Requirements:

- **Rust 1.88 or later** for the default build (the TUI + CLI, no plugins).
- **Rust 1.91 or later** only if you also want the optional `plugins`
  feature (`cargo build --release --locked --features plugins`) — it pulls
  in Extism/Wasmtime, which need a newer toolchain than the crate's own
  MSRV. Skip this if you just want the core client.

Check the install:

```bash
reqsmith --version
reqsmith --help
```

## 2. Write your first request

reqsmith requests are plain YAML files ending in `.req.yml` (or `.req.yaml`).
Create one in an empty project directory:

```bash
mkdir my-api-project && cd my-api-project
cat > get_users.req.yml <<'EOF'
name: "Get Users"
method: GET
url: "{{base_url}}/response.json"
headers:
  - key: Accept
    value: application/json
assertions:
  - expect_status: 200
EOF
```

`{{base_url}}` is a template placeholder — reqsmith resolves it at run time (see
[the local environment setup](#3-start-a-local-fixture-and-configure-the-environment)
below). Supported HTTP
methods are `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`.

Confirm reqsmith can discover and parse it:

```bash
reqsmith list          # shows ./get_users.req.yml
```

`reqsmith fmt` rewrites a request file into reqsmith's canonical field ordering
(useful as a pre-commit / CI check via `--check`):

```bash
reqsmith fmt get_users.req.yml --check   # exits 1 if the file isn't canonical
reqsmith fmt get_users.req.yml           # rewrites it in place
```

## 3. Start a local fixture and configure the environment

Keep the walkthrough deterministic and offline by serving a synthetic JSON
response from Python's standard library:

```bash
mkdir -p fixture
printf '%s\n' '{"message":"hello from reqsmith","version":1}' > fixture/response.json
python3 -m http.server 8765 --bind 127.0.0.1 --directory fixture > /tmp/reqsmith-fixture.log 2>&1 &
FIXTURE_PID=$!
```

Give the `{{base_url}}` placeholder a value with a `.env` file:

```bash
cat > .env <<'EOF'
base_url=http://127.0.0.1:8765
EOF

reqsmith validate .    # checks YAML schema and that referenced variables resolve
```

Run the request headlessly:

```bash
reqsmith run get_users.req.yml
```

You should see something like:

```
200 (312ms) [application/json]
{
  "message": "hello from reqsmith",
  "version": 1
}

Assertions: 1 passed, 0 failed
  PASS  expect_status: 200 (actual: 200)
```

For multiple named environments (dev/staging/prod), add `reqsmith_envs.yml`:

Named environments live under a top-level `environments:` key:

```bash
cat > reqsmith_envs.yml <<'EOF'
environments:
  dev:
    base_url: http://localhost:8080
  staging:
    base_url: http://127.0.0.1:8765
EOF

reqsmith run get_users.req.yml --env staging
```

**Resolution precedence** for a variable used in a request (highest wins):

1. `--var key=value` on the command line
2. The named environment in `reqsmith_envs.yml` selected with `--env <name>`
3. `.env.<name>` (e.g. `.env.staging`), if `--env <name>` was given
4. `.env` in the current directory
5. The OS environment (used as a fallback for any variable still
   unresolved after layers 1–4)

```bash
reqsmith run get_users.req.yml --env staging --var base_url=http://127.0.0.1:8765
```

### Launching the TUI instead

Everything above also works interactively. From the same directory:

```bash
reqsmith
```

The TUI opens a three-pane layout (collections, request editor, response
viewer). `Tab`/`Shift+Tab` cycles focus, `j`/`k` (or the arrow keys)
navigate lists, `Enter` opens the selected request, `Ctrl+R` sends it, and
`Ctrl+S` saves edits. See the full shortcut table in the
[README](../README.md#tui-keyboard-shortcuts).

## 4. Assertions

reqsmith's assertion engine supports status codes, response time budgets, and
JSONPath checks against the response body. Add a body assertion to the
request file:

```yaml
name: "Get Users"
method: GET
url: "{{base_url}}/response.json"
headers:
  - key: Accept
    value: application/json
assertions:
  - expect_status: 200
  - expect_body_path:
      path: "$.message"
      operator: eq
      expected: "hello from reqsmith"
```

Available `expect_body_path` operators: `eq`, `ne`, `gt`, `lt`, `gte`,
`lte`, `contains`, `exists`. Run it again and reqsmith reports pass/fail per
assertion, and exits non-zero if any fail (see the exit codes in the
[README](../README.md#exit-codes) — assertion failures exit `4`, which is
what makes `reqsmith run --save` useful in CI).

## 5. Saving and diffing runs

`--save` persists the run result as a **JSON** snapshot under
`.reqsmith/runs/` (this is JSON, not YAML, and sensitive response headers are
redacted before they're written — see [Security](#security-notes) below):

```bash
reqsmith run get_users.req.yml --save
# Run saved to .reqsmith/runs/Get_Users_<timestamp>.json
```

Run it again later (e.g. after a deploy) and compare the two snapshots:

```bash
reqsmith run get_users.req.yml --save
# Run saved to .reqsmith/runs/Get_Users_<timestamp2>.json

reqsmith diff .reqsmith/runs/Get_Users_<timestamp>.json .reqsmith/runs/Get_Users_<timestamp2>.json
```

`reqsmith diff` prints a unified-style diff of status, headers, and body
between the two stored runs — handy for spotting a regression between two
CI runs, or before/after a deploy, without needing both responses live at
the same time.

Stop the local fixture when you are finished:

```bash
kill "$FIXTURE_PID"
```

## Security notes

- **Response headers are redacted** before they're printed, diffed, or
  saved: `Authorization`, `Set-Cookie`, `Cookie`, `X-Api-Key`, and a few
  other sensitive header names are replaced with `<redacted>` by default.
  Extend the list with `REQSMITH_REDACT_HEADERS=x-my-secret,x-other-secret`.
  This is intentionally based on response-header names. Request files and
  response bodies are not rewritten, so do not hardcode credentials or save
  bodies that may contain secrets.
- **Project-local plugins require opt-in.** If a project ships a
  `./.reqsmith/plugins.toml`, reqsmith will not load it unless you run with
  `REQSMITH_ALLOW_PROJECT_PLUGINS=1` — this stops a `git clone` of an untrusted
  repo from silently executing WASM. See
  [`docs/plugins-security.md`](plugins-security.md).

Full details: [`docs/security-model.md`](security-model.md) and
[`docs/plugins-security.md`](plugins-security.md).

## Next steps

- [README: CLI subcommands](../README.md#cli-subcommands) — the full flag
  reference for `run`, `fmt`, `validate`, `list`, `diff`, and (with the
  `plugins` feature) `plugin`.
- [README: TUI keyboard shortcuts](../README.md#tui-keyboard-shortcuts)
- [README: Request File Format](../README.md#request-file-format) — the
  full request schema, including `params`, `body`, and disabling a
  header/param row with `enabled: false`.
- [README: Security](../README.md#security) and the two docs linked above,
  if you're evaluating reqsmith for use against real credentials or plan to
  write/run plugins.
