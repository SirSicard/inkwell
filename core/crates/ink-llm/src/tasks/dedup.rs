//! One promise, one entry: folding the "said twice" case.
//!
//! The summary and the commitment harvester both read the same meeting and both file what they
//! find. That redundancy is deliberate (the summary catches what a reader would notice, the
//! harvester walks every first-person sentence), but it means one promise often arrives twice, in
//! different words. Word overlap alone cannot see it: "Send the revised budget to the finance
//! team" and "Get the finance team the updated budget sheet before the review" share few words
//! in the ordinary (Jaccard) sense.
//!
//! So the shortlist is cheap and the decision is a judgement. Distinctive words find candidate
//! pairs ([`candidate_pairs`]); a model decides whether each pair is the same obligation. The
//! shortlist is generous on purpose: a false pair costs one model call, a missed one a permanent
//! double entry. Merges are applied with [`Store::merge_commitment`], which keeps the folded
//! commitment for audit rather than deleting it.

use std::collections::BTreeSet;

use ink_core::{
    CancelToken, CommitmentId, Llm, LlmError, LlmRequest, NewCommitment, Store, StoreError,
};

use super::{ask, is_fatal};
use crate::json::{Fields, bad_field, object_in};

/// Words that carry no identity. Kept short: an aggressive list starts deleting the nouns that
/// make two tasks different.
const NOISE: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "will", "would", "can", "could", "should", "she",
    "they", "you", "him", "her", "them", "his", "our", "have", "has", "had", "does", "did", "get",
    "got", "make", "made", "out", "then", "than", "from", "into", "about", "when",
];

/// The identity-carrying words of a task: lowercase, longer than two characters, not noise.
pub fn signature(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 2 && !NOISE.contains(w))
        .map(str::to_owned)
        .collect()
}

/// Overlap over the **smaller** signature. Jaccard (over the union) punishes a detailed wording
/// for its detail: the harvester's version of a promise often has twice the words, so the union
/// grows and the score falls exactly when the two are most obviously the same thing.
pub fn similarity(left: &str, right: &str) -> f64 {
    let (a, b) = (signature(left), signature(right));
    let smaller = a.len().min(b.len());
    if smaller == 0 {
        return 0.0;
    }
    // Counts of words in one sentence; far below f64's exact-integer range.
    a.intersection(&b).count() as f64 / smaller as f64
}

/// Pairs scoring at least this go to the model. Low on purpose: a same-obligation pair that
/// shares only three or four distinctive words scores about a third, and the model, not the
/// arithmetic, should decide it.
pub const SHORTLIST_THRESHOLD: f64 = 0.33;

/// Index pairs `(i, j)`, `i < j`, worth asking about, best first.
pub fn candidate_pairs(texts: &[&str]) -> Vec<(usize, usize, f64)> {
    let mut pairs = Vec::new();
    for i in 0..texts.len() {
        for j in i + 1..texts.len() {
            let score = similarity(texts[i], texts[j]);
            if score >= SHORTLIST_THRESHOLD {
                pairs.push((i, j, score));
            }
        }
    }
    pairs.sort_by(|x, y| y.2.total_cmp(&x.2).then((x.0, x.1).cmp(&(y.0, y.1))));
    pairs
}

/// The dedup judge's answer, as JSON Schema. [`parse_verdict`] enforces the same required fields.
pub const DEDUP_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "same": {"type": "boolean"},
    "keep": {"type": "string", "enum": ["A", "B"]},
    "why": {"type": "string"}
  },
  "required": ["same", "keep"],
  "additionalProperties": false
}"#;

/// Fields [`parse_verdict`] requires; [`DEDUP_SCHEMA`] lists the same.
pub const DEDUP_REQUIRED: [&str; 2] = ["same", "keep"];

const DEDUP_SYSTEM: &str = r#"Two task descriptions were taken from the same meeting by two different processes. Decide whether they describe THE SAME obligation, in which case one of them should leave a to-do list.

The same obligation means doing one would discharge the other. A different scope, different people, or two steps in a sequence are NOT the same obligation, even when they concern the same subject.

Answer with JSON only:
{"same": true or false, "keep": "A" or "B", "why": "under fifteen words"}

When they are the same, "keep" is whichever is more specific and more useful to read cold in a week."#;

/// The request for one pair.
pub fn judge_request(a: &str, b: &str) -> LlmRequest {
    LlmRequest {
        system: DEDUP_SYSTEM.to_owned(),
        user: format!("A: \"{a}\"\nB: \"{b}\""),
        max_tokens: 200,
        temperature: 0.0,
        json_schema: Some(DEDUP_SCHEMA.to_owned()),
    }
}

/// Which of a pair to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// The first.
    A,
    /// The second.
    B,
}

/// The dedup judge's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// Whether the two are one obligation.
    pub same: bool,
    /// Which to keep when they are.
    pub keep: Keep,
}

const DEDUP_TASK: &str = "dedup judgement";

