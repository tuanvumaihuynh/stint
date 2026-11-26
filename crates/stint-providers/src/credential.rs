use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::keychain::Item;

/// The Keychain service every Stint account login is saved under, one item per account.
const SERVICE: &str = "Stint";
// Bumped when the saved layout changes incompatibly.
const VERSION: u32 = 1;

/// An account's login, as Stint saves it after sign-in and renews it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum Credential {
    Claude(ClaudeLogin),
    Codex(CodexLogin),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeLogin {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    /// When the access token expires, in Unix milliseconds.
    pub(crate) expires_at_ms: Option<i64>,
    pub(crate) scopes: Vec<String>,
    /// The OAuth client the login was issued to, when Claude Code recorded it.
    pub(crate) client_id: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexLogin {
    pub(crate) id_token: String,
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    /// The ChatGPT workspace the login reads usage for.
    pub(crate) account_id: String,
    /// When the tokens were last issued, in Unix milliseconds.
    pub(crate) last_refresh_ms: Option<i64>,
}

// Tokens never reach logs.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claude(login) => f
                .debug_struct("Claude")
                .field("expires_at_ms", &login.expires_at_ms)
                .field("scopes", &login.scopes)
                .finish_non_exhaustive(),
            Self::Codex(login) => f
                .debug_struct("Codex")
                .field("account_id", &login.account_id)
                .field("last_refresh_ms", &login.last_refresh_ms)
                .finish_non_exhaustive(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    #[serde(flatten)]
    credential: Credential,
}

/// The Keychain items holding account logins, keyed by each account's credential key.
#[derive(Debug, Clone, Default)]
pub struct Vault {
    keychain: Option<PathBuf>,
}

impl Vault {
    /// The vault in the login keychain.
    pub fn new() -> Self {
        Self::default()
    }

    fn item<'a>(&'a self, key: &'a str) -> Item<'a> {
        Item {
            service: SERVICE,
            account: key,
            keychain: self.keychain.as_deref(),
        }
    }

    pub fn load(&self, key: &str) -> Result<Credential, ProviderError> {
        let secret = self.item(key).find()?.ok_or(ProviderError::AuthMissing)?;
        let record: Record = serde_json::from_str(&secret)?;
        if record.version != VERSION {
            return Err(ProviderError::Unsupported(format!(
                "saved login version {}",
                record.version
            )));
        }
        Ok(record.credential)
    }

    pub fn save(&self, key: &str, credential: &Credential) -> Result<(), ProviderError> {
        let record = Record {
            version: VERSION,
            credential: credential.clone(),
        };
        self.item(key).set(&serde_json::to_string(&record)?)
    }

    /// Forgets a login. Deleting a missing login succeeds.
    pub fn delete(&self, key: &str) -> Result<(), ProviderError> {
        self.item(key).delete().map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> Credential {
        Credential::Claude(ClaudeLogin {
            access_token: "sk-ant-oat01-FAKE".into(),
            refresh_token: "sk-ant-ort01-FAKE".into(),
            expires_at_ms: Some(1_790_000_000_000),
            scopes: vec!["user:inference".into()],
            client_id: None,
        })
    }

    #[test]
    fn record_round_trips_with_version_and_provider_tag() {
        let record = Record {
            version: VERSION,
            credential: claude(),
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains(r#""version":1"#));
        assert!(json.contains(r#""provider":"claude""#));
        let back: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(back.credential, claude());
    }

    #[test]
    fn debug_hides_tokens() {
        let shown = format!("{:?}", claude());
        assert!(!shown.contains("FAKE"), "{shown}");
    }
}
