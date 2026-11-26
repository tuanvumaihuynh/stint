use std::fs::{self, File};
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use stint_core::QuotaWindow;

use crate::antigravity;
use crate::claude;
use crate::codex;
use crate::credential::{Credential, Vault};
use crate::error::ProviderError;

const LOCK_DIR: &str = "locks";
// A rotated login that could not be saved is lost, so saving is retried.
const SAVE_ATTEMPTS: usize = 3;

/// Reads the quota of accounts whose logins live in the [`Vault`], renewing logins as
/// they expire.
#[derive(Debug, Clone)]
pub struct Accounts {
    client: reqwest::Client,
    vault: Vault,
    lock_dir: PathBuf,
}

impl Accounts {
    /// `data_dir` holds the lock files that keep two Stint processes from renewing the
    /// same login at once.
    pub fn new(client: reqwest::Client, vault: Vault, data_dir: &Path) -> Self {
        Self {
            client,
            vault,
            lock_dir: data_dir.join(LOCK_DIR),
        }
    }

    /// The live quota of the account whose login is saved under `key`.
    ///
    /// A login due for renewal is renewed first, and one the provider rejects is renewed
    /// once and retried. The lock is held from loading the login until any renewal is
    /// saved, so a second process waits and then uses the renewed login.
    pub async fn fetch(&self, key: &str) -> Result<Vec<QuotaWindow>, ProviderError> {
        let _lock = self.lock(key)?;
        let mut credential = self.vault.load(key)?;
        let mut renewed = false;
        if renewal_due(&credential, Timestamp::now()) {
            credential = self.renew(key, &credential).await?;
            renewed = true;
        }
        match usage(&self.client, &credential).await {
            Err(ProviderError::TokenExpired) if !renewed => {
                let credential = self.renew(key, &credential).await?;
                usage(&self.client, &credential)
                    .await
                    .map_err(signed_out_if_rejected)
            }
            result => result.map_err(signed_out_if_rejected),
        }
    }

    async fn renew(&self, key: &str, credential: &Credential) -> Result<Credential, ProviderError> {
        let now = Timestamp::now();
        let renewed = match credential {
            Credential::Claude(login) => {
                Credential::Claude(claude::refresh(&self.client, login, now).await?)
            }
            Credential::Codex(login) => {
                Credential::Codex(codex::refresh(&self.client, login, now).await?)
            }
            Credential::Antigravity(login) => {
                Credential::Antigravity(antigravity::refresh(&self.client, login, now).await?)
            }
        };
        let mut saved = self.vault.save(key, &renewed);
        for _ in 1..SAVE_ATTEMPTS {
            if saved.is_ok() {
                break;
            }
            saved = self.vault.save(key, &renewed);
        }
        saved?;
        Ok(renewed)
    }

    fn lock(&self, key: &str) -> Result<File, ProviderError> {
        fs::create_dir_all(&self.lock_dir).map_err(ProviderError::io(&self.lock_dir))?;
        let path = self.lock_dir.join(format!("{key}.lock"));
        let file = File::create(&path).map_err(ProviderError::io(&path))?;
        file.lock().map_err(ProviderError::io(&path))?;
        Ok(file)
    }
}

fn renewal_due(credential: &Credential, now: Timestamp) -> bool {
    match credential {
        Credential::Claude(login) => claude::renewal_due(login, now),
        Credential::Codex(login) => codex::renewal_due(login, now),
        Credential::Antigravity(login) => antigravity::renewal_due(login, now),
    }
}

async fn usage(
    client: &reqwest::Client,
    credential: &Credential,
) -> Result<Vec<QuotaWindow>, ProviderError> {
    match credential {
        Credential::Claude(login) => claude::usage(client, login).await,
        Credential::Codex(login) => codex::usage(client, login).await,
        Credential::Antigravity(login) => antigravity::usage(client, login).await,
    }
}

// A freshly renewed token that is still rejected means the account must sign in again.
fn signed_out_if_rejected(err: ProviderError) -> ProviderError {
    match err {
        ProviderError::TokenExpired => ProviderError::SignInExpired,
        err => err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_per_key() {
        let data = tempfile::TempDir::new().unwrap();
        let client = reqwest::Client::new();
        let accounts = Accounts::new(client, Vault::new(), data.path());
        let held = accounts.lock("claude-a").unwrap();

        let path = data.path().join("locks/claude-a.lock");
        let other = File::open(&path).unwrap();
        assert!(other.try_lock().is_err());
        assert!(accounts.lock("claude-b").is_ok());

        drop(held);
        // A process another test is spawning can hold a copy of the lock until it execs.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while other.try_lock().is_err() {
            assert!(
                std::time::Instant::now() < deadline,
                "the lock was never released"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
