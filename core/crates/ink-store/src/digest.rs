//! [`Store::digests`](ink_core::Store::digests) and
//! [`Store::commitment_states`](ink_core::Store::commitment_states) on SQLite.
//!
//! Digests are kept in `record_digest` (schema V5) and dropped by triggers whenever what they
//! count changes, so a stats read only counts the records that changed since the last one.
//! Measured on an M-series Mac, release build, with a library of 75,000 dictations of 30 words and
//! 500 meetings of 1,000 lines: counting every transcript took 690 ms; with every digest kept, a
//! read takes about 15 ms.

use ink_core::stats::{ChannelDigest, CommitmentState, RecordDigest, TranscriptDigest, digest};
use ink_core::{CommitmentId, RecordId, StoreError};
use rusqlite::{Connection, Row, params};

use crate::codec::{Fail, kind_at, kind_text};
use crate::{SqliteStore, segments_of};

/// How many records' digests one transaction counts when they are missing (the first read after
/// an upgrade, or after a large import). The store's one lock is let go between batches, so a
/// live meeting's append waits for one batch at most: about 40 ms for 2,000 dictations, measured
/// as above (the first read of that library took 1.4 s in all, once).
pub const DIGEST_BATCH: usize = 2_000;

fn u64_at(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let v: i64 = row.get(index)?;
    // The schema's CHECKs keep every count at or above 0.
    Ok(v.max(0) as u64)
}

fn count(conn: &Connection, table: &str) -> Result<i64, Fail> {
    Ok(conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
}

/// Whether some record has no kept digest. Every digest row names a record that exists (the
/// cascade) and at most once (its key), so equal counts mean every record has one.
fn any_missing(conn: &Connection) -> Result<bool, Fail> {
    Ok(count(conn, "record_digest")? < count(conn, "record")?)
}

/// The records with no kept digest, at most `limit`, each with its digest counted now.
fn count_missing(conn: &Connection, limit: i64) -> Result<Vec<RecordDigest>, Fail> {
    let mut missing = conn.prepare(
        "SELECT id, kind, started_at_unix_ms, ended_at_unix_ms, imported, revision
         FROM record AS r
         WHERE NOT EXISTS (SELECT 1 FROM record_digest AS d WHERE d.record_id = r.id)
         LIMIT ?1",
    )?;
    let records = missing
        .query_map([limit], |row| {
            Ok((
                RecordDigest {
                    record: RecordId(row.get(0)?),
                    kind: kind_at(row, 1)?,
                    started_at_unix_ms: row.get(2)?,
                    ended_at_unix_ms: row.get(3)?,
                    imported: row.get(4)?,
                    transcript: TranscriptDigest::default(),
                },
                row.get::<_, u32>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    records
        .into_iter()
        .map(|(mut d, revision)| {
            d.transcript = digest(&segments_of(conn, &d.record, revision)?);
            Ok(d)
        })
        .collect()
}

/// Counts and keeps the digests of up to [`DIGEST_BATCH`] records that have none; how many.
fn fill_batch(conn: &Connection) -> Result<usize, Fail> {
    if !any_missing(conn)? {
        return Ok(0);
    }
    let counted = count_missing(conn, DIGEST_BATCH as i64)?;
    let mut insert = conn.prepare(
        "INSERT INTO record_digest (record_id, kind, started_at_unix_ms, ended_at_unix_ms,
             imported, mic_words, mic_speech_ms, mic_lines, far_words, far_speech_ms, far_lines,
             longest_monologue_ms, mic_questions)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )?;
    // Counts and milliseconds of one record: far inside i64.
    let n = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    for d in &counted {
        let t = &d.transcript;
        insert.execute(params![
            d.record.0,
            kind_text(d.kind),
            d.started_at_unix_ms,
            d.ended_at_unix_ms,
            d.imported,
            n(t.mic.words),
            n(t.mic.speech_ms),
            n(t.mic.lines),
            n(t.far.words),
            n(t.far.speech_ms),
            n(t.far.lines),
            n(t.longest_monologue_ms),
            n(t.mic_questions),
        ])?;
    }
    Ok(counted.len())
}

/// Every record with its digest, in table order (the trait promises none). A record that has no
/// kept digest yet (one made between the batches and this read) is counted here, in the same
/// snapshot, and not kept.
fn read_all(conn: &Connection) -> Result<Vec<RecordDigest>, Fail> {
    let mut select = conn.prepare(
        "SELECT record_id, kind, started_at_unix_ms, ended_at_unix_ms, imported, mic_words,
             mic_speech_ms, mic_lines, far_words, far_speech_ms, far_lines, longest_monologue_ms,
             mic_questions
         FROM record_digest",
    )?;
    let mut out = select
        .query_map([], |row| {
            Ok(RecordDigest {
                record: RecordId(row.get(0)?),
                kind: kind_at(row, 1)?,
                started_at_unix_ms: row.get(2)?,
                ended_at_unix_ms: row.get(3)?,
                imported: row.get(4)?,
                transcript: TranscriptDigest {
                    mic: ChannelDigest {
                        words: u64_at(row, 5)?,
                        speech_ms: u64_at(row, 6)?,
                        lines: u64_at(row, 7)?,
                    },
                    far: ChannelDigest {
                        words: u64_at(row, 8)?,
                        speech_ms: u64_at(row, 9)?,
                        lines: u64_at(row, 10)?,
                    },
                    longest_monologue_ms: u64_at(row, 11)?,
                    mic_questions: u64_at(row, 12)?,
                },
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if any_missing(conn)? {
        out.extend(count_missing(conn, -1)?);
    }
    Ok(out)
}

impl SqliteStore {
    /// [`Store::digests`](ink_core::Store::digests): the missing digests counted and kept a batch
    /// per transaction, then every one read in one snapshot.
    pub(crate) fn digests_kept(&self) -> Result<Vec<RecordDigest>, StoreError> {
        while self.write("digests", |tx| fill_batch(tx))? == DIGEST_BATCH {}
        self.read("digests", |tx| read_all(tx))
    }

    /// [`Store::commitment_states`](ink_core::Store::commitment_states) in one query.
    pub(crate) fn commitment_states_read(&self) -> Result<Vec<CommitmentState>, StoreError> {
        self.read("commitment_states", |tx| {
            let mut select = tx.prepare(
                "SELECT c.id, c.record_id, r.started_at_unix_ms, c.due_at_unix_ms, c.done,
                     c.merged_into IS NOT NULL
                 FROM commitment AS c JOIN record AS r ON r.id = c.record_id
                 ORDER BY c.seq",
            )?;
            let states = select
                .query_map([], |row| {
                    Ok(CommitmentState {
                        commitment: CommitmentId(row.get(0)?),
                        record: RecordId(row.get(1)?),
                        record_started_at_unix_ms: row.get(2)?,
                        due_at_unix_ms: row.get(3)?,
                        done: row.get(4)?,
                        merged: row.get(5)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            Ok(states)
        })
    }
}
