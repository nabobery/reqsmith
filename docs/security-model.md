# Security model

`reqsmith` is a local, user-driven terminal API client: you author your own
`.req.yml` request files and run them against services you choose, including
`localhost` and machines on your private network. This document describes the
threat model this hardening work assumes, the guarantees `reqsmith` provides, and
what it explicitly does not protect against.

## Threat model

- **Request files are trusted input.** You wrote (or reviewed) the
  `.req.yml` files, `reqsmith_envs.yml`, and `.env` files reqsmith reads. reqsmith does
  not sandbox or restrict what a request file can ask it to do (which host to
  hit, which headers to send, which body to post). Treat a `.req.yml` file
  the same way you'd treat a shell script: don't run one you haven't read
  from a source you don't trust.
- **Response bodies and headers are untrusted.** reqsmith treats data coming back
  from a server as attacker-controllable. It never evaluates, executes, or
  interprets response content as code, and response headers are redacted
  before they're persisted, diffed, or displayed (see below) so a malicious
  or compromised server can't use reqsmith to leak your own tokens back into your
  terminal history or on-disk snapshots any more than necessary.
- **The local filesystem under `.reqsmith/`** is where reqsmith persists run
  snapshots and downloaded response bodies. reqsmith assumes an attacker with
  write access to that directory (e.g. a symlink planted before a run) should
  not be able to trick reqsmith into overwriting an arbitrary file elsewhere on
  disk.

## Redaction contract (`src/core/redaction.rs`)

Response headers can carry credentials handed back by a server — a
`Set-Cookie`, a rotated bearer token, a signed challenge in
`WWW-Authenticate`. reqsmith redacts these before they reach any sink:

- **Default sensitive header set** (case-insensitive): `authorization`,
  `proxy-authorization`, `cookie`, `set-cookie`, `x-api-key`, `api-key`,
  `x-auth-token`, `x-amz-security-token`, `www-authenticate`,
  `authentication`, `x-csrf-token`, `x-xsrf-token`, `x-amz-signature`,
  `private-token`, `x-hub-signature`, plus the config-style key names
  `secret`, `token`, `password`, `client_secret`.
- **Extending the set**: set `REQSMITH_REDACT_HEADERS` to a comma-separated list
  of additional header names (e.g.
  `REQSMITH_REDACT_HEADERS=x-internal-token,x-tenant-secret`). Entries are
  merged with the defaults, not replacing them.
- **Redacted value**: sensitive header values are replaced with
  `<redacted:sha256:XXXXXXXX>`, where the eight hex characters are the start of
  the SHA-256 digest of the real value. Header names and non-sensitive values
  pass through unchanged.

  Why a fingerprint rather than a flat `<redacted>` marker: snapshots persist
  only the placeholder, so with a flat marker a rotated `Set-Cookie` looked
  _identical_ across two stored runs and `reqsmith diff` reported no header
  change at all. Distinct values now produce distinct placeholders, so the
  rotation is still reported — while the same value produces the same
  placeholder, so an unchanged secret still produces no diff entry.

  The digest is **not reversible**, but it is a fingerprint, not a blindfold:
  a low-entropy value (a short PIN, a boolean-ish token, a value drawn from a
  small known set — e.g. `Authorization: Basic dXNlcjpwYXNz`, guessable by
  hashing candidate `user:pass` pairs) could in principle be confirmed by
  brute-forcing candidates against the eight published hex characters. That is
  an accepted trade-off for header values; it is one more reason response
  bodies and request files — which reqsmith does not redact at all — remain
  your responsibility.
- **Where it's applied**:
  - `reqsmith run --output json` (`src/output.rs`) — response headers in the
    printed JSON are redacted.
  - Diffing (`src/core/diffing.rs`) — a header diff reports that a sensitive
    header _changed_ without ever showing the real values: both sides render
    as different `<redacted:…>` fingerprints. This holds for two live
    responses (compared on the real values) and for two stored snapshots
    (compared on the persisted fingerprints).
  - TUI response viewer (`src/components/response_viewer.rs`) — the Headers
    tab and Diff tab both display redacted values. Headers are redacted once
    when the response is set, not per rendered frame.
  - Stored run snapshots — `storage::save_run` (`src/core/storage.rs`)
    redacts headers at the write boundary, immediately before serializing,
    so no route to `.reqsmith/runs/*.json` can skip it regardless of how the
    `StoredRun` passed in was built. `StoredRun::from_run_result`
    (`src/core/models.rs`) itself copies headers verbatim — a `StoredRun`
    value transiently holds real, unredacted values in memory until it
    reaches `save_run`.
