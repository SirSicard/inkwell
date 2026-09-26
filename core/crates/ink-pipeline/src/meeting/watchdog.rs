//! The silent-channel watchdog: a side that stops delivering, or delivers only digital zeros, is
//! said while the meeting is live. An earlier implementation recorded one side for four weeks
//! without saying so.
//!
//! It judges each block with `ink_audio::levels` ([`assess`]: exact zeros are
//! [`CaptureHealth::DigitalSilence`], a failure, never a quiet room) and keeps time on the platform
//! clock. What counts as broken depends on how each side is routed ([`Routing`]), because two
//! healthy captures look silent too (S2.1a's findings):
//!
//! | Side | What it shows | Means | Reported |
//! |---|---|---|---|
//! | mic | no audio for [`STOP_LIMIT`] | a stalled or vanished device: an input always calls back | [`SideState::Stopped`] |
//! | mic, not Bluetooth | only exact zeros for [`ZEROS_LIMIT`] | denied permission, a Bluetooth mic inside an aggregate, input muted to zero | [`SideState::Zeros`] |
//! | mic, Bluetooth | only exact zeros | the headset gates to zeros while the user is silent: normal | nothing |
//! | far, [`FarDelivery::WhilePlaying`] | no audio | nothing is playing (a macOS tap-only aggregate calls back only then): idle | nothing |
//! | far, [`FarDelivery::Continuous`] | no audio for [`STOP_LIMIT`] | a stalled source | [`SideState::Stopped`] |
//! | far | only exact zeros for [`ZEROS_LIMIT`] | a denied system-audio capture | [`SideState::Zeros`] |
//! | far | no signal for [`FAR_QUIET_LIMIT`] while the mic was audible for [`MIC_ACTIVE_LIMIT`] of it | maybe the wrong app is tapped, or its audio goes elsewhere; maybe only a presentation | a soft warning, once per quiet stretch |
//!
//! **The limits.** [`ZEROS_LIMIT`] is 10 s: a revoked system-audio permission must be reported
//! within 20 s (S2.8), and the zeros begin only once the meeting plays something, so the budget
//! leaves room for that and for the pump's latency. A shorter limit would call a brief muted or
//! gated stretch a failure. [`STOP_LIMIT`] is also 10 s: a device switch (a headset connecting)
//! can pause input for a few seconds.
//!
//! A state returns to [`SideState::Ok`] as soon as the side delivers again (real samples, for
//! [`SideState::Zeros`]).
//!
//! Pure and deterministic: the caller passes the time. The meeting chain feeds it every block and
//! calls [`Watchdog::check`] at [`Watchdog::deadline_ns`] when no block arrives.

use ink_audio::gain::{LEVEL_FRAME, from_dbfs, rms};
use ink_audio::{CaptureHealth, LevelMeter, assess};
use ink_core::{CANONICAL_RATE, Channel, Transport};

use crate::speech::AUDIBLE_FLOOR_DBFS;

const NS_PER_S: u64 = 1_000_000_000;

/// Only exact zeros for this long is a failure: 10 s (module docs).
pub const ZEROS_LIMIT_NS: u64 = 10 * NS_PER_S;
/// [`ZEROS_LIMIT_NS`] as a duration.
pub const ZEROS_LIMIT: std::time::Duration = std::time::Duration::from_nanos(ZEROS_LIMIT_NS);

/// No audio at all for this long, where audio must keep coming, is a failure: 10 s.
pub const STOP_LIMIT_NS: u64 = 10 * NS_PER_S;
/// [`STOP_LIMIT_NS`] as a duration.
pub const STOP_LIMIT: std::time::Duration = std::time::Duration::from_nanos(STOP_LIMIT_NS);

/// No far-end signal for this long while the mic is audible earns the soft warning: 60 s.
pub const FAR_QUIET_LIMIT_NS: u64 = 60 * NS_PER_S;
/// [`FAR_QUIET_LIMIT_NS`] as a duration.
pub const FAR_QUIET_LIMIT: std::time::Duration =
    std::time::Duration::from_nanos(FAR_QUIET_LIMIT_NS);

