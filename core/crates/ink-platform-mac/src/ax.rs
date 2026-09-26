//! Accessibility (AX): the focused element, its selected text, and writing over the selection.
//!
//! The few functions needed are declared here by hand from `HIServices/AXUIElement.h`; they are
//! stable C API, and no binding crate for them is available offline.
//!
//! **Threads.** AX calls are IPC: a message to the target app, answered on its main thread. They
//! are not AppKit and may be made from any thread, which is how the worker-thread trait methods
//! use them. Each call can block until the messaging timeout when the target is hung, so the
//! first use sets this process's global timeout to one second instead of the default six.
//!
//! **Permission.** Only `AXIsProcessTrusted`, which never prompts, is used to check.
#![cfg(target_os = "macos")]

use std::ptr::{self, NonNull};
use std::sync::Once;

use ink_core::PlatformError;
use objc2_core_foundation::{CFNumber, CFRetained, CFString, CFType};

use crate::insert::sequence::AxInsert;

/// `AXError`, an `SInt32`.
pub(crate) type AxError = i32;

/// `AXError` values (`HIServices/AXError.h`).
pub(crate) mod code {
    /// `kAXErrorSuccess`.
    pub const SUCCESS: i32 = 0;
    /// `kAXErrorInvalidUIElement`: the element went away.
    pub const INVALID_UI_ELEMENT: i32 = -25202;
    /// `kAXErrorCannotComplete`: the app did not answer in time.
    pub const CANNOT_COMPLETE: i32 = -25204;
    /// `kAXErrorAttributeUnsupported`.
    pub const ATTRIBUTE_UNSUPPORTED: i32 = -25205;
    /// `kAXErrorNotImplemented`: the app does not implement AX for this.
    pub const NOT_IMPLEMENTED: i32 = -25208;
    /// `kAXErrorAPIDisabled`: this process is not trusted.
    pub const API_DISABLED: i32 = -25211;
    /// `kAXErrorNoValue`.
    pub const NO_VALUE: i32 = -25212;
}

/// `kAXFocusedUIElementAttribute`.
const FOCUSED_UI_ELEMENT: &str = "AXFocusedUIElement";
/// `kAXSelectedTextAttribute`.
const SELECTED_TEXT: &str = "AXSelectedText";
/// `kAXNumberOfCharactersAttribute`.
const NUMBER_OF_CHARACTERS: &str = "AXNumberOfCharacters";

/// The process-wide AX messaging timeout, in seconds.
const MESSAGING_TIMEOUT_S: f32 = 1.0;

// SAFETY: the signatures match `HIServices/AXUIElement.h`. `Boolean` is an unsigned char, and an
// `AXUIElementRef` or `CFStringRef` is a pointer to a CF object, which `&CFType` and `&CFString`
// are in the ABI. Only `AXIsProcessTrusted`, which takes nothing, is declared safe.
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    /// `Boolean AXIsProcessTrusted(void)`. No arguments, no prompt, any thread.
    safe fn AXIsProcessTrusted() -> u8;
    /// `AXUIElementRef AXUIElementCreateSystemWide(void)`: a new (+1) element, or null.
    fn AXUIElementCreateSystemWide() -> Option<NonNull<CFType>>;
    /// `AXError AXUIElementCopyAttributeValue(AXUIElementRef, CFStringRef, CFTypeRef *)`. On
    /// success `value` holds a +1 reference.
    fn AXUIElementCopyAttributeValue(
        element: &CFType,
        attribute: &CFString,
        value: *mut *const CFType,
    ) -> AxError;
    /// `AXError AXUIElementSetAttributeValue(AXUIElementRef, CFStringRef, CFTypeRef)`.
    fn AXUIElementSetAttributeValue(
        element: &CFType,
        attribute: &CFString,
        value: &CFType,
    ) -> AxError;
    /// `AXError AXUIElementSetMessagingTimeout(AXUIElementRef, float)`. On the system-wide
    /// element it sets the timeout for the whole process.
    fn AXUIElementSetMessagingTimeout(element: &CFType, timeout_in_seconds: f32) -> AxError;
}

