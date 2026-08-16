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
