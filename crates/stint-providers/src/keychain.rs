use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::error::ProviderError;

const SECURITY: &str = "/usr/bin/security";

// `security` exits with the low byte of the Security framework's `OSStatus`.
const ITEM_NOT_FOUND: i32 = 44; // errSecItemNotFound
const INTERACTION_NOT_ALLOWED: i32 = 36; // errSecInteractionNotAllowed: keychain locked

/// A generic password in a keychain.
///
/// Every call shells out to `/usr/bin/security` on purpose. Items it creates trust that
/// binary, and so do the CLIs' items, so they are read without a permission dialog. An
/// in-process read through the Security framework prompts, and leaves every later
/// `security` read of the item slow. Calls block for a few milliseconds; run them off any
/// UI thread.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Item<'a> {
    pub(crate) service: &'a str,
    pub(crate) account: &'a str,
    /// A keychain file, or `None` for the login keychain.
    pub(crate) keychain: Option<&'a Path>,
}

impl Item<'_> {
    fn args(&self, command: &str) -> Command {
        let mut security = Command::new(SECURITY);
        security.args([command, "-s", self.service, "-a", self.account]);
        security
    }

    fn with_keychain(&self, mut security: Command) -> Command {
        if let Some(keychain) = self.keychain {
            security.arg(keychain);
        }
        security
    }

    /// The secret, or `None` when there is no such item.
    pub(crate) fn find(&self) -> Result<Option<String>, ProviderError> {
        let mut security = self.args("find-generic-password");
        security.arg("-w");
        let output = run(self.with_keychain(security))?;
        match interpret(&output)? {
            Some(stdout) => {
                let secret = stdout.trim().to_owned();
                Ok((!secret.is_empty()).then_some(secret))
            }
            None => Ok(None),
        }
    }

    /// Creates the item or replaces its secret.
    ///
    /// The secret goes to `security` on stdin, never on the command line where other
    /// processes could read it.
    pub(crate) fn set(&self, secret: &str) -> Result<(), ProviderError> {
        // The interactive parser splits on spaces and honors double quotes only.
        let unquotable = |s: &str| s.contains(['"', '\\', '\n']);
        if unquotable(self.service) || unquotable(self.account) {
            return Err(ProviderError::Keychain(format!(
                "unsupported item name {:?} / {:?}",
                self.service, self.account
            )));
        }
        let mut line = format!(
            "add-generic-password -U -s \"{}\" -a \"{}\" -X {}",
            self.service,
            self.account,
            hex(secret.as_bytes())
        );
        if let Some(keychain) = self.keychain {
            let path = keychain.to_string_lossy();
            if unquotable(&path) {
                return Err(ProviderError::Keychain(format!(
                    "unsupported keychain path {path:?}"
                )));
            }
            write!(line, " \"{path}\"").ok();
        }
        line.push('\n');

        let mut child = Command::new(SECURITY)
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| ProviderError::Keychain(err.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(line.as_bytes())
                .map_err(|err| ProviderError::Keychain(err.to_string()))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|err| ProviderError::Keychain(err.to_string()))?;
        interpret(&output)?;

        // Interactive mode can exit 0 after a failed command, so read the secret back.
        if self.find()?.as_deref() != Some(secret.trim()) {
            return Err(ProviderError::Keychain(format!(
                "saving {:?} did not take effect",
                self.service
            )));
        }
        Ok(())
    }

    /// Deletes the item. Returns whether it existed.
    pub(crate) fn delete(&self) -> Result<bool, ProviderError> {
        let output = run(self.with_keychain(self.args("delete-generic-password")))?;
        Ok(interpret(&output)?.is_some())
    }
}

fn run(mut security: Command) -> Result<Output, ProviderError> {
    security
        .output()
        .map_err(|err| ProviderError::Keychain(err.to_string()))
}

/// The output on success, or `None` when the item does not exist.
fn interpret(output: &Output) -> Result<Option<String>, ProviderError> {
    match output.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned())),
        Some(ITEM_NOT_FOUND) => Ok(None),
        Some(INTERACTION_NOT_ALLOWED) => Err(ProviderError::KeychainLocked),
        code => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(ProviderError::Keychain(format!(
                "security exited with {code:?}: {stderr}"
            )))
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        write!(out, "{byte:02x}").ok();
        out
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    use super::*;

    fn output(code: i32, stdout: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: b"details".to_vec(),
        }
    }

    #[test]
    fn success_returns_stdout() {
        let stdout = interpret(&output(0, "{\"token\":\"fake\"}\n")).unwrap();
        assert_eq!(stdout.as_deref(), Some("{\"token\":\"fake\"}\n"));
    }

    #[test]
    fn missing_item_is_none() {
        assert_eq!(interpret(&output(ITEM_NOT_FOUND, "")).unwrap(), None);
    }

    #[test]
    fn locked_keychain_is_reported() {
        assert!(matches!(
            interpret(&output(INTERACTION_NOT_ALLOWED, "")),
            Err(ProviderError::KeychainLocked)
        ));
    }

    #[test]
    fn other_exit_codes_carry_stderr() {
        let err = interpret(&output(1, "")).unwrap_err();
        assert!(matches!(err, ProviderError::Keychain(message) if message.contains("details")));
    }

    #[test]
    fn hex_encodes_every_byte() {
        assert_eq!(hex(b"{\"a\": 1}\n"), "7b2261223a20317d0a");
    }

    /// Exercises the real `security` binary against a throwaway keychain file.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "creates a scratch keychain; run with `cargo test -- --ignored`"]
    fn round_trips_through_a_scratch_keychain() {
        let dir = tempfile::TempDir::new().unwrap();
        let keychain = dir.path().join("test.keychain-db");
        let status = Command::new(SECURITY)
            .args(["create-keychain", "-p", "test"])
            .arg(&keychain)
            .status()
            .unwrap();
        assert!(status.success());

        let item = Item {
            service: "Stint test",
            account: "claude-1",
            keychain: Some(&keychain),
        };
        assert_eq!(item.find().unwrap(), None);
        item.set(r#"{"token": "fake one"}"#).unwrap();
        assert_eq!(
            item.find().unwrap().as_deref(),
            Some(r#"{"token": "fake one"}"#)
        );
        item.set("replaced").unwrap();
        assert_eq!(item.find().unwrap().as_deref(), Some("replaced"));
        assert!(item.delete().unwrap());
        assert!(!item.delete().unwrap());

        Command::new(SECURITY)
            .arg("delete-keychain")
            .arg(&keychain)
            .status()
            .unwrap();
    }
}