/// How an `AXError` reads to a caller that wants a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// A value.
    Value,
    /// Nothing there: no value, the attribute or element is unsupported or gone.
    Absent,
    /// This process is not trusted for Accessibility.
    Denied,
    /// The app did not answer within the messaging timeout.
    TimedOut,
    /// Anything else.
    Failed,
}

/// Sorts an `AXError`.
pub(crate) fn classify(error: AxError) -> Class {
    match error {
        code::SUCCESS => Class::Value,
        code::NO_VALUE
        | code::ATTRIBUTE_UNSUPPORTED
        | code::NOT_IMPLEMENTED
        | code::INVALID_UI_ELEMENT => Class::Absent,
        code::API_DISABLED => Class::Denied,
        code::CANNOT_COMPLETE => Class::TimedOut,
        _ => Class::Failed,
    }
}

/// Whether this process is trusted for Accessibility. Never prompts.
pub(crate) fn is_process_trusted() -> bool {
    AXIsProcessTrusted() != 0
}

/// The focused element's selected text. `None` when nothing is selected, nothing is focused, the
/// app does not expose a selection, or this process is not trusted (the brief: no permission reads
/// as no selection, never as a prompt).
pub(crate) fn selected_text() -> Result<Option<String>, PlatformError> {
    if !is_process_trusted() {
        return Ok(None);
    }
    let Some(element) = focused_element()? else {
        return Ok(None);
    };
    Ok(string_attribute(&element, SELECTED_TEXT)?.filter(|s| !s.is_empty()))
}

/// Replaces the focused element's selection with `text`.
///
/// Some apps answer success and ignore the write. When the element reports its length, a write
/// that should have changed it and did not is treated as refused, so the typing fallback runs.
pub(crate) fn insert_text(text: &str) -> AxInsert {
    let Ok(Some(element)) = focused_element() else {
        return AxInsert::Refused;
    };
    let before = character_count(&element);
    let selected = string_attribute(&element, SELECTED_TEXT)
        .ok()
        .flatten()
        .map_or(0, |s| s.encode_utf16().count());
    let attribute = CFString::from_str(SELECTED_TEXT);
    let value = CFString::from_str(text);
    // SAFETY: `element` is a live AXUIElement, retained for the call; `attribute` and `value` are
    // live CFStrings. The call copies what it needs.
    let error = unsafe { AXUIElementSetAttributeValue(&element, &attribute, &value) };
    match classify(error) {
        Class::Value => {
            let after = character_count(&element);
            if write_took(before, after, selected, text.encode_utf16().count()) {
                AxInsert::Took
            } else {
                AxInsert::Refused
            }
        }
        Class::TimedOut => AxInsert::Unknown,
        Class::Absent | Class::Denied | Class::Failed => AxInsert::Refused,
    }
}

/// Whether a write that reported success evidently happened. With the element's length before and
/// after, a write that should have changed the length (the selection and the text differ in
/// length) and did not is an app that ignored it. Without a length, the success code is all there
/// is.
pub(crate) fn write_took(
    before: Option<i64>,
    after: Option<i64>,
    selected_utf16: usize,
    inserted_utf16: usize,
) -> bool {
    match (before, after) {
        (Some(before), Some(after)) => before != after || selected_utf16 == inserted_utf16,
        _ => true,
    }
}

fn focused_element() -> Result<Option<CFRetained<CFType>>, PlatformError> {
    let system = system_wide()?;
    copy_attribute(&system, FOCUSED_UI_ELEMENT)
}

fn system_wide() -> Result<CFRetained<CFType>, PlatformError> {
    static TIMEOUT: Once = Once::new();
    // SAFETY: no arguments; returns a new element or null.
    let raw = unsafe { AXUIElementCreateSystemWide() }
        .ok_or_else(|| PlatformError::Failed("no system-wide accessibility element".into()))?;
    // SAFETY: a Create function returns +1, which `CFRetained` now owns.
    let system = unsafe { CFRetained::from_raw(raw) };
    TIMEOUT.call_once(|| {
        // SAFETY: `system` is a live AXUIElement. A failure leaves the default timeout, which is
        // slower but correct, so the result is not needed.
        let _ = unsafe { AXUIElementSetMessagingTimeout(&system, MESSAGING_TIMEOUT_S) };
    });
    Ok(system)
}

