//! SQLite implementation of [`Store`]: WAL, STRICT tables, FTS5 search, versioned migrations and
//! the supersede guard.
//!
//! Open one [`SqliteStore`] per database, when the core starts, and share it (`Arc<dyn Store>`)
//! with everything that reads or writes records: the store is the core's one connection to the
//! file, not something each caller opens for itself.
//!
//! What the schema holds and why is in the `schema` module's documentation. Four decisions
//! differ from an earlier implementation of the same design:
//! - one connection owned by the core, instead of one per caller;
//! - search runs on the FTS5 index and ranks with bm25, instead of `LIKE` over every row;
//! - live segments are stored with the times the engine gave them. Nothing here rewrites a
//!   segment's `start_ms` or `end_ms`;
//! - deleted means deleted: SQLite's `secure_delete` and the index's `secure-delete` option
//!   overwrite the text of deleted and superseded rows instead of leaving it in free pages.
//!
//! Importers for the stores of earlier versions arrive in a later step.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod codec;
mod fts;
mod schema;

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use ink_core::store::{
    Commitment, CommitmentId, NewCommitment, NewRecord, Note, NoteId, Record, RecordId,
    RecordQuery, SearchHit, Segment, Span, Store, Summary, check_supersede,
};
use ink_core::{SpeakerId, StoreError};
use rusqlite::types::ValueRef;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior, params,
};

use codec::{Fail, channel_at, channel_text, kind_at, kind_text, ms, ms_at};

pub use schema::SCHEMA_VERSION;

/// How long a call waits for another process holding the write lock (a backup tool, a second
/// copy of the app) before it fails with a busy error.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The SQLite [`Store`].
///
/// **Threading.** Every method is a worker-thread call and may block on disk. The store is
/// `Send + Sync` and holds **one connection behind a mutex**: calls from different threads run
/// one at a time, each in its own transaction, and the lock is held only for the SQL of that
/// call and, in a supersede, the guard that must see the rows it replaces (ids are generated and
/// inputs converted before the lock is taken). One connection is enough
/// because every call is short. A reader pool would only pay off if a long read (a search over a
/// very large archive) measurably delayed live appends, and an in-memory database cannot be
/// shared between connections anyway, so the tests would stop exercising the real code path.
///
/// **WAL** (file databases) keeps a crash from corrupting the file and lets other processes read
/// while the app writes. `synchronous = FULL`: commits are rare (a live final every few seconds),
/// so an fsync per commit costs nothing noticeable, and power loss never undoes a committed one.
///
/// **A panic while the lock is held** poisons it. Every later call then returns
/// [`StoreError::Backend`] instead of panicking in the caller, and the core should reopen the
/// store.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Opens (creating if needed) the database at `path` and migrates it to [`SCHEMA_VERSION`].
    ///
    /// The directory must exist: the caller owns the data directory. On Unix the file is created
    /// owner-only (0600) before SQLite first touches it, and an existing file, WAL or shared-memory
    /// file is tightened to 0600 on every open. SQLite gives the WAL and shared-memory files it
    /// creates the database file's mode. A database written by a newer build is refused, not
    /// modified.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        files::create_owner_only(path)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut conn =
            Connection::open_with_flags(path, flags).map_err(|e| codec::sql_error("open", &e))?;
        configure(&mut conn, true).map_err(|e| e.into_store("open"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// An in-memory database with the same schema, for tests. It has no WAL (SQLite keeps an
    /// in-memory journal) and vanishes when the store is dropped.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let mut conn = Connection::open_in_memory().map_err(|e| codec::sql_error("open", &e))?;
        configure(&mut conn, false).map_err(|e| e.into_store("open"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Runs `f` with the connection. Only SQL and the row mapping it needs run under the lock.
    fn with<T>(
        &self,
        op: &'static str,
        f: impl FnOnce(&mut Connection) -> Result<T, Fail>,
    ) -> Result<T, StoreError> {
        let mut conn = self.conn.lock().map_err(|_| {
            StoreError::Backend(format!(
                "{op}: the store's lock was poisoned by an earlier panic"
            ))
        })?;
        f(&mut conn).map_err(|e| e.into_store(op))
    }

    /// Runs `f` in an immediate (write) transaction: all of it commits, or none of it.
    fn write<T>(
        &self,
        op: &'static str,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, Fail>,
    ) -> Result<T, StoreError> {
        self.with(op, |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let out = f(&tx)?;
            tx.commit()?;
            Ok(out)
        })
    }

    /// Runs `f` in a deferred (read) transaction, so its statements see one snapshot.
    fn read<T>(
        &self,
        op: &'static str,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, Fail>,
    ) -> Result<T, StoreError> {
        self.with(op, |conn| {
            let tx = conn.transaction()?;
            let out = f(&tx)?;
            tx.commit()?;
            Ok(out)
        })
    }
}

/// Per-connection settings, then the migrations.
fn configure(conn: &mut Connection, file: bool) -> Result<(), Fail> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    if file {
        let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Fail::backend(
                "open: the database file cannot use WAL mode".to_string(),
            ));
        }
    }
    conn.pragma_update(None, "synchronous", "FULL")?;
    // Deleted means deleted: SQLite zeroes the space a deleted or replaced row leaves, instead of
    // leaving the old text in free pages. Together with the search index's own `secure-delete`
    // option (set in the schema) no superseded or deleted transcript text stays in the file.
    // This pragma adds about a tenth to a delete; the index option is the expensive half, and the
    // schema records the measurement.
    conn.pragma_update(None, "secure_delete", true)?;
    let scrubbing: bool = conn.pragma_query_value(None, "secure_delete", |row| row.get(0))?;
    if !scrubbing {
        return Err(Fail::backend(
            "open: this SQLite build cannot overwrite deleted content".to_string(),
        ));
    }
    // Off by default in SQLite, per connection. The cascades that delete a record's rows depend
    // on it, so it is checked rather than assumed.
    conn.pragma_update(None, "foreign_keys", true)?;
    let enforced: bool = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    if !enforced {
        return Err(Fail::backend(
            "open: this SQLite build does not enforce foreign keys".to_string(),
        ));
    }
    schema::migrate(conn)
}

