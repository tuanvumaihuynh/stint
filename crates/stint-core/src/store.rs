use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::account::{AccountId, AccountProfile, NewAccount};
use crate::error::StoreError;
use crate::quota::{Percentage, QuotaSnapshot, QuotaWindow};

pub trait QuotaStore {
    /// Adds an account. The first account of a provider becomes active.
    ///
    /// # Errors
    ///
    /// Fails when the provider already has an account with the same label.
    fn insert_account(
        &mut self,
        account: &NewAccount,
        created_at: DateTime<Utc>,
    ) -> Result<AccountProfile, StoreError>;

    /// # Errors
    ///
    /// Fails on database errors or rows that no longer decode.
    fn accounts(&self) -> Result<Vec<AccountProfile>, StoreError>;

    /// Makes `id` the only active account of its provider.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AccountNotFound`] for an unknown id.
    fn set_active_account(&mut self, id: AccountId) -> Result<(), StoreError>;

    /// # Errors
    ///
    /// Returns [`StoreError::AccountNotFound`] for an unknown account.
    fn record_snapshot(&mut self, snapshot: &QuotaSnapshot) -> Result<(), StoreError>;

    /// # Errors
    ///
    /// Fails on database errors or rows that no longer decode.
    fn latest_snapshot(&self, id: AccountId) -> Result<Option<QuotaSnapshot>, StoreError>;
}

// Append only: each entry moves `user_version` up by one.
const MIGRATIONS: &[&str] = &["
    CREATE TABLE accounts (
        id INTEGER PRIMARY KEY,
        provider TEXT NOT NULL,
        label TEXT NOT NULL,
        email TEXT,
        is_active INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        UNIQUE (provider, label)
    );
    CREATE UNIQUE INDEX accounts_one_active_per_provider
        ON accounts (provider) WHERE is_active = 1;

    CREATE TABLE quota_snapshots (
        id INTEGER PRIMARY KEY,
        account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
        window_kind TEXT NOT NULL,
        window_scope TEXT,
        used_percent REAL NOT NULL,
        resets_at TEXT,
        fetched_at TEXT NOT NULL
    );
    CREATE INDEX quota_snapshots_account_fetched
        ON quota_snapshots (account_id, fetched_at);
"];

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Opens or creates the database in WAL mode and applies pending migrations.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened, migrations fail, or the schema
    /// was written by a newer Stint.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        let supported = u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX);
        let found: u32 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if found > supported {
            return Err(StoreError::SchemaTooNew { found, supported });
        }
        for (version, sql) in (1u32..).zip(MIGRATIONS).skip(found as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
        Ok(())
    }

    fn require_account(conn: &Connection, id: AccountId) -> Result<String, StoreError> {
        conn.query_row(
            "SELECT provider FROM accounts WHERE id = ?1",
            [id.get()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(StoreError::AccountNotFound(id))
    }
}

const ACCOUNT_COLUMNS: &str = "id, provider, label, email, is_active, created_at";

struct AccountRow {
    id: i64,
    provider: String,
    label: String,
    email: Option<String>,
    is_active: bool,
    created_at: DateTime<Utc>,
}

impl AccountRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            provider: row.get(1)?,
            label: row.get(2)?,
            email: row.get(3)?,
            is_active: row.get(4)?,
            created_at: row.get(5)?,
        })
    }

    fn into_profile(self) -> Result<AccountProfile, StoreError> {
        Ok(AccountProfile {
            id: AccountId::new(self.id),
            provider: self.provider.parse()?,
            label: self.label,
            email: self.email,
            is_active: self.is_active,
            created_at: self.created_at,
        })
    }
}

