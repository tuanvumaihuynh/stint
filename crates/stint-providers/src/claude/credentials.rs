//! Reads the login `claude auth login` leaves in a private config directory.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::credential::{ClaudeLogin, Credential};
use crate::error::ProviderError;
use crate::keychain::Item;
use crate::plan;
use crate::signin::Enrolled;

const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
// Claude Code's account name when `USER` is unusable.
const FALLBACK_USER: &str = "claude-code-user";
const CREDENTIALS_FILE: &str = ".credentials.json";
const CONFIG_FILE: &str = ".claude.json";

/// The Keychain item Claude Code keeps a config directory's login in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    pub(crate) service: String,
    pub(crate) account: String,
}

impl Slot {
    /// Claude Code names the item after the first 8 hex digits of the SHA-256 of the
    /// NFC-normalized `CLAUDE_CONFIG_DIR` string, unresolved, and the user name.
    pub(crate) fn for_dir(config_dir: &str, user: &str) -> Self {
        let normalized: String = config_dir.nfc().collect();
        let digest = Sha256::digest(normalized.as_bytes());
        let hash = digest.iter().take(4).fold(String::new(), |mut hash, byte| {
            write!(hash, "{byte:02x}").ok();
            hash
        });
        Self {
            service: format!("{KEYCHAIN_SERVICE}-{hash}"),
            account: user.to_owned(),
        }
    }

    /// Claude Code splits a large login across companion items.
    pub(crate) fn overflow_services(&self) -> [String; 2] {
        ["a", "b"].map(|part| format!("{}-overflow-{part}", self.service))
    }
}

/// The user name Claude Code files Keychain items under: `USER` when it is a plain name.
pub(crate) fn keychain_user(user: Option<&str>) -> String {
    let plain = |name: &&str| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    user.filter(plain).unwrap_or(FALLBACK_USER).to_owned()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    claude_ai_oauth: Option<OAuth>,
    keychain_overflow: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuth {
    access_token: Option<String>,
    refresh_token: Option<String>,
    // Unix milliseconds.
    expires_at: Option<i64>,
    #[serde(default)]
    scopes: Vec<String>,
    subscription_type: Option<String>,
    rate_limit_tier: Option<String>,
    client_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    oauth_account: Option<OAuthAccount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthAccount {
    email_address: Option<String>,
}

/// Reads the new login: the directory's Keychain item first, then the plaintext file
/// Claude Code falls back to.
pub(crate) fn import(config_dir: &Path, slot: &Slot) -> Result<Enrolled, ProviderError> {
    let item = Item {
        service: &slot.service,
        account: &slot.account,
        keychain: None,
    };
    let document = if let Some(secret) = item.find()? {
        secret.into_bytes()
    } else {
        let path = config_dir.join(CREDENTIALS_FILE);
        match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(ProviderError::AuthMissing);
            }
            Err(err) => return Err(ProviderError::io(path)(err)),
        }
    };
    let mut enrolled = parse(&document)?;
    enrolled.email = email(config_dir);
    Ok(enrolled)
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Enrolled, ProviderError> {
    let document: Document = serde_json::from_slice(bytes)?;
    if document.keychain_overflow.is_some() {
        return Err(ProviderError::Unsupported(
            "Claude Code split this login across several Keychain items".into(),
        ));
    }
    let oauth = document.claude_ai_oauth.ok_or(ProviderError::AuthMissing)?;
    let access_token = non_empty(oauth.access_token).ok_or(ProviderError::AuthMissing)?;
    let refresh_token = non_empty(oauth.refresh_token).ok_or(ProviderError::AuthMissing)?;

    let plan = oauth
        .subscription_type
        .as_deref()
        .map(|kind| plan::claude(kind, oauth.rate_limit_tier.as_deref()));
    Ok(Enrolled {
        credential: Credential::Claude(ClaudeLogin {
            access_token,
            refresh_token,
            expires_at_ms: oauth.expires_at,
            scopes: oauth.scopes,
            client_id: non_empty(oauth.client_id),
        }),
        email: None,
        plan,
    })
}

/// The signed-in email Claude Code records in the directory's config, if any.
fn email(config_dir: &Path) -> Option<String> {
    let bytes = fs::read(config_dir.join(CONFIG_FILE)).ok()?;
    let config: Config = serde_json::from_slice(&bytes).ok()?;
    non_empty(config.oauth_account?.email_address)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDENTIALS: &str = include_str!("testdata/credentials.json");

    fn login(enrolled: &Enrolled) -> &ClaudeLogin {
        match &enrolled.credential {
            Credential::Claude(login) => login,
            _ => panic!("not a Claude login"),
        }
    }

    #[test]
    fn slot_hashes_the_unresolved_directory() {
        // SHA-256("/tmp/stint/home") starts with ea86a8bb.
        let slot = Slot::for_dir("/tmp/stint/home", "me");
        assert_eq!(slot.service, "Claude Code-credentials-ea86a8bb");
        assert_eq!(slot.account, "me");
        assert_eq!(
            slot.overflow_services(),
            [
                "Claude Code-credentials-ea86a8bb-overflow-a".to_owned(),
                "Claude Code-credentials-ea86a8bb-overflow-b".to_owned(),
            ]
        );
    }

    #[test]
    fn slot_normalizes_to_nfc() {
        // "é" composed and decomposed name the same item.
        assert_eq!(
            Slot::for_dir("/Users/caf\u{e9}", "u"),
            Slot::for_dir("/Users/cafe\u{301}", "u")
        );
    }

    #[test]
    fn keychain_user_falls_back_for_unusual_names() {
        assert_eq!(keychain_user(Some("me.work-1")), "me.work-1");
        for user in [None, Some(""), Some("vu x"), Some("vũ")] {
            assert_eq!(keychain_user(user), FALLBACK_USER);
        }
    }

    #[test]
    fn parses_tokens_scopes_and_plan() {
        let enrolled = parse(CREDENTIALS.as_bytes()).unwrap();
        let login = login(&enrolled);
        assert_eq!(login.access_token, "sk-ant-oat01-FAKE");
        assert_eq!(login.refresh_token, "sk-ant-ort01-FAKE");
        assert_eq!(login.expires_at_ms, Some(1_790_000_000_000));
        assert_eq!(login.scopes, ["user:inference", "user:profile"]);
        assert_eq!(enrolled.plan.as_deref(), Some("Max 5x"));
    }

    #[test]
    fn incomplete_login_is_auth_missing() {
        for body in [
            r"{}",
            r#"{"claudeAiOauth":{}}"#,
            r#"{"claudeAiOauth":{"accessToken":"a"}}"#,
            r#"{"claudeAiOauth":{"accessToken":"","refreshToken":"r"}}"#,
        ] {
            assert!(matches!(
                parse(body.as_bytes()),
                Err(ProviderError::AuthMissing)
            ));
        }
    }

    #[test]
    fn split_login_is_unsupported() {
        let body =
            r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r"},"keychainOverflow":{}}"#;
        assert!(matches!(
            parse(body.as_bytes()),
            Err(ProviderError::Unsupported(_))
        ));
    }

    #[test]
    fn reads_email_from_the_config() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE),
            r#"{"oauthAccount":{"emailAddress":"me@example.com"}}"#,
        )
        .unwrap();
        assert_eq!(email(dir.path()).as_deref(), Some("me@example.com"));
        assert_eq!(email(&dir.path().join("missing")), None);
    }

    #[test]
    fn malformed_json_is_decode_error() {
        assert!(matches!(parse(b"not json"), Err(ProviderError::Decode(_))));
    }
}
