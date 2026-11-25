use chrono::{DateTime, Utc};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, params};

use crate::account::{AccountId, AccountProfile, NewAccount};
use crate::error::StoreError;
use crate::sql::Millis;

// The first account of a provider becomes active.
const INSERT: &str = "
    INSERT INTO accounts (provider, label, email, is_active, created_at)
    VALUES (
        ?1, ?2, ?3,
        NOT EXISTS (SELECT 1 FROM accounts WHERE provider = ?1 AND is_active = 1),
        ?4
    )";

const SELECT_BY_ID: &str = "
    SELECT id, provider, label, email, is_active, created_at
    FROM accounts
    WHERE id = ?1";

const SELECT_ALL: &str = "
    SELECT id, provider, label, email, is_active, created_at
    FROM accounts
    ORDER BY id";

const EXISTS: &str = "SELECT 1 FROM accounts WHERE id = ?1";

const DEACTIVATE_PROVIDER_OF: &str = "
    UPDATE accounts
    SET is_active = 0
    WHERE is_active = 1
      AND provider = (SELECT provider FROM accounts WHERE id = ?1)";

const ACTIVATE: &str = "UPDATE accounts SET is_active = 1 WHERE id = ?1";

pub(super) fn insert(
    conn: &mut Connection,
    account: &NewAccount,
    created_at: DateTime<Utc>,
) -> Result<AccountProfile, StoreError> {
    let tx = conn.transaction()?;

    let mut insert = tx.prepare_cached(INSERT)?;
    let inserted = insert.execute(params![
        account.provider,
        account.label,
        account.email,
        Millis(created_at),
    ]);
    if let Err(err) = inserted {
        return Err(duplicate_or(err, account));
    }
    drop(insert);

    let id = AccountId::new(tx.last_insert_rowid());
    let mut select = tx.prepare_cached(SELECT_BY_ID)?;
    let profile = select.query_row([id], from_row)?;
    drop(select);

    tx.commit()?;
    Ok(profile)
}

pub(super) fn list(conn: &Connection) -> Result<Vec<AccountProfile>, StoreError> {
    let mut select = conn.prepare_cached(SELECT_ALL)?;
    let rows = select.query_map([], from_row)?;
    let accounts = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(accounts)
}

pub(super) fn set_active(conn: &mut Connection, id: AccountId) -> Result<(), StoreError> {
    let tx = conn.transaction()?;

    require(&tx, id)?;
    tx.prepare_cached(DEACTIVATE_PROVIDER_OF)?.execute([id])?;
    tx.prepare_cached(ACTIVATE)?.execute([id])?;

    tx.commit()?;
    Ok(())
}

/// Fails with [`StoreError::AccountNotFound`] unless `id` exists.
pub(super) fn require(conn: &Connection, id: AccountId) -> Result<(), StoreError> {
    let mut select = conn.prepare_cached(EXISTS)?;
    let found = select.query_row([id], |_| Ok(())).optional()?;
    found.ok_or(StoreError::AccountNotFound(id))
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<AccountProfile> {
    let created_at: Millis = row.get(5)?;
    Ok(AccountProfile {
        id: row.get(0)?,
        provider: row.get(1)?,
        label: row.get(2)?,
        email: row.get(3)?,
        is_active: row.get(4)?,
        created_at: created_at.0,
    })
}

fn duplicate_or(err: rusqlite::Error, account: &NewAccount) -> StoreError {
    if err.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) {
        return StoreError::DuplicateAccount {
            provider: account.provider,
            label: account.label.clone(),
        };
    }
    err.into()
}

#[cfg(test)]
mod tests {
    use super::super::tests::{new_account, store, t0};
    use crate::account::{AccountId, ProviderId};
    use crate::error::StoreError;
    use crate::store::QuotaStore;

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
}
