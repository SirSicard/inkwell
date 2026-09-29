//! Synthetic keys: Ctrl+V, and text typed as Unicode key events.
//!
//! Every event carries [`SYNTHETIC_EVENT_MARK`], so this crate's own hotkey hook lets it through.
//!
//! **Held modifiers.** `SendInput` adds to whatever the user is physically holding: Ctrl+V with
//! Alt held is Ctrl+Alt+V, and typed text with Alt held fires menu accelerators. After a chord
//! hotkey the user may still hold its modifiers, so an insertion first waits (once, briefly) for
//! every modifier to be released ([`wait_for_release`]), and each send refuses at once rather than
//! go out into a held one.
#![cfg(windows)]

use std::thread;
use std::time::{Duration, Instant};

use ink_core::PlatformError;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY,
};

use crate::hotkey::SYNTHETIC_EVENT_MARK;
use crate::hotkey::binding::vk;

/// How long to wait for held modifiers to be released before refusing.
const RELEASE_WAIT: Duration = Duration::from_millis(750);

/// Inputs per `SendInput` call when typing: small enough that a long text does not starve the
/// target's input queue.
const TYPE_BATCH: usize = 64;

/// One key event.
fn key(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: SYNTHETIC_EVENT_MARK,
            },
        },
    }
}

/// What a key event is, for the tests: (virtual key, UTF-16 unit, key up).
#[cfg(test)]
fn describe(input: &INPUT) -> (u16, u16, bool) {
    // SAFETY: every INPUT built here is a keyboard input.
    let ki = unsafe { input.Anonymous.ki };
    (ki.wVk.0, ki.wScan, ki.dwFlags.contains(KEYEVENTF_KEYUP))
}

/// Ctrl down, V down, V up, Ctrl up.
pub(crate) fn paste_inputs() -> [INPUT; 4] {
    let (ctrl, v) = (vk::CONTROL as u16, vk::V as u16);
    [
        key(ctrl, 0, KEYBD_EVENT_FLAGS(0)),
        key(v, 0, KEYBD_EVENT_FLAGS(0)),
        key(v, 0, KEYEVENTF_KEYUP),
        key(ctrl, 0, KEYEVENTF_KEYUP),
    ]
}

/// `text` as key events: each UTF-16 unit down and up as a Unicode event (a surrogate pair is two
/// units, which Windows joins), and a line break as Return (`\r\n` is one).
pub(crate) fn text_inputs(text: &str) -> Vec<INPUT> {
    let mut out = Vec::with_capacity(text.len() * 2);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' || c == '\n' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            let enter = vk::RETURN as u16;
            out.push(key(enter, 0, KEYBD_EVENT_FLAGS(0)));
            out.push(key(enter, 0, KEYEVENTF_KEYUP));
            continue;
        }
        let mut units = [0u16; 2];
        for &unit in c.encode_utf16(&mut units).iter() {
            out.push(key(0, unit, KEYEVENTF_UNICODE));
            out.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
    }
    out
}

/// Whether any modifier is physically down now.
fn modifier_held() -> bool {
    [vk::CONTROL, vk::SHIFT, vk::MENU, vk::LWIN, vk::RWIN]
        .into_iter()
        // SAFETY: GetAsyncKeyState reads state for any virtual key.
        .any(|key| unsafe { GetAsyncKeyState(key as i32) } < 0)
}

/// Waits until no modifier is held, up to [`RELEASE_WAIT`]. `false` if one still is.
pub(crate) fn wait_for_release() -> bool {
    let deadline = Instant::now() + RELEASE_WAIT;
    while modifier_held() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    true
}

/// Refuses at once, without waiting, when a modifier is held.
fn refuse_if_held() -> Result<(), PlatformError> {
    if modifier_held() {
        return Err(PlatformError::Failed(
            "a modifier key was pressed, so nothing more was sent".into(),
        ));
    }
    Ok(())
}

/// Sends `inputs` in one call; an error unless all went in.
fn send(inputs: &[INPUT]) -> Result<(), PlatformError> {
    send_counted(inputs).map_err(|(_, error)| error)
}

/// As [`send`], with how many went in when not all did.
fn send_counted(inputs: &[INPUT]) -> Result<(), (u32, PlatformError)> {
    // SAFETY: a live slice of keyboard inputs and the struct's size.
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err((
            sent,
            PlatformError::Failed(format!(
                "Windows accepted {sent} of {} key events (another app may be blocking input)",
                inputs.len()
            )),
        ))
    }
}

/// Injects the hotkey's mask key (down and up), marked so the hook passes it on. A failure only
/// means the stray tap it prevents may happen; it is not reported.
pub(crate) fn send_mask_key() {
    let _ = send(&mask_inputs());
}

/// The mask key down and up.
fn mask_inputs() -> [INPUT; 2] {
    let mask = crate::hotkey::MASK_VK;
    [
        key(mask, 0, KEYBD_EVENT_FLAGS(0)),
        key(mask, 0, KEYEVENTF_KEYUP),
    ]
}