/// Reads a dedup answer, checking it has **the dedup judge's** shape.
pub fn parse_verdict(text: &str) -> Result<Verdict, LlmError> {
    let map = object_in(text, DEDUP_TASK)?;
    let fields = Fields::new(DEDUP_TASK, &map);
    let same = fields.boolean("same")?;
    let keep = match fields.string("keep")?.trim() {
        "A" | "a" => Keep::A,
        "B" | "b" => Keep::B,
        _ => return Err(bad_field(DEDUP_TASK, "keep", "is not \"A\" or \"B\"")),
    };
    Ok(Verdict { same, keep })
}

/// Fold `from` into `into`, by index into the list given to [`dedup`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Merge {
    /// The commitment folded away.
    pub from: usize,
    /// The one kept.
    pub into: usize,
}

/// What [`dedup`] decided.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dedup {
    /// Merges to apply, in the order they were decided.
    pub merges: Vec<Merge>,
    /// Pairs the model was asked about.
    pub judged: usize,
    /// Answers without the dedup judge's shape (those pairs stay apart).
    pub malformed: usize,
}

/// **Worker.** Finds the duplicate commitments in `items`. Each commitment is folded at most once
/// and never into one that was itself folded, so merges never chain. A malformed answer leaves
/// its pair apart; a cancellation, refusal or network failure ends the run with that error.
pub fn dedup(
    items: &[NewCommitment],
    llm: &dyn Llm,
    cancel: &CancelToken,
) -> Result<Dedup, LlmError> {
    let texts: Vec<&str> = items.iter().map(|c| c.text.as_str()).collect();
    let mut result = Dedup::default();
    let mut folded = vec![false; items.len()];
    let mut kept = vec![false; items.len()];
    for (a, b, _) in candidate_pairs(&texts) {
        if folded[a] || folded[b] {
            continue;
        }
        result.judged += 1;
        let answer = match ask(llm, &judge_request(texts[a], texts[b]), cancel) {
            Ok(answer) => answer,
            Err(e) if is_fatal(&e) => return Err(e),
            Err(_) => {
                result.malformed += 1;
                continue;
            }
        };
        let Ok(verdict) = parse_verdict(&answer) else {
            result.malformed += 1;
            continue;
        };
        if !verdict.same {
            continue;
        }
        let (from, into) = match verdict.keep {
            Keep::A => (b, a),
            Keep::B => (a, b),
        };
        // A commitment that already absorbed another stays, so merges never chain: fold the
        // other way instead, or, when both have absorbed others, leave the pair apart.
        let (from, into) = if kept[from] {
            (into, from)
        } else {
            (from, into)
        };
        if kept[from] {
            continue;
        }
        folded[from] = true;
        kept[into] = true;
        result.merges.push(Merge { from, into });
    }
    Ok(result)
}

/// Applies `merges` to commitments stored in the same order as the list given to [`dedup`]
/// (`ids[i]` is item `i`'s id, as [`Store::add_commitments`] returns them).
pub fn apply_merges(
    store: &dyn Store,
    ids: &[CommitmentId],
    merges: &[Merge],
) -> Result<(), StoreError> {
    for merge in merges {
        let (Some(from), Some(into)) = (ids.get(merge.from), ids.get(merge.into)) else {
            return Err(StoreError::Invalid("merge index out of range".into()));
        };
        store.merge_commitment(from, into)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_keep_distinctive_words_only() {
        let sig = signature("Send the revised budget to the finance team, and then THE deck.");
        let words: Vec<&str> = sig.iter().map(String::as_str).collect();
        assert_eq!(
            words,
            ["budget", "deck", "finance", "revised", "send", "team"]
        );
    }

    #[test]
    fn similarity_is_containment_not_jaccard() {
        let short = "Send the revised budget to the finance team";
        let long = "Send the finance team the revised budget spreadsheet before the review";
        // Every distinctive word of the short one is in the long one.
        assert_eq!(similarity(short, long), 1.0);
        assert_eq!(similarity(short, "Book a room for the offsite"), 0.0);
        assert_eq!(similarity("", short), 0.0);
    }

    #[test]
    fn pairs_below_the_threshold_are_not_shortlisted() {
        let texts = [
            "Send the revised budget to the finance team",
            "Book a room for the offsite",
            "Send the finance team the revised budget spreadsheet before the review",
        ];
        let pairs = candidate_pairs(&texts);
        assert_eq!(pairs.len(), 1);
        assert_eq!((pairs[0].0, pairs[0].1), (0, 2));
    }

    #[test]
    fn the_dedup_schema_and_the_parser_agree() {
        let schema: serde_json::Value = serde_json::from_str(DEDUP_SCHEMA).unwrap();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(required, DEDUP_REQUIRED);
    }

    #[test]
    fn verdicts_are_parsed_and_malformed_ones_refused() {
        assert_eq!(
            parse_verdict(r#"{"same": true, "keep": "B", "why": "same deliverable"}"#).unwrap(),
            Verdict {
                same: true,
                keep: Keep::B
            }
        );
        assert!(parse_verdict(r#"{"same": "yes", "keep": "B"}"#).is_err());
        assert!(parse_verdict(r#"{"same": true, "keep": "both"}"#).is_err());
        assert!(parse_verdict(r#"{"same": true}"#).is_err());
    }
}
