use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};

use super::accounts;
use crate::account::AccountId;
use crate::error::StoreError;
use crate::quota::{QuotaSnapshot, QuotaWindow};
use crate::sql::Millis;

const INSERT_SNAPSHOT: &str = "
    INSERT INTO quota_snapshots (account_id, fetched_at)
    VALUES (?1, ?2)";

const INSERT_WINDOW: &str = "
    INSERT INTO quota_windows
        (snapshot_id, window_kind, window_scope, used_percent, resets_at)
    VALUES (?1, ?2, ?3, ?4, ?5)";

// Ties on `fetched_at` go to the poll recorded last.
const SELECT_LATEST_SNAPSHOT: &str = "
    SELECT id, fetched_at
    FROM quota_snapshots
    WHERE account_id = ?1
    ORDER BY fetched_at DESC, id DESC
    LIMIT 1";

const SELECT_WINDOWS: &str = "
    SELECT window_kind, window_scope, used_percent, resets_at
    FROM quota_windows
    WHERE snapshot_id = ?1
    ORDER BY id";

pub(super) fn record(conn: &mut Connection, snapshot: &QuotaSnapshot) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    accounts::require(&tx, snapshot.account_id)?;

    let mut insert_snapshot = tx.prepare_cached(INSERT_SNAPSHOT)?;
    insert_snapshot.execute(params![snapshot.account_id, Millis(snapshot.fetched_at)])?;
    drop(insert_snapshot);
    let snapshot_id = tx.last_insert_rowid();

    let mut insert_window = tx.prepare_cached(INSERT_WINDOW)?;
    for window in &snapshot.windows {
        insert_window.execute(params![
            snapshot_id,
            window.kind,
            window.scope,
            window.used,
            window.resets_at.map(Millis),
        ])?;
    }
    drop(insert_window);

    tx.commit()?;
    Ok(())
}

pub(super) fn latest(
    conn: &Connection,
    id: AccountId,
) -> Result<Option<QuotaSnapshot>, StoreError> {
    // Both reads share one view of the database. Deferred, because the
    // connection default is IMMEDIATE and read-only connections cannot take it.
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Deferred)?;

    let mut select_snapshot = tx.prepare_cached(SELECT_LATEST_SNAPSHOT)?;
    let latest = select_snapshot
        .query_row([id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Millis>(1)?))
        })
        .optional()?;
    drop(select_snapshot);
    let Some((snapshot_id, fetched_at)) = latest else {
        return Ok(None);
    };

    let mut select_windows = tx.prepare_cached(SELECT_WINDOWS)?;
    let rows = select_windows.query_map([snapshot_id], window_from_row)?;
    let windows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(select_windows);

    tx.finish()?;
    Ok(Some(QuotaSnapshot {
        account_id: id,
        fetched_at: fetched_at.0,
        windows,
    }))
}

fn window_from_row(row: &Row<'_>) -> rusqlite::Result<QuotaWindow> {
    let resets_at: Option<Millis> = row.get(3)?;
    Ok(QuotaWindow {
        kind: row.get(0)?,
        scope: row.get(1)?,
        used: row.get(2)?,
        resets_at: resets_at.map(|millis| millis.0),
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::super::tests::{new_account, store, t0, window};
    use crate::account::{AccountId, ProviderId};
    use crate::error::StoreError;
    use crate::quota::{QuotaSnapshot, WindowKind};
    use crate::store::QuotaStore;

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
    fn empty_poll_becomes_latest_snapshot() {
        let mut store = store();
        let account = store
            .insert_account(&new_account(ProviderId::Claude, "personal"), t0())
            .unwrap();
        let full = QuotaSnapshot {
            account_id: account.id,
            fetched_at: t0(),
            windows: vec![window(WindowKind::FiveHour, 10.0)],
        };
        let empty = QuotaSnapshot {
            account_id: account.id,
            fetched_at: t0() + TimeDelta::minutes(5),
            windows: Vec::new(),
        };
        store.record_snapshot(&full).unwrap();
        store.record_snapshot(&empty).unwrap();

        assert_eq!(store.latest_snapshot(account.id).unwrap(), Some(empty));
    }

    #[test]
    fn polls_in_the_same_millisecond_stay_separate() {
        let mut store = store();
        let account = store
            .insert_account(&new_account(ProviderId::Claude, "personal"), t0())
            .unwrap();
        let first = QuotaSnapshot {
            account_id: account.id,
            fetched_at: t0(),
            windows: vec![window(WindowKind::FiveHour, 10.0)],
        };
        let second = QuotaSnapshot {
            account_id: account.id,
            fetched_at: t0() + TimeDelta::microseconds(300),
            windows: vec![window(WindowKind::FiveHour, 20.0)],
        };
        store.record_snapshot(&first).unwrap();
        store.record_snapshot(&second).unwrap();

        let latest = store.latest_snapshot(account.id).unwrap().unwrap();
        assert_eq!(latest.windows, second.windows);
        assert_eq!(latest.fetched_at, t0());
    }

    #[test]
    fn corrupt_stored_values_fail_to_decode() {
        let mut store = store();
        let account = store
            .insert_account(&new_account(ProviderId::Claude, "personal"), t0())
            .unwrap();
        let empty = QuotaSnapshot {
            account_id: account.id,
            fetched_at: t0(),
            windows: Vec::new(),
        };
        store.record_snapshot(&empty).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO quota_windows (snapshot_id, window_kind, used_percent)
                 SELECT id, 'monthly', 1.0 FROM quota_snapshots",
                [],
            )
            .unwrap();

        assert!(matches!(
            store.latest_snapshot(account.id),
            Err(StoreError::Sqlite(_))
        ));
    }
}
