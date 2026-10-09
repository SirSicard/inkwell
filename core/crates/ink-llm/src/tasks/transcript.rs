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

/// What the record screen calls each far-end speaker the diarizer told apart, in the order they
/// first speak in `segments` (the whole record): the name the user gave, else `Speaker N`, N being
/// that order, named or not. So naming one renumbers no one, clearing a name brings back the same
/// number, and Ask and the summaries call people what the screen shows. A name for a label the
/// transcript does not have is left out.
pub fn speaker_labels(
    segments: &[Segment],
    names: &[(SpeakerId, String)],
) -> Vec<(SpeakerId, String)> {
    let mut labels: Vec<(SpeakerId, String)> = Vec::new();
    for id in segments
        .iter()
        .filter(|s| s.channel == Channel::Far)
        .filter_map(|s| s.speaker.as_ref())
    {
        if labels.iter().any(|(seen, _)| seen == id) {
            continue;
        }
        let label = names.iter().find(|(named, _)| named == id).map_or_else(
            || format!("Speaker {}", labels.len() + 1),
            |(_, n)| n.clone(),
        );
        labels.push((id.clone(), label));
    }
    labels
}

/// Who said a segment: `You` on the mic, the far-end speaker's label from [`speaker_labels`], or
/// `Them` (the far end as a whole, when labels were not kept).
pub fn speaker_label(segment: &Segment, labels: &[(SpeakerId, String)]) -> String {
    match (segment.channel, &segment.speaker) {
        (Channel::Mic, _) => "You".to_owned(),
        (Channel::Far, Some(id)) => labels
            .iter()
            .find(|(labelled, _)| labelled == id)
            .map_or_else(|| "Them".to_owned(), |(_, label)| label.clone()),
        (Channel::Far, None) => "Them".to_owned(),
    }
}

/// One line: `L{index} [{stamp}] {speaker}: {text}`. `labels` is [`speaker_labels`] of the whole
/// record, worked out once by a caller that renders line by line.
pub fn render_line(index: usize, segment: &Segment, labels: &[(SpeakerId, String)]) -> String {
    format!(
        "L{index} [{}] {}: {}",
        stamp(segment.start_ms),
        speaker_label(segment, labels),
        segment.text.trim()
    )
}

/// The lines `indices` of `segments` (the whole record), one per line, the far end's speakers
/// named or numbered over the whole record ([`speaker_labels`]).
pub fn render(
    segments: &[Segment],
    indices: impl IntoIterator<Item = usize>,
    names: &[(SpeakerId, String)],
) -> String {
    let labels = speaker_labels(segments, names);
    indices
        .into_iter()
        .filter_map(|i| segments.get(i).map(|s| render_line(i, s, &labels)))
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
             L1 [00:04] Speaker 1: Thanks.\n\
             L2 [00:09] Alex: Sounds good.\n\
             L3 [00:12] Them: Bye."
        );
        assert_eq!(render(&segments, [7], &names), "");
    }

    /// The far end's speakers are called what the record screen calls them: the name the user
    /// gave, else "Speaker N", numbered by the order they first speak in the whole record, named
    /// or not (the Mac's RecordDocument, LibraryTests' testEachDiarizedSpeakerKeepsItsNumber...).
    /// So naming one renumbers no one, and a window of lines keeps the record's numbers.
    #[test]
    fn far_end_speakers_are_named_or_numbered_as_the_record_shows_them() {
        let segments = [
            seg(Channel::Far, 0, "The review needs a week.", Some("spk0")),
            seg(Channel::Mic, 486, "Let's settle the date.", None),
            seg(
                Channel::Far,
                10_000,
                "Can you share the budget?",
                Some("spk1"),
            ),
            seg(Channel::Far, 18_278, "The fourteenth works.", Some("spk2")),
            seg(Channel::Far, 19_000, "And the room?", Some("spk1")),
            seg(Channel::Far, 20_000, "Thanks, all.", None),
        ];
        let names = [(SpeakerId("spk0".into()), "Alex".into())];
        assert_eq!(
            speaker_labels(&segments, &names),
            [
                (SpeakerId("spk0".into()), "Alex".to_owned()),
                (SpeakerId("spk1".into()), "Speaker 2".to_owned()),
                (SpeakerId("spk2".into()), "Speaker 3".to_owned()),
            ]
        );
        assert_eq!(
            render(&segments, 2..5, &names),
            "L2 [00:10] Speaker 2: Can you share the budget?\n\
             L3 [00:18] Speaker 3: The fourteenth works.\n\
             L4 [00:19] Speaker 2: And the room?",
            "numbered over the whole record, not the window"
        );
        // Unnamed, Alex is Speaker 1 again, and the others keep their numbers.
        assert_eq!(
            render(&segments, [0, 3], &[]),
            "L0 [00:00] Speaker 1: The review needs a week.\n\
             L3 [00:18] Speaker 3: The fourteenth works."
        );
        // A name for a label the transcript no longer has names nobody.
        let stale = [(SpeakerId("spk9".into()), "Gone".into())];
        assert_eq!(speaker_labels(&segments, &stale).len(), 3);
    }
}
