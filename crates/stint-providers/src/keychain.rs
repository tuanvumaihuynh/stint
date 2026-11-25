use std::process::{Command, Output};

use crate::error::ProviderError;

const SECURITY: &str = "/usr/bin/security";

// `security` exits with the low byte of the Security framework's `OSStatus`.
const ITEM_NOT_FOUND: i32 = 44; // errSecItemNotFound
const INTERACTION_NOT_ALLOWED: i32 = 36; // errSecInteractionNotAllowed: keychain locked

/// Reads a generic password from the login keychain, or `None` when there is no such item.
///
/// This shells out to `/usr/bin/security` on purpose. CLI keychain items trust that binary,
/// so it reads them without a permission dialog, while an in-process read through the
/// Security framework prompts and leaves every later `security` read of the item slow.
/// The call blocks for a few milliseconds; run it off any UI thread.
pub(crate) fn find_generic_password(service: &str) -> Result<Option<String>, ProviderError> {
    let output = Command::new(SECURITY)
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
        .map_err(|err| ProviderError::Keychain(err.to_string()))?;
    interpret(&output)
}

fn interpret(output: &Output) -> Result<Option<String>, ProviderError> {
    match output.status.code() {
        Some(0) => {
            let secret = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            Ok((!secret.is_empty()).then_some(secret))
        }
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
    fn found_item_returns_trimmed_secret() {
        let secret = interpret(&output(0, "{\"token\":\"fake\"}\n")).unwrap();
        assert_eq!(secret.as_deref(), Some("{\"token\":\"fake\"}"));
    }

    #[test]
    fn empty_item_counts_as_missing() {
        assert_eq!(interpret(&output(0, "  \n")).unwrap(), None);
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
}