mod files {
    use std::path::Path;

    use ink_core::StoreError;

    /// Creates the database file owner-only before SQLite opens it, so it never exists with the
    /// umask's wider mode, and tightens files an older build or another tool left readable.
    #[cfg(unix)]
    pub(super) fn create_owner_only(path: &Path) -> Result<(), StoreError> {
        use std::fs::{OpenOptions, Permissions};
        use std::io::ErrorKind;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let owner_only = || Permissions::from_mode(0o600);
        // An empty file is a valid new database to SQLite. Never truncate an existing one.
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(io_error)?;
        file.set_permissions(owner_only()).map_err(io_error)?;
        for suffix in ["-wal", "-shm"] {
            let mut side = path.as_os_str().to_owned();
            side.push(suffix);
            match std::fs::set_permissions(&side, owner_only()) {
                Err(e) if e.kind() != ErrorKind::NotFound => return Err(io_error(e)),
                _ => {}
            }
        }
        Ok(())
    }

    /// On Windows the file inherits the ACL of the user's data directory, which is already
    /// private to the user.
    #[cfg(not(unix))]
    pub(super) fn create_owner_only(_path: &Path) -> Result<(), StoreError> {
        Ok(())
    }

    /// The error kind only: a path can name the user.
    #[cfg(unix)]
    fn io_error(e: std::io::Error) -> StoreError {
        StoreError::Backend(format!(
            "open: could not prepare the database file ({:?})",
            e.kind()
        ))
    }
}

/// A new UUID v4 as lowercase hyphenated text. The random bytes come from the OS; a failure is
/// an error, never a panic.
fn new_id() -> Result<String, StoreError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| {
        StoreError::Backend("could not read the system's random source for a new id".to_string())
    })?;
    Ok(uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .hyphenated()
        .to_string())
}

