use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use super::lock;
use crate::engine::SpeakerId;
use crate::error::StoreError;
use crate::store::{
    Commitment, CommitmentId, NewCommitment, NewRecord, Record, RecordId, SearchHit, Segment,
    Store, Summary, check_supersede, word_count,
};

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

    fn commitment(&mut self, id: &CommitmentId) -> Result<&mut Commitment, StoreError> {
        self.commitments
            .iter_mut()
            .find(|c| &c.id == id)
            .ok_or(StoreError::NotFound)
    }
}

/// An in-memory [`Store`]. One lock around everything makes every call atomic, so supersede is
/// trivially all-or-nothing; the SQLite store earns that with a transaction.
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

fn sorted(mut segments: Vec<Segment>) -> Vec<Segment> {
    segments.sort_by_key(|s| (s.start_ms, s.channel));
    segments
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

    fn recent_records(&self, limit: usize) -> Result<Vec<Record>, StoreError> {
        let inner = lock(&self.inner);
        let mut records: Vec<Record> = inner.records.values().map(|d| d.record.clone()).collect();
        records
            .sort_by(|a, b| (b.started_at_unix_ms, &b.id.0).cmp(&(a.started_at_unix_ms, &a.id.0)));
        records.truncate(limit);
        Ok(records)
    }

    fn finish_record(&self, id: &RecordId, ended_at_unix_ms: i64) -> Result<(), StoreError> {
        lock(&self.inner).data(id)?.record.ended_at_unix_ms = Some(ended_at_unix_ms);
        Ok(())
    }

    fn delete_record(&self, id: &RecordId) -> Result<(), StoreError> {
        let mut inner = lock(&self.inner);
        inner.records.remove(id).ok_or(StoreError::NotFound)?;
        inner.commitments.retain(|c| &c.record != id);
        Ok(())
    }

    fn append_segments(&self, id: &RecordId, segments: &[Segment]) -> Result<(), StoreError> {
        lock(&self.inner)
            .data(id)?
            .segments
            .extend_from_slice(segments);
        Ok(())
    }

    fn segments(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError> {
        let mut inner = lock(&self.inner);
        Ok(sorted(inner.data(id)?.segments.clone()))
    }

    fn supersede(&self, id: &RecordId, segments: &[Segment]) -> Result<u32, StoreError> {
        let mut inner = lock(&self.inner);
        let data = inner.data(id)?;
        check_supersede(word_count(&data.segments), word_count(segments))?;
        data.segments = segments.to_vec();
        data.record.revision += 1;
        Ok(data.record.revision)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let inner = lock(&self.inner);
        let mut hits: Vec<SearchHit> = inner
            .records
            .values()
            .flat_map(|d| {
                d.segments
                    .iter()
                    .filter(|s| s.text.to_lowercase().contains(&needle))
                    .map(|s| SearchHit {
                        record: d.record.id.clone(),
                        start_ms: s.start_ms,
                        snippet: s.text.clone(),
                    })
            })
            .collect();
        hits.sort_by(|a, b| (&a.record.0, a.start_ms).cmp(&(&b.record.0, b.start_ms)));
        hits.truncate(limit);
        Ok(hits)
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
        inner.commitment(into)?;
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