- **Logging**: nothing in the response pipeline passes header values to
  `tracing::*`. If you add a new log line touching response headers, route
  the values through `redaction::redact_headers` first.
- **Single policy source**: the default set plus any `REQSMITH_REDACT_HEADERS`
  extension are resolved once into a `RedactionPolicy` (reading/parsing the
  env var a single time), which every sink consults — rather than each sink
  re-parsing the environment per header. New response consumers should build a
  `RedactionPolicy::from_env()` (or call `redact_headers`) instead of
  re-implementing the match.

This is a _display and persistence_ control, not encryption: it prevents
sensitive response-header values from being echoed back to your screen,
terminal scrollback, or on-disk snapshots by reqsmith itself. It does not
rewrite request files, request values, response bodies, or arbitrary error
text. Do not hardcode credentials in request files, and do not print or save a
response body that may contain a secret. Redaction is based on response-header
names, not heuristic matching of arbitrary payload values.

## Network policy (`src/infra/http_client.rs`, `src/core/execution.rs`)

- **Scheme allowlist (always enforced, not opt-in)**: after variable
  interpolation, the request URL is parsed and only `http`/`https` schemes
  are accepted. Anything else (`file://`, `ftp://`, etc.) is rejected with
  `ExecutionError::InvalidRequest` before any connection is attempted.
- **Private/loopback addresses are allowed by default.** reqsmith is built for
  hitting `localhost`, `127.0.0.1`, and machines on your LAN — that's a
  primary use case, not an edge case, so it is not blocked by default.
- **Opt-in private-network denial**: pass `--deny-private-networks` to
  `reqsmith run`, or set `REQSMITH_DENY_PRIVATE_NETWORKS=1` (or `true`,
  case-insensitive) in the environment. The two are OR'd together, and the
  environment variable is the only way to enable the guard for the
  interactive TUI, which has no flag. It is wired through `RunOptions` →
  `execution::ExecutionOptions { deny_private_networks: true }` →
  `execution::execute_request_with_options`; the shared client disables proxy
  use and automatic redirects so the execution layer can check every hop
  asynchronously. When enabled the
  guard **fails closed**:
  - it rejects if the host is, or resolves to, an address that is not public
    unicast — unspecified, loopback, RFC1918 private, "this network"
    (`0.0.0.0/8`), link-local
    (`169.254.0.0/16` and IPv6 `fe80::/10`), shared address space
    (`100.64.0.0/10`), IETF protocol assignments
    (`192.0.0.0/24`), benchmarking (`198.18.0.0/15`), multicast
    (`224.0.0.0/4`, `ff00::/8`), reserved (`240.0.0.0/4`, including
    `255.255.255.255`), IPv6 unique-local (`fc00::/7`), or Teredo
    (`2001::/32`). IPv6 forms that tunnel an IPv4 address — mapped,
    compatible, 6to4 (`2002::/16`), and NAT64 (`64:ff9b::/96`) — are
    classified by the IPv4 address they embed. It rejects when _any_ resolved
    address is non-public, not only when all of them are, so a single private
    A/AAAA record mixed in with public ones is still blocked;
  - it rejects a host that cannot be resolved at all (rather than letting an
    unchecked connection proceed);
  - it applies to every redirect hop, not just the initial URL (see
    **Redirects** below);
  - it disables proxy support for the client (`.no_proxy()`), because a proxy
    would dial the target on our behalf and the addresses just vetted would
    not be the ones actually connected to.

  The default (flag and environment variable absent → `false`) leaves
  localhost/LAN working, which is the primary use case for a local API client.

  **Known limitation — DNS rebinding / TOCTOU:** reqwest re-resolves the
  hostname when it actually connects, so a name that resolves to a public
  address during this check could resolve to a private one moments later at
  connect time. Fully closing that gap requires pinning the checked IPs into
  the connection (a custom resolver per request), which reqsmith does not yet do.
  The flag meaningfully raises the bar against static private targets and
  obvious mixed records, but is not a complete SSRF defense on its own.

- **Timeouts**: a 30s total request timeout and a separate 10s connect
  timeout, so a non-responsive host fails fast instead of hanging.
