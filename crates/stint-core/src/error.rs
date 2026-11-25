use std::io;
use std::path::PathBuf;

use crate::account::{AccountId, ProviderId};

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
    #[error("migrate database: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("create database directory {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("account {0} not found")]
    AccountNotFound(AccountId),
    #[error("{provider} account {label:?} already exists")]
    DuplicateAccount { provider: ProviderId, label: String },
    #[error("database schema version {found} is newer than supported version {supported}")]
    SchemaTooNew { found: usize, supported: usize },
    #[error("database schema version {found} is older than {supported}; open it read-write first")]
    SchemaOutdated { found: usize, supported: usize },
}
