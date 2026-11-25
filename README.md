# Stint

Stint is a macOS menu bar utility, desktop dashboard, and CLI written in Rust that tracks subscription usage limits and local coding session metrics across Claude, Codex, and Gemini.

Stint reads existing local CLI credentials and session logs on your Mac, stores state in a single local SQLite database, and renders the status bar popover and desktop window with `gpui` and `gpui-kit` without embedding a browser engine.

> [!WARNING]
> **Stint is in pre-release development for macOS.** Application crates are being built, and live quota checks rely on internal CLI OAuth endpoints that providers may change without notice.

## Run locally

Requirements: macOS, [rustup](https://rustup.rs) (the pinned toolchain installs on first build), and [just](https://github.com/casey/just).

1. Sign in with the CLIs you use, so Stint has credentials to read:
   - Claude: sign in with Claude Code (`claude`).
   - Codex: sign in with the Codex CLI (`codex`).
2. Start the menu bar app:

   ```sh
   just app
   ```

   The Stint icon appears in the menu bar; there is no Dock icon. Click it to open the usage popover. `⌘R` refreshes and `⌘Q` quits.

macOS may ask whether `security` can read the `Claude Code-credentials` keychain item the first time; allow it so Stint can read the Claude login. Stint only reads credentials and never refreshes or writes them, so an expired login shows a notice until you open the CLI again.

Usage history is stored in `~/Library/Application Support/Stint/stint.db`. To try Stint against a throwaway database, set `STINT_DATA_DIR`:

```sh
STINT_DATA_DIR=/tmp/stint-try just app
```

`CLAUDE_CONFIG_DIR` and `CODEX_HOME` are honored the same way the CLIs honor them.

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
