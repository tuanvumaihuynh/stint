use crate::account::AccountId;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CoreError {
    #[error("percentage must be a finite number, got {0}")]
    InvalidPercentage(f64),
    #[error("unknown provider id: {0}")]
    UnknownProvider(String),
    #[error("unknown window kind: {0}")]
    UnknownWindowKind(String),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("decode stored row: {0}")]
    Decode(#[from] CoreError),
    #[error("account {0} not found")]
    AccountNotFound(AccountId),
    #[error("database schema version {found} is newer than supported version {supported}")]
    SchemaTooNew { found: u32, supported: u32 },
}