/// A `LIMIT` value. `usize` beyond `i64` means "no limit" in practice.
fn sql_limit(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

/// `NotFound` when an update or delete touched no row.
fn changed(rows: usize) -> Result<(), Fail> {
    if rows == 0 {
        Err(StoreError::NotFound.into())
    } else {
        Ok(())
    }
}

/// The record's current revision, or `NotFound`.
fn revision(conn: &Connection, id: &RecordId) -> Result<u32, Fail> {
    conn.query_row(
        "SELECT revision FROM record WHERE id = ?1",
        [&id.0],
        |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound.into())
}

/// The columns [`record_at`] reads, as a macro so queries can be `concat!`-ed constants.
macro_rules! record_columns {
    () => {
        "id, kind, title, started_at_unix_ms, ended_at_unix_ms, source_app, audio_dir, revision"
    };
}

fn record_at(row: &Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        id: RecordId(row.get(0)?),
        kind: kind_at(row, 1)?,
        title: row.get(2)?,
        started_at_unix_ms: row.get(3)?,
        ended_at_unix_ms: row.get(4)?,
        source_app: row.get(5)?,
        audio_dir: row.get(6)?,
        revision: row.get(7)?,
    })
}

/// A segment converted for binding, before the lock is taken.
struct SegmentRow<'a> {
    channel: &'static str,
    start_ms: i64,
    end_ms: i64,
    text: &'a str,
    speaker: Option<&'a str>,
}

fn segment_rows(segments: &[Segment]) -> Result<Vec<SegmentRow<'_>>, StoreError> {
    segments
        .iter()
        .map(|s| {
            Ok(SegmentRow {
                channel: channel_text(s.channel),
                start_ms: ms(s.start_ms)?,
                end_ms: ms(s.end_ms)?,
                text: &s.text,
                speaker: s.speaker.as_ref().map(|sp| sp.0.as_str()),
            })
        })
        .collect()
}

