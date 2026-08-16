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

reqsmith looks for `plugins.toml` in two places, in order:

1. `$cwd/.reqsmith/plugins.toml` — **project-local**. This file (and the `.wasm`
   files it points to) can arrive as part of a cloned/downloaded repository,
   so reqsmith treats it as **untrusted by default**.
2. `$XDG_CONFIG_HOME/reqsmith/plugins.toml` (typically `~/.config/reqsmith/plugins.toml`)
   — **user-global**. You placed this file yourself, so it's trusted the way
   it always has been: it loads and runs its plugins normally.

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

If a project-local config is found but you haven't opted in, reqsmith logs a
single warning naming the file and how to enable it, then behaves exactly as
if no plugin config had been found at all (an empty plugin registry — no
plugin is loaded, no `.wasm` file is even read).

**User-global plugins are unaffected by this gate** — they're the config you
put on your own machine, not something a repository can plant.

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
on-disk `.wasm` file and compares it (case-insensitively) against `sha256`.
If they don't match, the plugin fails to load with a clear
`IntegrityMismatch` error naming the plugin, the expected digest, and the
digest actually found — and that plugin is not run. If `sha256` is omitted,
behavior is unchanged from before (the file loads as-is).

This protects against the WASM file being swapped (accidentally or
maliciously) without the config being updated to match — e.g. a build
artifact getting overwritten, or a compromised dependency replacing the
binary a `plugins.toml` on disk still points to.

Compute a digest to pin with, e.g.: `shasum -a 256 plugins/auth.wasm`.

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

- `timeout_ms` (default `5000`) — wall-clock timeout for a single plugin
  call. A call that exceeds this returns `PluginError::Timeout` instead of
  hanging the request pipeline.
- `memory_limit_pages` (default `256`, i.e. 256 * 64KiB = 16MiB) — the
  maximum WASM linear memory the plugin's instance may grow to.

These are per-plugin, driven by the top-level `timeout_ms` /
`memory_limit_pages` keys in `plugins.toml` (not currently configurable
per-entry). There is no separate fuel/instruction-metering knob in use today
— the wall-clock timeout is the guard against a plugin that spins instead of
allocating.

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

## Summary checklist

- Only run plugins whose source you've reviewed or whose publisher you
  trust — the sandbox limits _what_ a plugin can reach, not _whether_ it
  behaves the way you expect with what it's given.
- Project-local plugins (`.reqsmith/plugins.toml` in a repo) are off by default;
  set `REQSMITH_ALLOW_PROJECT_PLUGINS=1` only for repositories you trust, and
  only for the invocation where you actually need them.
- Pin `sha256` on plugin entries you care about, especially ones sourced
  from anywhere outside your own build.
- Keep `env_allowlist` as small as each plugin actually needs — start from
  empty and add names one at a time.
