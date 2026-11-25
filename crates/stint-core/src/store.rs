use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Row, TransactionBehavior, params,
};
use rusqlite_migration::{M, Migrations};

use crate::account::{AccountId, AccountProfile, NewAccount};
use crate::error::StoreError;
use crate::quota::{QuotaSnapshot, QuotaWindow};
use crate::sql::Millis;

pub trait QuotaStore {
    /// Adds an account. The first account of a provider becomes active.
    fn insert_account(
        &mut self,
        account: &NewAccount,
        created_at: DateTime<Utc>,
    ) -> Result<AccountProfile, StoreError>;

    fn accounts(&self) -> Result<Vec<AccountProfile>, StoreError>;

    /// Makes `id` the only active account of its provider.
    fn set_active_account(&mut self, id: AccountId) -> Result<(), StoreError>;

    fn record_snapshot(&mut self, snapshot: &QuotaSnapshot) -> Result<(), StoreError>;

    fn latest_snapshot(&self, id: AccountId) -> Result<Option<QuotaSnapshot>, StoreError>;
}

// Append only. Each entry is one schema version tracked in `user_version`.
static MIGRATION_LIST: [M<'static>; 1] = [M::up(include_str!("../migrations/0001_init.sql"))];
static MIGRATIONS: Migrations<'static> = Migrations::from_slice(&MIGRATION_LIST);
const SCHEMA_VERSION: usize = MIGRATION_LIST.len();

const STATEMENT_CACHE_CAPACITY: usize = 64;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Opens or creates the database in WAL mode, creating parent directories
    /// and applying pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
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
        Self::init(Connection::open_in_memory()?)
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

// Switching to WAL upgrades a shared lock to an exclusive one. SQLite reports
// SQLITE_BUSY for that upgrade without calling the busy handler (to avoid
// deadlock) when another process opens the same fresh file, so retry here.
fn enable_wal(conn: &Connection) -> Result<(), StoreError> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Err(err)
                if err.sqlite_error_code() == Some(ErrorCode::DatabaseBusy)
                    && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            result => return Ok(result?),
        }
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
    match MIGRATIONS.to_latest(conn) {
        Ok(()) => Ok(()),
        // The migrator reads `user_version` before its transaction starts, so a
        // second process opening a fresh database can lose the race and replay
        // a migration. The winner has already brought the schema up to date.
        Err(_) if schema_version(conn)? == SCHEMA_VERSION => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn require_account(conn: &Connection, id: AccountId) -> Result<(), StoreError> {
    conn.prepare_cached("SELECT 1 FROM accounts WHERE id = ?1")?
        .query_row([id], |_| Ok(()))
        .optional()?
        .ok_or(StoreError::AccountNotFound(id))
}

const SELECT_ACCOUNT: &str =
    "SELECT id, provider, label, email, is_active, created_at FROM accounts";

fn account_from_row(row: &Row<'_>) -> rusqlite::Result<AccountProfile> {
    Ok(AccountProfile {
        id: row.get(0)?,
        provider: row.get(1)?,
        label: row.get(2)?,
        email: row.get(3)?,
        is_active: row.get(4)?,
        created_at: row.get::<_, Millis>(5)?.0,
    })
}

impl QuotaStore for SqliteStore {
    fn insert_account(
        &mut self,
        account: &NewAccount,
        created_at: DateTime<Utc>,
    ) -> Result<AccountProfile, StoreError> {
        let tx = self.conn.transaction()?;
        let inserted = tx
            .prepare_cached(
                "INSERT INTO accounts (provider, label, email, is_active, created_at)
                 VALUES (?1, ?2, ?3,
                         NOT EXISTS (SELECT 1 FROM accounts WHERE provider = ?1 AND is_active = 1),
                         ?4)",
            )?
            .execute(params![
                account.provider,
                account.label,
                account.email,
                Millis(created_at)
            ]);
        if let Err(err) = inserted {
            return Err(match err.sqlite_error_code() {
                Some(ErrorCode::ConstraintViolation) => StoreError::DuplicateAccount {
                    provider: account.provider,
                    label: account.label.clone(),
                },
                _ => err.into(),
            });
        }
        let id = AccountId::new(tx.last_insert_rowid());
        let profile = tx
            .prepare_cached(&format!("{SELECT_ACCOUNT} WHERE id = ?1"))?
            .query_row([id], account_from_row)?;
        tx.commit()?;
        Ok(profile)
    }

    fn accounts(&self) -> Result<Vec<AccountProfile>, StoreError> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!("{SELECT_ACCOUNT} ORDER BY id"))?;
        let rows = stmt.query_map([], account_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn set_active_account(&mut self, id: AccountId) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        require_account(&tx, id)?;
        tx.prepare_cached(
            "UPDATE accounts SET is_active = 0
             WHERE is_active = 1 AND provider = (SELECT provider FROM accounts WHERE id = ?1)",
        )?
        .execute([id])?;
        tx.prepare_cached("UPDATE accounts SET is_active = 1 WHERE id = ?1")?
            .execute([id])?;
        tx.commit()?;
        Ok(())
    }

    fn record_snapshot(&mut self, snapshot: &QuotaSnapshot) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        require_account(&tx, snapshot.account_id)?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO quota_snapshots
                     (account_id, window_kind, window_scope, used_percent, resets_at, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for window in &snapshot.windows {
                stmt.execute(params![
                    snapshot.account_id,
                    window.kind,
                    window.scope,
                    window.used,
                    window.resets_at.map(Millis),
                    Millis(snapshot.fetched_at),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn latest_snapshot(&self, id: AccountId) -> Result<Option<QuotaSnapshot>, StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT fetched_at, window_kind, window_scope, used_percent, resets_at
             FROM quota_snapshots
             WHERE account_id = ?1
               AND fetched_at = (SELECT MAX(fetched_at) FROM quota_snapshots WHERE account_id = ?1)
             ORDER BY id",
        )?;
        let rows = stmt.query_map([id], |row| {
            let window = QuotaWindow {
                kind: row.get(1)?,
                scope: row.get(2)?,
                used: row.get(3)?,
                resets_at: row.get::<_, Option<Millis>>(4)?.map(|m| m.0),
            };
            Ok((row.get::<_, Millis>(0)?.0, window))
        })?;

        let mut snapshot: Option<QuotaSnapshot> = None;
        for row in rows {
            let (fetched_at, window) = row?;
            snapshot
                .get_or_insert_with(|| QuotaSnapshot {
                    account_id: id,
                    fetched_at,
                    windows: Vec::new(),
                })
                .windows
                .push(window);
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, TimeZone};
    use tempfile::TempDir;

    use super::*;
    use crate::account::ProviderId;
    use crate::quota::{Percentage, WindowKind};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
    }

    fn store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    fn new_account(provider: ProviderId, label: &str) -> NewAccount {
        NewAccount {
            provider,
            label: label.into(),
            email: None,
        }
    }

    fn window(kind: WindowKind, used: f64) -> QuotaWindow {
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
            1
        );
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

    #[test]
    fn first_account_per_provider_becomes_active() {
        let mut store = store();
        let mut personal = new_account(ProviderId::Claude, "personal");
        personal.email = Some("me@example.com".into());
        let personal = store.insert_account(&personal, t0()).unwrap();
        let work = store
            .insert_account(&new_account(ProviderId::Claude, "work"), t0())
            .unwrap();
        let gemini = store
            .insert_account(&new_account(ProviderId::Gemini, "personal"), t0())
            .unwrap();

        assert!(personal.is_active);
        assert_eq!(personal.email.as_deref(), Some("me@example.com"));
        assert_eq!(personal.created_at, t0());
        assert!(!work.is_active);
        assert!(gemini.is_active);
        assert_eq!(store.accounts().unwrap(), vec![personal, work, gemini]);
    }

    #[test]
    fn duplicate_label_per_provider_is_rejected() {
        let mut store = store();
        let account = new_account(ProviderId::Codex, "personal");
        store.insert_account(&account, t0()).unwrap();
        assert!(matches!(
            store.insert_account(&account, t0()),
            Err(StoreError::DuplicateAccount { provider: ProviderId::Codex, label }) if label == "personal"
        ));
    }

    #[test]
    fn set_active_account_switches_within_provider_only() {
        let mut store = store();
        store
            .insert_account(&new_account(ProviderId::Claude, "a"), t0())
            .unwrap();
        let b = store
            .insert_account(&new_account(ProviderId::Claude, "b"), t0())
            .unwrap();
        let other = store
            .insert_account(&new_account(ProviderId::Codex, "a"), t0())
            .unwrap();

        store.set_active_account(b.id).unwrap();

        let active: Vec<_> = store
            .accounts()
            .unwrap()
            .into_iter()
            .filter(|acc| acc.is_active)
            .map(|acc| acc.id)
            .collect();
        assert_eq!(active, vec![b.id, other.id]);
    }

    #[test]
    fn set_active_account_rejects_unknown_id() {
        let mut store = store();
        assert!(matches!(
            store.set_active_account(AccountId::new(7)),
            Err(StoreError::AccountNotFound(id)) if id == AccountId::new(7)
        ));
    }

    #[test]
    fn latest_snapshot_returns_newest_poll() {
        let mut store = store();
        let account = store
            .insert_account(&new_account(ProviderId::Claude, "personal"), t0())
            .unwrap();
        assert_eq!(store.latest_snapshot(account.id).unwrap(), None);

        let older = QuotaSnapshot {
            account_id: account.id,
            fetched_at: t0(),
            windows: vec![window(WindowKind::FiveHour, 10.0)],
        };
        let mut opus = window(WindowKind::SevenDay, 55.5);
        opus.scope = Some("opus".into());
        opus.resets_at = None;
        let newer = QuotaSnapshot {
            account_id: account.id,
            fetched_at: t0() + TimeDelta::minutes(5),
            windows: vec![window(WindowKind::FiveHour, 20.0), opus],
        };
        store.record_snapshot(&older).unwrap();
        store.record_snapshot(&newer).unwrap();

        assert_eq!(store.latest_snapshot(account.id).unwrap(), Some(newer));
    }

    #[test]
    fn record_snapshot_rejects_unknown_account() {
        let mut store = store();
        let snapshot = QuotaSnapshot {
            account_id: AccountId::new(3),
            fetched_at: t0(),
            windows: vec![window(WindowKind::Daily, 1.0)],
        };
        assert!(matches!(
            store.record_snapshot(&snapshot),
            Err(StoreError::AccountNotFound(_))
        ));
    }

    #[test]
    fn corrupt_stored_values_fail_to_decode() {
        let mut store = store();
        let account = store
            .insert_account(&new_account(ProviderId::Claude, "personal"), t0())
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO quota_snapshots (account_id, window_kind, used_percent, fetched_at)
                 VALUES (?1, 'monthly', 1.0, 0)",
                [account.id],
            )
            .unwrap();
        assert!(matches!(
            store.latest_snapshot(account.id),
            Err(StoreError::Sqlite(_))
        ));
    }
}
