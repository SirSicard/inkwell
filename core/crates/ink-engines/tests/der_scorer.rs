//! The DER scorer in `der/`, proved on toy annotations with answers worked out by hand before it
//! scores a diarizer (`tests/nemo.rs`). The first seven are the cases the diarization gate proved
//! its pyannote.metrics scorer on, with the same expected values.

mod der;

use der::{Turn, clusters, der, min_speaker_recall, parse_rttm, to_rttm};

fn close(got: f64, want: f64) {
    assert!((got - want).abs() < 1e-9, "got {got}, want {want}");
}

#[test]
fn confusion_only() {
    // Reference A 0-10, B 10-20; hypothesis X 0-10, Y 10-15, X 15-20. The best map is X to A, Y
    // to B, so 15-20 is X on B: 5 s confused of 20 s.
    let reference = [Turn::new(0.0, 10.0, "A"), Turn::new(10.0, 20.0, "B")];
    let hypothesis = [
        Turn::new(0.0, 10.0, "X"),
        Turn::new(10.0, 15.0, "Y"),
        Turn::new(15.0, 20.0, "X"),
    ];
    let d = der(&reference, &hypothesis, 0.0);
    close(d.der, 25.0);
    close(d.confusion, 25.0);
    close(d.miss + d.false_alarm, 0.0);
    // B's cluster got half of B.
    close(min_speaker_recall(&reference, &hypothesis), 50.0);
}

#[test]
fn miss_and_false_alarm_without_a_collar() {
    // Reference A 0-10; hypothesis A 2-12: 2 s missed and 2 s false alarm over 10 s.
    let d = der(
        &[Turn::new(0.0, 10.0, "A")],
        &[Turn::new(2.0, 12.0, "A")],
        0.0,
    );
    close(d.der, 40.0);
    close(d.miss, 20.0);
    close(d.false_alarm, 20.0);
}

#[test]
fn the_collar_is_per_side() {
    // The same with ±0.25 s: 0-0.25 and 9.75-10.25 are not scored, so 9.5 s of reference with
    // 1.75 s missed and 1.75 s false alarm: 36.842 %. pyannote's own `collar` argument is the
    // total width; passing it 0.25 would give 38.46 %, the trap the gate's toy caught.
    let d = der(
        &[Turn::new(0.0, 10.0, "A")],
        &[Turn::new(2.0, 12.0, "A")],
        0.25,
    );
    close(d.total, 9.5);
    close(d.der, 100.0 * 3.5 / 9.5);
    assert!((d.der - 38.46).abs() > 1.0);
}

#[test]
fn overlap_is_scored() {
    // Reference A 0-10 and B 5-10; hypothesis A 0-10 only: B's 5 s missed of 15 s.
    let reference = [Turn::new(0.0, 10.0, "A"), Turn::new(5.0, 10.0, "B")];
    let d = der(&reference, &[Turn::new(0.0, 10.0, "A")], 0.0);
    close(d.total, 15.0);
    close(d.der, 100.0 / 3.0);
    close(d.miss, 100.0 / 3.0);
}

#[test]
fn a_merged_quiet_speaker_gets_zero_recall() {
    // Two people, one cluster: the one who spoke less gets none of their time.
    let reference = [Turn::new(0.0, 18.0, "loud"), Turn::new(18.0, 20.0, "quiet")];
    let hypothesis = [Turn::new(0.0, 20.0, "X")];
    close(min_speaker_recall(&reference, &hypothesis), 0.0);
    assert_eq!(clusters(&hypothesis), (1, 1));
}

#[test]
fn clusters_under_two_percent_are_not_substantial() {
    let hypothesis = [
        Turn::new(0.0, 60.0, "a"),
        Turn::new(60.0, 99.0, "b"),
        Turn::new(99.0, 100.0, "c"),
    ];
    assert_eq!(clusters(&hypothesis), (3, 2));
}

#[test]
fn a_perfect_hypothesis_scores_zero_whatever_its_labels() {
    let reference = [
        Turn::new(0.0, 4.0, "A"),
        Turn::new(3.0, 6.0, "B"),
        Turn::new(7.0, 9.0, "A"),
    ];
    let hypothesis = [
        Turn::new(0.0, 4.0, "spk1"),
        Turn::new(3.0, 6.0, "spk0"),
        Turn::new(7.0, 9.0, "spk1"),
    ];
    for collar in [0.0, 0.25] {
        close(der(&reference, &hypothesis, collar).der, 0.0);
    }
    close(min_speaker_recall(&reference, &hypothesis), 100.0);
}

#[test]
fn an_extra_cluster_is_false_alarm_or_confusion_never_correct() {
    // Reference A 0-10. Hypothesis X 0-6 and Y 6-10: Y cannot also map to A, so 4 s confused.
    let d = der(
        &[Turn::new(0.0, 10.0, "A")],
        &[Turn::new(0.0, 6.0, "X"), Turn::new(6.0, 10.0, "Y")],
        0.0,
    );
    close(d.confusion, 40.0);
    close(d.der, 40.0);
}

#[test]
fn rttm_round_trips_and_a_repeated_segment_replaces_the_first() {
    let text = "SPEAKER m 1 0.500 1.250 <NA> <NA> speaker_1 <NA> <NA>\n\
                SPEAKER m 1 2.000 0.000 <NA> <NA> speaker_2 <NA> <NA>\n\
                SPEAKER m 1 0.500 1.250 <NA> <NA> speaker_3 <NA> <NA>\n\
                # a comment\n";
    let turns = parse_rttm(text);
    assert_eq!(turns, [Turn::new(0.5, 1.75, "speaker_3")]);
    assert_eq!(parse_rttm(&to_rttm("m", &turns)), turns);
}