/// How much of the far end's quiet stretch the mic must have been audible for: 20 s.
pub const MIC_ACTIVE_LIMIT_NS: u64 = 20 * NS_PER_S;
/// [`MIC_ACTIVE_LIMIT_NS`] as a duration.
pub const MIC_ACTIVE_LIMIT: std::time::Duration =
    std::time::Duration::from_nanos(MIC_ACTIVE_LIMIT_NS);

/// How the far end delivers audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FarDelivery {
    /// Only while something plays: no audio means nothing is playing (a macOS tap-only
    /// aggregate). The Mac default.
    WhilePlaying,
    /// Always, silence included: no audio means the source stalled.
    Continuous,
}

/// What the watchdog needs to know about the capture's routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Routing {
    /// How the recorded mic connects. A Bluetooth headset mic (the user's setting) gives exact
    /// zeros while the user is silent.
    pub mic: Transport,
    /// How the far end delivers.
    pub far: FarDelivery,
}

impl Default for Routing {
    /// The built-in mic and a macOS system-audio tap.
    fn default() -> Self {
        Self {
            mic: Transport::BuiltIn,
            far: FarDelivery::WhilePlaying,
        }
    }
}

/// What a side is delivering, as the watchdog judges it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideState {
    /// Audio (or, on a far end that plays only while something plays, nothing to play).
    Ok,
    /// No audio for [`STOP_LIMIT`] where it must keep coming: a stalled or vanished device.
    Stopped,
    /// Only exact zeros for [`ZEROS_LIMIT`]: no data at all (a denied capture, most often).
    Zeros,
}

/// What the watchdog has to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watch {
    /// A side's state changed.
    State {
        /// Which side.
        channel: Channel,
        /// Its new state.
        state: SideState,
    },
    /// The far end has delivered no signal for [`FAR_QUIET_LIMIT`] or more while the mic was
    /// audible: a soft warning.
    FarQuietWhileYouSpeak {
        /// How long the far end has been without signal, ms.
        quiet_ms: u64,
    },
}

#[derive(Clone, Copy, Debug)]
struct Side {
    /// When the last block arrived.
    last_block: Option<u64>,
    /// When the current run of zero blocks began.
    zeros_since: Option<u64>,
    state: SideState,
}

impl Side {
    fn new() -> Self {
        Self {
            last_block: None,
            zeros_since: None,
            state: SideState::Ok,
        }
    }
}

/// The watchdog. See the module docs. **Worker** (the meeting chain's thread).
#[derive(Clone, Debug)]
pub struct Watchdog {
    routing: Routing,
    start: u64,
    sides: [Side; 2],
    /// When the far end last delivered real samples (or the start).
    far_signal: u64,
    /// Mic time above the audible floor since then, ns.
    mic_audible: u64,
    far_quiet_warned: bool,
}

fn side(channel: Channel) -> usize {
    usize::from(channel == Channel::Far)
}

impl Watchdog {
    /// A watchdog for a meeting that started at host time `start_ns`.
    pub fn new(routing: Routing, start_ns: u64) -> Self {
        Self {
            routing,
            start: start_ns,
            sides: [Side::new(), Side::new()],
            far_signal: start_ns,
            mic_audible: 0,
            far_quiet_warned: false,
        }
    }

    /// The routing changed (a device switched). Judged from the next check on.
    pub fn set_routing(&mut self, routing: Routing) {
        self.routing = routing;
    }

    /// A block of `channel` (16 kHz mono) arrived at `now_ns`. Returns what changed.
    pub fn observe(&mut self, channel: Channel, samples: &[f32], now_ns: u64) -> Vec<Watch> {
        let mut meter = LevelMeter::new();
        meter.add(samples);
        let s = &mut self.sides[side(channel)];
        match assess(channel, 1, &meter) {
            CaptureHealth::DigitalSilence => {
                s.last_block = Some(now_ns);
                // The zeros began with the block's first sample, a block's length before it
                // arrived: ten seconds of zeros are ten seconds of audio.
                let length = samples.len() as u64 * NS_PER_S / u64::from(CANONICAL_RATE);
                s.zeros_since.get_or_insert(now_ns.saturating_sub(length));
            }
            CaptureHealth::Signal => {
                s.last_block = Some(now_ns);
                s.zeros_since = None;
                if channel == Channel::Far {
                    self.far_signal = now_ns;
                    self.mic_audible = 0;
                    self.far_quiet_warned = false;
                }
            }
            // An empty block says nothing.
            CaptureHealth::NoCallbacks | CaptureHealth::Idle => {}
        }
        if channel == Channel::Mic {
            let floor = from_dbfs(AUDIBLE_FLOOR_DBFS);
            let audible: u64 = samples
                .chunks(LEVEL_FRAME)
                .filter(|frame| rms(frame) > floor)
                .map(|frame| frame.len() as u64)
                .sum();
            self.mic_audible += audible * NS_PER_S / u64::from(CANONICAL_RATE);
        }
        self.check(now_ns)
    }

