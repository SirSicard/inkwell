//! Synthetic key events: the paste keystroke and the Unicode-typing fallback.
//!
//! Every event goes out with an explicit keycode and explicit flags. The paste key is the raw
//! keycode 0x09 (`kVK_ANSI_V`), never a layout lookup: resolving a character to a keycode goes
//! through Text Services, which asserts it is on the main thread and kills the process from a
//! worker (Inkwell 0.2 shipped that crash). Every event also carries
//! [`SYNTHETIC_EVENT_MARK`], so this crate's own hotkey tap lets it through.
//!
//! `CGEventPost` may be called from any thread.
#![cfg(target_os = "macos")]

use std::thread;
use std::time::Duration;

use ink_core::PlatformError;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGPreflightPostEventAccess,
};

use crate::hotkey::SYNTHETIC_EVENT_MARK;
use crate::hotkey::binding::keycode;

/// `kVK_ANSI_V`, the paste key by position.
pub(crate) const KEY_V: u16 = keycode::ANSI_V;
/// The keycode the Unicode events carry. Apps read the string, not the key; `kVK_ANSI_A` is the
/// conventional placeholder.
const KEY_FOR_UNICODE: u16 = 0x00;
/// The most UTF-16 units one Unicode key event carries; longer strings are truncated by the
/// system.
pub(crate) const UNICODE_CHUNK: usize = 20;
/// A breath between Unicode events, so a slow target does not drop or reorder them.
const BETWEEN_CHUNKS: Duration = Duration::from_millis(2);

/// Whether this process may post synthetic events. Never prompts.
pub(crate) fn can_post_events() -> bool {
    CGPreflightPostEventAccess()
}

/// Posts Cmd+V: key down and key up of `kVK_ANSI_V` with only the Command flag, so a modifier the
/// user still holds cannot turn it into another shortcut.
pub(crate) fn post_paste() -> Result<(), PlatformError> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
    let down = key_event(source.as_deref(), KEY_V, true, CGEventFlags::MaskCommand)?;
    let up = key_event(source.as_deref(), KEY_V, false, CGEventFlags::MaskCommand)?;
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&down));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&up));
    Ok(())
}

/// Types `text` as Unicode key events, after [`sanitize_for_typing`].
pub(crate) fn type_text(text: &str) -> Result<(), PlatformError> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
    for (i, chunk) in utf16_chunks(&sanitize_for_typing(text), UNICODE_CHUNK)
        .iter()
        .enumerate()
    {
        if i > 0 {
            thread::sleep(BETWEEN_CHUNKS);
        }
        for down in [true, false] {
            let event = key_event(
                source.as_deref(),
                KEY_FOR_UNICODE,
                down,
                CGEventFlags::empty(),
            )?;
            // SAFETY: `chunk` is a live slice of UTF-16 units and the length passed is its
            // length; CoreGraphics copies the string into the event.
            unsafe {
                CGEvent::keyboard_set_unicode_string(
                    Some(&event),
                    chunk.len() as _,
                    chunk.as_ptr(),
                );
            }
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        }
    }
    Ok(())
}

fn key_event(
    source: Option<&CGEventSource>,
    keycode: u16,
    down: bool,
    flags: CGEventFlags,
) -> Result<CFRetained<CGEvent>, PlatformError> {
    let event = CGEvent::new_keyboard_event(source, keycode, down)
        .ok_or_else(|| PlatformError::Failed("could not create a key event".into()))?;
    CGEvent::set_flags(Some(&event), flags);
    CGEvent::set_integer_value_field(
        Some(&event),
        CGEventField::EventSourceUserData,
        SYNTHETIC_EVENT_MARK,
    );
    Ok(event)
}

/// Makes text safe to type as key events. **Decided:** line breaks and tabs become spaces, and
/// other control characters are dropped. A typed line break is indistinguishable from the user
/// pressing Return, which submits a form, sends a chat message or runs a command in a terminal;
/// a paste (the normal path) and an Accessibility write insert them as text, typing cannot.
pub(crate) fn sanitize_for_typing(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\n' | '\t' | '\u{2028}' | '\u{2029}' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Splits `text` into runs of at most `max` UTF-16 units, never between the two halves of a
/// surrogate pair.
pub(crate) fn utf16_chunks(text: &str, max: usize) -> Vec<Vec<u16>> {
    let mut chunks = Vec::new();
    let mut current: Vec<u16> = Vec::with_capacity(max);
    let mut units = [0u16; 2];
    for c in text.chars() {
        let encoded = c.encode_utf16(&mut units);
        if !current.is_empty() && current.len() + encoded.len() > max {
            chunks.push(std::mem::take(&mut current));
        }
        current.extend_from_slice(encoded);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_paste_key_is_the_raw_ansi_v_keycode() {
        assert_eq!(KEY_V, 0x09);
        assert_eq!(
            CGEventFlags::MaskCommand.bits(),
            0x0010_0000,
            "kCGEventFlagMaskCommand"
        );
    }

    #[test]
    fn line_breaks_become_spaces_when_typed() {
        assert_eq!(sanitize_for_typing("one\ntwo"), "one two");
        assert_eq!(sanitize_for_typing("one\r\ntwo\rthree"), "one two three");
        assert_eq!(sanitize_for_typing("a\tb"), "a b");
        assert_eq!(sanitize_for_typing("a\u{2028}b\u{2029}c"), "a b c");
    }

    #[test]
    fn other_control_characters_are_dropped() {
        assert_eq!(sanitize_for_typing("a\u{0}b\u{7}c\u{1b}d\u{7f}e"), "abcde");
        assert_eq!(
            sanitize_for_typing("naïve café, ok 👍"),
            "naïve café, ok 👍"
        );
    }

    #[test]
    fn chunks_hold_at_most_twenty_units() {
        let text = "a".repeat(45);
        let chunks = utf16_chunks(&text, UNICODE_CHUNK);
        assert_eq!(chunks.iter().map(Vec::len).collect::<Vec<_>>(), [20, 20, 5]);
    }

    #[test]
    fn a_surrogate_pair_is_never_split() {
        // 19 units, then an emoji of 2: it must move whole to the next chunk.
        let text = format!("{}😀b", "a".repeat(19));
        let chunks = utf16_chunks(&text, UNICODE_CHUNK);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 19);
        assert_eq!(String::from_utf16(&chunks[1]).expect("whole pairs"), "😀b");
        let rejoined: Vec<u16> = chunks.concat();
        assert_eq!(String::from_utf16(&rejoined).expect("valid"), text);
    }

    #[test]
    fn empty_text_is_no_chunks() {
        assert!(utf16_chunks("", UNICODE_CHUNK).is_empty());
    }
}
