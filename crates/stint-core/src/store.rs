mod sqlite;

use jiff::Timestamp;

use crate::account::{AccountId, AccountProfile, NewAccount, ProviderId};
use crate::error::StoreError;
use crate::quota::QuotaSnapshot;

pub use sqlite::SqliteStore;

pub trait QuotaStore {
    /// Adds an account. The first account of a provider becomes active.
    fn insert_account(
        &mut self,
        account: &NewAccount,
        created_at: Timestamp,
    ) -> Result<AccountProfile, StoreError>;

    fn accounts(&self) -> Result<Vec<AccountProfile>, StoreError>;

    /// Makes `id` the only active account of its provider.
    fn set_active_account(&mut self, id: AccountId) -> Result<(), StoreError>;

    fn record_snapshot(&mut self, snapshot: &QuotaSnapshot) -> Result<(), StoreError>;

    fn latest_snapshot(&self, id: AccountId) -> Result<Option<QuotaSnapshot>, StoreError>;

    /// Returns the provider's active account. The first time a provider reports usage it
    /// has none, so a `default` account is enrolled for the login its CLI is signed in to.
    fn ensure_active_account(
        &mut self,
        provider: ProviderId,
        now: Timestamp,
    ) -> Result<AccountProfile, StoreError> {
        let active = self
            .accounts()?
            .into_iter()
            .find(|account| account.provider == provider && account.is_active);
        if let Some(account) = active {
            return Ok(account);
        }

        let account = NewAccount {
            provider,
            label: DEFAULT_ACCOUNT_LABEL.to_owned(),
            email: None,
        };
        self.insert_account(&account, now)
    }
}

/// Label of the account enrolled for a provider's signed-in CLI login.
pub const DEFAULT_ACCOUNT_LABEL: &str = "default";
