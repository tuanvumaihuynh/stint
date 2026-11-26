mod sqlite;

use jiff::Timestamp;

use crate::account::{AccountId, AccountProfile, NewAccount};
use crate::error::StoreError;
use crate::quota::QuotaSnapshot;

pub use sqlite::SqliteStore;

pub trait QuotaStore {
    fn insert_account(
        &mut self,
        account: &NewAccount,
        created_at: Timestamp,
    ) -> Result<AccountProfile, StoreError>;

    fn accounts(&self) -> Result<Vec<AccountProfile>, StoreError>;

    /// Removes an account and its snapshots.
    fn remove_account(&mut self, id: AccountId) -> Result<(), StoreError>;

    /// Gives an account a new nickname, unique per provider.
    fn rename_account(&mut self, id: AccountId, label: &str) -> Result<(), StoreError>;

    /// Records what a new sign-in to an account reported.
    fn set_account_login(
        &mut self,
        id: AccountId,
        email: Option<&str>,
        plan: Option<&str>,
    ) -> Result<(), StoreError>;

    /// Shows or hides an account in the status menu.
    fn set_shown_in_menu(&mut self, id: AccountId, shown: bool) -> Result<(), StoreError>;

    /// An app preference, or `None` when it was never set.
    fn setting(&self, key: &str) -> Result<Option<String>, StoreError>;

    fn set_setting(&mut self, key: &str, value: &str) -> Result<(), StoreError>;

    fn record_snapshot(&mut self, snapshot: &QuotaSnapshot) -> Result<(), StoreError>;

    fn latest_snapshot(&self, id: AccountId) -> Result<Option<QuotaSnapshot>, StoreError>;
}
