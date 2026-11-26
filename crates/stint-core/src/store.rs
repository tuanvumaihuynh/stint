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

    fn record_snapshot(&mut self, snapshot: &QuotaSnapshot) -> Result<(), StoreError>;

    fn latest_snapshot(&self, id: AccountId) -> Result<Option<QuotaSnapshot>, StoreError>;
}
