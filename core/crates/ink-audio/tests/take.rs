//! Dictation takes: 300 ms of lead before the key press and 300 ms of tail after the release.
//!
//! The test signal's samples are their own positions, so every assertion names exactly which audio
//! a take holds.

use ink_audio::take::{HISTORY_SAMPLES, LEAD_SAMPLES, TAIL_SAMPLES, Take, TakeRecorder};

const SR: u64 = 16_000;

/// Samples `from..to` of the position-valued stream.
fn stream(from: u64, to: u64) -> Vec<f32> {
    (from..to).map(|p| p as f32).collect()
}

/// Pushes `from..to` in 10 ms blocks and returns the first take that completes.
fn push_until(rec: &mut TakeRecorder, from: u64, to: u64) -> Option<Take> {
    let mut done = None;
    let mut at = from;
    while at < to {
        let next = (at + 160).min(to);
        if let Some(t) = rec.push(&stream(at, next)) {
            assert!(done.is_none(), "two takes completed");
            done = Some(t);
        }
        at = next;
    }
    done
}

fn assert_holds(take: &Take, from: u64, to: u64) {
    assert_eq!((take.start, take.end), (from, to));
    assert_eq!(take.samples, stream(from, to), "the take's audio");
}

#[test]
fn dictation_keeps_300ms_before_the_press_and_after_the_release() {
    // People start speaking as they press and are still finishing the last word as they let go.
    assert_eq!((LEAD_SAMPLES, TAIL_SAMPLES), (4_800, 4_800));
    let mut rec = TakeRecorder::new();
    assert!(push_until(&mut rec, 0, SR).is_none());
    assert!(rec.press(SR));
    assert!(push_until(&mut rec, SR, 3 * SR).is_none());
    assert!(
        rec.release(3 * SR).is_none(),
        "the tail has not been heard yet"
    );
    let take = push_until(&mut rec, 3 * SR, 4 * SR).expect("take");
    assert_holds(&take, SR - 4_800, 3 * SR + 4_800);
    assert_eq!(
        (take.lead(), take.live(), take.tail()),
        (4_800, 2 * SR as usize, 4_800)
    );
}

#[test]
fn the_tail_is_waited_for_in_audio_not_in_time() {
    // No timer: the take completes when the stream has delivered 300 ms past the release, however
    // long that takes to arrive.
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, SR);
    rec.press(SR);
    push_until(&mut rec, SR, 2 * SR);
    assert!(rec.release(2 * SR).is_none());
    assert!(rec.push(&stream(2 * SR, 2 * SR + 4_799)).is_none());
    let take = rec
        .push(&stream(2 * SR + 4_799, 2 * SR + 5_000))
        .expect("take");
    assert_holds(&take, SR - 4_800, 2 * SR + 4_800);
}

#[test]
fn a_release_heard_late_is_cut_at_its_own_tail() {
    // The key event reached the core after more audio than the tail: the take ends 300 ms after
    // the release itself, at once, and the audio after it stays free for the next take.
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, SR);
    rec.press(SR);
    push_until(&mut rec, SR, 3 * SR);
    let take = rec.release(2 * SR).expect("the tail is already here");
    assert_holds(&take, SR - 4_800, 2 * SR + 4_800);
}

#[test]
fn back_to_back_takes_never_share_audio() {
    // A quick second press must not re-transcribe (and paste) the end of the first take.
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, SR);
    rec.press(SR);
    push_until(&mut rec, SR, 2 * SR);
    rec.release(2 * SR);
    let first = push_until(&mut rec, 2 * SR, 2 * SR + 4_800).expect("first take");
    // 100 ms after the first take's tail ended.
    let second_press = first.end + 1_600;
    push_until(&mut rec, first.end, second_press);
    rec.press(second_press);
    push_until(&mut rec, second_press, second_press + SR);
    rec.release(second_press + SR);
    let second = push_until(&mut rec, second_press + SR, second_press + 2 * SR).expect("second");
    assert_eq!(
        second.start, first.end,
        "the lead starts where the first take ended"
    );
    assert_eq!(second.lead(), 1_600);
    assert_holds(&second, first.end, second_press + SR + 4_800);
}

