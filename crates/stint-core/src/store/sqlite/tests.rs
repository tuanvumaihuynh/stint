use chrono::{TimeDelta, TimeZone};
use tempfile::TempDir;

use super::*;
use crate::account::ProviderId;
use crate::quota::{Percentage, QuotaWindow, WindowKind};

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
