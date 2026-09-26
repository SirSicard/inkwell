//! Transcript lines as the prompts show them: `L12 [03:25] You: text`.
//!
//! Every line carries its index in the record's segments, and models cite lines by that number
//! rather than by time. Models are poor at turning `[03:25]` into milliseconds, and a line number
//! maps back to the exact segment, so provenance survives the model.

use ink_core::{Channel, Segment, SpeakerId};

/// `mm:ss`, or `h:mm:ss` from an hour on.
pub fn stamp(ms: u64) -> String {
    let seconds = ms / 1000;
    let (h, m, s) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Who said a segment: `You` on the mic, the speaker's name or label on the far end, or `Them`.
pub fn speaker_label(segment: &Segment, names: &[(SpeakerId, String)]) -> String {
    match (segment.channel, &segment.speaker) {
        (Channel::Mic, _) => "You".to_owned(),
        (Channel::Far, Some(id)) => names
            .iter()
            .find(|(named, _)| named == id)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| format!("Them ({})", id.0)),
        (Channel::Far, None) => "Them".to_owned(),
    }
}

/// One line: `L{index} [{stamp}] {speaker}: {text}`.
pub fn render_line(index: usize, segment: &Segment, names: &[(SpeakerId, String)]) -> String {
    format!(
        "L{index} [{}] {}: {}",
        stamp(segment.start_ms),
        speaker_label(segment, names),
        segment.text.trim()
    )
}

/// The lines `indices` of `segments`, one per line.
pub fn render(
    segments: &[Segment],
    indices: impl IntoIterator<Item = usize>,
    names: &[(SpeakerId, String)],
) -> String {
    indices
        .into_iter()
        .filter_map(|i| segments.get(i).map(|s| render_line(i, s, names)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(channel: Channel, start_ms: u64, text: &str, speaker: Option<&str>) -> Segment {
        Segment {
            channel,
            start_ms,
            end_ms: start_ms + 1000,
            text: text.into(),
            speaker: speaker.map(|s| SpeakerId(s.into())),
        }
    }

    #[test]
    fn stamps_switch_to_hours_after_an_hour() {
        assert_eq!(stamp(0), "00:00");
        assert_eq!(stamp(205_000), "03:25");
        assert_eq!(stamp(3_725_000), "1:02:05");
    }

    #[test]
    fn lines_carry_their_index_and_who_spoke() {
        let segments = [
            seg(Channel::Mic, 0, " I'll send the notes. ", None),
            seg(Channel::Far, 4_000, "Thanks.", Some("spk0")),
            seg(Channel::Far, 9_000, "Sounds good.", Some("spk1")),
            seg(Channel::Far, 12_000, "Bye.", None),
        ];
        let names = [(SpeakerId("spk1".into()), "Alex".into())];
        assert_eq!(
            render(&segments, 0..segments.len(), &names),
            "L0 [00:00] You: I'll send the notes.\n\
             L1 [00:04] Them (spk0): Thanks.\n\
             L2 [00:09] Alex: Sounds good.\n\
             L3 [00:12] Them: Bye."
        );
        assert_eq!(render(&segments, [7], &names), "");
    }
}
