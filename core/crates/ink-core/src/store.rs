//! Persistence: records, their transcripts, summaries, commitments and settings.
//!
//! `ink-store` implements [`Store`] on SQLite (S1.3). Two rules shape the trait:
//! - **Partials are never stored.** Live finals are appended as the current revision; the offline
//!   pass replaces them with [`Store::supersede`] in one transaction (architecture rule 4).
//! - **Supersede refuses** an empty result and one with fewer than half the previous words, both
//!   far more likely an engine failure than a correction. [`check_supersede`] is the one
//!   definition every implementation calls.

use crate::audio::Channel;
use crate::engine::SpeakerId;
use crate::error::StoreError;

/// A record's id: UUID text in the SQLite store.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordId(pub String);

/// A commitment's id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommitmentId(pub String);

/// What produced a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecordKind {
    /// One dictation.
    Dictation,
    /// A meeting: mic and far end.
    Meeting,
    /// An imported audio or video file.
    FileImport,
}

/// A record to create.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewRecord {
    /// What produced it.
    pub kind: RecordKind,
    /// A title, when one is known up front (a calendar event, a file name).
    pub title: Option<String>,
    /// When it started, Unix ms.
    pub started_at_unix_ms: i64,
    /// The application involved (the meeting app, the dictation target).
    pub source_app: Option<String>,
}

/// A stored record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Its id.
    pub id: RecordId,
    /// What produced it.
    pub kind: RecordKind,
    /// Its title.
    pub title: Option<String>,
    /// When it started, Unix ms.
    pub started_at_unix_ms: i64,
    /// When it ended, Unix ms; `None` while live.
    pub ended_at_unix_ms: Option<i64>,
    /// The application involved.
    pub source_app: Option<String>,
    /// The transcript revision: 1 while live, raised by each supersede.
    pub revision: u32,
}

/// Settled transcript text on the record's timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Mic ("you") or far end ("them").
    pub channel: Channel,
    /// Start, ms from the start of the record.
    pub start_ms: u64,
    /// End, ms from the start of the record.
    pub end_ms: u64,
    /// The text.
    pub text: String,
    /// The diarized speaker, far end only, when labels were kept.
    pub speaker: Option<SpeakerId>,
}

/// A record's summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// The summary text.
    pub text: String,
    /// The model that wrote it, as its engine or provider id.
    pub model: String,
    /// When it was written, Unix ms.
    pub created_at_unix_ms: i64,
}

/// A stretch of a record. Commitments cite spans rather than segment rows, because a supersede
/// replaces every row while the times stay meaningful.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Which side said it.
    pub channel: Channel,
    /// Start, ms from the start of the record.
    pub start_ms: u64,
    /// End, ms from the start of the record.
    pub end_ms: u64,
}

/// A commitment to add.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewCommitment {
    /// What was promised.
    pub text: String,
    /// Who owes it, as said.
    pub owner: Option<String>,
    /// When, as said.
    pub due: Option<String>,
    /// Where in the record it was said: its provenance.
    pub provenance: Vec<Span>,
}

/// A stored commitment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commitment {
    /// Its id.
    pub id: CommitmentId,
    /// The record it came from.
    pub record: RecordId,
    /// What was promised.
    pub text: String,
    /// Who owes it.
    pub owner: Option<String>,
    /// When.
    pub due: Option<String>,
    /// Where in the record it was said.
    pub provenance: Vec<Span>,
    /// Set when deduplication folded it into another commitment ("said twice"). Merged
    /// commitments are kept, not deleted, so the merge can be audited.
    pub merged_into: Option<CommitmentId>,
    /// Whether it is done.
    pub done: bool,
}

/// A search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// The record.
    pub record: RecordId,
    /// Where in the record, ms.
    pub start_ms: u64,
    /// The matching text.
    pub snippet: String,
}

/// The store.
///
/// **Worker**, every method. Implementations are `Send + Sync` and serialise writes themselves;
/// callers never hold a lock across calls. Errors carry no transcript text.
pub trait Store: Send + Sync {
    /// Creates a record at revision 1.
    fn create_record(&self, record: NewRecord) -> Result<RecordId, StoreError>;

    /// One record, or `None`.
    fn record(&self, id: &RecordId) -> Result<Option<Record>, StoreError>;

    /// The most recent records, newest first.
    fn recent_records(&self, limit: usize) -> Result<Vec<Record>, StoreError>;

