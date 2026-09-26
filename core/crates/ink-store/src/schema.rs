//! The schema and its migrations.
//!
//! Migrations are append-only: once one has shipped it is never edited, because the database
//! holds the user's only copy of their records and a rewritten migration corrupts an archive on
//! someone else's machine. A change is a new entry at the end of [`MIGRATIONS`].
//!
//! Rules every table follows:
//! - **STRICT.** SQLite's default affinity stores "hello" in an INTEGER column without complaint;
//!   a transcript archive is the last place to find that out.
//! - **UUID text ids** for everything that leaves the store (records, notes, commitments), so rows
//!   can later be merged across machines without renumbering.
//! - **An explicit `seq INTEGER PRIMARY KEY`** where order of insertion or a stable rowid matters.
//!   Without one, `VACUUM` may renumber rowids, which would silently break the FTS index's link
//!   to its rows and the "in the order they were added" tie-breaks.
//! - **Times as INTEGER milliseconds**: Unix ms for wall-clock times, ms from the record's start
//!   for positions within it.
//! - **A DEFAULT on every NOT NULL column a later migration adds**, so no migration ever has to
//!   backfill a database it cannot see.

use rusqlite::{Connection, TransactionBehavior};

use crate::codec::Fail;

/// Every migration, in order. The database's `user_version` counts how many have run.
const MIGRATIONS: &[&str] = &[V1];

/// The schema version this build writes: the number of migrations. A database with a higher
/// `user_version` came from a newer build and is refused rather than guessed at.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

const V1: &str = "
CREATE TABLE record (
    id                 TEXT PRIMARY KEY NOT NULL,
    kind               TEXT NOT NULL CHECK (kind IN ('dictation', 'meeting', 'file_import')),
    title              TEXT,
    started_at_unix_ms INTEGER NOT NULL,
    ended_at_unix_ms   INTEGER,
    source_app         TEXT,
    audio_dir          TEXT,
    -- 1 while live; each supersede raises it. Segment rows carry the revision they belong to.
    revision           INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1)
) STRICT;
-- The Library: newest first, optionally one kind, paging on the start time.
CREATE INDEX record_by_start ON record (started_at_unix_ms, id);
CREATE INDEX record_by_kind_start ON record (kind, started_at_unix_ms, id);

-- Settled transcript text. Segments have no identity outside their record (a supersede replaces
-- them all), so they need no UUID; `seq` is the FTS index's stable rowid and the append order.
CREATE TABLE segment (
    seq       INTEGER PRIMARY KEY,
    record_id TEXT NOT NULL REFERENCES record (id) ON DELETE CASCADE,
    revision  INTEGER NOT NULL CHECK (revision >= 1),
    channel   TEXT NOT NULL CHECK (channel IN ('mic', 'far')),
    -- Where the engine placed it, ms from the record's start. Live finals carry real times too.
    start_ms  INTEGER NOT NULL CHECK (start_ms >= 0),
    end_ms    INTEGER NOT NULL CHECK (end_ms >= start_ms),
    text      TEXT NOT NULL,
    speaker   TEXT
) STRICT;
CREATE INDEX segment_by_record ON segment (record_id, revision, start_ms);

-- External-content FTS5: the index holds terms only and joins back to `segment`, so the
-- transcript has one home. unicode61 with diacritics removed, so 'cafe' finds 'Café'.
CREATE VIRTUAL TABLE segment_fts USING fts5 (
    text,
    content = 'segment',
    content_rowid = 'seq',
    tokenize = 'unicode61 remove_diacritics 2'
);
-- Remove a deleted row's entries from the index at once, instead of leaving delete markers
-- beside the original terms until a later merge: with the connection's `secure_delete`, the
-- words of deleted and superseded segments are then gone from the file (SQLite 3.44 and later).
-- The cost is on deletes only, and this option is nearly all of it. Measured on an M-series Mac,
-- release build, an index of 21,000 segments of about 20 words: superseding a 1,000-segment
-- meeting went from 10 ms to 240 ms, deleting one from 1 ms to 220 ms; one live append stayed
-- near 0.1 ms. Both run once per meeting on a worker thread, so privacy wins.
INSERT INTO segment_fts (segment_fts, rank) VALUES ('secure-delete', 1);
CREATE TRIGGER segment_fts_insert AFTER INSERT ON segment BEGIN
    INSERT INTO segment_fts (rowid, text) VALUES (new.seq, new.text);
END;
-- Also fires for rows removed by the record's ON DELETE CASCADE.
CREATE TRIGGER segment_fts_delete AFTER DELETE ON segment BEGIN
    INSERT INTO segment_fts (segment_fts, rowid, text) VALUES ('delete', old.seq, old.text);
