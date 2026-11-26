//! Adds an account by running the provider's own CLI sign-in in a throwaway directory.
//!
//! Each attempt gets a fresh config home, so the sign-in never touches the CLI's normal
//! login. Once the CLI exits, [`SignIn::finish`] reads the new login out, and dropping
//! the attempt removes the directory and any Keychain item the CLI made for it. The CLI
//! is never told to log out: that would revoke the login Stint just copied.

mod watch;

use std::fs;
use std::hash::{BuildHasher, RandomState};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use stint_core::ProviderId;

use crate::claude;
use crate::codex;
use crate::credential::Credential;
use crate::error::ProviderError;
use crate::keychain::Item;

pub use watch::{SignInEvent, Watched};

const SIGNIN_DIR: &str = "signin";
// Variables that would point the CLI at another login or credential store.
const CLAUDE_UNSET: [&str; 5] = [
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
    "CLAUDE_CODE_FORCE_WINDOWS_CREDMAN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "ANTHROPIC_API_KEY",
];
const CODEX_UNSET: [&str; 2] = ["OPENAI_API_KEY", "CODEX_API_KEY"];
// Where the CLIs' installers put them, for an app started without a shell's `PATH`.
const INSTALL_DIRS: [&str; 3] = [".local/bin", "/opt/homebrew/bin", "/usr/local/bin"];

/// A login read out of a finished sign-in.
#[derive(Debug)]
pub struct Enrolled {
    pub credential: Credential,
    pub email: Option<String>,
    /// The subscription, such as "Max 5x", when the login says.
    pub plan: Option<String>,
}

/// A provider whose own CLI signs accounts in. Antigravity accounts sign in through
/// the browser instead; see [`crate::antigravity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliProvider {
    Claude,
    Codex,
}

impl CliProvider {
    /// The CLI that signs `provider`'s accounts in, if it has one.
    pub fn of(provider: ProviderId) -> Option<Self> {
        match provider {
            ProviderId::Claude => Some(Self::Claude),
            ProviderId::Codex => Some(Self::Codex),
            ProviderId::Antigravity => None,
        }
    }
}

/// One sign-in attempt.
#[derive(Debug)]
pub struct SignIn {
    provider: CliProvider,
    root: PathBuf,
    // The config home handed to the CLI, spelled exactly as passed.
    home: String,
    // Claude Code's Keychain account name for the attempt.
    user: String,
}

impl SignIn {
    /// Prepares a fresh attempt under `<data_dir>/signin/`.
    pub fn start(provider: CliProvider, data_dir: &Path) -> Result<Self, ProviderError> {
        let root = data_dir.join(SIGNIN_DIR).join(random_name());
        let home = root.join("home");
        for dir in [&home, &root.join("work")] {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(ProviderError::io(dir))?;
        }
        let home = home.into_os_string().into_string().map_err(|path| {
            ProviderError::Unsupported(format!("non-UTF-8 path {}", Path::new(&path).display()))
        })?;
        let user = claude::keychain_user(std::env::var("USER").ok().as_deref());
        Ok(Self {
            provider,
            root,
            home,
            user,
        })
    }

