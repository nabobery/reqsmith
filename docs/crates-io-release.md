# Publishing `reqsmith` to crates.io

A step-by-step runbook for publishing the `reqsmith` crate to
[crates.io](https://crates.io), following current best practices. Read
top-to-bottom the first time; on later releases you'll mostly use
[§4 Pre-flight](#4-pre-flight-checklist-every-release) and
[§6 Automated release](#6-automated-releases-trusted-publishing).

> **Publishing is permanent.** A published `name@version` can never be deleted
> or overwritten — only *yanked* (hidden from new dependency resolution). Get
> it right with a dry run before you push the button. Versions are never
> reusable.

---

## 0. Where things stand today

Verified at the time this doc was written:

- ✅ **`reqsmith` is available** on the crates.io index (not taken).
- ✅ **Metadata is complete** in `Cargo.toml` (`description`, `license`,
  `repository`, `readme`, `keywords`, `categories`, `rust-version`).
- ✅ **`cargo publish --dry-run --locked` passes** — packaged and
  verify-compiled with no errors. Run the dry run and inspect its reported
  file count and compressed size before publishing.
- ✅ **Publish automation is wired but dormant.** The `publish-crate` job in
  `release.yml` runs only when the repository variable
  `PUBLISH_TO_CRATES_IO=true`; see [§6](#6-automated-releases-trusted-publishing).
- ℹ️ **`reqsmith` is a binary crate** (a CLI/TUI, no library target). Consumers
  install it with `cargo install reqsmith`; there is no public library API, so
  docs.rs will show the crate page but little/no API documentation. That's
  expected.

**The order that matters:** crates.io Trusted Publishing (the no-token CI path)
can only be configured for a crate that *already exists*. So the **first
release must be a manual, token-based publish** ([§5](#5-first-release-manual-token-based));
after that, switch to Trusted Publishing for every subsequent release
([§6](#6-automated-releases-trusted-publishing)).

---

## 1. One-time crates.io account setup

1. Sign in at <https://crates.io> with your GitHub account (`nabobery`).
2. **Verify your email** under *Account Settings* — crates.io **refuses to
   publish** until your email is verified.
3. **Enable two-factor auth** on your GitHub account (crates.io auth rides on
   GitHub; 2FA protects your ability to publish).

---

## 2. Confirm `Cargo.toml` metadata

Already present and correct — confirm it still is before a release:

```toml
[package]
name = "reqsmith"
version = "0.1.0"          # bump per release; must match the git tag
edition = "2024"
rust-version = "1.88"      # default-build MSRV (plugins feature needs 1.91)
description = "Terminal-native API client — …"
license = "MIT"            # SPDX expression; LICENSE file is in the repo
repository = "https://github.com/nabobery/reqsmith"
readme = "README.md"
keywords = ["http", "api-client", "tui", "cli", "rest"]   # ≤5, each ≤20 chars
categories = ["command-line-utilities", "development-tools", "web-programming"]
# Allowlist: only what a consumer of the published crate needs.
include = ["src/**", "Cargo.toml", "Cargo.lock", "README.md", "LICENSE*", "CHANGELOG.md"]
```

Optional polish:

- **docs.rs with the plugin API**: to have docs.rs build the `plugins`
  feature, add:
  ```toml
  [package.metadata.docs.rs]
  all-features = true
  ```
  (docs.rs uses a recent stable toolchain, so the plugins MSRV of 1.91 is fine.)
- **README image on crates.io**: the README embeds the demo GIF via an
  absolute raw GitHub URL
  (`https://raw.githubusercontent.com/nabobery/reqsmith/main/docs/demo.gif`),
  not a relative path. The GIF is **not** included in the package (it's under
  `docs/`, which the `include` allowlist above omits), so a relative path
  would 404 on crates.io — the absolute URL is required for the image to
  render there.
- **Package contents are an allowlist, not a denylist**: `include` above
  already keeps the package to just `src/**` and the top-level metadata
  files — `.github/`, `docs/`, `justfile`, etc. are never packaged. Keep
  `Cargo.lock` in the package — it makes `cargo install --locked`
  reproducible for a binary crate.

---

## 3. Sanity-check the SPDX license

`license = "MIT"` must be a valid SPDX expression **and** there must be a
`LICENSE` file (there is). If you ever dual-license, use the SPDX `OR` form,
e.g. `license = "MIT OR Apache-2.0"`.

---

## 4. Pre-flight checklist (every release)

Run these from a clean checkout of the exact commit you intend to release:

- [ ] Bump `version` in `Cargo.toml`.
- [ ] `cargo build --locked` so `Cargo.lock` updates; commit both.
- [ ] Move `CHANGELOG.md` `Unreleased` items under a `## [x.y.z] - <date>`
      heading.
- [ ] Re-check the name is still free (first release only):
      `curl -sI https://index.crates.io/re/qs/reqsmith` → `404` = free.
- [ ] `cargo publish --dry-run --locked` is clean.
- [ ] **Review exactly what will ship:** `cargo package --list` — confirm no
      secrets, no stray large files, no local junk.
- [ ] Full gate green: `cargo fmt --check`, `cargo clippy --all-targets
      --all-features --locked -- -D warnings`, `cargo test --locked
      --all-features`. (CI's reusable `checks.yml` covers all of this.)
- [ ] Secret scan clean: `gitleaks git --redact .` and `gitleaks dir
      --redact .`.

---

## 5. First release (manual, token-based)

Required once, to create the crate on crates.io. After this, prefer
[§6 Trusted Publishing](#6-automated-releases-trusted-publishing).

1. **Create a scoped API token** at *crates.io → Account Settings → API
   Tokens*:
   - Scopes: **`publish-new`** (needed to create a brand-new crate) and
     **`publish-update`**.
   - You cannot *crate-scope* the token to `reqsmith` yet (the crate doesn't
     exist), so scope it to publish-new for this one time.
   - Set a **short expiry** and a descriptive name (e.g.
     `reqsmith-first-publish`).
2. **Authenticate locally without persisting the token in the repo:**
   ```bash
   # Interactive (stores in ~/.cargo/credentials.toml, NOT the repo):
   cargo login
   # …or one-shot via env var, so nothing is written to disk:
   #   export CARGO_REGISTRY_TOKEN=cio_xxx
   ```
   Never commit a token or put it in `Cargo.toml`/workflow files.
3. **Dry run, then publish** from the tagged commit:
   ```bash
   cargo publish --dry-run --locked
   cargo publish --locked
   ```
4. **Verify:**
   - Crate page: <https://crates.io/crates/reqsmith>
   - Install works: `cargo install reqsmith --locked`
   - docs.rs build (may take a few minutes): <https://docs.rs/reqsmith>
5. **Rotate the token:** revoke the first-publish token now, and either create
   a new **crate-scoped** (`reqsmith`) token if you'll ever publish manually
   again, or — better — move to Trusted Publishing and keep **no** token at
   all.

---

## 6. Automated releases (Trusted Publishing)

The modern best practice: no long-lived token in GitHub secrets. crates.io
trusts a specific GitHub repo + workflow and hands it a **30-minute** token via
OIDC at publish time.

### 6a. Configure the trusted publisher on crates.io (one-time)

*crates.io → your crate `reqsmith` → Settings → Trusted Publishing → Add*:

| Field       | Value                                                    |
| ----------- | -------------------------------------------------------- |
| Repository owner | `nabobery`                                          |
| Repository name  | `reqsmith`                                           |
| Workflow filename | `release.yml` (the workflow that runs `cargo publish`) |
| Environment | `release` (matches the release job's `environment:`; optional but recommended) |

### 6b. Enable the `publish-crate` job (already wired)

The `publish-crate` job is **already present** in
[`.github/workflows/release.yml`](../.github/workflows/release.yml). It's gated
on the `release` job (so a tag can't publish without the release itself
succeeding), authenticates via `rust-lang/crates-io-auth-action` (Trusted
Publishing), and runs `cargo publish --locked` on a `v*.*.*` tag.

**It is dormant by default** — it only runs when the repository variable
`PUBLISH_TO_CRATES_IO` is `true`. Until then GitHub *skips* it (skipped, not
failed), so tagging never accidentally auto-publishes before you're ready.

**To turn it on — do this only after §5 (first manual publish) and §6a (Trusted
Publishing configured):**

*GitHub → repo Settings → Secrets and variables → Actions → Variables → New
repository variable:*

| Name                  | Value  |
| --------------------- | ------ |
| `PUBLISH_TO_CRATES_IO` | `true` |

From the next tag onward, the release will publish the crate automatically.
To pause auto-publishing, delete the variable (or set it to anything other than
`true`).

Notes:

- **Idempotent-ish:** crates.io rejects re-publishing an existing
  `name@version`, and the release `guard` already enforces `tag ==
  Cargo.toml version`. Re-running an already-published tag fails cleanly rather
  than doing anything unexpected.
- **Keep it pinned:** all third-party actions are pinned by full commit SHA
  (repo convention). Dependabot proposes bumps.
- **Don't enable the variable before the crate exists** — Trusted Publishing
  settings can only be configured on an existing crate, so the job would fail
  its auth exchange until the first manual publish is done.

---

## 7. After publishing

- [ ] Add badges to the top of `README.md`:
  ```markdown
  [![crates.io](https://img.shields.io/crates/v/reqsmith.svg)](https://crates.io/crates/reqsmith)
  [![docs.rs](https://img.shields.io/docsrs/reqsmith)](https://docs.rs/reqsmith)
  ```
- [ ] Confirm `cargo install reqsmith --locked` works from a clean machine.
- [ ] Confirm the docs.rs build succeeded.

### Yanking a bad release

Yanking does **not** delete the version; it stops new dependency resolution
from selecting it (existing `Cargo.lock`s keep working):

```bash
cargo yank --version x.y.z          # yank
cargo yank --version x.y.z --undo   # reverse
```

Publish a fixed **new** version rather than trying to "replace" a bad one.

---

## Quick reference

```bash
# Every release, from the clean tagged commit:
cargo publish --dry-run --locked     # must be clean
cargo package --list                 # eyeball what ships

# First release only (manual token):
cargo login                          # paste a publish-new scoped token
cargo publish --locked

# All later releases: tag & push; the release.yml publish-crate job (Trusted
# Publishing) does it with no stored secret.
git tag vX.Y.Z && git push origin vX.Y.Z
```

See also [`publishing.md`](publishing.md) for the GitHub *binary* release
pipeline, branch protection, and repo security settings.
