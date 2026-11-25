default:
    @just --list

# Format all Rust crates
fmt:
    cargo fmt --all

# Check Rust formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Run clippy across all workspace targets
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Run offline unit tests across the workspace
test:
    cargo test --workspace --locked

# Run all pre-PR verification checks
check: fmt-check lint test

# Run the stint CLI binary
cli *args:
    cargo run -p stint-cli -- {{args}}

# Run the stint macOS desktop and status bar app
app:
    cargo run -p stint-app