/// Injects the hook's heartbeat (the mask key's code, down and up, with
/// [`HEARTBEAT_MARK`](crate::hotkey::HEARTBEAT_MARK)); the live hook swallows it. `false` when
/// Windows took none of it (an elevated window has focus, say), so nothing is pending.
pub(crate) fn send_heartbeat() -> bool {
    let mut inputs = mask_inputs();
    for input in &mut inputs {
        input.Anonymous.ki.dwExtraInfo = crate::hotkey::HEARTBEAT_MARK;
    }
    // SAFETY: a live slice of keyboard inputs and the struct's size.
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) > 0 }
}

/// Presses Ctrl+V. On failure, whether any of its keystrokes went in (then a paste may still
/// happen).
pub(crate) fn post_paste() -> Result<(), bool> {
    refuse_if_held().map_err(|_| false)?;
    send_counted(&paste_inputs()).map_err(|(sent, _)| sent > 0)
}

/// Types `text`, calling `still_focused` before each batch: once it says no, nothing more is sent.
pub(crate) fn type_text(
    text: &str,
    still_focused: impl FnMut() -> bool,
) -> Result<(), PlatformError> {
    type_batches(&text_inputs(text), still_focused, |batch| {
        refuse_if_held()?;
        send(batch)
    })
}

/// The typing loop over `inputs`, in batches: check focus, send; stop at the first failure. The
/// error says how much went in. Pure over `still_focused` and `send`.
fn type_batches(
    inputs: &[INPUT],
    mut still_focused: impl FnMut() -> bool,
    mut send: impl FnMut(&[INPUT]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    let total = inputs.len() / 2;
    let mut done = 0;
    for batch in inputs.chunks(TYPE_BATCH) {
        let result = if still_focused() {
            send(batch)
        } else {
            Err(super::sequence::focus_changed("while typing"))
        };
        if let Err(error) = result {
            return Err(if done == 0 {
                error
            } else {
                PlatformError::Failed(format!(
                    "the text was only partly typed ({done} of {total} keys): {error}"
                ))
            });
        }
        done += batch.len() / 2;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_v_is_four_marked_events() {
        let inputs = paste_inputs();
        let seen: Vec<_> = inputs.iter().map(describe).collect();
        assert_eq!(
            seen,
            [
                (0x11, 0, false),
                (0x56, 0, false),
                (0x56, 0, true),
                (0x11, 0, true)
            ]
        );
        for input in &inputs {
            // SAFETY: keyboard inputs.
            assert_eq!(
                unsafe { input.Anonymous.ki.dwExtraInfo },
                SYNTHETIC_EVENT_MARK
            );
        }
    }

    #[test]
    fn the_mask_key_is_an_unassigned_key_marked_as_ours() {
        let inputs = mask_inputs();
        let seen: Vec<_> = inputs.iter().map(describe).collect();
        assert_eq!(seen, [(0xE8, 0, false), (0xE8, 0, true)]);
        for input in &inputs {
            // SAFETY: keyboard inputs.
            assert_eq!(
                unsafe { input.Anonymous.ki.dwExtraInfo },
                SYNTHETIC_EVENT_MARK
            );
        }
    }

    #[test]
    fn typing_stops_at_the_batch_where_focus_moved_and_says_how_far_it_got() {
        let inputs = text_inputs(&"x".repeat(100)); // 200 inputs: batches of 64, 64, 64, 8
        let mut checks = 0;
        let mut sent = 0;
        let result = type_batches(
            &inputs,
            || {
                checks += 1;
                checks <= 2
            },
            |batch| {
                sent += batch.len();
                Ok(())
            },
        );
        assert_eq!(sent, 128, "two batches went in, none after focus moved");
        match result {
            Err(PlatformError::Failed(m)) => {
                assert!(m.contains("64 of 100"), "{m}");
                assert!(m.contains("focused window changed"), "{m}");
            }
            other => panic!("{other:?}"),
        }
        // Focus gone before the first batch: nothing was inserted.
        let result = type_batches(&inputs, || false, |_| panic!("nothing sent"));
        assert!(
            matches!(result, Err(PlatformError::Failed(m)) if m.starts_with("nothing was inserted"))
        );
        assert!(type_batches(&inputs, || true, |_| Ok(())).is_ok());
    }

    #[test]
    fn text_is_unicode_units_down_and_up() {
        let seen: Vec<_> = text_inputs("Hé").iter().map(describe).collect();
        assert_eq!(
            seen,
            [
                (0, 0x48, false),
                (0, 0x48, true),
                (0, 0xE9, false),
                (0, 0xE9, true)
            ]
        );
    }

    #[test]
    fn a_character_outside_the_bmp_is_a_surrogate_pair() {
        let seen: Vec<_> = text_inputs("🙂").iter().map(describe).collect();
        assert_eq!(
            seen,
            [
                (0, 0xD83D, false),
                (0, 0xD83D, true),
                (0, 0xDE42, false),
                (0, 0xDE42, true)
            ]
        );
    }

    #[test]
    fn line_breaks_are_return_once_each() {
        let returns = |t: &str| {
            text_inputs(t)
                .iter()
                .map(describe)
                .filter(|&(vk, _, up)| vk == 0x0D && !up)
                .count()
        };
        assert_eq!(returns("a\nb"), 1);
        assert_eq!(returns("a\r\nb"), 1);
        assert_eq!(returns("a\n\nb"), 2);
        assert_eq!(returns("a\rb"), 1);
        assert_eq!(text_inputs("a\r\nb").len(), 6);
    }
}
