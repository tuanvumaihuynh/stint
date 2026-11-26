use rusqlite::{Connection, OptionalExtension, params};

use crate::error::StoreError;

const SELECT: &str = "SELECT value FROM settings WHERE key = ?1";

const UPSERT: &str = "
    INSERT INTO settings (key, value) VALUES (?1, ?2)
    ON CONFLICT (key) DO UPDATE SET value = excluded.value";

pub(super) fn get(conn: &Connection, key: &str) -> Result<Option<String>, StoreError> {
    let mut select = conn.prepare_cached(SELECT)?;
    let value = select.query_row([key], |row| row.get(0)).optional()?;
    Ok(value)
}

pub(super) fn set(conn: &Connection, key: &str, value: &str) -> Result<(), StoreError> {
    conn.prepare_cached(UPSERT)?.execute(params![key, value])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::store;
    use crate::store::QuotaStore;

    #[test]
    fn settings_start_unset_and_keep_the_last_value() {
        let mut store = store();
        assert_eq!(store.setting("refresh_minutes").unwrap(), None);
        store.set_setting("refresh_minutes", "5").unwrap();
        store.set_setting("refresh_minutes", "10").unwrap();
        assert_eq!(
            store.setting("refresh_minutes").unwrap().as_deref(),
            Some("10")
        );
    }
}