    /// The CLI sign-in to run. The caller wires its input and output: the CLI opens the
    /// browser, prints a link, and Claude Code also reads a pasted code from stdin.
    pub fn command(&self) -> Result<Command, ProviderError> {
        let mut command = match self.provider {
            CliProvider::Claude => {
                let mut command = Command::new(find_program("claude")?);
                command
                    .args(["auth", "login", "--claudeai"])
                    .env("CLAUDE_CONFIG_DIR", &self.home)
                    // Claude Code files the Keychain item under this name.
                    .env("USER", &self.user);
                for name in CLAUDE_UNSET {
                    command.env_remove(name);
                }
                command
            }
            CliProvider::Codex => {
                let mut command = Command::new(find_program("codex")?);
                command
                    .args(["-c", r#"cli_auth_credentials_store="file""#, "login"])
                    .env("CODEX_HOME", &self.home);
                for name in CODEX_UNSET {
                    command.env_remove(name);
                }
                command
            }
        };
        command.current_dir(self.root.join("work"));
        Ok(command)
    }

    /// Reads the login the CLI saved. Call it after the CLI exited successfully.
    pub fn finish(&self) -> Result<Enrolled, ProviderError> {
        let home = Path::new(&self.home);
        match self.provider {
            CliProvider::Claude => claude::import(home, &self.claude_slot()),
            CliProvider::Codex => codex::import(home),
        }
    }

    fn claude_slot(&self) -> claude::Slot {
        claude::Slot::for_dir(&self.home, &self.user)
    }
}

impl Drop for SignIn {
    fn drop(&mut self) {
        if self.provider == CliProvider::Claude {
            let slot = self.claude_slot();
            let [overflow_a, overflow_b] = slot.overflow_services();
            for service in [&slot.service, &overflow_a, &overflow_b] {
                let item = Item {
                    service,
                    account: &slot.account,
                    keychain: None,
                };
                if let Err(err) = item.delete() {
                    eprintln!("stint: remove sign-in keychain item {service:?}: {err}");
                }
            }
        }
        if let Err(err) = fs::remove_dir_all(&self.root) {
            eprintln!("stint: remove {}: {err}", self.root.display());
        }
    }
}

/// Generates the Keychain key for a new account's login.
pub fn new_credential_key(provider: ProviderId) -> String {
    format!("{provider}-{}", random_name())
}

fn random_name() -> String {
    let random = RandomState::new().hash_one(jiff::Timestamp::now());
    format!("{random:016x}")
}

fn find_program(name: &'static str) -> Result<PathBuf, ProviderError> {
    let home = std::env::home_dir();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let fallbacks = INSTALL_DIRS.iter().filter_map(|dir| {
        let dir = Path::new(dir);
        if dir.is_absolute() {
            Some(dir.to_path_buf())
        } else {
            home.as_ref().map(|home| home.join(dir))
        }
    });
    std::env::split_paths(&path)
        .chain(fallbacks)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
        .ok_or(ProviderError::CliMissing(name))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn sets(command: &Command, name: &str) -> Option<OsString> {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value.map(OsStr::to_owned))
    }

    fn removes(command: &Command, name: &str) -> bool {
        command
            .get_envs()
            .any(|(key, value)| key == name && value.is_none())
    }

    #[test]
    fn attempts_get_private_fresh_directories_that_drop_removes() {
        let data = tempfile::TempDir::new().unwrap();
        let first = SignIn::start(CliProvider::Codex, data.path()).unwrap();
        let second = SignIn::start(CliProvider::Codex, data.path()).unwrap();
        assert_ne!(first.root, second.root);

        let mode = fs::metadata(&first.home).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);

        let root = first.root.clone();
        drop(first);
        assert!(!root.exists());
    }

    #[test]
    fn codex_missing_login_is_auth_missing() {
        let data = tempfile::TempDir::new().unwrap();
        let attempt = SignIn::start(CliProvider::Codex, data.path()).unwrap();
        assert!(matches!(attempt.finish(), Err(ProviderError::AuthMissing)));
    }

    #[test]
    fn only_claude_and_codex_sign_in_through_a_cli() {
        assert_eq!(
            CliProvider::of(ProviderId::Claude),
            Some(CliProvider::Claude)
        );
        assert_eq!(CliProvider::of(ProviderId::Codex), Some(CliProvider::Codex));
        assert_eq!(CliProvider::of(ProviderId::Antigravity), None);
    }

    #[test]
    fn credential_keys_are_unique_per_call() {
        let a = new_credential_key(ProviderId::Claude);
        let b = new_credential_key(ProviderId::Claude);
        assert!(a.starts_with("claude-"));
        assert_ne!(a, b);
    }

    #[test]
    fn missing_program_is_reported() {
        assert!(matches!(
            find_program("stint-no-such-cli"),
            Err(ProviderError::CliMissing("stint-no-such-cli"))
        ));
    }

    #[test]
    fn commands_isolate_the_config_home() {
        // Only meaningful where the CLIs are installed.
        let data = tempfile::TempDir::new().unwrap();
        for provider in [CliProvider::Claude, CliProvider::Codex] {
            let attempt = SignIn::start(provider, data.path()).unwrap();
            let Ok(command) = attempt.command() else {
                continue;
            };
            let home = OsString::from(&attempt.home);
            let (var, removed) = match provider {
                CliProvider::Claude => ("CLAUDE_CONFIG_DIR", "CLAUDE_CODE_OAUTH_TOKEN"),
                CliProvider::Codex => ("CODEX_HOME", "OPENAI_API_KEY"),
            };
            assert_eq!(sets(&command, var), Some(home));
            assert!(removes(&command, removed));
        }
    }
}
