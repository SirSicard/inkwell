//! Downmix: one mono signal from a device's interleaved channels, chosen per stream.
//!
//! - **Mic: the primary channel.** Averaging is wrong for microphones. A built-in array's channels
//!   are spatially separated, so one voice arrives phase-shifted on each and their sum comb-filters
//!   it; a second channel that is dead halves the speech (−6 dB); one wired phase-inverted cancels
//!   it. Channel 0 is the one every device calls primary.
//! - **Far end: the average.** System audio is a real mix: the conferencing app may pan
//!   participants apart (spatial audio), and a voice panned hard to one side exists on that side
//!   only, so taking one channel could drop a whole speaker. Averaging keeps every source. Its cost
//!   is a few dB of level when channels carry different content, and the gain stage restores that.
//!   A dead or inverted channel is a microphone fault, not something a mixed output does.

use ink_core::Channel;

/// How a stream's channels become one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Downmix {
    /// Channel 0 only.
    Primary,
    /// The mean of every channel.
    Average,
}

impl Downmix {
    /// The choice for each side of the conversation (see the module docs).
    pub fn for_channel(channel: Channel) -> Self {
        match channel {
            Channel::Mic => Self::Primary,
            Channel::Far => Self::Average,
        }
    }

    /// Appends one mono sample per frame of `interleaved` to `out`.
    ///
    /// **Pump.** Allocation-free when `out` has room for `interleaved.len() / channels` more
    /// samples. A channel count of zero is treated as mono, like
    /// [`AudioBlock::frames`](ink_core::AudioBlock::frames). A trailing partial frame is ignored.
    pub fn apply(self, interleaved: &[f32], channels: u16, out: &mut Vec<f32>) {
        let channels = usize::from(channels.max(1));
        if channels == 1 {
            out.extend_from_slice(interleaved);
            return;
        }
        let frames = interleaved.chunks_exact(channels);
        match self {
            Self::Primary => out.extend(frames.map(|f| f[0])),
            Self::Average => {
                let scale = 1.0 / channels as f32;
                out.extend(frames.map(|f| f.iter().sum::<f32>() * scale));
            }
        }
    }
}
