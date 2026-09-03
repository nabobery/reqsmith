# Contributing to reqsmith

Thanks for your interest in contributing to `reqsmith`, a terminal-native API
client! This document covers how to get set up, the project's conventions,
and what to expect when opening a pull request.

## Getting started

`reqsmith` is a Rust 2024 project. **For day-to-day development, use a current
stable toolchain** (`rustup update stable`). The **MSRV of 1.88** is a
_guarantee for users building the default binary_ — it is verified in CI and
you generally don't need to develop on it. Note that building the optional
`plugins` feature (i.e. anything with `--all-features`) pulls in
extism/wasmtime and requires **Rust >= 1.91**; see the [MSRV](#msrv) section.

```bash
rustc --version
```

Build the project (this repo commits `Cargo.lock`, so builds should be
reproducible):

```bash
cargo build --locked
```

Run the local CI suite before opening a PR:

```bash
just ci        # fmt-check + lint + test — needs only a Rust toolchain
just ci-full   # everything CI runs: ci + audit + deny + secrets
```

`just ci-full` additionally needs [`cargo-audit`], [`cargo-deny`], and
[`gitleaks`] on your `PATH`; `just ci` needs none of them, so use `ci` for the
fast loop and `ci-full` (or the PR's CI run) before you request review.

[`cargo-audit`]: https://github.com/rustsec/rustsec/tree/main/cargo-audit
[`cargo-deny`]: https://github.com/EmbarkStudios/cargo-deny
[`gitleaks`]: https://github.com/gitleaks/gitleaks

If you don't have [`just`](https://github.com/casey/just) installed, these are
the exact commands the recipes run:

```bash
# just fmt-check
cargo fmt --all -- --check

# just lint
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --all-features --locked -- -D warnings

# just test
cargo test --locked
cargo test --locked --all-features

# just audit  (ignore chain mirrors .github/workflows/checks.yml)
cargo audit \
  --ignore RUSTSEC-2026-0222 --ignore RUSTSEC-2026-0269 \
  --ignore RUSTSEC-2026-0247 --ignore RUSTSEC-2026-0250 \
  --ignore RUSTSEC-2026-0251 --ignore RUSTSEC-2026-0255 \
  --ignore RUSTSEC-2026-0253

# just deny
cargo deny --all-features check

# just secrets  (the dir scan also walks target/ locally)
gitleaks git --redact --no-banner .
gitleaks dir --redact --no-banner .
```

Run `just --list` to see every recipe.

## MSRV

The **default build** targets Rust **1.88** as the minimum supported version.
Avoid using language or standard-library features newer than that in code that
compiles under default features, unless the MSRV is bumped as part of the same
change (and documented in `Cargo.toml` and the CHANGELOG). CI enforces this with
a dedicated `msrv` job that runs `cargo check --locked --all-targets` on Rust
1.88 with default features.

The optional **`plugins` feature** depends on extism/wasmtime and requires
**Rust >= 1.91**. That floor applies only to `--features plugins` /
`--all-features` builds; it does not affect the default-build MSRV. Code behind
`#[cfg(feature = "plugins")]` may use language features up to the plugins MSRV.

## Code style

- Formatting is enforced by `rustfmt` with `max_width = 100` (see
  `rustfmt.toml`). Run `cargo fmt` before committing.
- Lints are enforced by `clippy` with warnings denied
  (`-D warnings`), run against both default and `--all-features` builds.
- Prefer small, focused commits and pull requests.

## Tests

This project follows a test-first convention where practical: when fixing a
bug, add a regression test that fails before your fix and passes after it.
When adding a feature, add tests that cover the new behavior. Run:

```bash
cargo test --locked
cargo test --locked --all-features
```

## Pull requests

Before opening a PR:

- [ ] `just ci` passes locally (or the equivalent `fmt-check` / `lint` /
      `test` commands above); `just ci-full` if you have cargo-audit,
      cargo-deny, and gitleaks installed.
- [ ] Tests were added or updated for the change.
- [ ] The description explains **why** the change is needed, not just what
      changed, and links any related issue.
- [ ] No real secrets, tokens, or credentials appear anywhere in the diff,
      commit messages, or example request files — use synthetic/fake data
      only.

CI runs formatting, clippy (default and all-features), tests on Linux,
macOS, and Windows, an MSRV build, dependency auditing (`cargo audit`,
`cargo deny`), and a secret scan (`gitleaks`, history + working tree) on
every pull request. Please make sure these pass before requesting review.

## Sign-off (DCO)

We don't currently require a Developer Certificate of Origin sign-off, but
adding `Signed-off-by: Your Name <email>` (via `git commit -s`) to your
commits is welcome and appreciated, especially for larger contributions.

## Reporting bugs and requesting features

Use the issue templates under "New Issue" on GitHub. Please redact any real
tokens, credentials, or captured request/response data from logs and
examples — use synthetic data instead. See `SECURITY.md` for how to report
security-sensitive issues privately.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By
participating, you're expected to uphold it.
