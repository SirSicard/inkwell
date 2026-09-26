//! macOS implementations of the `ink_core::platform` traits: capture taps, meeting
//! detection and permission probes (S2.1a); the host clock, the hotkey event tap, text
//! insertion and focus (S2.1b).
//!
//! Every module is macOS-only. On any other OS this crate builds empty, so the workspace still
//! builds and tests on Windows.
//!
//! | Trait | Type | Module |
//! |---|---|---|
//! | [`Clock`](ink_core::Clock) | `MacClock` | `clock` |
//! | [`HotkeySource`](ink_core::HotkeySource) | `MacHotkeySource` | `hotkey` |
//! | [`TextInserter`](ink_core::TextInserter) | `MacTextInserter` | `insert` |
//! | [`FocusReader`](ink_core::FocusReader) | `MacFocusReader` | `focus` |
//!
//! Permissions: the hotkey tap, insertion and selected-text reads all need Accessibility, and
//! nothing in this crate ever asks for it. Every check is the non-prompting kind; the prompt
//! belongs to the shell, behind a button the user presses (`PermissionProbe::request`).
//!
//! What an agent's shell or CI cannot prove (the tap, posting events, Accessibility reads and
//! writes) is covered by `examples/input_check.rs` and `INPUT-CHECKLIST.md`, which the maintainer
//! runs by hand.

// Unlike `ink-core`, this crate is FFI: every `unsafe` block carries a `// SAFETY:` comment, and
// unsafe operations inside unsafe functions still need their own block.
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod ax;
pub mod clock;
pub mod focus;
pub mod hotkey;
pub mod insert;

#[cfg(target_os = "macos")]
pub use clock::MacClock;
#[cfg(target_os = "macos")]
pub use focus::MacFocusReader;
#[cfg(target_os = "macos")]
pub use hotkey::MacHotkeySource;
#[cfg(target_os = "macos")]
pub use insert::MacTextInserter;

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// The prompting permission calls, spelled in pieces so this test does not find itself.
    const PROMPTING_CALLS: [&str; 3] = [
        concat!("AXIsProcessTrusted", "WithOptions"),
        concat!("CGRequest", "PostEventAccess"),
        concat!("CGRequest", "ListenEventAccess"),
    ];

    /// Every source file in the crate, read at compile time.
    const SOURCES: [(&str, &str); 13] = [
        ("lib.rs", include_str!("lib.rs")),
        ("ax.rs", include_str!("ax.rs")),
        ("clock.rs", include_str!("clock.rs")),
        ("focus.rs", include_str!("focus.rs")),
        ("hotkey.rs", include_str!("hotkey.rs")),
        ("hotkey/binding.rs", include_str!("hotkey/binding.rs")),
        ("hotkey/machine.rs", include_str!("hotkey/machine.rs")),
        ("hotkey/tap.rs", include_str!("hotkey/tap.rs")),
        ("insert.rs", include_str!("insert.rs")),
        ("insert/keys.rs", include_str!("insert/keys.rs")),
        ("insert/pasteboard.rs", include_str!("insert/pasteboard.rs")),
        ("insert/sequence.rs", include_str!("insert/sequence.rs")),
        ("input_check.rs", include_str!("../examples/input_check.rs")),
    ];

    /// The brief: never let the input path prompt for a permission. A prompt from the paste path
    /// fires once per dictation, forever, whenever the grant does not match the running binary.
    #[test]
    fn nothing_in_the_crate_can_prompt_for_a_permission() {
        for (file, source) in SOURCES {
            for call in PROMPTING_CALLS {
                assert!(!source.contains(call), "{file} calls {call}");
            }
        }
    }
}
