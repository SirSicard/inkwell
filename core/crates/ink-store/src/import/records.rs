//! Whole records from another source, written in one transaction behind a marker:
//! [`SqliteStore::import_records`].

use std::collections::BTreeSet;

use ink_core::store::{NewCommitment, NewRecord, RecordId, Segment, Summary};
use ink_core::{SpeakerId, StoreError};

use super::ImportError;
use crate::{
    CommitmentRow, SegmentRow, SqliteStore, commitment_rows, get_setting, insert_commitments,
    insert_record_at, insert_segments, new_id, put_setting, segment_rows, set_ended,
    upsert_speaker, upsert_summary,
};

/// One record to import, with everything that hangs off it, as another source kept it.
///
/// **Audio is the caller's business.** `record.audio_dir` is stored as given: it must name
/// chunks the caller has already written under the data directory, in `ink-audio`'s chunk
/// format. The store neither reads nor checks them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordImport {
    /// The record: kind, title, start, application and audio folder.
    pub record: NewRecord,
    /// When it ended, Unix ms: not before the start. `None` leaves it live, as a record being
    /// captured is.
    pub ended_at_unix_ms: Option<i64>,
    /// Its transcript revision, from 1 (live only). A record whose final pass already ran arrives
    /// at 2 or more, set directly: nothing is superseded on import.
    pub revision: u32,
    /// The transcript at that revision, with each segment's channel and speaker. Checked as
    /// [`Store::append_segments`](ink_core::Store::append_segments) checks them.
    pub segments: Vec<Segment>,
    /// Its summary, stored as being of `revision`.
    pub summary: Option<Summary>,
    /// Names for its diarized speakers; each speaker at most once.
    pub speaker_names: Vec<(SpeakerId, String)>,
    /// Its commitments, in order, with their spans.
    pub commitments: Vec<ImportedCommitment>,
}

/// A commitment to import: what was promised, and whether it is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedCommitment {
    /// The commitment, with its provenance spans.
    pub commitment: NewCommitment,
    /// Whether it is done.
    pub done: bool,
}

/// A record converted for binding, before the lock is taken.
struct Prepared<'a> {
    id: String,
    source: &'a RecordImport,
    segments: Vec<SegmentRow<'a>>,
    commitments: Vec<CommitmentRow<'a>>,
}

fn invalid(what: &str) -> StoreError {
    StoreError::Invalid(what.to_string())
}

impl<'a> Prepared<'a> {
    /// Every check that needs no database, and the new id.
    fn new(source: &'a RecordImport) -> Result<Self, StoreError> {
        if source.revision == 0 {
            return Err(invalid("an imported record's revision starts at 1"));
        }
        if source
            .ended_at_unix_ms
            .is_some_and(|end| end < source.record.started_at_unix_ms)
        {
            return Err(invalid("an imported record ends before it starts"));
        }
        let mut named = BTreeSet::new();
        if !source.speaker_names.iter().all(|(s, _)| named.insert(s)) {
            // One name per speaker, so the import never replaces a row it wrote itself.
            return Err(invalid("an imported record names one speaker twice"));
        }
        Ok(Self {
            id: new_id()?,
            source,
            segments: segment_rows(&source.segments)?,
            commitments: commitment_rows(
                source.commitments.iter().map(|c| (&c.commitment, c.done)),
            )?,
        })
    }

    fn insert(&self, tx: &rusqlite::Transaction<'_>) -> Result<(), crate::codec::Fail> {
        let r = self.source;
        let id = RecordId(self.id.clone());
        insert_record_at(tx, &self.id, &r.record, r.revision, true)?;
        insert_segments(tx, &id, r.revision, &self.segments)?;
        if let Some(end) = r.ended_at_unix_ms {
            set_ended(tx, &self.id, end)?;
        }
        if let Some(summary) = &r.summary {
            upsert_summary(tx, &id, summary, r.revision)?;
        }
        for (speaker, name) in &r.speaker_names {
            upsert_speaker(tx, &id, speaker, name)?;
        }
        insert_commitments(tx, &id, &self.commitments)
    }
}

impl SqliteStore {
    /// Imports whole records in **one transaction** and returns their new ids, in order. Every
    /// record, segment, summary, speaker name and commitment goes through the same inserts as the
    /// [`Store`](ink_core::Store) methods that write them one at a time, each record is marked as
    /// imported ([`Record::imported`](ink_core::Record::imported), which retention never
    /// deletes), and the setting `marker_key` is set to `marker_value` in the same transaction.
    ///
    /// - **Refused** with [`ImportError::MarkerPresent`] when `marker_key` is already set: one
    ///   import per source and store. Nothing is written then.
    /// - **All or nothing.** Every record is checked before the store is locked (times in range,
    ///   stretches forward, a revision from 1, an end not before the start, each speaker named
    ///   once); one bad value refuses the whole call with [`StoreError::Invalid`]. A failure while
    ///   writing rolls every row back, the marker included.
    /// - **Additive.** Only new rows are written, so there is nothing to scrub when it succeeds.
    ///   When it fails, the log is scrubbed as after a delete: pages spilled there before the
    ///   failure still hold the text being imported.
    /// - Records already in the store are left as they are, and so are other markers.
    ///
    /// Notes, removed lines and commitment merges are not imported; audio is the caller's (see
    /// [`RecordImport`]).
    ///
    /// **Worker**: one write transaction, which holds the store for the length of the import.
    pub fn import_records(
        &self,
        marker_key: &str,
        marker_value: &str,
        records: &[RecordImport],
    ) -> Result<Vec<RecordId>, ImportError> {
        if marker_key.is_empty() {
            return Err(invalid("an import needs a marker key").into());
        }
        // Everything that can fail without the database happens before the lock.
        let prepared = records
            .iter()
            .map(Prepared::new)
            .collect::<Result<Vec<_>, StoreError>>()?;
        let outcome = self.write_scrubbing_failure("import_records", |tx| {
            if get_setting(tx, marker_key)?.is_some() {
                // Nothing written yet: the empty transaction commits harmlessly.
                return Ok(Err(ImportError::MarkerPresent));
            }
            for record in &prepared {
                record.insert(tx)?;
            }
            put_setting(tx, marker_key, marker_value)?;
            Ok(Ok(()))
        })?;
        outcome?;
        Ok(prepared.into_iter().map(|p| RecordId(p.id)).collect())
    }
}