impl QuotaStore for SqliteStore {
    fn insert_account(
        &mut self,
        account: &NewAccount,
        created_at: DateTime<Utc>,
    ) -> Result<AccountProfile, StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO accounts (provider, label, email, is_active, created_at)
             VALUES (?1, ?2, ?3,
                     NOT EXISTS (SELECT 1 FROM accounts WHERE provider = ?1 AND is_active = 1),
                     ?4)",
            params![
                account.provider.as_str(),
                account.label,
                account.email,
                created_at
            ],
        )?;
        let id = AccountId::new(tx.last_insert_rowid());
        let row = tx.query_row(
            &format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1"),
            [id.get()],
            AccountRow::read,
        )?;
        tx.commit()?;
        row.into_profile()
    }

    fn accounts(&self) -> Result<Vec<AccountProfile>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY id"
        ))?;
        let rows = stmt.query_map([], AccountRow::read)?;
        rows.map(|row| row?.into_profile()).collect()
    }

    fn set_active_account(&mut self, id: AccountId) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        let provider = Self::require_account(&tx, id)?;
        tx.execute(
            "UPDATE accounts SET is_active = 0 WHERE provider = ?1 AND is_active = 1",
            [&provider],
        )?;
        tx.execute(
            "UPDATE accounts SET is_active = 1 WHERE id = ?1",
            [id.get()],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn record_snapshot(&mut self, snapshot: &QuotaSnapshot) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        Self::require_account(&tx, snapshot.account_id)?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO quota_snapshots
                     (account_id, window_kind, window_scope, used_percent, resets_at, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for window in &snapshot.windows {
                stmt.execute(params![
                    snapshot.account_id.get(),
                    window.kind.as_str(),
                    window.scope,
                    window.used.get(),
                    window.resets_at,
                    snapshot.fetched_at,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn latest_snapshot(&self, id: AccountId) -> Result<Option<QuotaSnapshot>, StoreError> {
        let fetched_at: Option<DateTime<Utc>> = self.conn.query_row(
            "SELECT MAX(fetched_at) FROM quota_snapshots WHERE account_id = ?1",
            [id.get()],
            |row| row.get(0),
        )?;
        let Some(fetched_at) = fetched_at else {
            return Ok(None);
        };

        let mut stmt = self.conn.prepare(
            "SELECT window_kind, window_scope, used_percent, resets_at
             FROM quota_snapshots
             WHERE account_id = ?1 AND fetched_at = ?2
             ORDER BY id",
        )?;
        let rows = stmt.query_map(params![id.get(), fetched_at], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, Option<DateTime<Utc>>>(3)?,
            ))
        })?;
        let windows = rows
            .map(|row| {
                let (kind, scope, used, resets_at) = row?;
                Ok(QuotaWindow {
                    kind: kind.parse()?,
                    scope,
                    used: Percentage::new(used)?,
                    resets_at,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;

        Ok(Some(QuotaSnapshot {
            account_id: id,
            fetched_at,
            windows,
        }))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, TimeZone};
    use tempfile::TempDir;

    use super::*;
    use crate::account::ProviderId;
    use crate::quota::WindowKind;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
    }

    fn open() -> (TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("stint.db")).unwrap();
        (dir, store)
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
    fn open_enables_wal_and_reopens_cleanly() {
        let (dir, store) = open();
        let mode: String = store
            .conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        drop(store);
        SqliteStore::open(dir.path().join("stint.db")).unwrap();
    }

    #[test]
    fn open_rejects_newer_schema() {
        let (dir, store) = open();
        store.conn.pragma_update(None, "user_version", 99).unwrap();
        drop(store);
        let err = SqliteStore::open(dir.path().join("stint.db"))
            .err()
            .unwrap();
        assert!(matches!(
            err,
            StoreError::SchemaTooNew {
                found: 99,
                supported: 1
            }
        ));
    }

    #[test]
    fn first_account_per_provider_becomes_active() {
        let (_dir, mut store) = open();
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
        let (_dir, mut store) = open();
        let account = new_account(ProviderId::Codex, "personal");
        store.insert_account(&account, t0()).unwrap();
        assert!(matches!(
            store.insert_account(&account, t0()),
            Err(StoreError::Sqlite(_))
        ));
    }

    #[test]
    fn set_active_account_switches_within_provider_only() {
        let (_dir, mut store) = open();
        let a = store
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
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn set_active_account_rejects_unknown_id() {
        let (_dir, mut store) = open();
        assert!(matches!(
            store.set_active_account(AccountId::new(7)),
            Err(StoreError::AccountNotFound(id)) if id == AccountId::new(7)
        ));
    }

    #[test]
    fn latest_snapshot_returns_newest_poll() {
        let (_dir, mut store) = open();
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
        let (_dir, mut store) = open();
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
}
