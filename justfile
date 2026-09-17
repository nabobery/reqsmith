# reqsmith development commands

# List available recipes
default:
    @just --list

# Build in debug mode
build:
    cargo build --locked

# Build in release mode
release:
    cargo build --release --locked

# Run the application
run *ARGS:
    cargo run --locked -- {{ARGS}}

# Run all tests (same two invocations as the `test` CI job)
test:
    cargo test --locked
    cargo test --locked --all-features

# Format code
fmt:
    cargo fmt

# Check formatting without modifying
fmt-check:
    cargo fmt --all -- --check

# Run clippy lints (same two invocations as the `clippy` CI job)
lint:
    cargo clippy --all-targets --locked -- -D warnings
    cargo clippy --all-targets --all-features --locked -- -D warnings

# Auto-fix clippy and format issues
fix:
    cargo clippy --fix --allow-dirty --allow-staged
    cargo fmt

# Advisory scan (requires cargo-audit). Ignore chain mirrors .github/workflows/checks.yml.
audit:
    cargo audit \
      --ignore RUSTSEC-2026-0222 \
      --ignore RUSTSEC-2026-0269 \
      --ignore RUSTSEC-2026-0247 \
      --ignore RUSTSEC-2026-0250 \
      --ignore RUSTSEC-2026-0251 \
      --ignore RUSTSEC-2026-0255 \
      --ignore RUSTSEC-2026-0253

# License/advisory/source policy (requires cargo-deny)
deny:
    cargo deny --all-features check

# Secret scan, identical to the `secrets` CI job (requires gitleaks 8.30.x).
# The dir scan also walks target/ locally; `cargo clean` first if it is slow.
secrets:
    gitleaks git --redact --no-banner .
    gitleaks dir --redact --no-banner .

# Fast pre-push gate (fmt + lint + test) — no extra tools needed
ci: fmt-check lint test

# Everything CI runs (adds audit + deny + secrets; needs cargo-audit, cargo-deny, gitleaks)
ci-full: ci audit deny secrets

# Remove build artifacts
clean:
    cargo clean

# Generate and open documentation
doc:
    cargo doc --no-deps --open

# Watch for changes and rebuild (requires cargo-watch)
watch:
    cargo watch -x 'clippy --all-targets -- -D warnings' -x test