    /// Judges every side at `now_ns`. Returns what changed.
    pub fn check(&mut self, now_ns: u64) -> Vec<Watch> {
        let mut out = Vec::new();
        for channel in [Channel::Mic, Channel::Far] {
            let state = self.judge(channel, now_ns);
            let s = &mut self.sides[side(channel)];
            if s.state != state {
                s.state = state;
                out.push(Watch::State { channel, state });
            }
        }
        let quiet = now_ns.saturating_sub(self.far_signal);
        if !self.far_quiet_warned
            && self.sides[1].state == SideState::Ok
            && quiet >= FAR_QUIET_LIMIT_NS
            && self.mic_audible >= MIC_ACTIVE_LIMIT_NS
        {
            self.far_quiet_warned = true;
            out.push(Watch::FarQuietWhileYouSpeak {
                quiet_ms: quiet / 1_000_000,
            });
        }
        out
    }

    fn judge(&self, channel: Channel, now: u64) -> SideState {
        let s = &self.sides[side(channel)];
        let must_flow = match channel {
            Channel::Mic => true,
            Channel::Far => self.routing.far == FarDelivery::Continuous,
        };
        if must_flow && now.saturating_sub(s.last_block.unwrap_or(self.start)) >= STOP_LIMIT_NS {
            return SideState::Stopped;
        }
        let zeros_expected = channel == Channel::Mic && self.routing.mic == Transport::Bluetooth;
        match s.zeros_since {
            Some(since) if !zeros_expected && now.saturating_sub(since) >= ZEROS_LIMIT_NS => {
                SideState::Zeros
            }
            _ => SideState::Ok,
        }
    }

