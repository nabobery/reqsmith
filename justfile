# hurl development commands

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

# Run all tests
test:
    cargo test --locked

# Format code
fmt:
    cargo fmt

# Check formatting without modifying
fmt-check:
    cargo fmt -- --check

# Run clippy lints
lint:
    cargo clippy --all-targets --all-features --locked -- -D warnings

# Auto-fix clippy and format issues
fix:
    cargo clippy --fix --allow-dirty --allow-staged
    cargo fmt

# Run full CI checks (fmt + lint + test)
ci: fmt-check lint test

# Remove build artifacts
clean:
    cargo clean

# Generate and open documentation
doc:
    cargo doc --no-deps --open

# Watch for changes and rebuild (requires cargo-watch)
watch:
    cargo watch -x 'clippy --all-targets -- -D warnings' -x test
