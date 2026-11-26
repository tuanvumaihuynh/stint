use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::keychain::{Item, MAX_SECRET};

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

/// The item under an account's key: which parts hold the saved record.
///
/// A record is too large for one item, so it is split. Each save writes its parts under a
/// new generation before pointing the manifest at them, so a save cut short leaves the
/// previous login readable.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
    generation: String,
    parts: usize,
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

    fn item<'a>(&'a self, account: &'a str) -> Item<'a> {
        Item {
            service: SERVICE,
            account,
            keychain: self.keychain.as_deref(),
        }
    }

    pub fn load(&self, key: &str) -> Result<Credential, ProviderError> {
        let manifest = self.manifest(key)?.ok_or(ProviderError::AuthMissing)?;
        let mut json = String::new();
        for index in 0..manifest.parts {
            let part = part_name(key, &manifest.generation, index);
            let secret = self.item(&part).find()?.ok_or_else(|| {
                ProviderError::Keychain(format!("saved login part {part:?} is missing"))
            })?;
            json.push_str(&secret);
        }
        let record: Record = serde_json::from_str(&json)?;
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
        let json = serde_json::to_string(&record)?;
        let parts = split(&json, MAX_SECRET);
        let previous = self.manifest(key).ok().flatten();
        let manifest = Manifest {
            generation: generation(previous.as_ref()),
            parts: parts.len(),
        };

        for (index, part) in parts.iter().enumerate() {
            self.item(&part_name(key, &manifest.generation, index))
                .set(part)?;
        }
        self.item(key).set(&serde_json::to_string(&manifest)?)?;
        if let Some(previous) = previous {
            self.delete_parts(key, &previous);
        }
        Ok(())
    }

    /// Forgets a login. Deleting a missing login succeeds.
    pub fn delete(&self, key: &str) -> Result<(), ProviderError> {
        if let Some(manifest) = self.manifest(key).ok().flatten() {
            self.delete_parts(key, &manifest);
        }
        self.item(key).delete().map(drop)
    }

    fn manifest(&self, key: &str) -> Result<Option<Manifest>, ProviderError> {
        match self.item(key).find()? {
            Some(secret) => Ok(Some(serde_json::from_str(&secret)?)),
            None => Ok(None),
        }
    }

    fn delete_parts(&self, key: &str, manifest: &Manifest) {
        for index in 0..manifest.parts {
            let part = part_name(key, &manifest.generation, index);
            if let Err(err) = self.item(&part).delete() {
                eprintln!("stint: remove old login part {part:?}: {err}");
            }
        }
    }
}

fn part_name(key: &str, generation: &str, index: usize) -> String {
    format!("{key}.{generation}.{index}")
}

// Counts up from the previous save, so a new save never overwrites the parts in use.
fn generation(previous: Option<&Manifest>) -> String {
    let next = previous
        .and_then(|manifest| manifest.generation.parse::<u64>().ok())
        .map_or(1, |generation| generation + 1);
    next.to_string()
}

/// Splits `text` into pieces of at most `limit` bytes, on character boundaries.
fn split(text: &str, limit: usize) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(limit);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (part, tail) = rest.split_at(end);
        parts.push(part);
        rest = tail;
    }
    parts
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
    fn split_keeps_pieces_under_the_limit_and_whole() {
        let text = "aé".repeat(1000);
        let parts = split(&text, 7);
        assert!(parts.iter().all(|part| part.len() <= 7));
        assert_eq!(parts.concat(), text);
        assert_eq!(split("", 7), Vec::<&str>::new());
    }

    #[test]
    fn generations_count_up() {
        assert_eq!(generation(None), "1");
        let manifest = Manifest {
            generation: "41".into(),
            parts: 3,
        };
        assert_eq!(generation(Some(&manifest)), "42");
    }

    /// Exercises the real `security` binary against a throwaway keychain file.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "creates a scratch keychain; run with `cargo test -- --ignored`"]
    fn large_logins_round_trip_across_parts() {
        use std::process::Command;

        let dir = tempfile::TempDir::new().unwrap();
        let keychain = dir.path().join("vault.keychain-db");
        let created = Command::new("/usr/bin/security")
            .args(["create-keychain", "-p", "test"])
            .arg(&keychain)
            .status()
            .unwrap();
        assert!(created.success());
        let vault = Vault {
            keychain: Some(keychain.clone()),
        };

        let large = Credential::Codex(CodexLogin {
            id_token: "i".repeat(2500),
            access_token: "a".repeat(2500),
            refresh_token: "r".repeat(200),
            account_id: "acct".into(),
            last_refresh_ms: None,
        });
        vault.save("codex-1", &large).unwrap();
        assert_eq!(vault.load("codex-1").unwrap(), large);
        vault.save("codex-1", &claude()).unwrap();
        assert_eq!(vault.load("codex-1").unwrap(), claude());
        // The first save's parts are gone.
        assert_eq!(vault.item("codex-1.1.0").find().unwrap(), None);
        vault.delete("codex-1").unwrap();
        assert!(matches!(
            vault.load("codex-1"),
            Err(ProviderError::AuthMissing)
        ));

        Command::new("/usr/bin/security")
            .arg("delete-keychain")
            .arg(&keychain)
            .status()
            .unwrap();
    }

    #[test]
    fn debug_hides_tokens() {
        let shown = format!("{:?}", claude());
        assert!(!shown.contains("FAKE"), "{shown}");
    }
}
