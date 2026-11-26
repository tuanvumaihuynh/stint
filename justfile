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

# Build the Rust core for the macOS app and write its Swift bindings (macOS only)
ffi profile="debug":
    macos/scripts/build-ffi.sh {{profile}}

# Build the macOS app with Xcode; pass `Release` for an optimized build (macOS only)
app-build config="Debug":
    just ffi {{ if config == "Release" { "release" } else { "debug" } }}
    xcodebuild -quiet -project macos/Stint.xcodeproj -scheme Stint -configuration {{config}} -destination "platform=macOS,arch=$(uname -m)" -derivedDataPath macos/build build

# Run the macOS app's unit tests (macOS only)
app-test:
    just ffi
    xcodebuild -quiet -project macos/Stint.xcodeproj -scheme Stint -configuration Debug -destination "platform=macOS,arch=$(uname -m)" -derivedDataPath macos/build test

# Photograph the status menu in every mock scenario, light and dark (macOS only)
app-snapshot dir="target/snapshots": app-build
    macos/build/Build/Products/Debug/Stint.app/Contents/MacOS/Stint --snapshot "$(cd {{justfile_directory()}} && mkdir -p {{dir}} && cd {{dir}} && pwd)"
    macos/build/Build/Products/Debug/Stint.app/Contents/MacOS/Stint --window-snapshot "$(cd {{justfile_directory()}} && mkdir -p {{dir}} && cd {{dir}} && pwd)"

# Audit dependencies against the RustSec advisory database
vuln:
    cargo audit

# Run all pre-PR verification checks
check: lint test

# Add a real account by signing in, e.g. `just add-account claude Personal` or `just add-account antigravity Personal`
add-account provider name:
    cargo run -p stint-dev -- add-account {{provider}} "{{name}}"

# Remove an account and its saved login
remove-account provider name:
    cargo run -p stint-dev -- remove-account {{provider}} "{{name}}"

# Print an account's live quota windows, e.g. `just usage antigravity Personal`
usage provider name:
    cargo run -p stint-dev -- usage {{provider}} "{{name}}"

# Run the macOS app; pass a scenario to use mock accounts, e.g. `just app default` (macOS only)
app mock="": app-build
    STINT_MOCK={{mock}} macos/build/Build/Products/Debug/Stint.app/Contents/MacOS/Stint
