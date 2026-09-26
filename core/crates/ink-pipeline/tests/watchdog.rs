//! The silent-channel watchdog in the live meeting: a side that stops, or delivers only digital
//! zeros, is said within the limit, and the two captures that look silent while healthy (an idle
//! system-audio tap, a Bluetooth headset mic while you say nothing) are not.

mod meeting_rig;

use ink_core::{Channel, Transport};
use ink_pipeline::meeting::events::{MeetingEvent, MeetingWarning};
use ink_pipeline::meeting::watchdog::{Routing, SideState, ZEROS_LIMIT_NS};
use meeting_rig::*;

fn states(rig: &Rig) -> Vec<(Channel, SideState)> {
    rig.events()
        .into_iter()
        .filter_map(|e| match e {
            MeetingEvent::SideState { channel, state } => Some((channel, state)),
            _ => None,
        })
        .collect()
}

fn moving() -> RigBuilder {
    RigBuilder {
        moving_clock: true,
        ..RigBuilder::default()
    }
}

/// The silent-channel fixture: system audio denied, so the tap delivers exact zeros while the
/// meeting plays. Reported within 10 s (S2.8 needs a revoked permission said within 20 s), and
/// cleared when real audio comes back.
#[test]
fn a_far_end_of_digital_zeros_is_reported_within_ten_seconds() {
    assert_eq!(ZEROS_LIMIT_NS, 10_000_000_000);
    let mut rig = moving().build();
    let talk = speech(12.0, -30.0, 141);
    // Up to 9.99 s: nothing yet.
    rig.feed(&talk[..159_840], &silence(9.99));
    assert!(states(&rig).is_empty(), "{:?}", states(&rig));
    // At 10 s of zeros: the far end is reported.
    rig.feed(&talk[159_840..160_000], &silence(0.01));
    assert_eq!(states(&rig), [(Channel::Far, SideState::Zeros)]);
    // The permission comes back: real audio, and the state clears.
    rig.feed(&talk[160_000..176_000], &speech(1.0, -30.0, 142));
    assert_eq!(
        states(&rig),
        [
            (Channel::Far, SideState::Zeros),
            (Channel::Far, SideState::Ok)
        ]
    );
}

/// An idle tap (nothing plays, so no callbacks) and a Bluetooth headset mic that gates to zeros
/// while you say nothing are healthy: nothing is raised, however long they last.
#[test]
fn an_idle_far_end_and_a_quiet_bluetooth_mic_raise_nothing() {
    let mut rig = RigBuilder {
        routing: Routing {
            mic: Transport::Bluetooth,
            ..Routing::default()
        },
        ..moving()
    }
    .build();
    rig.feed_side(Channel::Mic, &silence(40.0));
    rig.advance(1_000_000);
    rig.tick();
    assert!(states(&rig).is_empty(), "{:?}", states(&rig));
    assert!(rig.warnings().is_empty(), "{:?}", rig.warnings());

    // The same zeros from the built-in mic are a failure.
    let mut rig = moving().build();
    rig.feed_side(Channel::Mic, &silence(12.0));
    assert_eq!(states(&rig), [(Channel::Mic, SideState::Zeros)]);
}

/// A mic that stops calling back is reported by the clock alone, at the chain's deadline.
#[test]
fn a_mic_that_stops_is_reported_at_the_deadline() {
    let mut rig = moving().build();
    rig.feed_side(Channel::Mic, &speech(2.0, -30.0, 143));
    let deadline = rig.chain().deadline_ns().expect("a deadline while live");
    rig.advance(deadline - rig.clock.as_ref().unwrap().now_ns() - 1);
    rig.tick();
    assert!(states(&rig).is_empty());
    rig.advance(1);
    rig.tick();
    assert_eq!(states(&rig), [(Channel::Mic, SideState::Stopped)]);
    rig.feed_side(Channel::Mic, &speech(0.1, -30.0, 144));
    assert_eq!(
        states(&rig),
        [
            (Channel::Mic, SideState::Stopped),
            (Channel::Mic, SideState::Ok)
        ]
    );
}

/// You talk for a minute and nothing plays from the meeting: a soft warning, once.
#[test]
fn the_far_end_quiet_while_you_talk_is_a_soft_warning() {
    let mut rig = moving().build();
    rig.feed_side(Channel::Mic, &speech(61.0, -30.0, 145));
    let warnings: Vec<_> = rig
        .warnings()
        .into_iter()
        .filter(|w| matches!(w, MeetingWarning::FarEndQuietWhileYouSpeak { .. }))
        .collect();
    assert_eq!(
        warnings,
        [MeetingWarning::FarEndQuietWhileYouSpeak { quiet_ms: 60_000 }]
    );
    assert!(states(&rig).is_empty(), "idle is not a failure");
}

use ink_core::Clock;
