use chrono::{DateTime, Utc};
use rusqlite::{Connection, Row, params};

use super::accounts;
use crate::account::AccountId;
use crate::error::StoreError;
use crate::quota::{QuotaSnapshot, QuotaWindow};
use crate::sql::Millis;

const INSERT_WINDOW: &str = "
    INSERT INTO quota_snapshots
        (account_id, window_kind, window_scope, used_percent, resets_at, fetched_at)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

// Every window row of the account's most recent poll.
const SELECT_LATEST: &str = "
    SELECT fetched_at, window_kind, window_scope, used_percent, resets_at
    FROM quota_snapshots
    WHERE account_id = ?1
      AND fetched_at = (SELECT MAX(fetched_at) FROM quota_snapshots WHERE account_id = ?1)
    ORDER BY id";

pub(super) fn record(conn: &mut Connection, snapshot: &QuotaSnapshot) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    accounts::require(&tx, snapshot.account_id)?;

    let mut insert = tx.prepare_cached(INSERT_WINDOW)?;
    let fetched_at = Millis(snapshot.fetched_at);
    for window in &snapshot.windows {
        insert.execute(params![
            snapshot.account_id,
            window.kind,
            window.scope,
            window.used,
            window.resets_at.map(Millis),
            fetched_at,
        ])?;
    }
    drop(insert);

    tx.commit()?;
    Ok(())
}

pub(super) fn latest(
    conn: &Connection,
    id: AccountId,
) -> Result<Option<QuotaSnapshot>, StoreError> {
    let mut select = conn.prepare_cached(SELECT_LATEST)?;
    let rows = select.query_map([id], window_from_row)?;

    let mut fetched_at = None;
    let mut windows = Vec::new();
    for row in rows {
        let (row_fetched_at, window) = row?;
        fetched_at = Some(row_fetched_at);
        windows.push(window);
    }

    let snapshot = fetched_at.map(|fetched_at| QuotaSnapshot {
        account_id: id,
        fetched_at,
        windows,
    });
    Ok(snapshot)
}

fn window_from_row(row: &Row<'_>) -> rusqlite::Result<(DateTime<Utc>, QuotaWindow)> {
    let fetched_at: Millis = row.get(0)?;
    let resets_at: Option<Millis> = row.get(4)?;
    let window = QuotaWindow {
        kind: row.get(1)?,
        scope: row.get(2)?,
        used: row.get(3)?,
        resets_at: resets_at.map(|millis| millis.0),
    };
    Ok((fetched_at.0, window))
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