    /// Marks a record as ended.
    fn finish_record(&self, id: &RecordId, ended_at_unix_ms: i64) -> Result<(), StoreError>;

    /// Deletes a record with its transcript, summary, speakers and commitments.
    fn delete_record(&self, id: &RecordId) -> Result<(), StoreError>;

    /// Appends live finals to the current revision.
    fn append_segments(&self, id: &RecordId, segments: &[Segment]) -> Result<(), StoreError>;

    /// The current revision's segments, ordered by start time.
    fn segments(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError>;

    /// Replaces the current revision with `segments` in one transaction and returns the new
    /// revision. Refuses what [`check_supersede`] refuses; a refused supersede changes nothing.
    fn supersede(&self, id: &RecordId, segments: &[Segment]) -> Result<u32, StoreError>;

    /// Full-text search across every record's current revision.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError>;

    /// Saves (or replaces) a record's summary.
    fn save_summary(&self, id: &RecordId, summary: &Summary) -> Result<(), StoreError>;

    /// A record's summary, or `None`.
    fn summary(&self, id: &RecordId) -> Result<Option<Summary>, StoreError>;

    /// Names a diarized speaker in one record.
    fn set_speaker_name(
        &self,
        id: &RecordId,
        speaker: &SpeakerId,
        name: &str,
    ) -> Result<(), StoreError>;

    /// A record's named speakers, ordered by id.
    fn speaker_names(&self, id: &RecordId) -> Result<Vec<(SpeakerId, String)>, StoreError>;

    /// Adds commitments to a record and returns their ids, in order.
    fn add_commitments(
        &self,
        id: &RecordId,
        items: &[NewCommitment],
    ) -> Result<Vec<CommitmentId>, StoreError>;

    /// A record's commitments in the order they were added, merged ones included.
    fn commitments(&self, id: &RecordId) -> Result<Vec<Commitment>, StoreError>;

    /// Marks a commitment done or not done.
    fn set_commitment_done(&self, id: &CommitmentId, done: bool) -> Result<(), StoreError>;

    /// Folds `id` into `into`: the deduplicated "said twice" case. Merging a commitment into
    /// itself is [`StoreError::Invalid`].
    fn merge_commitment(&self, id: &CommitmentId, into: &CommitmentId) -> Result<(), StoreError>;

    /// A setting's value, or `None`.
    fn setting(&self, key: &str) -> Result<Option<String>, StoreError>;

    /// Sets a setting.
    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError>;
}

/// Words in a set of segments, split on whitespace.
pub fn word_count(segments: &[Segment]) -> usize {
    segments
        .iter()
        .map(|s| s.text.split_whitespace().count())
        .sum()
}

/// The supersede guard (architecture rule 4), shared by every [`Store`].
///
/// Refuses a revision with no words, and one with fewer than half the words of the current one.
/// An earlier implementation only applied the ratio above 200 previous words, so a short record's offline pass could
/// legitimately drop filler; whether that floor comes back is S1.3's call, made here so every
/// store agrees.
pub fn check_supersede(previous_words: usize, new_words: usize) -> Result<(), StoreError> {
    if new_words == 0 {
        return Err(StoreError::EmptySupersede);
    }
    if new_words.saturating_mul(2) < previous_words {
        return Err(StoreError::SuspiciousSupersede {
            previous_words,
            new_words,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str) -> Segment {
        Segment {
            channel: Channel::Mic,
            start_ms: 0,
            end_ms: 0,
            text: text.into(),
            speaker: None,
        }
    }

    #[test]
    fn words_are_counted_across_segments() {
        assert_eq!(word_count(&[seg("one two"), seg("  three  "), seg("")]), 3);
    }

    #[test]
    fn supersede_guard_refuses_empty_and_less_than_half() {
        assert_eq!(check_supersede(10, 0), Err(StoreError::EmptySupersede));
        assert_eq!(check_supersede(0, 0), Err(StoreError::EmptySupersede));
        assert_eq!(
            check_supersede(10, 4),
            Err(StoreError::SuspiciousSupersede {
                previous_words: 10,
                new_words: 4
            })
        );
        assert_eq!(check_supersede(10, 5), Ok(()));
        assert_eq!(
            check_supersede(11, 5),
            Err(StoreError::SuspiciousSupersede {
                previous_words: 11,
                new_words: 5
            })
        );
        assert_eq!(check_supersede(0, 3), Ok(()));
        assert_eq!(check_supersede(3, 30), Ok(()));
    }
}
