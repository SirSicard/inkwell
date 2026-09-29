//! Windows implementations of the `ink_core::platform` traits: WASAPI capture, meeting detection
//! and permissions; the host clock, the hotkey hook, text insertion and focus (S3.1).
//!
//! Every module is Windows-only. On any other OS this crate builds empty, so the workspace still
//! builds and tests on macOS.
//!
//! | Trait | Type | Module |
//! |---|---|---|
//! | [`CaptureControl`](ink_core::CaptureControl) | `WinCapture` | `capture` |
//! | [`AudioSource`](ink_core::AudioSource) | `WasapiSource` (mic, device loopback, process loopback) | `capture` |
//! | [`MeetingDetector`](ink_core::MeetingDetector) | `WinMeetingDetector` | `detect` |
//! | [`PermissionProbe`](ink_core::PermissionProbe) | `WinPermissionProbe` | `permissions` |
//! | [`Clock`](ink_core::Clock) | `WinClock` | `clock` |
//! | [`HotkeySource`](ink_core::HotkeySource) | `WinHotkeySource` | `hotkey` |
//! | [`TextInserter`](ink_core::TextInserter) | `WinTextInserter` | `insert` |
//! | [`FocusReader`](ink_core::FocusReader) | `WinFocusReader` | `focus` |
//!
//! Permissions: Windows gates only the microphone for a desktop app, through three privacy
//! switches that this crate reads and never changes. Nothing here prompts.
//!
//! What an agent's shell or CI cannot prove (capture from real calls, the hook, typing and pasting
//! into other apps, an elevated target) is covered by `examples/win_check.rs` with
//! `windows/S3.1-CHECKLIST.md`, which the maintainer runs from his desktop session.

// This crate is FFI: every `unsafe` block carries a `// SAFETY:` comment, and unsafe operations
// inside unsafe functions still need their own block.
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod capture;
pub mod clock;
mod com;
pub mod detect;
pub mod focus;
pub mod hotkey;
pub mod insert;
mod integrity;
pub mod permissions;
mod process;
mod sessions;

#[cfg(windows)]
pub use capture::{WasapiSource, WinCapture};
#[cfg(windows)]
pub use clock::WinClock;
#[cfg(windows)]
pub use detect::WinMeetingDetector;
#[cfg(windows)]
pub use focus::WinFocusReader;
#[cfg(windows)]
pub use hotkey::WinHotkeySource;
#[cfg(windows)]
pub use insert::WinTextInserter;
#[cfg(windows)]
pub use permissions::WinPermissionProbe;
