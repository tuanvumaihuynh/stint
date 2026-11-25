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
