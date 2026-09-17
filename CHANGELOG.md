# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once it reaches 1.0.

## [Unreleased]

### Added

- Nothing yet.

## [0.1.0] - unreleased

Initial (pre-release) feature set:

### Added

- Terminal UI (TUI) and CLI modes for building and running API requests.
- YAML-based request file format (`.req.yml`) for defining requests,
  collections, and environments.
- Environment variable interpolation into request files (headers, URLs,
  bodies, etc.), including `.env` file support.
- Response assertions against status codes, headers, and bodies.
- Response diffing against previously saved run snapshots.
- Optional, experimental WASM plugin system (via Extism) for pre/post
  request hooks and custom variable providers, gated behind the `plugins`
  feature flag.
- **Security hardening.** Opt-in SSRF guard: `reqsmith run
  --deny-private-networks` (or `REQSMITH_DENY_PRIVATE_NETWORKS=1`) rejects
  requests whose host resolves to a loopback, link-local, unspecified, or
  private address, and applies the same check to every redirect hop (capped
  at 10 hops).
- **Security hardening.** Sensitive response headers are redacted before they
  are printed, diffed, or saved, using a `<redacted:sha256:XXXXXXXX>`
  placeholder so a rotated secret still shows as changed; the list is
  extendable via `REQSMITH_REDACT_HEADERS`.
- **Security hardening.** Request files and run snapshots are written
  atomically (write to a temp file in the same directory, then rename) so an
  interrupted write cannot truncate or corrupt an existing file, and are
  created `0600` on Unix.
- **Security hardening.** Project-local plugins require explicit consent
  (`REQSMITH_ALLOW_PROJECT_PLUGINS=1`), are integrity-checked against the
  SHA-256 recorded in `plugins.toml`, and only receive the environment
  variables named in their `env_allowlist`. Project-local and user-global
  `plugins.toml` files are both discovered and merged into one registry.

### Changed

- **Breaking.** The project was renamed from `hurl` to `reqsmith`. Everything
  user-visible moved with it:
  - the binary and crate are now `reqsmith` (was `hurl`);
  - request files use the `.req.yml` / `.req.yaml` suffixes (was
    `.hurl.yml` / `.hurl.yaml`);
  - the named-environments file is `reqsmith_envs.yml` (was `hurl_envs.yml`);
  - run snapshots, plugin manifests, and other local state live under
    `.reqsmith/` (was `.hurl/`);
  - the config and log paths moved to the `reqsmith` application directory
    (was the `hurl` one), so plugin config is now read from
    `$XDG_CONFIG_HOME/reqsmith/plugins.toml`;
  - all environment variables reqsmith reads use the `REQSMITH_` prefix
    (`REQSMITH_REDACT_HEADERS`, `REQSMITH_ALLOW_PROJECT_PLUGINS`) — these are
    new in this release, so there is no `HURL_*` equivalent to migrate.

  There is no automatic migration: rename your request files, environment
  file, and local state directories before upgrading.
- Plugin module `path`s must be relative to their `plugins.toml`; absolute, `..`, and `~/` paths are rejected.

[Unreleased]: https://github.com/nabobery/reqsmith/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/nabobery/reqsmith/releases/tag/v0.1.0
