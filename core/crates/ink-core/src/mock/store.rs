use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use super::lock;
use crate::engine::SpeakerId;
use crate::error::StoreError;
use crate::store::{
    Commitment, CommitmentId, MAX_TIME_MS, NewCommitment, NewRecord, Note, NoteId, Record,
    RecordId, RecordQuery, SearchHit, Segment, Store, Summary, check_supersede,
};

/// Refuses times a SQLite store could not hold, before anything changes.
fn check_times(times: impl IntoIterator<Item = u64>) -> Result<(), StoreError> {
    if times.into_iter().any(|t| t > MAX_TIME_MS) {
        return Err(StoreError::Invalid(
            "a time is beyond the range a store can hold".into(),
        ));
    }
    Ok(())
}

fn segment_times(segments: &[Segment]) -> impl Iterator<Item = u64> + '_ {
    segments.iter().flat_map(|s| [s.start_ms, s.end_ms])
}

/// Lowercased words: runs of letters and digits. Everything else separates words.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// How often one query word (its parts, in sequence, the last as a prefix) occurs in `text`.
fn occurrences(term: &[String], text: &[String]) -> usize {
    let Some((last, head)) = term.split_last() else {
        return 0;
    };
    text.windows(term.len())
        .filter(|w| w[..head.len()] == *head && w[head.len()].starts_with(last.as_str()))
        .count()
}

struct RecordData {
    record: Record,
    segments: Vec<Segment>,
    summary: Option<Summary>,
    speakers: BTreeMap<SpeakerId, String>,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    records: HashMap<RecordId, RecordData>,
    notes: Vec<Note>,
    commitments: Vec<Commitment>,
    settings: HashMap<String, String>,
}

impl Inner {
    fn next(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}-{}", self.next_id)
    }

    fn data(&mut self, id: &RecordId) -> Result<&mut RecordData, StoreError> {
        self.records.get_mut(id).ok_or(StoreError::NotFound)
    }

    fn note(&mut self, id: &NoteId) -> Result<&mut Note, StoreError> {
        self.notes
            .iter_mut()
            .find(|n| &n.id == id)
            .ok_or(StoreError::NotFound)
    }

    fn commitment(&mut self, id: &CommitmentId) -> Result<&mut Commitment, StoreError> {
        self.commitments
            .iter_mut()
            .find(|c| &c.id == id)
            .ok_or(StoreError::NotFound)
    }
}

/// An in-memory [`Store`]. One lock around everything makes every call atomic, so supersede is
/// trivially all-or-nothing; the SQLite store earns that with a transaction. Notes and
/// commitments live in insertion order, which is the tie-break the trait promises.
#[derive(Default)]
pub struct MemStore {
    inner: Mutex<Inner>,
}

impl MemStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for MemStore {
    fn create_record(&self, new: NewRecord) -> Result<RecordId, StoreError> {
        let mut inner = lock(&self.inner);
        let id = RecordId(inner.next("rec"));
        let record = Record {
            id: id.clone(),
            kind: new.kind,
            title: new.title,
            started_at_unix_ms: new.started_at_unix_ms,
            ended_at_unix_ms: None,
            source_app: new.source_app,
            audio_dir: new.audio_dir,
            revision: 1,
        };
        inner.records.insert(
            id.clone(),
            RecordData {
                record,
                segments: Vec::new(),
                summary: None,
                speakers: BTreeMap::new(),
            },
        );
        Ok(id)
    }

    fn record(&self, id: &RecordId) -> Result<Option<Record>, StoreError> {
        Ok(lock(&self.inner).records.get(id).map(|d| d.record.clone()))
    }

    fn records(&self, query: &RecordQuery) -> Result<Vec<Record>, StoreError> {
        let inner = lock(&self.inner);
        let mut records: Vec<Record> = inner
            .records
            .values()
            .map(|d| &d.record)
            .filter(|r| query.kind.is_none_or(|k| r.kind == k))
            // Strictly after the cursor in the listing order (start, then id, both descending).
            .filter(|r| {
                query
                    .before
                    .as_ref()
                    .is_none_or(|c| (r.started_at_unix_ms, &r.id) < (c.started_at_unix_ms, &c.id))
            })
            .cloned()
            .collect();
        records
            .sort_by(|a, b| (b.started_at_unix_ms, &b.id.0).cmp(&(a.started_at_unix_ms, &a.id.0)));
        records.truncate(query.limit);
        Ok(records)
    }