fn insert_segments(
    conn: &Connection,
    id: &RecordId,
    revision: u32,
    rows: &[SegmentRow<'_>],
) -> Result<(), Fail> {
    let mut insert = conn.prepare(
        "INSERT INTO segment (record_id, revision, channel, start_ms, end_ms, text, speaker)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for row in rows {
        insert.execute(params![
            id.0,
            revision,
            row.channel,
            row.start_ms,
            row.end_ms,
            row.text,
            row.speaker
        ])?;
    }
    Ok(())
}

/// One revision's segments by start time; at the same start the mic comes first, then the order
/// they were written.
fn segments_of(conn: &Connection, id: &RecordId, revision: u32) -> Result<Vec<Segment>, Fail> {
    let mut select = conn.prepare(
        "SELECT channel, start_ms, end_ms, text, speaker FROM segment
         WHERE record_id = ?1 AND revision = ?2
         ORDER BY start_ms, CASE channel WHEN 'mic' THEN 0 ELSE 1 END, seq",
    )?;
    let segments = select
        .query_map(params![id.0, revision], |row| {
            Ok(Segment {
                channel: channel_at(row, 0)?,
                start_ms: ms_at(row, 1)?,
                end_ms: ms_at(row, 2)?,
                text: row.get(3)?,
                speaker: row.get::<_, Option<String>>(4)?.map(SpeakerId),
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(segments)
}

/// A commitment converted for binding, before the lock is taken.
struct CommitmentRow<'a> {
    id: String,
    item: &'a NewCommitment,
    spans: Vec<(&'static str, i64, i64)>,
}

/// The columns [`commitments_from`] reads: a commitment joined with its spans. Queries order by
/// commitment, then `sp.ord`, so each commitment's rows are adjacent.
macro_rules! commitment_columns {
    () => {
        "c.id, c.record_id, c.text, c.owner, c.due, c.due_at_unix_ms, c.merged_into, c.done, \
         sp.channel, sp.start_ms, sp.end_ms"
    };
}

fn commitments_from(
    select: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> Result<Vec<Commitment>, Fail> {
    let mut rows = select.query(params)?;
    let mut out: Vec<Commitment> = Vec::new();
    while let Some(row) = rows.next()? {
        let id = row.get_ref(0)?.as_str().map_err(rusqlite::Error::from)?;
        if out.last().is_none_or(|c| c.id.0 != id) {
            out.push(Commitment {
                id: CommitmentId(id.to_string()),
                record: RecordId(row.get(1)?),
                text: row.get(2)?,
                owner: row.get(3)?,
                due: row.get(4)?,
                due_at_unix_ms: row.get(5)?,
                provenance: Vec::new(),
                merged_into: row.get::<_, Option<String>>(6)?.map(CommitmentId),
                done: row.get(7)?,
            });
        }
        if !matches!(row.get_ref(8)?, ValueRef::Null)
            && let Some(commitment) = out.last_mut()
        {
            commitment.provenance.push(Span {
                channel: channel_at(row, 8)?,
                start_ms: ms_at(row, 9)?,
                end_ms: ms_at(row, 10)?,
            });
        }
    }
    Ok(out)
}

impl Store for SqliteStore {
    fn create_record(&self, record: NewRecord) -> Result<RecordId, StoreError> {
        let id = new_id()?;
        self.with("create_record", |conn| {
            conn.execute(
                "INSERT INTO record (id, kind, title, started_at_unix_ms, source_app, audio_dir)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    kind_text(record.kind),
                    record.title,
                    record.started_at_unix_ms,
                    record.source_app,
                    record.audio_dir
                ],
            )?;
            Ok(())
        })?;
        Ok(RecordId(id))
    }

    fn record(&self, id: &RecordId) -> Result<Option<Record>, StoreError> {
        self.with("record", |conn| {
            Ok(conn
                .query_row(
                    concat!("SELECT ", record_columns!(), " FROM record WHERE id = ?1"),
                    [&id.0],
                    record_at,
                )
                .optional()?)
        })
    }

    fn records(&self, query: &RecordQuery) -> Result<Vec<Record>, StoreError> {
        // Fixed fragments only, so each filter can use its index; every value is a parameter.
        let kind = if query.kind.is_some() {
            "kind = ?1"
        } else {
            "?1 IS NULL"
        };
        // Keyset paging: strictly after the cursor in (start, id) descending order. The row-value
        // comparison is a range on the (kind,) start, id index, so records sharing a millisecond
        // are neither skipped nor repeated. BINARY collation orders ids as `RecordId`'s `Ord` does.
        let before = if query.before.is_some() {
            "(started_at_unix_ms, id) < (?2, ?3)"
        } else {
            "?2 IS NULL AND ?3 IS NULL"
        };
        let sql = format!(
            concat!(
                "SELECT ",
                record_columns!(),
                " FROM record WHERE {} AND {} ORDER BY started_at_unix_ms DESC, id DESC LIMIT ?4"
            ),
            kind, before
        );
        let cursor = query.before.as_ref();
        self.with("records", |conn| {
            let mut select = conn.prepare(&sql)?;
            let records = select
                .query_map(
                    params![
                        query.kind.map(kind_text),
                        cursor.map(|c| c.started_at_unix_ms),
                        cursor.map(|c| c.id.0.as_str()),
                        sql_limit(query.limit)
                    ],
                    record_at,
                )?
                .collect::<Result<_, _>>()?;
            Ok(records)
        })
    }

    fn set_title(&self, id: &RecordId, title: &str) -> Result<(), StoreError> {
        self.with("set_title", |conn| {
            changed(conn.execute(
                "UPDATE record SET title = ?2 WHERE id = ?1",
                params![id.0, title],
            )?)
        })
    }

    fn finish_record(&self, id: &RecordId, ended_at_unix_ms: i64) -> Result<(), StoreError> {
        self.with("finish_record", |conn| {
            changed(conn.execute(
                "UPDATE record SET ended_at_unix_ms = ?2 WHERE id = ?1",
                params![id.0, ended_at_unix_ms],
            )?)
        })
    }

    fn delete_record(&self, id: &RecordId) -> Result<(), StoreError> {
        // The foreign keys cascade to segments (and through a trigger, the search index), notes,
        // the summary, speaker names and commitments with their spans. Commitments elsewhere
        // that were merged into this record's are un-merged, since they are still owed.
        self.write("delete_record", |tx| {
            changed(tx.execute("DELETE FROM record WHERE id = ?1", [&id.0])?)
        })
    }

    fn append_segments(&self, id: &RecordId, segments: &[Segment]) -> Result<(), StoreError> {
        let rows = segment_rows(segments)?;
        self.write("append_segments", |tx| {
            let current = revision(tx, id)?;
            insert_segments(tx, id, current, &rows)
        })
    }

    fn segments(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError> {
        self.read("segments", |tx| {
            let current = revision(tx, id)?;
            segments_of(tx, id, current)
        })
    }

    fn supersede(&self, id: &RecordId, segments: &[Segment]) -> Result<u32, StoreError> {
        let rows = segment_rows(segments)?;
        // The guard has to see the rows this transaction replaces, so it runs under the lock.
        self.write("supersede", |tx| {
            let current = revision(tx, id)?;
            let previous = segments_of(tx, id, current)?;
            // A refusal returns before anything is written, and dropping the transaction rolls
            // back regardless.
            check_supersede(&previous, segments)?;
            let next = current
                .checked_add(1)
                .ok_or_else(|| Fail::backend("supersede: revision overflow".to_string()))?;
            insert_segments(tx, id, next, &rows)?;
            // Only with the new revision written do the old rows go.
            tx.execute(
                "DELETE FROM segment WHERE record_id = ?1 AND revision <> ?2",
                params![id.0, next],
            )?;
            tx.execute(
                "UPDATE record SET revision = ?2 WHERE id = ?1",
                params![id.0, next],
            )?;
            Ok(next)
        })
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError> {
        let Some(expression) = fts::match_expression(query) else {
            return Ok(Vec::new());
        };
        if limit == 0 {
            return Ok(Vec::new());
        }
        self.read("search", |tx| {
            // bm25 is lower for a better match. The revision join is belt and braces: outside a
            // supersede's own transaction only the current revision's rows exist.
            let mut select = tx.prepare(
                "SELECT r.id, r.title, r.started_at_unix_ms, s.start_ms, s.text
                 FROM segment_fts
                 JOIN segment AS s ON s.seq = segment_fts.rowid
                 JOIN record AS r ON r.id = s.record_id AND r.revision = s.revision
                 WHERE segment_fts MATCH ?1
                 ORDER BY bm25(segment_fts), r.started_at_unix_ms DESC, s.seq
                 LIMIT ?2",
            )?;
            let hits = select
                .query_map(params![expression, sql_limit(limit)], |row| {
                    Ok(SearchHit {
                        record: RecordId(row.get(0)?),
                        title: row.get(1)?,
                        started_at_unix_ms: row.get(2)?,
                        start_ms: ms_at(row, 3)?,
                        snippet: row.get(4)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            Ok(hits)
        })
    }

    fn add_note(&self, id: &RecordId, at_ms: u64, text: &str) -> Result<NoteId, StoreError> {
        let at = ms(at_ms)?;
        let note = new_id()?;
        self.write("add_note", |tx| {
            revision(tx, id)?;
            tx.execute(
                "INSERT INTO note (id, record_id, at_ms, text) VALUES (?1, ?2, ?3, ?4)",
                params![note, id.0, at, text],
            )?;
            Ok(())
        })?;
        Ok(NoteId(note))
    }

    fn update_note(&self, id: &NoteId, text: &str) -> Result<(), StoreError> {
        self.with("update_note", |conn| {
            changed(conn.execute(
                "UPDATE note SET text = ?2 WHERE id = ?1",
                params![id.0, text],
            )?)
        })
    }

    fn delete_note(&self, id: &NoteId) -> Result<(), StoreError> {
        self.with("delete_note", |conn| {
            changed(conn.execute("DELETE FROM note WHERE id = ?1", [&id.0])?)
        })
    }

    fn notes(&self, id: &RecordId) -> Result<Vec<Note>, StoreError> {
        self.read("notes", |tx| {
            revision(tx, id)?;
            let mut select = tx.prepare(
                "SELECT id, at_ms, text FROM note WHERE record_id = ?1 ORDER BY at_ms, seq",
            )?;
            let notes = select
                .query_map([&id.0], |row| {
                    Ok(Note {
                        id: NoteId(row.get(0)?),
                        record: id.clone(),
                        at_ms: ms_at(row, 1)?,
                        text: row.get(2)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            Ok(notes)
        })
    }

    fn save_summary(&self, id: &RecordId, summary: &Summary) -> Result<(), StoreError> {
        self.write("save_summary", |tx| {
            let current = revision(tx, id)?;
            tx.execute(
                "INSERT INTO summary (record_id, text, model, created_at_unix_ms, transcript_revision)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (record_id) DO UPDATE SET
                     text = excluded.text,
                     model = excluded.model,
                     created_at_unix_ms = excluded.created_at_unix_ms,
                     transcript_revision = excluded.transcript_revision",
                params![
                    id.0,
                    summary.text,
                    summary.model,
                    summary.created_at_unix_ms,
                    current
                ],
            )?;
            Ok(())
        })
    }

    fn summary(&self, id: &RecordId) -> Result<Option<Summary>, StoreError> {
        self.read("summary", |tx| {
            revision(tx, id)?;
            Ok(tx
                .query_row(
                    "SELECT text, model, created_at_unix_ms FROM summary WHERE record_id = ?1",
                    [&id.0],
                    |row| {
                        Ok(Summary {
                            text: row.get(0)?,
                            model: row.get(1)?,
                            created_at_unix_ms: row.get(2)?,
                        })
                    },
                )
                .optional()?)
        })
    }

    fn set_speaker_name(
        &self,
        id: &RecordId,
        speaker: &SpeakerId,
        name: &str,
    ) -> Result<(), StoreError> {
        self.write("set_speaker_name", |tx| {
            revision(tx, id)?;
            tx.execute(
                "INSERT INTO speaker (record_id, speaker, name) VALUES (?1, ?2, ?3)
                 ON CONFLICT (record_id, speaker) DO UPDATE SET name = excluded.name",
                params![id.0, speaker.0, name],
            )?;
            Ok(())
        })
    }

    fn speaker_names(&self, id: &RecordId) -> Result<Vec<(SpeakerId, String)>, StoreError> {
        self.read("speaker_names", |tx| {
            revision(tx, id)?;
            // BINARY collation compares UTF-8 bytes, the same order as `SpeakerId`'s `Ord`.
            let mut select = tx.prepare(
                "SELECT speaker, name FROM speaker WHERE record_id = ?1 ORDER BY speaker",
            )?;
            let names = select
                .query_map([&id.0], |row| Ok((SpeakerId(row.get(0)?), row.get(1)?)))?
                .collect::<Result<_, _>>()?;
            Ok(names)
        })
    }

    fn add_commitments(
        &self,
        id: &RecordId,
        items: &[NewCommitment],
    ) -> Result<Vec<CommitmentId>, StoreError> {
        let rows = items
            .iter()
            .map(|item| {
                Ok(CommitmentRow {
                    id: new_id()?,
                    item,
                    spans: item
                        .provenance
                        .iter()
                        .map(|s| Ok((channel_text(s.channel), ms(s.start_ms)?, ms(s.end_ms)?)))
                        .collect::<Result<_, StoreError>>()?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        self.write("add_commitments", |tx| {
            revision(tx, id)?;
            let mut insert = tx.prepare(
                "INSERT INTO commitment (id, record_id, text, owner, due, due_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let mut insert_span = tx.prepare(
                "INSERT INTO commitment_span (commitment_id, ord, channel, start_ms, end_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for row in &rows {
                insert.execute(params![
                    row.id,
                    id.0,
                    row.item.text,
                    row.item.owner,
                    row.item.due,
                    row.item.due_at_unix_ms
                ])?;
                for (ord, (channel, start, end)) in (0_i64..).zip(&row.spans) {
                    insert_span.execute(params![row.id, ord, channel, start, end])?;
                }
            }
            Ok(())
        })?;
        Ok(rows.into_iter().map(|r| CommitmentId(r.id)).collect())
    }

    fn commitments(&self, id: &RecordId) -> Result<Vec<Commitment>, StoreError> {
        self.read("commitments", |tx| {
            revision(tx, id)?;
            let mut select = tx.prepare(concat!(
                "SELECT ",
                commitment_columns!(),
                " FROM commitment AS c
                 LEFT JOIN commitment_span AS sp ON sp.commitment_id = c.id
                 WHERE c.record_id = ?1
                 ORDER BY c.seq, sp.ord"
            ))?;
            commitments_from(&mut select, [&id.0])
        })
    }

    fn open_commitments(&self, limit: usize) -> Result<Vec<Commitment>, StoreError> {
        self.read("open_commitments", |tx| {
            // `seq` is the insertion order, so ties never fall back on the random UUIDs.
            let mut select = tx.prepare(concat!(
                "WITH owed AS (
                     SELECT * FROM commitment
                     WHERE done = 0 AND merged_into IS NULL
                     ORDER BY due_at_unix_ms NULLS LAST, seq
                     LIMIT ?1
                 )
                 SELECT ",
                commitment_columns!(),
                " FROM owed AS c
                 LEFT JOIN commitment_span AS sp ON sp.commitment_id = c.id
                 ORDER BY c.due_at_unix_ms NULLS LAST, c.seq, sp.ord"
            ))?;
            commitments_from(&mut select, [sql_limit(limit)])
        })
    }

    fn set_commitment_done(&self, id: &CommitmentId, done: bool) -> Result<(), StoreError> {
        self.with("set_commitment_done", |conn| {
            changed(conn.execute(
                "UPDATE commitment SET done = ?2 WHERE id = ?1",
                params![id.0, done],
            )?)
        })
    }

    fn merge_commitment(&self, id: &CommitmentId, into: &CommitmentId) -> Result<(), StoreError> {
        if id == into {
            return Err(StoreError::Invalid(
                "a commitment cannot be merged into itself".to_string(),
            ));
        }
        self.write("merge_commitment", |tx| {
            let into_is_merged: bool = tx
                .query_row(
                    "SELECT merged_into IS NOT NULL FROM commitment WHERE id = ?1",
                    [&into.0],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or(StoreError::NotFound)?;
            // Merges point at a canonical commitment, so they can never form a cycle.
            if into_is_merged {
                return Err(StoreError::Invalid(
                    "merge into the canonical commitment, not one that is itself merged"
                        .to_string(),
                )
                .into());
            }
            changed(tx.execute(
                "UPDATE commitment SET merged_into = ?2 WHERE id = ?1",
                params![id.0, into.0],
            )?)?;
            // Flatten in the same transaction: what was folded into `id` now points at `into`,
            // so `merged_into` always names an unmerged canonical (no chains, no cycles).
            tx.execute(
                "UPDATE commitment SET merged_into = ?2 WHERE merged_into = ?1",
                params![id.0, into.0],
            )?;
            Ok(())
        })
    }

    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.with("setting", |conn| {
            Ok(conn
                .query_row("SELECT value FROM setting WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .optional()?)
        })
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.with("set_setting", |conn| {
            conn.execute(
                "INSERT INTO setting (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;

    use super::*;

    #[test]
    fn foreign_keys_are_enforced_on_the_store_connection() {
        let store = SqliteStore::open_in_memory().unwrap();
        let conn = store.conn.lock().unwrap();
        let on: bool = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert!(on);
        let orphan = conn.execute(
            "INSERT INTO note (id, record_id, at_ms, text) VALUES ('n', 'no-such-record', 0, 'x')",
            [],
        );
        assert!(orphan.is_err());
    }

    #[test]
    fn deleted_text_is_overwritten_by_sqlite_and_the_search_index() {
        let store = SqliteStore::open_in_memory().unwrap();
        let conn = store.conn.lock().unwrap();
        let pragma: i64 = conn
            .pragma_query_value(None, "secure_delete", |row| row.get(0))
            .unwrap();
        assert_eq!(pragma, 1);
        let fts: i64 = conn
            .query_row(
                "SELECT v FROM segment_fts_config WHERE k = 'secure-delete'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts, 1);
    }

    #[test]
    fn a_poisoned_lock_is_a_backend_error_not_a_panic() {
        let store = SqliteStore::open_in_memory().unwrap();
        let poisoned = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = store.conn.lock().unwrap();
            panic!("a bug while holding the store's lock");
        }));
        assert!(poisoned.is_err());
        assert!(store.conn.is_poisoned());
        match store.setting("anything") {
            Err(StoreError::Backend(message)) => {
                assert!(message.starts_with("setting: "), "{message}");
                assert!(message.contains("poisoned"), "{message}");
            }
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[test]
    fn ids_are_uuid_v4_text() {
        let a = new_id().unwrap();
        let b = new_id().unwrap();
        assert_ne!(a, b);
        let parsed = uuid::Uuid::parse_str(&a).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
        assert_eq!(a, parsed.hyphenated().to_string(), "lowercase, hyphenated");
    }
}
