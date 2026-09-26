//! The word error rate scorer the real-model tests use, proved on toy cases with known answers
//! before it scores anything real. These are the reference scorer's own self-test cases.

mod bench;

use bench::{Edits, edit_distance, measure, normalise, score};

fn words(s: &[&str]) -> Vec<String> {
    s.iter().map(|w| (*w).to_owned()).collect()
}

#[test]
fn the_normaliser_lowercases_and_splits_on_everything_but_letters_and_numbers() {
    assert_eq!(
        normalise("Hello, World! It's 5pm."),
        words(&["hello", "world", "it", "s", "5pm"])
    );
    assert_eq!(normalise("Café—naïve"), words(&["café", "naïve"]));
    assert_eq!(
        normalise("mm-hmm uh-huh"),
        words(&["mm", "hmm", "uh", "huh"])
    );
    assert_eq!(
        normalise("  tabs\tand\nlines  "),
        words(&["tabs", "and", "lines"])
    );
    assert!(normalise("—, !").is_empty());
}

#[test]
fn one_substitution_and_one_insertion_in_six_words_is_33_percent() {
    let m = score("The cat sat on the mat.", "the cat sit on the mat today");
    assert_eq!(
        m,
        Edits {
            reference: 6,
            substitutions: 1,
            deletions: 0,
            insertions: 1
        }
    );
    assert!((m.wer() - 100.0 * 2.0 / 6.0).abs() < 1e-9);
}

#[test]
fn one_deletion_in_four_words_is_25_percent() {
    let d = score("one two three four", "one three four");
    assert_eq!((d.deletions, d.total()), (1, 1));
    assert!((d.wer() - 25.0).abs() < 1e-9);
}

#[test]
fn identical_is_zero_and_an_empty_hypothesis_is_all_deletions() {
    assert_eq!(score("a b", "a b").wer(), 0.0);
    let empty = score("a b c", "");
    assert_eq!((empty.deletions, empty.wer()), (3, 100.0));
    assert!(score("", "a").wer().is_nan());
}

#[test]
fn a_corpus_rate_sums_edits_and_words_rather_than_averaging_rates() {
    // (2 edits + 1 edit) / (6 + 4 words) = 30 %, not the mean of 33.3 % and 25 %.
    let corpus = score("The cat sat on the mat.", "the cat sit on the mat today")
        + score("one two three four", "one three four");
    assert!((corpus.wer() - 30.0).abs() < 1e-9);
}

#[test]
fn the_alignment_agrees_with_an_independent_edit_distance_on_random_pairs() {
    // A fixed linear congruential generator: the same 500 pairs every run.
    let mut state: u64 = 7;
    let mut next = |bound: u64| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) % bound
    };
    let vocab = ["a", "b", "c", "d", "e", "f", "g", "h"];
    for _ in 0..500 {
        let r: Vec<&str> = (0..1 + next(30)).map(|_| vocab[next(8) as usize]).collect();
        let h: Vec<&str> = (0..next(31)).map(|_| vocab[next(8) as usize]).collect();
        let m = measure(&r, &h);
        assert_eq!(m.total(), edit_distance(&r, &h), "{r:?} vs {h:?}");
        // Every reference word is matched, substituted or deleted exactly once.
        assert!(m.substitutions + m.deletions <= r.len());
        assert_eq!(h.len() + m.deletions, r.len() + m.insertions);
    }
}