    /// The next time a check could change something with no block arriving, or `None`.
    pub fn deadline_ns(&self) -> Option<u64> {
        let mut next: Option<u64> = None;
        let mut consider = |t: u64| next = Some(next.map_or(t, |n| n.min(t)));
        for channel in [Channel::Mic, Channel::Far] {
            let s = &self.sides[side(channel)];
            let must_flow = channel == Channel::Mic || self.routing.far == FarDelivery::Continuous;
            if must_flow && s.state != SideState::Stopped {
                consider(s.last_block.unwrap_or(self.start) + STOP_LIMIT_NS);
            }
            if let Some(since) = s.zeros_since
                && s.state == SideState::Ok
            {
                consider(since + ZEROS_LIMIT_NS);
            }
        }
        if !self.far_quiet_warned {
            consider(self.far_signal + FAR_QUIET_LIMIT_NS);
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = NS_PER_S;
    const BLOCK: u64 = S / 100;

    fn tone() -> Vec<f32> {
        (0..160).map(|i| (i as f32 * 0.3).sin() * 0.1).collect()
    }

    /// Feeds `seconds` of 10 ms blocks of `samples` to `channel` from `from`; returns the events
    /// with the time each came at.
    fn run(
        w: &mut Watchdog,
        feeds: &[(Channel, &[f32])],
        from: u64,
        seconds: u64,
    ) -> Vec<(u64, Watch)> {
        let mut out = Vec::new();
        let mut t = from;
        while t < from + seconds * S {
            for (channel, samples) in feeds {
                for e in w.observe(*channel, samples, t) {
                    out.push((t, e));
                }
            }
            for e in w.check(t) {
                out.push((t, e));
            }
            t += BLOCK;
        }
        out
    }

    #[test]
    fn a_far_end_of_zeros_is_reported_within_the_limit_and_clears_on_signal() {
        let mut w = Watchdog::new(Routing::default(), 0);
        let zeros = [0.0f32; 160];
        let events = run(
            &mut w,
            &[(Channel::Mic, &tone()), (Channel::Far, &zeros)],
            0,
            12,
        );
        assert_eq!(
            events,
            [(
                ZEROS_LIMIT_NS,
                Watch::State {
                    channel: Channel::Far,
                    state: SideState::Zeros
                }
            )]
        );
        let back = w.observe(Channel::Far, &tone(), 12 * S);
        assert_eq!(
            back,
            [Watch::State {
                channel: Channel::Far,
                state: SideState::Ok
            }]
        );
    }

    #[test]
    fn an_idle_tap_far_end_is_not_broken() {
        let mut w = Watchdog::new(Routing::default(), 0);
        // The far end calls back not once in 50 s; the mic is quiet.
        let events = run(&mut w, &[(Channel::Mic, &[0.0005; 160])], 0, 50);
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn a_continuous_far_end_that_stops_is_reported() {
        let routing = Routing {
            far: FarDelivery::Continuous,
            ..Routing::default()
        };
        let mut w = Watchdog::new(routing, 0);
        let events = run(&mut w, &[(Channel::Mic, &tone())], 0, 11);
        assert_eq!(
            events,
            [(
                STOP_LIMIT_NS,
                Watch::State {
                    channel: Channel::Far,
                    state: SideState::Stopped
                }
            )]
        );
    }

    #[test]
    fn a_bluetooth_mic_giving_zeros_is_normal_and_a_built_in_one_is_not() {
        let zeros = [0.0f32; 160];
        let bluetooth = Routing {
            mic: Transport::Bluetooth,
            ..Routing::default()
        };
        let mut w = Watchdog::new(bluetooth, 0);
        assert!(run(&mut w, &[(Channel::Mic, &zeros)], 0, 30).is_empty());
        let mut w = Watchdog::new(Routing::default(), 0);
        let events = run(&mut w, &[(Channel::Mic, &zeros)], 0, 30);
        assert_eq!(
            events,
            [(
                ZEROS_LIMIT_NS,
                Watch::State {
                    channel: Channel::Mic,
                    state: SideState::Zeros
                }
            )]
        );
    }

    #[test]
    fn a_mic_that_stops_calling_back_is_reported_by_the_clock_alone() {
        let mut w = Watchdog::new(Routing::default(), 0);
        run(&mut w, &[(Channel::Mic, &tone())], 0, 2);
        let last = 2 * S - BLOCK;
        assert_eq!(w.deadline_ns(), Some(last + STOP_LIMIT_NS));
        assert!(w.check(last + STOP_LIMIT_NS - 1).is_empty());
        assert_eq!(
            w.check(last + STOP_LIMIT_NS),
            [Watch::State {
                channel: Channel::Mic,
                state: SideState::Stopped
            }]
        );
        // Bluetooth or not: a device that stops calling back has stopped.
    }

    #[test]
    fn the_far_end_quiet_while_you_talk_is_a_soft_warning_once() {
        let mut w = Watchdog::new(Routing::default(), 0);
        let events = run(&mut w, &[(Channel::Mic, &tone())], 0, 70);
        assert_eq!(
            events,
            [(
                FAR_QUIET_LIMIT_NS,
                Watch::FarQuietWhileYouSpeak { quiet_ms: 60_000 }
            )]
        );
        // The far end plays: re-armed, and nothing until another minute.
        assert!(w.observe(Channel::Far, &tone(), 70 * S).is_empty());
        let later = run(&mut w, &[(Channel::Mic, &tone())], 70 * S, 30);
        assert!(later.is_empty(), "{later:?}");
    }

    #[test]
    fn a_quiet_minute_with_a_quiet_mic_is_no_warning() {
        let mut w = Watchdog::new(Routing::default(), 0);
        // You speak for 10 s of the minute: not enough to suspect the far end.
        let talk = run(&mut w, &[(Channel::Mic, &tone())], 0, 10);
        let hush = run(&mut w, &[(Channel::Mic, &[0.0005; 160])], 10 * S, 60);
        assert!(talk.is_empty() && hush.is_empty(), "{talk:?} {hush:?}");
    }
}
