# Plugin security & trust model

`reqsmith`'s plugin system (the optional `plugins` Cargo feature) lets you extend
request/response handling with WASM modules run through
[Extism](https://extism.org/): pre-request mutation, post-response
inspection, authentication, and variable providers. A loaded plugin is
**native-capability code** — it runs inside a WASM sandbox, but within that
sandbox it can do anything the plugin author wrote it to do with the inputs
and host functions reqsmith exposes to it. This document explains the trust
boundaries reqsmith enforces around that, and how to configure them.

If you haven't reviewed the source (or trust the publisher) of a plugin,
don't run it. Nothing described below is a substitute for that.

## Requirements

Building reqsmith with `--features plugins` (which pulls in Extism/Wasmtime)
requires **Rust >= 1.91**. The default, plugin-free build keeps the crate's
usual MSRV (1.88). If you only need the core CLI/TUI, you don't need the
newer toolchain.

## Threat model

Two things can go wrong with plugins, and reqsmith treats them differently:

1. **A `plugins.toml` you didn't intend to trust gets loaded.** The most
   realistic version of this: you `git clone` or `cd` into someone else's
   repository, and it ships a `.reqsmith/plugins.toml` plus a `.wasm` file. If
   reqsmith auto-loaded and ran that WASM the moment you next ran `reqsmith` in that
   directory, cloning a hostile repo would be enough to get attacker code
   execution — no explicit "run this plugin" action on your part. This is
   the supply-chain risk C1 below closes.
2. **A plugin you _did_ choose to run gets more access than it needs.**
   Even a plugin you trust shouldn't be handed your entire process
   environment (API keys, cloud credentials, CI tokens) by default, and a
   plugin's `.wasm` file shouldn't be silently swappable out from under a
   pinned config. C2 and C3 below address this.

## C1 — Project-local plugins require explicit opt-in

reqsmith reads `plugins.toml` from **both** of these locations, project-local
first:

1. `$cwd/.reqsmith/plugins.toml` — **project-local**. This file (and the `.wasm`
   files it points to) can arrive as part of a cloned/downloaded repository,
   so reqsmith treats it as **untrusted by default**.
2. `dirs::config_dir()/reqsmith/plugins.toml` — **user-global**: on Linux,
   `$XDG_CONFIG_HOME/reqsmith/plugins.toml` (default `~/.config/reqsmith/plugins.toml`);
   on macOS, `~/Library/Application Support/reqsmith/plugins.toml`. You placed
   this file yourself, so it's trusted: it loads and runs its plugins normally.

Both are loaded, and their entries are merged into one registry. A repository
cannot disable your global plugins simply by shipping a `plugins.toml` of its
own — only the project-local file is ever gated. If a project-local config
declares a plugin with the same name as one in your user-global config, the
user-global definition wins: the project-local duplicate is rejected (with a
load error naming the config that already claimed the name) instead of
shadowing it or running alongside it.

When reqsmith finds a project-local `.reqsmith/plugins.toml`, it does **not**
instantiate any WASM from it unless you explicitly opt in for that run:

```sh
REQSMITH_ALLOW_PROJECT_PLUGINS=1 reqsmith run ./my-request.req.yml
```

Accepted truthy values (case-insensitive): `1`, `true`, `yes`. Anything
else — including the variable being unset — means "not opted in". There is
no interactive prompt: reqsmith runs in CLI/TUI/CI contexts where a stdin prompt
either can't happen or would be silently skipped, so the opt-in is
env-var-only and must be a deliberate, out-of-band decision each time you
choose to trust a given project's plugins for that invocation.

If a project-local config is found but you haven't opted in, no `.wasm` file
from it is even read. The refusal is visible, not just logged: `reqsmith plugin
list` shows those entries with status `blocked_untrusted` and prints the
variable to set, and `reqsmith plugin info <name>` says the same.

**User-global plugins are unaffected by this gate** — they're the config you
put on your own machine, not something a repository can plant — and they still
load normally in a directory whose project-local config was refused.

## C2 — Pin plugin integrity with `sha256`

Add an optional `sha256` field to a `[[plugin]]` entry to pin the exact WASM
binary that's allowed to load:

```toml
[[plugin]]
name = "auth-sigv4"
path = "plugins/auth.wasm"
capabilities = ["authenticate"]
sha256 = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
```

Before instantiating the plugin, reqsmith computes the SHA-256 digest of the
on-disk `.wasm` file and compares it against `sha256`. If they don't match,
that plugin fails to load with an `IntegrityMismatch` error naming the plugin,
the expected digest, and the digest actually found — and it is not run. If
`sha256` is omitted, the file loads as-is.

The digest is normalized before comparison: surrounding whitespace is trimmed,
an optional `sha256:` prefix is stripped, and case is ignored. What remains
must be exactly **64 hex characters** — anything else is rejected at config
load as a *malformed sha256*, not silently treated as a mismatch, so a
truncated or mistyped digest can't quietly turn into a load failure you'd
misread as tampering.

This protects against the WASM file being swapped (accidentally or
maliciously) without the config being updated to match — e.g. a build
artifact getting overwritten, or a compromised dependency replacing the
binary a `plugins.toml` on disk still points to.

Compute a digest to pin with, e.g.: `shasum -a 256 plugins/auth.wasm`.

## Where plugin modules may live

A `[[plugin]]` `path` is resolved relative to the directory containing its
`plugins.toml`, and the declared path must be relative and must not escape
that directory. Absolute paths, home-relative (`~/…`) paths, and any path
with a `..` component are rejected with a config error. If the resolved path
exists, reqsmith additionally canonicalizes both it and the config directory
and checks the real path still lives inside the real directory, so a symlink
planted inside the config directory can't point the loader somewhere else on
disk. This keeps a `plugins.toml` from reaching somewhere else on your
filesystem for the bytes it runs, and keeps the file you review (the config)
adjacent to the bytes it names.

## C3 — Env-var least privilege (`env_allowlist`)

Plugins can call a `provide_env_var(key)` host function to read a single
environment variable value. By default, **a plugin gets no environment
values at all**, even if reqsmith itself has a full environment (including
secrets/tokens) available internally. To grant a plugin access to specific
variables, list them explicitly:

```toml
[[plugin]]
name = "auth-sigv4"
path = "plugins/auth.wasm"
capabilities = ["authenticate"]
env_allowlist = ["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"]
```

Only the exact (case-sensitive) names in `env_allowlist` are ever passed
through; any other variable a plugin asks for via `provide_env_var` comes
back empty, regardless of whether reqsmith itself has that variable set. An
empty or absent `env_allowlist` (the default) means the plugin cannot read
_any_ environment variable through this mechanism.

This is a deliberate, default-deny, breaking change from earlier behavior
where every plugin implicitly saw every env value passed to a hook call. If
a plugin you rely on needs environment access, you must now list exactly
which variables it needs.

## Resource limits

Two limits from `plugins.toml` bound what a loaded plugin can consume:

- `timeout_ms` (default `5000`, must be `1..=30000`) — wall-clock timeout for
  a single plugin call. A call that exceeds this returns
  `PluginError::Timeout` instead of hanging the request pipeline.
- `memory_limit_pages` (default `256`, must be `1..=1024`; a page is 64KiB, so
  the ceiling is 64MiB) — the maximum WASM linear memory the plugin's instance
  may grow to.

Both are validated at config load: `0` (which would mean "no guard at all") and
values past the ceilings above are rejected with a config error naming the
field and its bound, so a `plugins.toml` cannot quietly opt out of the limits
that make the sandbox bounded.

These are per-plugin, driven by the top-level `timeout_ms` /
`memory_limit_pages` keys in `plugins.toml` (not currently configurable
per-entry). No fuel/instruction budget is metered: the wall-clock timeout is
the guard against a plugin that spins instead of allocating.

## Per-entry failure isolation

A plugin that fails to load takes only itself out. An unreadable module, a
digest mismatch, a path that escapes the config directory, or a module Extism
can't instantiate is recorded against that entry and skipped; every other
plugin in both configs still loads. A module the config names but that isn't
on disk is a skip rather than a failure.

`reqsmith plugin list` shows the outcome per plugin (`loaded`, `missing_wasm`,
`load_failed`, `blocked_untrusted`, `disabled`, `configured` — shown when the
registry as a whole failed to build, so status can't be resolved further),
and `reqsmith plugin info <name>` prints that entry's load error. This
matters for security review: a tampered plugin's failure is reported next to
the plugin that caused it, instead of silently disabling your whole plugin
set. A `plugins.toml` that itself fails to read/parse/validate does not
suppress the others: it's reported as its own warning, and every config that
*did* parse is still shown (H1).

## The sandbox boundary

Plugins run inside Extism's WASM sandbox with **no host filesystem access
and no outbound network access**. The `Manifest` reqsmith builds for each
plugin does not set `allowed_paths` or `allowed_hosts`, so there is nothing
for a plugin to read/write on disk or connect to over the network from
inside the guest. The WASI runtime reqsmith enables for the guest is limited to
things like stdio/clock — it does not map any host directory or permit any
outbound connection.

The _only_ way a plugin can affect the outside world, or observe anything
beyond the JSON payload it's called with, is through the host functions reqsmith
explicitly registers:

- `provide_env_var(key) -> String` — read an environment variable, subject
  to the C3 allowlist above.
- `plugin_log(msg)` — emit a `tracing::info!` log line (visible in reqsmith's
  own logs, tagged `source = "plugin"`).
- `read_config(key) -> String` — read a value from that plugin's own
  `[plugin.config]` table in `plugins.toml`.

A plugin's actual leverage, then, comes entirely from what you (a) let it
receive as input (request/response contents it's called with, which may
include secrets already present in headers/body), (b) allow into its
`env_allowlist`, and (c) do with its `HookResult`/`AuthResult` output, which
reqsmith applies back onto the request or response. Reviewing a plugin's source
means checking what it does with those three things — not worrying about it
reaching outside the sandbox, since it structurally cannot.

### A `post_response` hook can defeat header redaction

reqsmith redacts sensitive response headers **by name** (`set-cookie`,
`authorization`, …) before printing, diffing, or storing a run snapshot. A
`post_response` plugin runs *before* that and can rewrite the response it is
handed — including renaming `set-cookie` to something the redaction policy
doesn't recognize, which puts the secret's value into stdout and the stored
snapshot in the clear.

This is inherent to letting a plugin mutate a response at all; there is no fix
short of not running the plugin. Treat `post_response` as a capability that
can expose any secret the response carries, and grant it only to plugins you
have reviewed.

## Summary checklist

- Only run plugins whose source you've reviewed or whose publisher you
  trust — the sandbox limits _what_ a plugin can reach, not _whether_ it
  behaves the way you expect with what it's given.
- Project-local plugins (`.reqsmith/plugins.toml` in a repo) are off by default;
  set `REQSMITH_ALLOW_PROJECT_PLUGINS=1` only for repositories you trust, and
  only for the invocation where you actually need them.
- **Never export `REQSMITH_ALLOW_PROJECT_PLUGINS=1` from your shell profile**
  (or a CI job's global env). It is per-invocation on purpose: a permanent
  export silently opts in *every* repository you ever `cd` into, which is
  exactly the supply-chain hole C1 exists to close. Prefix the one command
  that needs it instead.
- Pin `sha256` on plugin entries you care about, especially ones sourced
  from anywhere outside your own build.
- Keep `env_allowlist` as small as each plugin actually needs — start from
  empty and add names one at a time. `reqsmith plugin info <name>` flags
  allowlisted names that look sensitive, and loading warns about them.
- Treat `post_response` as the most dangerous capability: it can rename
  headers and so defeat name-based redaction of stdout and stored snapshots.
