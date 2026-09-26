//! Conversions between the trait's types and SQLite values, and the error mapping.

use ink_core::{Channel, RecordKind, StoreError};
use rusqlite::Row;
use rusqlite::types::Type;

/// Why a store call failed, before it leaves the crate: either the store's own verdict
/// (`NotFound`, a refused supersede) or an SQLite error still to be mapped. Keeping SQLite's
/// error until the edge lets every call site use `?` on both and name its operation once.
pub(crate) enum Fail {
    Store(StoreError),
    Sql(rusqlite::Error),
}

impl Fail {
    pub(crate) fn backend(message: String) -> Self {
        Self::Store(StoreError::Backend(message))
    }

    pub(crate) fn into_store(self, op: &'static str) -> StoreError {
        match self {
            Self::Store(e) => e,
            Self::Sql(e) => sql_error(op, &e),
        }
    }
}

impl From<rusqlite::Error> for Fail {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}

impl From<StoreError> for Fail {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// Maps an SQLite error to [`StoreError::Backend`], naming the operation and SQLite's result code
/// and nothing else. SQLite's messages can quote a fragment of the query or a value near the
/// failure (an FTS5 syntax error quotes the search text), and store errors end up in logs (I5),
/// so no message text from SQLite is ever copied.
pub(crate) fn sql_error(op: &str, e: &rusqlite::Error) -> StoreError {
    use rusqlite::Error as E;
    let what = match e {
        E::SqliteFailure(err, _) => format!("SQLite {:?} (code {})", err.code, err.extended_code),
        E::SqlInputError { error, .. } => format!(
            "SQLite rejected a statement: {:?} (code {})",
            error.code, error.extended_code
        ),
        E::QueryReturnedNoRows => "no row where one was expected".to_string(),
        E::QueryReturnedMoreThanOneRow => "several rows where one was expected".to_string(),
        E::FromSqlConversionFailure(..)
        | E::InvalidColumnType(..)
        | E::IntegralValueOutOfRange(..)
        | E::Utf8Error(..) => "a stored value has an unexpected type or range".to_string(),
        _ => "unexpected SQLite error".to_string(),
    };
    StoreError::Backend(format!("{op}: {what}"))
}

/// A time or position for an INTEGER column. SQLite integers are signed 64-bit, so a `u64`
/// above `i64::MAX` cannot be stored; it is refused rather than wrapped.
pub(crate) fn ms(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Invalid("a time is beyond the range the store can hold".into()))
}

/// A stretch (`start_ms`, `end_ms`) for binding: both in range, and the end not before the start.
pub(crate) fn stretch(start_ms: u64, end_ms: u64) -> Result<(i64, i64), StoreError> {
    let (start, end) = (ms(start_ms)?, ms(end_ms)?);
    if end < start {
        return Err(StoreError::Invalid(
            "a stretch ends before it starts".into(),
        ));
    }
    Ok((start, end))
}

/// A stored time or position back as `u64`. The schema's CHECKs keep them non-negative.
pub(crate) fn ms_at(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

pub(crate) fn channel_text(channel: Channel) -> &'static str {
    match channel {
        Channel::Mic => "mic",
        Channel::Far => "far",
    }
}

pub(crate) fn kind_text(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Dictation => "dictation",
        RecordKind::Meeting => "meeting",
        RecordKind::FileImport => "file_import",
    }
}

/// The error for a stored value the schema's CHECK constraints should have made impossible.
fn corrupt(index: usize) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, "unknown enum value".into())
}

pub(crate) fn channel_at(row: &Row<'_>, index: usize) -> rusqlite::Result<Channel> {
    match row.get_ref(index)?.as_str().map_err(|_| corrupt(index))? {
        "mic" => Ok(Channel::Mic),
        "far" => Ok(Channel::Far),
        _ => Err(corrupt(index)),
    }
}

pub(crate) fn kind_at(row: &Row<'_>, index: usize) -> rusqlite::Result<RecordKind> {
    match row.get_ref(index)?.as_str().map_err(|_| corrupt(index))? {
        "dictation" => Ok(RecordKind::Dictation),
        "meeting" => Ok(RecordKind::Meeting),
        "file_import" => Ok(RecordKind::FileImport),
        _ => Err(corrupt(index)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_errors_never_carry_query_text() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(x)")
            .unwrap();
        // FTS5 quotes a word of the search text back in its own message.
        let err = conn
            .query_row(
                "SELECT count(*) FROM t WHERE t MATCH ?1",
                ["confidential: launch"],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_err();
        assert!(err.to_string().contains("confidential"), "{err}");
        let StoreError::Backend(message) = sql_error("search", &err) else {
            panic!("expected Backend")
        };
        assert!(message.starts_with("search: "), "{message}");
        assert!(!message.contains("confidential"), "{message}");

        // A statement error quotes the SQL and the offending token.
        let err = conn
            .execute("INSERT INTO nowhere VALUES ('secret')", [])
            .unwrap_err();
        let StoreError::Backend(message) = sql_error("insert", &err) else {
            panic!("expected Backend")
        };
        assert!(
            !message.contains("secret") && !message.contains("nowhere"),
            "{message}"
        );
    }

    #[test]
    fn times_beyond_i64_are_invalid() {
        assert_eq!(ms(0), Ok(0));
        assert_eq!(ms(i64::MAX as u64), Ok(i64::MAX));
        assert!(matches!(ms(u64::MAX), Err(StoreError::Invalid(_))));
    }

    #[test]
    fn a_stretch_may_be_empty_but_not_reversed() {
        assert_eq!(stretch(5, 5), Ok((5, 5)));
        assert_eq!(stretch(5, 9), Ok((5, 9)));
        assert!(matches!(stretch(9, 5), Err(StoreError::Invalid(_))));
        assert!(matches!(stretch(0, u64::MAX), Err(StoreError::Invalid(_))));
    }
}
