mod accounts;
mod settings;
mod snapshots;

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use rusqlite::{Connection, ErrorCode, OpenFlags, TransactionBehavior};
use rusqlite_migration::{M, Migrations};

use super::QuotaStore;
use crate::account::{AccountId, AccountProfile, NewAccount};
use crate::error::StoreError;
use crate::quota::QuotaSnapshot;

// Append only. Each entry is one schema version tracked in `user_version`.
static MIGRATION_LIST: [M<'static>; 4] = [
    M::up(include_str!("sqlite/migrations/0001_init.sql")),
    M::up(include_str!("sqlite/migrations/0002_tracked_accounts.sql")),
    M::up(include_str!("sqlite/migrations/0003_window_settings.sql")),
    M::up(include_str!("sqlite/migrations/0004_antigravity.sql")),
];
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
        created_at: Timestamp,
    ) -> Result<AccountProfile, StoreError> {
        accounts::insert(&mut self.conn, account, created_at)
    }

    fn accounts(&self) -> Result<Vec<AccountProfile>, StoreError> {
        accounts::list(&self.conn)
    }

    fn remove_account(&mut self, id: AccountId) -> Result<(), StoreError> {
        accounts::remove(&self.conn, id)
    }

    fn rename_account(&mut self, id: AccountId, label: &str) -> Result<(), StoreError> {
        accounts::rename(&self.conn, id, label)
    }

    fn set_account_login(
        &mut self,
        id: AccountId,
        email: Option<&str>,
        plan: Option<&str>,
    ) -> Result<(), StoreError> {
        accounts::set_login(&self.conn, id, email, plan)
    }

    fn set_shown_in_menu(&mut self, id: AccountId, shown: bool) -> Result<(), StoreError> {
        accounts::set_shown_in_menu(&self.conn, id, shown)
    }

    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        settings::get(&self.conn, key)
    }

    fn set_setting(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
        settings::set(&self.conn, key, value)
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::account::ProviderId;
    use crate::quota::{Percentage, QuotaWindow, WindowKind};

    // Shared with the `accounts` and `snapshots` test modules.

    pub(super) fn t0() -> Timestamp {
        "2026-01-01T12:00:00Z".parse::<Timestamp>().unwrap()
    }

    pub(super) fn store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    pub(super) fn new_account(provider: ProviderId, label: &str) -> NewAccount {
        NewAccount {
            provider,
            label: label.into(),
            email: None,
            plan: None,
            credential_key: format!("{provider}-{label}"),
        }
    }

    pub(super) fn window(kind: WindowKind, used: f64) -> QuotaWindow {
        QuotaWindow {
            kind,
            scope: None,
            used: Percentage::new(used).unwrap(),
            resets_at: Some(t0() + kind.duration()),
        }
    }

    #[test]
    fn migrations_are_valid() {
        MIGRATIONS.validate().unwrap();
        assert_eq!(
            MIGRATIONS
                .pending_migrations(&Connection::open_in_memory().unwrap())
                .unwrap(),
            i32::try_from(SCHEMA_VERSION).unwrap()
        );
    }

    #[test]
    fn gemini_accounts_become_antigravity_accounts() {
        let mut conn = Connection::open_in_memory().unwrap();
        MIGRATIONS.to_version(&mut conn, 3).unwrap();
        conn.execute(
            "INSERT INTO accounts (provider, label, credential_key, created_at) \
             VALUES ('gemini', 'Personal', 'key', 0)",
            [],
        )
        .unwrap();
        MIGRATIONS.to_latest(&mut conn).unwrap();
        let provider: String = conn
            .query_row("SELECT provider FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(provider, "antigravity");
    }

    #[test]
    fn open_creates_parent_dirs_and_enables_wal() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested/stint.db");
        let store = SqliteStore::open(&path).unwrap();
        let mode: String = store
            .conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(schema_version(&store.conn).unwrap(), SCHEMA_VERSION);
        drop(store);
        SqliteStore::open(&path).unwrap();
    }

    #[test]
    fn concurrent_first_open_migrates_once() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("stint.db");
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        SqliteStore::open(&path).map(|_| ())
                    })
                })
                .collect();
            for handle in handles {
                handle.join().unwrap().unwrap();
            }
        });
        let store = SqliteStore::open_read_only(&path).unwrap();
        assert_eq!(store.accounts().unwrap(), Vec::new());
    }

    #[test]
    fn open_rejects_newer_schema() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("stint.db");
        let store = SqliteStore::open(&path).unwrap();
        store.conn.pragma_update(None, "user_version", 99).unwrap();
        drop(store);
        assert!(matches!(
            SqliteStore::open(&path),
            Err(StoreError::SchemaTooNew {
                found: 99,
                supported: SCHEMA_VERSION
            })
        ));
        assert!(matches!(
            SqliteStore::open_read_only(&path),
            Err(StoreError::SchemaTooNew { found: 99, .. })
        ));
    }

    #[test]
    fn read_only_store_reads_but_cannot_write() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("stint.db");
        let mut writer = SqliteStore::open(&path).unwrap();
        let account = writer
            .insert_account(&new_account(ProviderId::Claude, "personal"), t0())
            .unwrap();

        let mut reader = SqliteStore::open_read_only(&path).unwrap();
        assert_eq!(reader.latest_snapshot(account.id).unwrap(), None);
        assert_eq!(reader.accounts().unwrap(), vec![account]);
        assert!(matches!(
            reader.insert_account(&new_account(ProviderId::Codex, "personal"), t0()),
            Err(StoreError::Sqlite(_))
        ));
    }

    #[test]
    fn read_only_open_rejects_missing_or_unmigrated_database() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("stint.db");
        assert!(matches!(
            SqliteStore::open_read_only(&path),
            Err(StoreError::Sqlite(_))
        ));

        Connection::open(&path).unwrap();
        assert!(matches!(
            SqliteStore::open_read_only(&path),
            Err(StoreError::SchemaOutdated { found: 0, .. })
        ));
    }
}
