//! Small encoding helpers shared by the tables.

use std::path::Path;

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, Params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A scope's key column: `""` for global, the project id otherwise.
pub(crate) fn scope_key(s: &orch_team::Scope) -> String {
    s.project().unwrap_or_default().to_string()
}

/// The inverse of [`scope_key`].
pub(crate) fn scope_of(key: String) -> orch_team::Scope {
    if key.is_empty() { orch_team::Scope::Global } else { orch_team::Scope::Project(key) }
}

/// Epoch milliseconds of a timestamp (the `*_ms` columns).
pub(crate) fn ms(t: governor::jiff::Timestamp) -> i64 {
    t.as_millisecond()
}

/// [`json_row`] where a row that doesn't decode counts as no row.
pub(crate) fn json_row_lenient<T: DeserializeOwned>(
    c: &Connection,
    sql: &str,
    params: impl Params,
) -> Result<Option<T>> {
    let s: Option<String> = c.query_row(sql, params, |r| r.get(0)).optional()?;
    Ok(s.and_then(|s| serde_json::from_str(&s).ok()))
}

pub(crate) fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// JSON text for a column (`null` if it can't be encoded).
pub(crate) fn js<T: Serialize + ?Sized>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

pub(crate) fn from_js<T: DeserializeOwned>(s: &str) -> rusqlite::Result<T> {
    serde_json::from_str(s)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into()))
}

/// A nullable integer column as `u64`.
pub(crate) fn opt_u64(r: &rusqlite::Row, i: usize) -> rusqlite::Result<Option<u64>> {
    Ok(r.get::<_, Option<i64>>(i)?.map(|v| v as u64))
}

/// One row's JSON `data` column, decoded (`None` when there is no row).
pub(crate) fn json_row<T: DeserializeOwned>(
    c: &Connection,
    sql: &str,
    params: impl Params,
    what: &str,
) -> Result<Option<T>> {
    let s: Option<String> = c.query_row(sql, params, |r| r.get(0)).optional()?;
    s.map(|s| serde_json::from_str(&s).map_err(|e| anyhow!("{what}: {e}"))).transpose()
}

/// A unit enum's serde name (`Tier::High` → `high`); for an externally
/// tagged variant with data, its tag.
pub fn tag<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        Ok(Value::Object(m)) => m.keys().next().cloned().unwrap_or_default(),
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

/// The inverse of [`tag`] for unit enums (`"high"` → `Tier::High`).
pub fn untag<T: DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(Value::String(s.to_string())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_round_trip() {
        assert_eq!(tag(&governor::Tier::High), "high");
        assert_eq!(untag::<governor::Tier>("high"), Some(governor::Tier::High));
        assert_eq!(untag::<governor::Tier>("nope"), None);
    }
}