/// Copies one attribute. `Ok(None)` for every "nothing there" answer, including not trusted.
fn copy_attribute(
    element: &CFType,
    name: &str,
) -> Result<Option<CFRetained<CFType>>, PlatformError> {
    let attribute = CFString::from_str(name);
    let mut value: *const CFType = ptr::null();
    // SAFETY: `element` is a live AXUIElement and `attribute` a live CFString for the call;
    // `value` is a valid out-pointer.
    let error = unsafe { AXUIElementCopyAttributeValue(element, &attribute, &mut value) };
    match classify(error) {
        // SAFETY: on success the value comes back +1 (`CF_RETURNS_RETAINED`), which `CFRetained`
        // takes over.
        Class::Value => {
            Ok(NonNull::new(value.cast_mut()).map(|v| unsafe { CFRetained::from_raw(v) }))
        }
        Class::Absent | Class::Denied => Ok(None),
        Class::TimedOut => Err(PlatformError::Failed(
            "the focused app did not answer accessibility in time".into(),
        )),
        Class::Failed => Err(PlatformError::Failed(format!(
            "accessibility error {error}"
        ))),
    }
}

fn string_attribute(element: &CFType, name: &str) -> Result<Option<String>, PlatformError> {
    Ok(copy_attribute(element, name)?
        .and_then(|value| value.downcast_ref::<CFString>().map(ToString::to_string)))
}

fn character_count(element: &CFType) -> Option<i64> {
    copy_attribute(element, NUMBER_OF_CHARACTERS)
        .ok()
        .flatten()
        .and_then(|value| value.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ax_error_codes_match_the_header() {
        assert_eq!(code::INVALID_UI_ELEMENT, -25202);
        assert_eq!(code::CANNOT_COMPLETE, -25204);
        assert_eq!(code::ATTRIBUTE_UNSUPPORTED, -25205);
        assert_eq!(code::NOT_IMPLEMENTED, -25208);
        assert_eq!(code::API_DISABLED, -25211);
        assert_eq!(code::NO_VALUE, -25212);
    }

    #[test]
    fn errors_sort_into_what_a_caller_can_do() {
        assert_eq!(classify(code::SUCCESS), Class::Value);
        assert_eq!(classify(code::NO_VALUE), Class::Absent);
        assert_eq!(classify(code::ATTRIBUTE_UNSUPPORTED), Class::Absent);
        assert_eq!(classify(code::NOT_IMPLEMENTED), Class::Absent);
        assert_eq!(classify(code::INVALID_UI_ELEMENT), Class::Absent);
        assert_eq!(classify(code::API_DISABLED), Class::Denied);
        assert_eq!(classify(code::CANNOT_COMPLETE), Class::TimedOut);
        assert_eq!(classify(-25200), Class::Failed, "kAXErrorFailure");
        assert_eq!(classify(-25201), Class::Failed, "kAXErrorIllegalArgument");
        assert_eq!(classify(-1), Class::Failed);
    }

    #[test]
    fn a_write_that_changed_the_length_took() {
        assert!(write_took(Some(100), Some(120), 0, 20));
        assert!(
            write_took(Some(100), Some(95), 25, 20),
            "replaced a longer selection"
        );
    }

    #[test]
    fn a_success_with_no_change_is_an_ignored_write() {
        assert!(!write_took(Some(100), Some(100), 0, 20));
    }

    #[test]
    fn an_unchanged_length_proves_nothing_when_it_should_not_change() {
        assert!(write_took(Some(100), Some(100), 20, 20));
    }

    #[test]
    fn without_a_length_the_success_code_decides() {
        assert!(write_took(None, Some(100), 0, 20));
        assert!(write_took(Some(100), None, 0, 20));
        assert!(write_took(None, None, 0, 20));
    }

    /// Runs on CI and in an agent's shell: whatever the trust state, a read never prompts and
    /// never fails just because permission is missing.
    #[test]
    fn a_selection_read_without_trust_is_none_not_an_error() {
        if !is_process_trusted() {
            assert_eq!(selected_text(), Ok(None));
        }
    }
}