END;
-- The store never updates a segment; this keeps the index honest if a later migration does.
CREATE TRIGGER segment_fts_update AFTER UPDATE ON segment BEGIN
    INSERT INTO segment_fts (segment_fts, rowid, text) VALUES ('delete', old.seq, old.text);
    INSERT INTO segment_fts (rowid, text) VALUES (new.seq, new.text);
END;

CREATE TABLE note (
    seq       INTEGER PRIMARY KEY,
    id        TEXT NOT NULL UNIQUE,
    record_id TEXT NOT NULL REFERENCES record (id) ON DELETE CASCADE,
    at_ms     INTEGER NOT NULL CHECK (at_ms >= 0),
    text      TEXT NOT NULL
) STRICT;
CREATE INDEX note_by_record ON note (record_id, at_ms, seq);

-- One summary per record; saving again replaces it.
CREATE TABLE summary (
    record_id           TEXT PRIMARY KEY NOT NULL REFERENCES record (id) ON DELETE CASCADE,
    text                TEXT NOT NULL,
    model               TEXT NOT NULL,
    created_at_unix_ms  INTEGER NOT NULL,
    -- The record's revision when it was saved: a summary of the live transcript and one of the
    -- offline pass are different artifacts. Not in the trait yet; kept so it is never lost.
    transcript_revision INTEGER NOT NULL
) STRICT;

-- Names for diarized speakers, per record.
CREATE TABLE speaker (
    record_id TEXT NOT NULL REFERENCES record (id) ON DELETE CASCADE,
    speaker   TEXT NOT NULL,
    name      TEXT NOT NULL,
    PRIMARY KEY (record_id, speaker)
) STRICT, WITHOUT ROWID;

CREATE TABLE commitment (
    seq            INTEGER PRIMARY KEY,
    id             TEXT NOT NULL UNIQUE,
    record_id      TEXT NOT NULL REFERENCES record (id) ON DELETE CASCADE,
    text           TEXT NOT NULL,
    owner          TEXT,
    due            TEXT,
    due_at_unix_ms INTEGER,
    -- The deduplicated 'said twice' case. When the commitment it points at goes (its record was
    -- deleted), this one is still owed, so it becomes open again rather than vanishing with it.
    merged_into    TEXT REFERENCES commitment (id) ON DELETE SET NULL,
    done           INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
    CHECK (merged_into IS NULL OR merged_into <> id)
) STRICT;
CREATE INDEX commitment_by_record ON commitment (record_id, seq);
-- For the ON DELETE SET NULL lookup. Partial, so the planner never picks it for the open list's
-- `merged_into IS NULL` over the index below.
CREATE INDEX commitment_by_merged_into ON commitment (merged_into) WHERE merged_into IS NOT NULL;
-- Owed and Today: open commitments, soonest due first, ties in the order they were added.
CREATE INDEX commitment_open ON commitment (due_at_unix_ms, seq)
    WHERE done = 0 AND merged_into IS NULL;

-- Provenance: where in the record a commitment was said, in order.
CREATE TABLE commitment_span (
    commitment_id TEXT NOT NULL REFERENCES commitment (id) ON DELETE CASCADE,
    ord           INTEGER NOT NULL CHECK (ord >= 0),
    channel       TEXT NOT NULL CHECK (channel IN ('mic', 'far')),
    start_ms      INTEGER NOT NULL CHECK (start_ms >= 0),
    end_ms        INTEGER NOT NULL CHECK (end_ms >= start_ms),
    PRIMARY KEY (commitment_id, ord)
) STRICT, WITHOUT ROWID;

CREATE TABLE setting (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT, WITHOUT ROWID;
";

/// Brings the database up to [`SCHEMA_VERSION`] in one immediate transaction.
///
/// The version is read inside the transaction, so two processes opening a fresh file at once
/// cannot both run the same migration: the second waits for the first and then finds nothing to
/// do. Every pending migration and the version bump commit together or not at all.
pub(crate) fn migrate(conn: &mut Connection) -> Result<(), Fail> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(Fail::backend(format!(
            "the database was written by a newer build (schema {version}, this build knows {SCHEMA_VERSION})"
        )));
    }
    let done = usize::try_from(version)
        .map_err(|_| Fail::backend(format!("invalid schema version {version}")))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(done) {
        tx.execute_batch(sql)?;
        let reached = i64::try_from(index + 1)
            .map_err(|_| Fail::backend("too many migrations".to_string()))?;
        tx.pragma_update(None, "user_version", reached)?;
    }
    tx.commit()?;
    Ok(())
}
