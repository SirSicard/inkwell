//! What a hotkey edge means: the hold-versus-toggle decision, ported from Inkwell 0.2's
//! `pipeline.rs` (`decide_transition` and its seven tests).
//!
//! Pure, and apart from the chain, because every bug this logic had was in the decision rather
//! than the plumbing. The minimum hold that filters a modifier used in a shortcut is the chain's
//! (it needs the audio clock); see [`DictationChain`](crate::chain::DictationChain).

/// How the hotkey records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordingMode {
    /// Hold to record, release to stop.
    #[default]
    PushToTalk,
    /// Press to start, press again to stop.
    Toggle,
}

impl RecordingMode {
    /// The stored setting: `toggle` is [`Toggle`](Self::Toggle); anything else, unknown or blank
    /// included, is push to talk, so a hand-edited setting cannot leave the hotkey in a third
    /// state.
    pub fn from_setting(setting: &str) -> Self {
        if setting.trim().eq_ignore_ascii_case("toggle") {
            Self::Toggle
        } else {
            Self::PushToTalk
        }
    }
}

/// Whether a hotkey edge starts a take, stops one, or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    /// Open a take.
    pub start: bool,
    /// Close the take in progress.
    pub stop: bool,
}

/// The decision for one edge: `pressed` (down or up), whether a take is in progress, the mode, and
/// whether the hotkey is the voice-edit one (edits are always push to talk: a toggle would leave
/// the user holding a captured selection with no sign the app is waiting).
pub fn decide_transition(
    pressed: bool,
    is_recording: bool,
    mode: RecordingMode,
    is_edit: bool,
) -> Transition {
    let toggle = !is_edit && mode == RecordingMode::Toggle;
    Transition {
        // Guarded on state, not on the event: a press arriving while already recording once
        // cleared the buffer and started over, discarding what had been said.
        start: pressed && !is_recording,
        // Toggle stops on the next press, push to talk on release; both only when something is
        // running, or a stray release ran the whole stop path on an empty buffer.
        stop: if toggle {
            pressed && is_recording
        } else {
            !pressed && is_recording
        },
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;

    fn t(pressed: bool, recording: bool, mode: &str, edit: bool) -> (bool, bool) {
        let Transition { start, stop } =
            decide_transition(pressed, recording, RecordingMode::from_setting(mode), edit);
        (start, stop)
    }

    #[test]
    fn ptt_press_starts_release_stops() {
        assert_eq!(t(true, false, "ptt", false), (true, false));
        assert_eq!(t(false, true, "ptt", false), (false, true));
    }

    /// The bug that discarded a whole dictation: a second press mid-recording cleared the buffer
    /// and began again. It is also what a lost release looks like.
    #[test]
    fn ptt_press_while_recording_does_nothing() {
        assert_eq!(t(true, true, "ptt", false), (false, false));
    }

    /// A release with nothing in flight used to run the stop path on an empty buffer.
    #[test]
    fn ptt_release_while_idle_does_nothing() {
        assert_eq!(t(false, false, "ptt", false), (false, false));
    }

    #[test]
    fn toggle_stops_on_the_next_press_not_on_release() {
        assert_eq!(t(true, false, "toggle", false), (true, false));
        assert_eq!(t(true, true, "toggle", false), (false, true));
        // Releasing a toggle hotkey is not an event at all.
        assert_eq!(t(false, true, "toggle", false), (false, false));
        assert_eq!(t(false, false, "toggle", false), (false, false));
    }

    /// Voice edits capture a selection first; a toggle recording would strand the user.
    #[test]
    fn edits_are_push_to_talk_even_when_the_setting_says_toggle() {
        assert_eq!(t(true, false, "toggle", true), (true, false));
        assert_eq!(t(false, true, "toggle", true), (false, true));
        assert_eq!(t(true, true, "toggle", true), (false, false));
    }

    /// An unknown mode behaves like push to talk rather than a third state.
    #[test]
    fn unknown_mode_falls_back_to_push_to_talk() {
        assert_eq!(t(true, false, "", false), (true, false));
        assert_eq!(t(false, true, "wibble", false), (false, true));
    }

    /// Starting and stopping in one transition would run the stop path against a take the start
    /// just opened. Checked across the whole input space.
    #[test]
    fn never_starts_and_stops_at_once() {
        for pressed in [true, false] {
            for recording in [true, false] {
                for mode in ["ptt", "toggle", "", "nonsense"] {
                    for edit in [true, false] {
                        let (start, stop) = t(pressed, recording, mode, edit);
                        assert!(
                            !(start && stop),
                            "both for {pressed} {recording} {mode} {edit}"
                        );
                        if start {
                            assert!(!recording);
                        }
                        if stop {
                            assert!(recording);
                        }
                    }
                }
            }
        }
    }
}
