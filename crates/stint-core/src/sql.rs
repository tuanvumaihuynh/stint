use chrono::{DateTime, Utc};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

use crate::account::{AccountId, ProviderId};
use crate::quota::{Percentage, WindowKind};

/// UTC timestamp stored as Unix milliseconds; sub-millisecond precision is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Millis(pub(crate) DateTime<Utc>);

impl ToSql for Millis {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.0.timestamp_millis().into())
    }
}

impl FromSql for Millis {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let millis = value.as_i64()?;
        DateTime::from_timestamp_millis(millis)
            .map(Self)
            .ok_or(FromSqlError::OutOfRange(millis))
    }
}

impl ToSql for AccountId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.get().into())
    }
}

impl FromSql for AccountId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_i64().map(Self::new)
    }
}

impl ToSql for ProviderId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl FromSql for ProviderId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str()?.parse().map_err(FromSqlError::other)
    }
}

impl ToSql for WindowKind {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl FromSql for WindowKind {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str()?.parse().map_err(FromSqlError::other)
    }
}

impl ToSql for Percentage {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.get().into())
    }
}

impl FromSql for Percentage {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Self::new(value.as_f64()?).map_err(FromSqlError::other)
    }
}
