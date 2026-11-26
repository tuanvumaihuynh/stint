# Stint

Stint is a macOS menu bar utility that tracks subscription usage limits and local coding session metrics across Claude, Codex, and Gemini.

Stint signs accounts in through the providers' own CLIs, keeps each login in the macOS Keychain, stores usage history in a single local SQLite database, and shows it in a native Swift app built on a Rust core.

> [!WARNING]
> **Stint is in pre-release development for macOS.** Application crates are being built, and live quota checks rely on internal CLI OAuth endpoints that providers may change without notice.

## Run locally

Requirements: macOS, Xcode, [rustup](https://rustup.rs) (the pinned toolchain installs on first build), and [just](https://github.com/casey/just).

Start the menu bar app:

```sh
just app
```

The Stint icon appears in the menu bar; there is no Dock icon. Click it to open the menu. The app refreshes every account when it starts and every five minutes.

Until the desktop window exists, accounts are added from the terminal. Stint runs the provider's own sign-in (`claude auth login` or `codex login`) in a private directory, so your normal CLI login is untouched, then saves the login to your Keychain and renews it as it expires:

```sh
just add-account claude Personal
just add-account codex Work
just remove-account claude Personal
```

To see the menu with made-up accounts, pass a mock scenario: `default`, `refreshing`, `single`, `busy`, `offline`, or `empty`.

```sh
just app default
```

Usage history is stored in `~/Library/Application Support/Stint/stint.db`. To try Stint against a throwaway database, set `STINT_DATA_DIR`:

```sh
STINT_DATA_DIR=/tmp/stint-try just app
```

## Development

List available tasks with `just --list`, and run formatting, clippy, and unit tests before opening a pull request:

```sh
just check
```

On macOS, `just app-test` runs the app's unit tests, and `just app-snapshot` photographs the menu in every mock scenario, in light and dark mode, into `target/snapshots/`.

The macOS app lives in `macos/`, an Xcode project. Its Swift code calls the Rust core through `crates/stint-ffi`, whose static library and generated Swift bindings `just ffi` writes to `macos/Generated/`. Run `just ffi` once before building in Xcode.

Workspace crates live under `crates/`:

- `crates/stint-core`: Domain types, SQLite WAL storage, and pace calculations.
- `crates/stint-providers`: Account sign-in through the provider CLIs, Keychain logins with renewal, and subscription quota clients for Claude and Codex.
- `crates/stint-ffi`: The core as the macOS app sees it, exported to Swift with UniFFI.
- `crates/stint-dev`: The `add-account` and `remove-account` development commands.
