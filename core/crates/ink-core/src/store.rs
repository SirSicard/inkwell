//! Persistence: records, their transcripts, notes, summaries, commitments and settings.
//!
//! `ink-store` implements [`Store`] on SQLite (S1.3). Two rules shape the trait:
//! - **Partials are never stored.** Live finals are appended as the current revision; the offline
//!   pass replaces them with [`Store::supersede`] in one transaction (architecture rule 4).
//! - **Supersede refuses** an empty result, and any channel that falls below half its previous
//!   words: both are far more likely an engine failure than a correction. The one exception is a
//!   drop the pass accounts for ([`Explained`]: live "you" finals it judged to be echo of the far
//!   end). [`check_supersede_explained`] is the one definition every implementation calls.
//! - **Times fit SQLite's integers, and stretches run forward.** A `u64` time or position above
//!   [`MAX_TIME_MS`], or a [`Segment`] or [`Span`] whose `end_ms` is before its `start_ms`, is
//!   refused with [`StoreError::Invalid`] before anything is written, and the whole call with it,
//!   in every store.

use std::collections::BTreeMap;

use crate::audio::Channel;
use crate::engine::SpeakerId;
use crate::error::StoreError;

/// A record's id: UUID text in the SQLite store.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordId(pub String);

/// A commitment's id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommitmentId(pub String);

/// A note's id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NoteId(pub String);

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
    /// A title, when one is known up front (a calendar event, a file name). Otherwise it is set
    /// later from the summary's headline with [`Store::set_title`].
    pub title: Option<String>,
    /// When it started, Unix ms.
    pub started_at_unix_ms: i64,
    /// The application involved (the meeting app, the dictation target).
    pub source_app: Option<String>,
    /// Where the record's audio chunks live, relative to the data directory. `None` when the
    /// record kept no audio. Imported records point at the chunks they brought with them.
    pub audio_dir: Option<String>,
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
    /// Where its audio chunks live, relative to the data directory.
    pub audio_dir: Option<String>,
    /// The transcript revision: 1 while live, raised by each supersede.
    pub revision: u32,
}

/// The largest time or position, in ms, a store accepts: SQLite integers are signed 64-bit.
///
/// Every store checks each time against it, and each [`Segment`] and [`Span`] for
/// `end_ms >= start_ms` (zero length is fine), before any write: one bad value refuses the whole
/// call with [`StoreError::Invalid`].
pub const MAX_TIME_MS: u64 = i64::MAX as u64;

/// Which records to list: newest first, optionally one kind, optionally after a cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordQuery {
    /// Only this kind, or every kind.
    pub kind: Option<RecordKind>,
    /// Only records strictly after this one in the listing order (start time descending, then id
    /// descending). Pass the last record of the previous page, `RecordCursor::from(&last)`:
    /// records that started in the same millisecond are then neither repeated nor skipped.
    pub before: Option<RecordCursor>,
    /// At most this many.
    pub limit: usize,
}

/// A position in the record listing: a keyset cursor over (start time, id), so it stays exact
/// when several records share a start time (a batch of imports does).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordCursor {
    /// The record's start, Unix ms.
    pub started_at_unix_ms: i64,
    /// The record's id, which breaks ties between equal starts.
    pub id: RecordId,
}

impl From<&Record> for RecordCursor {
    fn from(record: &Record) -> Self {
        Self {
            started_at_unix_ms: record.started_at_unix_ms,
            id: record.id.clone(),
        }
    }
}

/// Settled transcript text on the record's timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Mic ("you") or far end ("them").
    pub channel: Channel,
    /// Start, ms from the start of the record, at most [`MAX_TIME_MS`].
    pub start_ms: u64,
    /// End, ms from the start of the record: not before `start_ms`, at most [`MAX_TIME_MS`].
    pub end_ms: u64,
    /// The text.
    pub text: String,
    /// The diarized speaker, far end only, when labels were kept.
    pub speaker: Option<SpeakerId>,
}

/// A note the user typed, stamped with where in the record it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// Its id.
    pub id: NoteId,
    /// The record it belongs to.
    pub record: RecordId,
    /// Where in the record it was written, ms from the start: the timestamp chip. At most
    /// [`MAX_TIME_MS`].
    pub at_ms: u64,
    /// The text.
    pub text: String,
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
    /// Start, ms from the start of the record, at most [`MAX_TIME_MS`].
    pub start_ms: u64,
    /// End, ms from the start of the record: not before `start_ms`, at most [`MAX_TIME_MS`].
    pub end_ms: u64,
}

