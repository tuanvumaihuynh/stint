default:
    @just --list

# Format all Rust crates
fmt:
    cargo fmt --all

# Check Rust formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Check formatting and run clippy across all workspace targets
lint: fmt-check
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Run offline unit tests across the workspace
test:
    cargo test --workspace --locked

# Lint and test the macOS menu bar app (macOS only)
app-check:
    cargo clippy -p stint-app --all-targets --locked -- -D warnings
    cargo test -p stint-app --locked

# Audit dependencies against the RustSec advisory database
vuln:
    cargo audit

# Run all pre-PR verification checks
check: lint test

# Run the stint CLI binary
cli *args:
    cargo run -p stint-cli -- {{args}}

# Run the stint macOS desktop and status bar app
app:
    cargo run -p stint-app
