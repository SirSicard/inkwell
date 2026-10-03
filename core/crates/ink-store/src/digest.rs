//! [`Store::digests`](ink_core::Store::digests) and
//! [`Store::commitment_states`](ink_core::Store::commitment_states) on SQLite.
//!
//! Digests are kept in `record_digest` (schema V5) and dropped by triggers whenever what they
//! count changes, so a stats read only counts the records that changed since the last one. A row
//! kept under other counting rules ([`DIGEST_VERSION`]) counts as missing and is counted again.
//! Measured on an M-series Mac, release build, with a library of 75,000 dictations of 30 words and
//! 500 meetings of 1,000 lines: counting every transcript took 690 ms (about 1.2 µs a line); with
//! every digest kept, a read takes about 18 ms. The first read of that library counts it all once,
//! in 63 batches of about 50 ms each, commit included (3.4 s in all), the lock let go between them.

use ink_core::stats::{
    ChannelDigest, CommitmentState, DIGEST_VERSION, RecordDigest, TranscriptDigest, digest,
};
use ink_core::{CommitmentId, RecordId, StoreError};
use rusqlite::{Connection, Row, params};

use crate::codec::{Fail, kind_at, kind_text};
use crate::{SqliteStore, segments_of};

/// The most records one transaction counts when digests are missing (the first read after an
/// upgrade, or after a large import).
pub const DIGEST_BATCH: usize = 2_000;

/// The most lines one transaction counts, whatever the records: the store's one lock is held for
/// the whole batch, and a live meeting's append waits behind it. At about 1.2 µs a line, 20,000
/// lines take about 25 ms to count; a batch of 2,000 dictations about the same.
pub const DIGEST_LINE_BUDGET: u64 = 20_000;

fn version() -> i64 {
    i64::from(DIGEST_VERSION)
}

fn u64_at(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let v: i64 = row.get(index)?;
    // The schema's CHECKs keep every count at or above 0.
    Ok(v.max(0) as u64)
}

/// How many records have no digest kept under the current rules. Every digest row names a record
/// that exists (the cascade) and at most once (its key), so the difference of the counts is it.
fn missing(conn: &Connection) -> Result<i64, Fail> {
    let records: i64 = conn.query_row("SELECT count(*) FROM record", [], |r| r.get(0))?;
    let kept: i64 = conn.query_row(
        "SELECT count(*) FROM record_digest WHERE version = ?1",
        [version()],
        |r| r.get(0),
    )?;
    Ok(records - kept)
}

/// Records with no digest kept under the current rules, counted now: up to [`DIGEST_BATCH`] of
/// them, and no more than [`DIGEST_LINE_BUDGET`] lines (at least one record, however long).
fn count_missing(conn: &Connection) -> Result<Vec<RecordDigest>, Fail> {
    let mut select = conn.prepare(
        "SELECT id, kind, started_at_unix_ms, ended_at_unix_ms, imported, revision,
             (SELECT count(*) FROM segment AS s WHERE s.record_id = r.id AND s.revision = r.revision)
         FROM record AS r
         WHERE NOT EXISTS (
             SELECT 1 FROM record_digest AS d WHERE d.record_id = r.id AND d.version = ?1)
         LIMIT ?2",
    )?;
    let mut rows = select.query(params![version(), DIGEST_BATCH as i64])?;
    let (mut picked, mut lines) = (Vec::new(), 0u64);
    while let Some(row) = rows.next()? {
        let these = u64_at(row, 6)?;
        if !picked.is_empty() && lines + these > DIGEST_LINE_BUDGET {
            break;
        }
        lines += these;
        picked.push((
            RecordDigest {
                record: RecordId(row.get(0)?),
                kind: kind_at(row, 1)?,
                started_at_unix_ms: row.get(2)?,
                ended_at_unix_ms: row.get(3)?,
                imported: row.get(4)?,
                transcript: TranscriptDigest::default(),
            },
            row.get::<_, u32>(5)?,
        ));
    }
    drop(rows);
    picked
        .into_iter()
        .map(|(mut d, revision)| {
            d.transcript = digest(&segments_of(conn, &d.record, revision)?);
            Ok(d)
        })
        .collect()
}