#[test]
fn the_live_length_excludes_the_seeded_lead() {
    // A stray tap of the hotkey: 50 ms held. The take always holds 600 ms of lead and tail, so a
    // too-short check must use the live length, or every tap would transcribe the room.
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, SR);
    rec.press(SR);
    push_until(&mut rec, SR, SR + 800);
    rec.release(SR + 800);
    let take = push_until(&mut rec, SR + 800, 2 * SR).expect("take");
    assert_eq!(take.live(), 800);
    assert_eq!(take.samples.len(), 800 + 9_600);
}

#[test]
fn a_press_older_than_the_history_gets_what_is_left_not_garbage() {
    // The history holds 2 s. A press stamped 4.5 s ago can only get what the history still has,
    // and it gets exactly that audio, never a lapped ring's leftovers.
    assert_eq!(HISTORY_SAMPLES, 32_000);
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, 5 * SR);
    rec.press(SR / 2);
    let oldest = 5 * SR - HISTORY_SAMPLES as u64;
    push_until(&mut rec, 5 * SR, 6 * SR);
    rec.release(6 * SR);
    let take = push_until(&mut rec, 6 * SR, 7 * SR).expect("take");
    assert_eq!(
        take.press, oldest,
        "the press moves up to the oldest audio held"
    );
    assert_holds(&take, oldest, 6 * SR + 4_800);
}

#[test]
fn a_press_at_the_very_start_gets_the_lead_there_is() {
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, 1_000);
    rec.press(1_000);
    push_until(&mut rec, 1_000, SR);
    rec.release(SR);
    let take = push_until(&mut rec, SR, 2 * SR).expect("take");
    assert_eq!(take.lead(), 1_000);
    assert_holds(&take, 0, SR + 4_800);
}

#[test]
fn a_press_stamped_ahead_of_the_audio_waits_for_it() {
    // The key event can arrive before the audio of that moment has been drained.
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, SR);
    rec.press(SR + 8_000);
    push_until(&mut rec, SR, 2 * SR);
    rec.release(2 * SR);
    let take = push_until(&mut rec, 2 * SR, 3 * SR).expect("take");
    assert_holds(&take, SR + 3_200, 2 * SR + 4_800);
}

#[test]
fn a_take_open_when_the_stream_stops_keeps_what_arrived() {
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, SR);
    rec.press(SR);
    push_until(&mut rec, SR, 2 * SR);
    let take = rec.finish().expect("the open take");
    assert_eq!(take.release, 2 * SR);
    assert_holds(&take, SR - 4_800, 2 * SR);
    assert!(rec.finish().is_none());
}

#[test]
fn a_repeated_press_while_recording_is_ignored() {
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, SR);
    assert!(rec.press(SR));
    push_until(&mut rec, SR, SR + 3_200);
    assert!(!rec.press(SR + 3_200), "key repeat");
    rec.release(SR + 3_200);
    let take = push_until(&mut rec, SR + 3_200, 2 * SR).expect("take");
    assert_eq!(take.press, SR);
    assert!(rec.release(2 * SR).is_none(), "a release with no take open");
}

#[test]
fn a_cancelled_take_is_not_reused_as_the_next_lead() {
    let mut rec = TakeRecorder::new();
    push_until(&mut rec, 0, SR);
    rec.press(SR);
    push_until(&mut rec, SR, 2 * SR);
    rec.cancel();
    assert!(rec.push(&stream(2 * SR, 2 * SR + 1_600)).is_none());
    rec.press(2 * SR + 1_600);
    push_until(&mut rec, 2 * SR + 1_600, 3 * SR);
    rec.release(3 * SR);
    let take = push_until(&mut rec, 3 * SR, 4 * SR).expect("take");
    assert_eq!(take.start, 2 * SR, "nothing of the cancelled take");
}
