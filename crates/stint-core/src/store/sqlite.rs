mod accounts;
mod snapshots;
#[cfg(test)]
mod tests;

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, ErrorCode, OpenFlags, TransactionBehavior};
use rusqlite_migration::{M, Migrations};

use super::QuotaStore;
use crate::account::{AccountId, AccountProfile, NewAccount};
use crate::error::StoreError;
use crate::quota::QuotaSnapshot;

// Append only. Each entry is one schema version tracked in `user_version`.
static MIGRATION_LIST: [M<'static>; 1] = [M::up(include_str!("../../migrations/0001_init.sql"))];
static MIGRATIONS: Migrations<'static> = Migrations::from_slice(&MIGRATION_LIST);
const SCHEMA_VERSION: usize = MIGRATION_LIST.len();

const STATEMENT_CACHE_CAPACITY: usize = 64;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const WAL_RETRY_INTERVAL: Duration = Duration::from_millis(10);

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Opens or creates the database in WAL mode, creating parent directories
    /// and applying pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        create_parent_dir(path)?;

        let conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        enable_wal(&conn)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        Self::init(conn)
    }

    /// Opens an existing, fully migrated database without taking write locks.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);

        let found = schema_version(&conn)?;
        if found > SCHEMA_VERSION {
            return Err(StoreError::SchemaTooNew {
                found,
                supported: SCHEMA_VERSION,
            });
        }
        if found < SCHEMA_VERSION {
            return Err(StoreError::SchemaOutdated {
                found,
                supported: SCHEMA_VERSION,
            });
        }

        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);
        // Writers take the lock up front so a read-then-write transaction never
        // fails with SQLITE_BUSY when the app and the CLI write concurrently.
        conn.set_transaction_behavior(TransactionBehavior::Immediate);

        migrate(&mut conn)?;

        Ok(Self { conn })
    }
}

impl QuotaStore for SqliteStore {
    fn insert_account(
        &mut self,
        account: &NewAccount,
        created_at: DateTime<Utc>,
    ) -> Result<AccountProfile, StoreError> {
        accounts::insert(&mut self.conn, account, created_at)
    }

    fn accounts(&self) -> Result<Vec<AccountProfile>, StoreError> {
        accounts::list(&self.conn)
    }

    fn set_active_account(&mut self, id: AccountId) -> Result<(), StoreError> {
        accounts::set_active(&mut self.conn, id)
    }

    fn record_snapshot(&mut self, snapshot: &QuotaSnapshot) -> Result<(), StoreError> {
        snapshots::record(&mut self.conn, snapshot)
    }

    fn latest_snapshot(&self, id: AccountId) -> Result<Option<QuotaSnapshot>, StoreError> {
        snapshots::latest(&self.conn, id)
    }
}

fn create_parent_dir(path: &Path) -> Result<(), StoreError> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|source| StoreError::Io {
        path: parent.to_path_buf(),
        source,
    })
}

// Switching to WAL upgrades a shared lock to an exclusive one. SQLite reports
// SQLITE_BUSY for that upgrade without calling the busy handler (to avoid
// deadlock) when another process opens the same fresh file, so retry here.
fn enable_wal(conn: &Connection) -> Result<(), StoreError> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        let result = conn.pragma_update(None, "journal_mode", "WAL");

        let busy =
            matches!(&result, Err(err) if err.sqlite_error_code() == Some(ErrorCode::DatabaseBusy));
        if !busy || Instant::now() >= deadline {
            return Ok(result?);
        }

        thread::sleep(WAL_RETRY_INTERVAL);
    }
}

fn schema_version(conn: &Connection) -> Result<usize, StoreError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    // A negative `user_version` was never written by Stint; treat it as unknown and newer.
    Ok(usize::try_from(version).unwrap_or(usize::MAX))
}

fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let found = schema_version(conn)?;
    if found > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }

    let Err(err) = MIGRATIONS.to_latest(conn) else {
        return Ok(());
    };

    // The migrator reads `user_version` before its transaction starts, so a
    // second process opening a fresh database can lose the race and replay a
    // migration. The winner has already brought the schema up to date.
    if schema_version(conn)? == SCHEMA_VERSION {
        return Ok(());
    }
    Err(err.into())
}
