# Stint

Stint is a macOS menu bar utility, desktop dashboard, and CLI written in Rust that tracks subscription usage limits and local coding session metrics across Claude, Codex, and Gemini.

Stint reads existing local CLI credentials and session logs on your Mac, stores state in a single local SQLite database, and renders the status bar popover and desktop window with `gpui` and `gpui-kit` without embedding a browser engine.

> [!WARNING]
> **Stint is in pre-release development for macOS.** Application crates are being built, and live quota checks rely on internal CLI OAuth endpoints that providers may change without notice.

## Development

List available tasks with `just --list`, and run formatting, clippy, and unit tests before opening a pull request:

```sh
just check
```

Workspace crates live under `crates/`:

- `crates/stint-core`: Domain types, provider traits, SQLite WAL storage, and pace/cost calculators.
- `crates/stint-providers`: Subscription quota clients and incremental JSONL log readers for Claude, Codex, and Gemini.
- `crates/stint-cli`: Terminal binary (`stint`) reading cached local state without UI dependencies.
- `crates/stint-app`: macOS accessory application with a status bar popover and desktop metrics window.
