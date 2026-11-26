use jiff::Timestamp;
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, params};

use crate::account::{AccountId, AccountProfile, NewAccount};
use crate::error::StoreError;
use crate::sql::Millis;

const INSERT: &str = "
    INSERT INTO accounts (provider, label, email, plan, credential_key, created_at)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

const COLUMNS: &str = "id, provider, label, email, plan, credential_key, shown_in_menu, created_at";

const EXISTS: &str = "SELECT 1 FROM accounts WHERE id = ?1";

const DELETE: &str = "DELETE FROM accounts WHERE id = ?1";

const RENAME: &str = "UPDATE accounts SET label = ?2 WHERE id = ?1";

const SET_LOGIN: &str = "UPDATE accounts SET email = ?2, plan = ?3 WHERE id = ?1";

const SET_SHOWN: &str = "UPDATE accounts SET shown_in_menu = ?2 WHERE id = ?1";

pub(super) fn insert(
    conn: &mut Connection,
    account: &NewAccount,
    created_at: Timestamp,
) -> Result<AccountProfile, StoreError> {
    let tx = conn.transaction()?;

    let mut insert = tx.prepare_cached(INSERT)?;
    let inserted = insert.execute(params![
        account.provider,
        account.label,
        account.email,
        account.plan,
        account.credential_key,
        Millis(created_at),
    ]);
    if let Err(err) = inserted {
        return Err(duplicate_or(err, account));
    }
    drop(insert);

    let id = AccountId::new(tx.last_insert_rowid());
    let mut select = tx.prepare_cached(&format!("SELECT {COLUMNS} FROM accounts WHERE id = ?1"))?;
    let profile = select.query_row([id], from_row)?;
    drop(select);

    tx.commit()?;
    Ok(profile)
}

pub(super) fn list(conn: &Connection) -> Result<Vec<AccountProfile>, StoreError> {
    let mut select = conn.prepare_cached(&format!("SELECT {COLUMNS} FROM accounts ORDER BY id"))?;
    let rows = select.query_map([], from_row)?;
    let accounts = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(accounts)
}

pub(super) fn remove(conn: &Connection, id: AccountId) -> Result<(), StoreError> {
    let removed = conn.prepare_cached(DELETE)?.execute([id])?;
    found(removed, id)
}

pub(super) fn rename(conn: &Connection, id: AccountId, label: &str) -> Result<(), StoreError> {
    let renamed = conn.prepare_cached(RENAME)?.execute(params![id, label]);
    match renamed {
        Ok(changed) => found(changed, id),
        Err(err) if err.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) => {
            let provider = list(conn)?
                .into_iter()
                .find(|account| account.id == id)
                .map(|account| account.provider)
                .ok_or(StoreError::AccountNotFound(id))?;
            Err(StoreError::DuplicateAccount {
                provider,
                label: label.to_owned(),
            })
        }
        Err(err) => Err(err.into()),
    }
}

pub(super) fn set_login(
    conn: &Connection,
    id: AccountId,
    email: Option<&str>,
    plan: Option<&str>,
) -> Result<(), StoreError> {
    let changed = conn
        .prepare_cached(SET_LOGIN)?
        .execute(params![id, email, plan])?;
    found(changed, id)
}

pub(super) fn set_shown_in_menu(
    conn: &Connection,
    id: AccountId,
    shown: bool,
) -> Result<(), StoreError> {
    let changed = conn
        .prepare_cached(SET_SHOWN)?
        .execute(params![id, shown])?;
    found(changed, id)
}

/// Fails with [`StoreError::AccountNotFound`] when a statement changed no row.
fn found(changed: usize, id: AccountId) -> Result<(), StoreError> {
    if changed == 0 {
        return Err(StoreError::AccountNotFound(id));
    }
    Ok(())
}

/// Fails with [`StoreError::AccountNotFound`] unless `id` exists.
pub(super) fn require(conn: &Connection, id: AccountId) -> Result<(), StoreError> {
    let mut select = conn.prepare_cached(EXISTS)?;
    let found = select.query_row([id], |_| Ok(())).optional()?;
    found.ok_or(StoreError::AccountNotFound(id))
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<AccountProfile> {
    let created_at: Millis = row.get(7)?;
    Ok(AccountProfile {
        id: row.get(0)?,
        provider: row.get(1)?,
        label: row.get(2)?,
        email: row.get(3)?,
        plan: row.get(4)?,
        credential_key: row.get(5)?,
        shown_in_menu: row.get(6)?,
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
    fn inserted_accounts_are_listed_in_order() {
        let mut store = store();
        let mut personal = new_account(ProviderId::Claude, "personal");
        personal.email = Some("me@example.com".into());
        personal.plan = Some("Max 5x".into());
        let personal = store.insert_account(&personal, t0()).unwrap();
        let work = store
            .insert_account(&new_account(ProviderId::Claude, "work"), t0())
            .unwrap();
        let codex = store
            .insert_account(&new_account(ProviderId::Codex, "personal"), t0())
            .unwrap();

        assert_eq!(personal.email.as_deref(), Some("me@example.com"));
        assert_eq!(personal.plan.as_deref(), Some("Max 5x"));
        assert_eq!(personal.credential_key, "claude-personal");
        assert_eq!(personal.created_at, t0());
        assert_eq!(store.accounts().unwrap(), vec![personal, work, codex]);
    }

    #[test]
    fn duplicate_label_per_provider_is_rejected() {
        let mut store = store();
        let account = new_account(ProviderId::Codex, "personal");
        store.insert_account(&account, t0()).unwrap();
        let mut again = account.clone();
        again.credential_key = "another-key".into();
        assert!(matches!(
            store.insert_account(&again, t0()),
            Err(StoreError::DuplicateAccount { provider: ProviderId::Codex, label }) if label == "personal"
        ));
    }

    #[test]
    fn accounts_can_be_renamed_relabelled_and_hidden() {
        let mut store = store();
        let a = store
            .insert_account(&new_account(ProviderId::Claude, "a"), t0())
            .unwrap();
        store
            .insert_account(&new_account(ProviderId::Claude, "b"), t0())
            .unwrap();
        assert!(a.shown_in_menu);

        store.rename_account(a.id, "Personal").unwrap();
        store
            .set_account_login(a.id, Some("me@example.com"), Some("Pro"))
            .unwrap();
        store.set_shown_in_menu(a.id, false).unwrap();
        let a = store.accounts().unwrap().remove(0);
        assert_eq!(a.label, "Personal");
        assert_eq!(a.email.as_deref(), Some("me@example.com"));
        assert_eq!(a.plan.as_deref(), Some("Pro"));
        assert!(!a.shown_in_menu);

        assert!(matches!(
            store.rename_account(a.id, "b"),
            Err(StoreError::DuplicateAccount { label, .. }) if label == "b"
        ));
        assert!(matches!(
            store.set_shown_in_menu(AccountId::new(9), true),
            Err(StoreError::AccountNotFound(_))
        ));
    }

    #[test]
    fn remove_account_deletes_only_that_account() {
        let mut store = store();
        let a = store
            .insert_account(&new_account(ProviderId::Claude, "a"), t0())
            .unwrap();
        let b = store
            .insert_account(&new_account(ProviderId::Claude, "b"), t0())
            .unwrap();

        store.remove_account(a.id).unwrap();

        assert_eq!(store.accounts().unwrap(), vec![b]);
        assert!(matches!(
            store.remove_account(a.id),
            Err(StoreError::AccountNotFound(id)) if id == a.id
        ));
        assert!(matches!(
            store.remove_account(AccountId::new(7)),
            Err(StoreError::AccountNotFound(_))
        ));
    }
}