    fn set_title(&self, id: &RecordId, title: &str) -> Result<(), StoreError> {
        lock(&self.inner).data(id)?.record.title = Some(title.into());
        Ok(())
    }

    fn finish_record(&self, id: &RecordId, ended_at_unix_ms: i64) -> Result<(), StoreError> {
        lock(&self.inner).data(id)?.record.ended_at_unix_ms = Some(ended_at_unix_ms);
        Ok(())
    }

    fn delete_record(&self, id: &RecordId) -> Result<(), StoreError> {
        let mut inner = lock(&self.inner);
        inner.records.remove(id).ok_or(StoreError::NotFound)?;
        inner.notes.retain(|n| &n.record != id);
        let removed: HashSet<CommitmentId> = inner
            .commitments
            .iter()
            .filter(|c| &c.record == id)
            .map(|c| c.id.clone())
            .collect();
        inner.commitments.retain(|c| &c.record != id);
        // A duplicate folded into a deleted commitment is still owed: un-merge it.
        for c in &mut inner.commitments {
            if c.merged_into.as_ref().is_some_and(|m| removed.contains(m)) {
                c.merged_into = None;
            }
        }
        Ok(())
    }

    fn append_segments(&self, id: &RecordId, segments: &[Segment]) -> Result<(), StoreError> {
        check_times(segment_times(segments))?;
        lock(&self.inner)
            .data(id)?
            .segments
            .extend_from_slice(segments);
        Ok(())
    }