/// A commitment to add.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewCommitment {
    /// What was promised.
    pub text: String,
    /// Who owes it, as said.
    pub owner: Option<String>,
    /// When, as said ("by Friday"), kept for display.
    pub due: Option<String>,
    /// When, resolved to a time, so "overdue" can be computed. `None` when it could not be
    /// resolved.
    pub due_at_unix_ms: Option<i64>,
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
    /// When, as said.
    pub due: Option<String>,
    /// When, resolved, Unix ms.
    pub due_at_unix_ms: Option<i64>,
    /// Where in the record it was said.
    pub provenance: Vec<Span>,
    /// Set when deduplication folded it into another commitment ("said twice"). Merged
    /// commitments are kept, not deleted, so the merge can be audited.
    pub merged_into: Option<CommitmentId>,
    /// Whether it is done.
    pub done: bool,
}

/// A search result, with enough of its record to list it without another lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// The record.
    pub record: RecordId,
    /// The record's title.
    pub title: Option<String>,
    /// When the record started, Unix ms.
    pub started_at_unix_ms: i64,
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

    /// Records matching `query`, newest first (by start time, then id, both descending).
    fn records(&self, query: &RecordQuery) -> Result<Vec<Record>, StoreError>;

    /// Sets a record's title.
    fn set_title(&self, id: &RecordId, title: &str) -> Result<(), StoreError>;

    /// Marks a record as ended.
    fn finish_record(&self, id: &RecordId, ended_at_unix_ms: i64) -> Result<(), StoreError>;

    /// Deletes a record with its transcript, removed lines, notes, summary, speakers and
    /// commitments.
    ///
    /// A commitment in another record that was merged into one of the deleted commitments is
    /// still owed: it is un-merged (`merged_into` cleared) and open again, not deleted with it.
    fn delete_record(&self, id: &RecordId) -> Result<(), StoreError>;

    /// Appends live finals to the current revision.
    fn append_segments(&self, id: &RecordId, segments: &[Segment]) -> Result<(), StoreError>;

    /// The current revision's segments, ordered by start time.
    fn segments(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError>;

    /// Replaces the current revision with `segments` in one transaction and returns the new
    /// revision. Refuses what [`check_supersede`] refuses; a refused supersede changes nothing.
    fn supersede(&self, id: &RecordId, segments: &[Segment]) -> Result<u32, StoreError> {
        self.supersede_explained(id, segments, &[])
    }

    /// [`supersede`](Self::supersede), where the segments of the current revision that
    /// `explained` names do not count against the guard ([`check_supersede_explained`], run on
    /// the rows the transaction replaces).
    fn supersede_explained(
        &self,
        id: &RecordId,
        segments: &[Segment],
        explained: &[Explained],
    ) -> Result<u32, StoreError>;

    /// Keeps lines a pass removed from a record's transcript, whole, so they can be put back: the
    /// "you" lines a meeting's final pass takes out as echo of the far end. Replaces what the
    /// record kept before (a pass that runs again keeps its own; an empty list clears them).
    ///
    /// They belong to the record, not to a revision: a later supersede leaves them. They are
    /// never searched, and they are deleted with the record, as its transcript is. Times are
    /// checked as [`append_segments`](Self::append_segments) checks them, the whole call refused.
    fn save_removed(&self, id: &RecordId, lines: &[Segment]) -> Result<(), StoreError>;

    /// The lines kept by [`save_removed`](Self::save_removed), by start time, then in the order
    /// they were given.
    fn removed(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError>;

    /// Full-text search across every record's current revision.
    ///
    /// The query is split into words on whitespace. Each word matches as a case-insensitive
    /// prefix of a word in a segment (`budg` finds "Budget"), and a segment matches when any of
    /// the words does. Nothing in the query is syntax: punctuation separates words, and a query
    /// word with punctuation inside (`e-mail`) matches its parts in sequence. Implementations may
    /// also fold diacritics. Results are best first: a segment matching more of the words, or
    /// matching them more often, comes before one matching fewer; beyond that the order is the
    /// implementation's. A query with no words, or a `limit` of 0, returns nothing.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError>;

    /// Adds a note at `at_ms` in the record.
    fn add_note(&self, id: &RecordId, at_ms: u64, text: &str) -> Result<NoteId, StoreError>;

    /// Replaces a note's text; its stamp stays.
    fn update_note(&self, id: &NoteId, text: &str) -> Result<(), StoreError>;

    /// Deletes a note.
    fn delete_note(&self, id: &NoteId) -> Result<(), StoreError>;

    /// A record's notes by stamp, then in the order they were added.
    fn notes(&self, id: &RecordId) -> Result<Vec<Note>, StoreError>;

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

    /// Open commitments across every record: not done and not merged into another. The soonest
    /// resolved due time first, undated ones last, ties in the order they were added.
    fn open_commitments(&self, limit: usize) -> Result<Vec<Commitment>, StoreError>;

    /// Marks a commitment done or not done.
    fn set_commitment_done(&self, id: &CommitmentId, done: bool) -> Result<(), StoreError>;

    /// Folds `id` into `into`: the deduplicated "said twice" case.
    ///
    /// `into` must be canonical: merging into a commitment that is itself merged into another is
    /// [`StoreError::Invalid`] (merge into its canonical instead), and so is merging a commitment
    /// into itself. Commitments already merged into `id` are re-pointed to `into` in the same
    /// call, so `merged_into` always names an unmerged canonical: no chains, no cycles. If
    /// `into`'s record is deleted later, everything merged into it is un-merged and open again
    /// (see [`Store::delete_record`]).
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

fn words_per_channel(segments: &[Segment]) -> BTreeMap<Channel, usize> {
    let mut words = BTreeMap::new();
    for s in segments {
        *words.entry(s.channel).or_default() += s.text.split_whitespace().count();
    }
    words
}

/// A segment of the revision being replaced that the new revision may drop without the guard
/// counting it: the pass that made the new revision judged it not to be speech of its side. The
/// meeting's final pass names the live "you" finals it judged to be echo of the far end (the far
/// end playing, and nobody on the near end heard over it once the echo was cancelled).
///
/// It is matched against the previous revision by channel and exact span, so it can only excuse
/// the segment it was judged on; one that matches nothing excuses nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Explained {
    /// The segment's side.
    pub channel: Channel,
    /// Its start, ms from the start of the record.
    pub start_ms: u64,
    /// Its end.
    pub end_ms: u64,
}

impl Explained {
    fn names(&self, segment: &Segment) -> bool {
        self.channel == segment.channel
            && self.start_ms == segment.start_ms
            && self.end_ms == segment.end_ms
    }
}

/// The supersede guard (architecture rule 4), shared by every [`Store`]: the guard with nothing
/// explained ([`check_supersede_explained`]).
pub fn check_supersede(previous: &[Segment], new: &[Segment]) -> Result<(), StoreError> {
    check_supersede_explained(previous, new, &[])
}

/// The supersede guard (architecture rule 4), shared by every [`Store`].
///
/// Refuses a revision with no words at all, and one where **any channel** falls below half the
/// words it had. The check is per channel because the meeting chain runs one offline pass per
/// channel and supersedes their merge: a mic pass that crashes to nothing would otherwise hide
/// behind a healthy far end in the total. A channel the previous revision did not have is
/// always allowed.
///
/// Previous segments that `explained` names do not count: the pass accounted for them (see
/// [`Explained`]). Every other drop is judged as before, so a pass that loses the user's words
/// for any other reason is still refused. `previous_words` in the error counts only what was
/// judged.
///
/// An earlier implementation only applied the ratio above 200 previous words, so a short record's
/// offline pass could legitimately drop filler; whether that floor comes back is S1.3's call, made
/// here so every store agrees.
pub fn check_supersede_explained(
    previous: &[Segment],
    new: &[Segment],
    explained: &[Explained],
) -> Result<(), StoreError> {
    if word_count(new) == 0 {
        return Err(StoreError::EmptySupersede);
    }
    let now = words_per_channel(new);
    let counted: Vec<Segment> = previous
        .iter()
        .filter(|s| !explained.iter().any(|e| e.names(s)))
        .cloned()
        .collect();
    for (channel, previous_words) in words_per_channel(&counted) {
        let new_words = now.get(&channel).copied().unwrap_or(0);
        if new_words.saturating_mul(2) < previous_words {
            return Err(StoreError::SuspiciousSupersede {
                channel,
                previous_words,
                new_words,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(channel: Channel, text: &str) -> Segment {
        Segment {
            channel,
            start_ms: 0,
            end_ms: 0,
            text: text.into(),
            speaker: None,
        }
    }

    #[test]
    fn words_are_counted_across_segments() {
        assert_eq!(
            word_count(&[
                seg(Channel::Mic, "one two"),
                seg(Channel::Far, "  three  "),
                seg(Channel::Mic, "")
            ]),
            3
        );
    }

    #[test]
    fn supersede_guard_refuses_empty_and_any_channel_under_half() {
        let mic = |t: &str| seg(Channel::Mic, t);
        let far = |t: &str| seg(Channel::Far, t);
        let ten = "a b c d e f g h i j";

        assert_eq!(
            check_supersede(&[mic(ten)], &[]),
            Err(StoreError::EmptySupersede)
        );
        assert_eq!(
            check_supersede(&[], &[mic(" ")]),
            Err(StoreError::EmptySupersede)
        );
        assert_eq!(
            check_supersede(&[mic(ten)], &[mic("a b c d")]),
            Err(StoreError::SuspiciousSupersede {
                channel: Channel::Mic,
                previous_words: 10,
                new_words: 4
            })
        );
        assert_eq!(check_supersede(&[mic(ten)], &[mic("a b c d e")]), Ok(()));
        assert_eq!(
            check_supersede(&[mic("a b c d e f g h i j k")], &[mic("a b c d e")]),
            Err(StoreError::SuspiciousSupersede {
                channel: Channel::Mic,
                previous_words: 11,
                new_words: 5
            })
        );
        // The total (11 of 20) would pass; the mic channel (0 of 10) does not.
        assert_eq!(
            check_supersede(&[mic(ten), far(ten)], &[far("a b c d e f g h i j k")]),
            Err(StoreError::SuspiciousSupersede {
                channel: Channel::Mic,
                previous_words: 10,
                new_words: 0
            })
        );
        assert_eq!(check_supersede(&[], &[mic("a b c")]), Ok(()));
        assert_eq!(
            check_supersede(&[mic(ten)], &[mic(ten), far("new side")]),
            Ok(())
        );
    }

    #[test]
    fn explained_segments_do_not_count_and_nothing_else_is_excused() {
        let at = |channel: Channel, start_ms: u64, text: &str| Segment {
            channel,
            start_ms,
            end_ms: start_ms + 1_000,
            text: text.into(),
            speaker: None,
        };
        let ten = "a b c d e f g h i j";
        // Live: two "you" finals that were echo, one that was the user; the far end.
        let previous = [
            at(Channel::Mic, 0, ten),
            at(Channel::Mic, 5_000, ten),
            at(Channel::Mic, 9_000, "yes that works"),
            at(Channel::Far, 0, ten),
        ];
        let echo = |start_ms: u64| Explained {
            channel: Channel::Mic,
            start_ms,
            end_ms: start_ms + 1_000,
        };
        let new = [
            at(Channel::Mic, 9_000, "yes that works"),
            at(Channel::Far, 0, ten),
        ];
        assert_eq!(
            check_supersede(&previous, &new),
            Err(StoreError::SuspiciousSupersede {
                channel: Channel::Mic,
                previous_words: 23,
                new_words: 3
            })
        );
        assert_eq!(
            check_supersede_explained(&previous, &new, &[echo(0), echo(5_000)]),
            Ok(())
        );
        // One echo final explained is not enough: 13 words judged, 3 left.
        assert_eq!(
            check_supersede_explained(&previous, &new, &[echo(0)]),
            Err(StoreError::SuspiciousSupersede {
                channel: Channel::Mic,
                previous_words: 13,
                new_words: 3
            })
        );
        // The user's own words are still guarded: dropping them is refused.
        assert_eq!(
            check_supersede_explained(&previous, &new[1..], &[echo(0), echo(5_000)]),
            Err(StoreError::SuspiciousSupersede {
                channel: Channel::Mic,
                previous_words: 3,
                new_words: 0
            })
        );
        // An explanation that names no segment exactly (another span, the other side) excuses
        // nothing.
        let off = Explained {
            channel: Channel::Mic,
            start_ms: 0,
            end_ms: 999,
        };
        let far_side = Explained {
            channel: Channel::Far,
            start_ms: 5_000,
            end_ms: 6_000,
        };
        assert!(check_supersede_explained(&previous, &new, &[off, far_side, echo(5_000)]).is_err());
        // Nothing left at all is still empty.
        assert_eq!(
            check_supersede_explained(&previous, &[], &[echo(0), echo(5_000)]),
            Err(StoreError::EmptySupersede)
        );
    }
}
