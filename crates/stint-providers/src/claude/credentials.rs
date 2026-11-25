use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::Deserialize;

use crate::error::ProviderError;
use crate::keychain;

const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const CREDENTIALS_FILE: &str = ".credentials.json";

/// The OAuth login Claude Code stores, reduced to what a usage read needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Credentials {
    pub(crate) access_token: String,
    pub(crate) expires_at: Option<Timestamp>,
}

impl Credentials {
    pub(crate) fn is_expired(&self, now: Timestamp) -> bool {
        self.expires_at.is_some_and(|expires_at| expires_at <= now)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    claude_ai_oauth: Option<OAuth>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuth {
    access_token: Option<String>,
    // Unix milliseconds.
    expires_at: Option<i64>,
}

/// Where Claude Code keeps its config, honoring `CLAUDE_CONFIG_DIR`.
pub(crate) fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::home_dir().map(|home| home.join(".claude"))
}

/// Loads the signed-in login: the keychain item first, then the plaintext file Claude Code
/// falls back to.
///
/// A custom `CLAUDE_CONFIG_DIR` makes Claude Code name its keychain item after a hash of
/// that directory; only the default item is read, so such setups use the file.
pub(crate) fn load(config_dir: &Path) -> Result<Credentials, ProviderError> {
    let uses_default_dir = std::env::var_os("CLAUDE_CONFIG_DIR").is_none_or(|dir| dir.is_empty());
    if uses_default_dir && let Some(secret) = keychain::find_generic_password(KEYCHAIN_SERVICE)? {
        return parse(secret.as_bytes());
    }

    let path = config_dir.join(CREDENTIALS_FILE);
    match fs::read(&path) {
        Ok(bytes) => parse(&bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(ProviderError::AuthMissing),
        Err(source) => Err(ProviderError::Io { path, source }),
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Credentials, ProviderError> {
    let document: Document = serde_json::from_slice(bytes)?;
    let Some(oauth) = document.claude_ai_oauth else {
        return Err(ProviderError::AuthMissing);
    };
    let Some(access_token) = oauth.access_token.filter(|token| !token.is_empty()) else {
        return Err(ProviderError::AuthMissing);
    };

    let expires_at = oauth
        .expires_at
        .and_then(|millis| Timestamp::from_millisecond(millis).ok());

    Ok(Credentials {
        access_token,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDENTIALS: &str = include_str!("testdata/credentials.json");

    #[test]
    fn parses_access_token_and_expiry() {
        let credentials = parse(CREDENTIALS.as_bytes()).unwrap();
        assert_eq!(credentials.access_token, "sk-ant-oat01-FAKE");
        assert_eq!(
            credentials.expires_at,
            Some(Timestamp::from_millisecond(1_790_000_000_000).unwrap())
        );
    }

    #[test]
    fn expiry_is_inclusive() {
        let credentials = parse(CREDENTIALS.as_bytes()).unwrap();
        let expires_at = credentials.expires_at.unwrap();
        assert!(credentials.is_expired(expires_at));
        assert!(!credentials.is_expired(expires_at - jiff::SignedDuration::from_secs(1)));
    }

    #[test]
    fn missing_expiry_never_expires() {
        let credentials = parse(br#"{"claudeAiOauth":{"accessToken":"t"}}"#).unwrap();
        assert!(!credentials.is_expired(Timestamp::MAX));
    }

    #[test]
    fn missing_login_is_auth_missing() {
        for body in [
            r"{}",
            r#"{"claudeAiOauth":{}}"#,
            r#"{"claudeAiOauth":{"accessToken":""}}"#,
        ] {
            assert!(matches!(
                parse(body.as_bytes()),
                Err(ProviderError::AuthMissing)
            ));
        }
    }

    #[test]
    fn malformed_json_is_decode_error() {
        assert!(matches!(parse(b"not json"), Err(ProviderError::Decode(_))));
    }
}