    fn segments(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError> {
        let mut inner = lock(&self.inner);
        let mut segments = inner.data(id)?.segments.clone();
        segments.sort_by_key(|s| (s.start_ms, s.channel));
        Ok(segments)
    }

    fn supersede(&self, id: &RecordId, segments: &[Segment]) -> Result<u32, StoreError> {
        check_times(segment_times(segments))?;
        let mut inner = lock(&self.inner);
        let data = inner.data(id)?;
        check_supersede(&data.segments, segments)?;
        data.segments = segments.to_vec();
        data.record.revision += 1;
        Ok(data.record.revision)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError> {
        // Query words split on whitespace (and control characters, as the SQLite store does);
        // each keeps its parts so `e-mail` matches "e" then "mail…" in sequence.
        let terms: Vec<Vec<String>> = query
            .split(|c: char| c.is_whitespace() || c.is_control())
            .map(words)
            .filter(|t| !t.is_empty())
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let inner = lock(&self.inner);
        // (words matched, occurrences) ranks a hit: more of the words first, then more often.
        let mut scored: Vec<((usize, usize), SearchHit)> = inner
            .records
            .values()
            .flat_map(|d| {
                d.segments.iter().filter_map(|s| {
                    let text = words(&s.text);
                    let counts: Vec<usize> = terms.iter().map(|t| occurrences(t, &text)).collect();
                    let matched = counts.iter().filter(|&&n| n > 0).count();
                    (matched > 0).then(|| {
                        let hit = SearchHit {
                            record: d.record.id.clone(),
                            title: d.record.title.clone(),
                            started_at_unix_ms: d.record.started_at_unix_ms,
                            start_ms: s.start_ms,
                            snippet: s.text.clone(),
                        };
                        ((matched, counts.iter().sum()), hit)
                    })
                })
            })
            .collect();
        scored.sort_by(|(sa, a), (sb, b)| {
            sb.cmp(sa)
                .then_with(|| (&a.record.0, a.start_ms).cmp(&(&b.record.0, b.start_ms)))
        });
        Ok(scored.into_iter().take(limit).map(|(_, hit)| hit).collect())
    }

    fn add_note(&self, id: &RecordId, at_ms: u64, text: &str) -> Result<NoteId, StoreError> {
        check_times([at_ms])?;
        let mut inner = lock(&self.inner);
        inner.data(id)?;
        let note_id = NoteId(inner.next("note"));
        inner.notes.push(Note {
            id: note_id.clone(),
            record: id.clone(),
            at_ms,
            text: text.into(),
        });
        Ok(note_id)
    }

    fn update_note(&self, id: &NoteId, text: &str) -> Result<(), StoreError> {
        lock(&self.inner).note(id)?.text = text.into();
        Ok(())
    }

    fn delete_note(&self, id: &NoteId) -> Result<(), StoreError> {
        let mut inner = lock(&self.inner);
        let before = inner.notes.len();
        inner.notes.retain(|n| &n.id != id);
        if inner.notes.len() == before {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    fn notes(&self, id: &RecordId) -> Result<Vec<Note>, StoreError> {
        let mut inner = lock(&self.inner);
        inner.data(id)?;
        let mut notes: Vec<Note> = inner
            .notes
            .iter()
            .filter(|n| &n.record == id)
            .cloned()
            .collect();
        // A stable sort keeps insertion order among notes with the same stamp.
        notes.sort_by_key(|n| n.at_ms);
        Ok(notes)
    }

    fn save_summary(&self, id: &RecordId, summary: &Summary) -> Result<(), StoreError> {
        lock(&self.inner).data(id)?.summary = Some(summary.clone());
        Ok(())
    }

    fn summary(&self, id: &RecordId) -> Result<Option<Summary>, StoreError> {
        Ok(lock(&self.inner).data(id)?.summary.clone())
    }

    fn set_speaker_name(
        &self,
        id: &RecordId,
        speaker: &SpeakerId,
        name: &str,
    ) -> Result<(), StoreError> {
        lock(&self.inner)
            .data(id)?
            .speakers
            .insert(speaker.clone(), name.into());
        Ok(())
    }

    fn speaker_names(&self, id: &RecordId) -> Result<Vec<(SpeakerId, String)>, StoreError> {
        let mut inner = lock(&self.inner);
        Ok(inner
            .data(id)?
            .speakers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn add_commitments(
        &self,
        id: &RecordId,
        items: &[NewCommitment],
    ) -> Result<Vec<CommitmentId>, StoreError> {
        check_times(
            items
                .iter()
                .flat_map(|i| &i.provenance)
                .flat_map(|s| [s.start_ms, s.end_ms]),
        )?;
        let mut inner = lock(&self.inner);
        inner.data(id)?;
        let mut ids = Vec::with_capacity(items.len());
        for item in items {
            let cid = CommitmentId(inner.next("com"));
            inner.commitments.push(Commitment {
                id: cid.clone(),
                record: id.clone(),
                text: item.text.clone(),
                owner: item.owner.clone(),
                due: item.due.clone(),
                due_at_unix_ms: item.due_at_unix_ms,
                provenance: item.provenance.clone(),
                merged_into: None,
                done: false,
            });
            ids.push(cid);
        }
        Ok(ids)
    }

    fn commitments(&self, id: &RecordId) -> Result<Vec<Commitment>, StoreError> {
        let mut inner = lock(&self.inner);
        inner.data(id)?;
        Ok(inner
            .commitments
            .iter()
            .filter(|c| &c.record == id)
            .cloned()
            .collect())
    }

    fn open_commitments(&self, limit: usize) -> Result<Vec<Commitment>, StoreError> {
        let inner = lock(&self.inner);
        let mut open: Vec<Commitment> = inner
            .commitments
            .iter()
            .filter(|c| !c.done && c.merged_into.is_none())
            .cloned()
            .collect();
        // Dated before undated, then soonest first; the sort is stable, so ties stay in the order
        // they were added.
        open.sort_by_key(|c| (c.due_at_unix_ms.is_none(), c.due_at_unix_ms));
        open.truncate(limit);
        Ok(open)
    }

    fn set_commitment_done(&self, id: &CommitmentId, done: bool) -> Result<(), StoreError> {
        lock(&self.inner).commitment(id)?.done = done;
        Ok(())
    }

    fn merge_commitment(&self, id: &CommitmentId, into: &CommitmentId) -> Result<(), StoreError> {
        if id == into {
            return Err(StoreError::Invalid(
                "a commitment cannot be merged into itself".into(),
            ));
        }
        let mut inner = lock(&self.inner);
        if inner.commitment(into)?.merged_into.is_some() {
            return Err(StoreError::Invalid(
                "merge into the canonical commitment, not one that is itself merged".into(),
            ));
        }
        inner.commitment(id)?.merged_into = Some(into.clone());
        Ok(())
    }

    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(lock(&self.inner).settings.get(key).cloned())
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        lock(&self.inner).settings.insert(key.into(), value.into());
        Ok(())
    }
}