/// Counts and keeps one batch of missing digests ([`count_missing`]); how many.
fn fill_batch(conn: &Connection) -> Result<usize, Fail> {
    if missing(conn)? == 0 {
        return Ok(0);
    }
    let counted = count_missing(conn)?;
    // Replaces a row kept under older rules.
    let mut insert = conn.prepare(
        "INSERT OR REPLACE INTO record_digest (record_id, version, kind, started_at_unix_ms,
             ended_at_unix_ms, imported, mic_words, mic_speech_ms, mic_lines, far_words,
             far_speech_ms, far_lines, longest_monologue_ms, mic_questions)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )?;
    // Counts and milliseconds of one record: far inside i64.
    let n = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    for d in &counted {
        let t = &d.transcript;
        insert.execute(params![
            d.record.0,
            version(),
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

/// Every record with its digest, in table order (the trait promises none), in one snapshot. A
/// record that has no kept digest yet (one made since the batches) is counted here and not kept,
/// within one batch's budget; `None` when more are missing than that, and the caller fills again.
fn read_all(conn: &Connection) -> Result<Option<Vec<RecordDigest>>, Fail> {
    let missing = missing(conn)?;
    let late = if missing > 0 {
        let late = count_missing(conn)?;
        if (late.len() as i64) < missing {
            return Ok(None);
        }
        late
    } else {
        Vec::new()
    };
    let mut select = conn.prepare(
        "SELECT record_id, kind, started_at_unix_ms, ended_at_unix_ms, imported, mic_words,
             mic_speech_ms, mic_lines, far_words, far_speech_ms, far_lines, longest_monologue_ms,
             mic_questions
         FROM record_digest WHERE version = ?1",
    )?;
    let mut out = select
        .query_map([version()], |row| {
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
    out.extend(late);
    Ok(Some(out))
}

impl SqliteStore {
    /// [`Store::digests`](ink_core::Store::digests): the missing digests counted and kept a batch
    /// per transaction (the lock let go between them), then every one read in one snapshot. A
    /// read that finds nothing missing takes no write transaction at all.
    pub(crate) fn digests_kept(&self) -> Result<Vec<RecordDigest>, StoreError> {
        loop {
            if self.read("digests", |tx| missing(tx))? > 0 {
                while self.write("digests", |tx| fill_batch(tx))? > 0 {}
            }
            // More arrived between the batches and the read than one batch takes: fill again.
            if let Some(all) = self.read("digests", |tx| read_all(tx))? {
                return Ok(all);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::{Channel, NewRecord, RecordKind, Segment, Store};

    fn meeting_of(store: &SqliteStore, started: i64, lines: u64) -> RecordId {
        let id = store
            .create_record(NewRecord {
                kind: RecordKind::Meeting,
                title: None,
                started_at_unix_ms: started,
                source_app: None,
                audio_dir: None,
            })
            .unwrap();
        let segments: Vec<Segment> = (0..lines)
            .map(|i| Segment {
                channel: if i % 2 == 0 {
                    Channel::Mic
                } else {
                    Channel::Far
                },
                start_ms: i * 1_000,
                end_ms: i * 1_000 + 900,
                text: "a b".into(),
                speaker: None,
            })
            .collect();
        store.append_segments(&id, &segments).unwrap();
        id
    }

    /// A batch stops at the line budget, not only at the record count: two long meetings take two
    /// transactions, so a live append never waits for both.
    #[test]
    fn a_batch_stops_at_the_line_budget() {
        let store = SqliteStore::open_in_memory().unwrap();
        let lines = DIGEST_LINE_BUDGET * 3 / 5;
        meeting_of(&store, 1, lines);
        meeting_of(&store, 2, lines);
        assert_eq!(store.write("test", |tx| fill_batch(tx)).unwrap(), 1);
        assert_eq!(store.write("test", |tx| fill_batch(tx)).unwrap(), 1);
        assert_eq!(store.write("test", |tx| fill_batch(tx)).unwrap(), 0);
        // One record longer than the budget is still counted, alone.
        meeting_of(&store, 3, DIGEST_LINE_BUDGET + 1);
        assert_eq!(store.write("test", |tx| fill_batch(tx)).unwrap(), 1);
        let all = store.digests().unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.iter().all(|d| d.transcript.mic.words > 0));
    }

    /// A read with more missing than one batch takes does not count them all under the lock: it
    /// says so, and the caller fills again.
    #[test]
    fn a_read_with_more_missing_than_a_batch_hands_back() {
        let store = SqliteStore::open_in_memory().unwrap();
        meeting_of(&store, 1, DIGEST_LINE_BUDGET * 3 / 5);
        meeting_of(&store, 2, DIGEST_LINE_BUDGET * 3 / 5);
        assert_eq!(store.read("test", |tx| read_all(tx)).unwrap(), None);
        meeting_of(&store, 3, 10);
        store.write("test", |tx| fill_batch(tx)).unwrap();
        // Two left, within one batch's budget: counted in the read, and not kept.
        assert_eq!(store.read("test", |tx| missing(tx)).unwrap(), 2);
        let all = store.read("test", |tx| read_all(tx)).unwrap().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(store.read("test", |tx| missing(tx)).unwrap(), 2);
    }
}
