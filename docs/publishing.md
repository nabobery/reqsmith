# Publishing & maintenance guide

Maintainer-facing notes for taking `reqsmith` public and cutting releases. End users
want [getting-started.md](getting-started.md); contributors want
[../CONTRIBUTING.md](../CONTRIBUTING.md).

## Before making the repository public

- [ ] **Secret scan (history + tree).** CI runs this automatically on every
      push/PR via the `secrets` job (`gitleaks git` over full history +
      `gitleaks dir` over the working tree, version-pinned). To reproduce
      locally: `just secrets`, i.e. `gitleaks git --redact .` and
      `gitleaks dir --redact .`. Both must report _no leaks_. The local `dir`
      scan also walks `target/` (CI checks out fresh, so it has none); run
      `cargo clean` first if it is slow.
- [ ] **Dependency review.** `cargo audit` and `cargo deny --all-features check`
      must pass (`--all-features` so the plugin/extism graph the exceptions
      target is actually evaluated). The
      accepted no-fix exceptions are documented in [`../deny.toml`](../deny.toml)
      and beside the `cargo audit --ignore` flags in
      `.github/workflows/checks.yml`. Cargo Audit additionally suppresses the
      warning-class unmaintained/unsoundness advisories that Cargo Deny does
      not match as deny findings. Every exception except `RUSTSEC-2026-0253`
      (`lru`, reached from the default build via `ratatui`) is confined to the
      optional `plugins` feature. Re-check every exception on each dependency
      bump and drop entries that become fixable. Locally: `just audit` and
      `just deny`.
- [ ] **Feature-flag hygiene.** The default build holds MSRV 1.88. The optional
      `plugins` feature pulls in extism/wasmtime and requires Rust >= 1.91.
      Review every temporary exception in `deny.toml` rather than relying on a
      fixed advisory count; see [plugins-security.md](plugins-security.md).
- [ ] **First CI validation.** Push to a branch and open a PR so the `CI`
      workflow runs on real GitHub runners. The Actions are SHA-pinned; if a pin
      is bad, the first run surfaces it. Confirm fmt/clippy/test/msrv/audit/deny
      are green before flipping the repo to public.

## Branch protection (GitHub → Settings → Branches → `main`)

- [ ] Require a pull request before merging (at least 1 approval).
- [ ] Require status checks to pass (the CI gate lives in the reusable
      `checks.yml` workflow, so on a PR the checks surface under the `checks`
      caller job): `checks / fmt`, `checks / clippy`,
      `checks / test (ubuntu-latest)`, `checks / test (macos-latest)`,
      `checks / test (windows-latest)`, `checks / msrv`,
      `checks / audit`, `checks / deny`, `checks / secrets`. (Pick the exact
      names from a first PR's checks list.)
- [ ] Require branches to be up to date before merging.
- [ ] Enable "Require signed commits" if you sign commits.
- [ ] Restrict force-pushes and deletions on `main`.

## Repository security settings (Settings → Code security)

- [ ] Enable **secret scanning** and **push protection**.
- [ ] Enable **Dependabot alerts** and **security updates** (the update PRs are
      already configured in [`dependabot.yml`](../.github/dependabot.yml)).
- [ ] Enable **private vulnerability reporting** (referenced by
      [`../SECURITY.md`](../SECURITY.md)).

## Cutting a release

1. Update [`../CHANGELOG.md`](../CHANGELOG.md): move `Unreleased` items under a
   new `## [x.y.z] - <date>` heading.
2. Bump `version` in `Cargo.toml`; run `cargo build --locked` so `Cargo.lock`
   updates; commit.
3. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`. The `Release`
   workflow first runs a `guard` job that refuses to proceed unless the run was
   triggered by a tag **and** the tag matches `version` in `Cargo.toml`, plus a
   `verify` job that runs the **same reusable `checks.yml` gate as CI**
   (fmt/clippy/tests/MSRV/audit/deny/secret-scan) on the tagged commit — so a
   tag placed on an unverified commit cannot publish. Build/SBOM depend on both.
   It then builds the five target binaries with `cargo auditable` (embedding the
   dependency list for later `cargo audit bin`), smoke-tests each native
   binary, generates a CycloneDX SBOM, writes `SHA256SUMS` and re-verifies it
   with `sha256sum -c`, attests build provenance
   (`actions/attest-build-provenance`, covering the archives, `SHA256SUMS`, and
   the SBOM), and publishes a GitHub Release with all assets attached. Only
   the final `release` job holds `contents: write` (build/SBOM jobs run
   read-only).
4. Verify the release assets, checksums, and the provenance attestation before
   announcing.

> **Protected release environment (one-time repo setup).** The `release` job
> references an `environment: release`. To gate publishing behind a required
> reviewer or wait timer, create that environment under **Settings →
> Environments** and add protection rules. It works unprotected too — this just
> lets you add a human gate before artifacts go public.

### Prereleases

Tag as `vX.Y.Z-rc.N`. The release workflow matches `v*.*.*`; mark the resulting
GitHub Release as a pre-release in the UI (or adjust the workflow) so it is not
advertised as `latest`. Prefer validating a prerelease before any `1.0.0`.

## crates.io

`reqsmith` is free on the crates.io index and its `Cargo.toml` metadata is
publish-ready. The full step-by-step runbook — first manual publish, then
Trusted Publishing (OIDC, no stored token) for subsequent releases — lives in
[`crates-io-release.md`](crates-io-release.md).

Until the crate is published, `cargo install --git
https://github.com/nabobery/reqsmith` and the GitHub Release binaries are the
install paths.