- **Redirects**: reqwest's automatic redirect handling is disabled. The
  execution layer follows each hop asynchronously under the request's single
  timeout and cancellation budget. A hop is refused when:
  - 10 redirects have already been followed (i.e. an 11th hop is refused);
  - the target scheme is not `http`/`https`;
  - the target downgrades `https` → `http` (the hop would travel in
    cleartext);
  - `--deny-private-networks` is active and the target host is, or resolves
    to, a non-public address — resolution failure is also a refusal.

  The explicit loop also strips the `Authorization`, `Cookie`,
  `Proxy-Authorization`, and `WWW-Authenticate` request headers whenever a
  redirect changes the **host or the port**. This intentionally matches
  reqwest's automatic policy. That policy does *not* compare
  the scheme, so an `https` → `http` redirect back to the same host:port
  would keep those headers — which is exactly why reqsmith refuses the
  downgrade itself. reqsmith locks in the stripping guarantee with a
  regression test (`redirect_strips_authorization_header_across_port_change`
  in `src/core/execution.rs`) that proves a same-host, different-port
  redirect does not forward an `Authorization` header to the second origin.
- **Proxies**: `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` are honored normally
  (reqwest's default behavior). They are ignored when the private-network
  guard is enabled, since a proxy connection would bypass the address check.
- **Error messages**: reqwest errors are stripped of their URL
  (`Error::without_url`) before being shown, logged, or persisted, so
  credentials carried in a query string do not leak into terminal history or
  saved run snapshots. A timeout is reported with the specific budget it
  tripped named — the 10s connect timeout for a connect-phase timeout, the
  30s total timeout otherwise — so the limit is discoverable.
- **Body cap**: response bodies are capped at 10 MB read into memory;
  anything beyond that is truncated rather than exhausting memory on a huge or
  slow-drip response — the 10 MB cap itself is unchanged by this hardening
  pass. Truncation is reported out of band — via the `truncated` field in
  JSON output and a note in the human and TUI views — never by editing the
  body, so assertions and `expect_body_path` see the real (cut) bytes.
  Similarly, a body that looks like binary data is reported via the
  `is_binary` field in JSON output rather than being dumped as text; `body`
  is `null` in that case.
- **Cancellation**: requests (including the header/connect phase and the body
  streaming loop) respect a `CancellationToken`, so a hung or slow request
  can always be interrupted. Unchanged by this hardening pass.

## Storage guarantees (`src/core/atomic_write.rs`, `src/core/storage.rs`)

Every user-data write reqsmith performs — run snapshots, downloads, and
(re)written request/formatter files — goes through one shared writer,
`src/core/atomic_write.rs`, so the filesystem-safety policy lives in a single
audited place rather than being re-implemented per call site.

- **Atomic writes**: data is written to a fresh temp file in the _same_
  directory as the final destination, `fsync`ed, then `rename`d into place
  (`std::fs::rename` maps to `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` on
  Windows). A reader never observes a partially written file. On failure the
  temp file (and any reservation, below) is cleaned up rather than leaked, and
  after the rename the containing directory is best-effort `fsync`ed for crash
  durability.
- **Symlink safety**: before writing, reqsmith checks (via `symlink_metadata`,
  which does not follow the link — unlike `Path::exists()`) whether the
  destination is a symlink, including a _dangling_ one. If so the write is
  refused instead of following the link. This closes the gap where a
  predictable run filename (`{sanitized_name}_{timestamp}.json`) could be
  pre-planted as a (possibly dangling) symlink by another process with write
  access to `.reqsmith/`. Request files written by `reqsmith fmt` and the TUI are
  protected the same way (`write_replace` refuses a symlinked target), which
  means **reqsmith writes only to regular files**: if your `.req.yml` is a
  symlink into a shared collection, point reqsmith at the target directly or
  replace the link with a file. On Windows this check does not flag directory
  junctions.
- **File modes**: on Unix the temp file each write is staged in is created
  `0600`, so a run snapshot or download in `.reqsmith/` is owner-only from the
  moment it exists — never briefly world-readable. When replacing an existing
  file, its permission bits are copied onto the temp file before the rename, so
  a `chmod 600` request file is still `0600` after `reqsmith fmt`. The working
  tree's `.reqsmith/` directory holds response snapshots and downloads and
  should be listed in your `.gitignore`.
- **Race-free naming**: run snapshots and downloads reserve their destination
  filename atomically with `create_new` before renaming into place, so two
  concurrent saves that pick the same name can't silently overwrite each
  other — the loser observes `AlreadyExists` and advances to the next
  `_{n}` suffix. (This replaces the earlier check-then-write on
  `Path::exists()`, which had a TOCTOU window.)
- **Parent-directory guard**: `save_run`/`save_response_body` refuse to write
  if `.reqsmith` (or the `runs`/`downloads` subdirectory) is itself a symlink,
  checked _before_ creating anything, so a `.reqsmith -> /somewhere/hostile` swap
  can't redirect writes out of the project tree. A fully attacker-controlled
  parent directory _tree_ is out of scope (see "What is NOT protected").
- **Filename sanitization**: the _whole_ snapshot filename stem
  (`{request_name}_{timestamp}`, not just the request name) is sanitized to
  `[A-Za-z0-9_-]` — path separators and `..` sequences are replaced with `_`,
  so a request named `../../etc/passwd` still resolves to a plain filename
  inside `.reqsmith/runs/`, never outside it.
- **Secrets redacted before persistence**: see the redaction contract above —
  `storage::save_run` redacts response headers at the write boundary, right
  before serializing, so `.reqsmith/runs/*.json` snapshots don't contain
  secrets that were present in the original response headers. A `StoredRun`
  value itself may transiently hold the real, unredacted headers in memory
  until it reaches `save_run`.

### Retention

Nothing prunes `.reqsmith/`. Run snapshots and downloaded response bodies are
kept until you delete them by hand — there is no age limit, no count limit, and
no `reqsmith` command that cleans them up. A long-lived project tree therefore
accumulates every run you saved, each one a file whose headers are redacted but
whose body is verbatim (bounded only by the network-read cap described under
Network policy above). Delete `.reqsmith/runs/` and `.reqsmith/downloads/`
yourself when you no longer need the history.

## What is NOT protected

This hardening pass is deliberately scoped. It does **not**:

- Protect the `.req.yml` request files, `reqsmith_envs.yml`, or `.env` files
  themselves — if a secret is hardcoded into a request file's headers or
  body, or into your `.env`, that's plaintext on your filesystem exactly as
  you put it there. reqsmith doesn't encrypt or restrict access to your own
  files.
- Redact **request** headers/body — only **response** headers are redacted.
  If a request file sends `Authorization: Bearer <token>`, that token is
  visible in the request file itself (by design — you wrote it) and is sent
  over the wire as specified.
- Prevent you from saving a response body that itself contains a secret.
  Pressing `w` in the TUI (or `--save`) writes the response body to
  `.reqsmith/downloads/` verbatim; if a server response body contains a secret
  (e.g. a JSON payload with an API key in it), that secret is saved as-is —
  only _headers_ are redacted, not bodies. If you save a response body
  containing sensitive data, treat that downloaded file the same way you'd
  treat any other file containing a secret.
- Provide encryption at rest for `.reqsmith/runs/` or `.reqsmith/downloads/` —
  atomicity and symlink-safety protect write _integrity_, not
  confidentiality against another local user/process with read access to
  your filesystem.
- Validate or sanitize response bodies against injection/XXE/etc. — reqsmith
  displays response bodies as text/JSON; it does not execute or render them
  as HTML/JS, so classic response-body injection classes don't apply to how
  reqsmith itself processes them, but this document makes no claim about how
  _you_ might pipe that output elsewhere.
- Block private-network requests by default — see Network policy above. This
  is a deliberate default for a local API client, not an oversight; pass
  `reqsmith run --deny-private-networks` if you need that guard, and note its
  documented DNS-rebinding limitation.
- Close the DNS-rebinding / TOCTOU gap on the private-network guard: the
  address check and the actual connection are two separate resolutions, so a
  name that resolves to a public address at check time could resolve to a
  private one by connect time. See "Known limitation" under Network policy
  above.
- Stop a plugin from defeating header-based redaction. A `post_response`
  plugin hook runs before redaction and can rename a sensitive response
  header (e.g. `set-cookie` to something the policy doesn't recognize),
  putting its value into stdout and stored snapshots in the clear. See
  [`docs/plugins-security.md`](plugins-security.md).
- Defend against a fully attacker-controlled parent directory _tree_ under
  `.reqsmith/`. The storage guard refuses a symlinked destination file and a
  symlinked `.reqsmith`/`runs`/`downloads` directory, but portably validating
  every path component against a TOCTOU needs `openat`/`O_NOFOLLOW` handles
  that `std` doesn't expose. If an attacker already controls your working
  tree's directory structure they can do far more than redirect a snapshot.
- Validate TLS certificates beyond what `reqwest`'s `rustls` backend does by
  default — reqsmith does not add its own certificate pinning or custom trust
  store handling.
