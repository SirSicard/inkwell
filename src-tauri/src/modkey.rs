//! Modifier-only hotkeys: Fn (the globe key), right Command, right Option,
//! right Control.
//!
//! The OS hotkey API cannot see these. RegisterEventHotKey wants a key plus
//! modifiers; a modifier alone never produces a key event at all, only a
//! `flagsChanged`. So these arrive from a CGEventTap listening to that one
//! event type, and the tap is strictly listen-only: the key must keep working
//! as a modifier everywhere else, which also means Inkwell cannot stop macOS
//! from giving the globe key its own meaning (System Settings > Keyboard >
//! "Press globe key to" should be set to Do Nothing).
//!
//! Left-side modifiers are deliberately not offered. Watching left Command
//! would fire a recording on every Cmd+C in every app; the right-side keys
//! and Fn are the ones a hand does not use mid-shortcut, which is exactly why
//! the commercial dictation apps settled on them. Telling left from right
//! takes the device-dependent flag bits (IOLLEvent.h), because the public
//! Command flag is set for either key.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKey {
    Fn,
    RightCmd,
    RightOpt,
    RightCtrl,
}

impl ModKey {
    pub fn from_token(s: &str) -> Option<Self> {
        match s {
            "fn" => Some(Self::Fn),
            "right_cmd" => Some(Self::RightCmd),
            "right_opt" => Some(Self::RightOpt),
            "right_ctrl" => Some(Self::RightCtrl),
            _ => None,
        }
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::Fn => "fn",
            Self::RightCmd => "right_cmd",
            Self::RightOpt => "right_opt",
            Self::RightCtrl => "right_ctrl",
        }
    }

    /// Carbon virtual keycode, as carried by flagsChanged events.
    pub fn keycode(self) -> i64 {
        match self {
            Self::Fn => 63,        // kVK_Function
            Self::RightCmd => 54,  // kVK_RightCommand
            Self::RightOpt => 61,  // kVK_RightOption
            Self::RightCtrl => 62, // kVK_RightControl
        }
    }

    /// The flag bit that answers "is this key down after the change". The
    /// public modifier masks cannot: with both Command keys held, releasing
    /// the right one leaves the Command flag set. The device-dependent bits
    /// (IOLLEvent.h NX_DEVICE*) are per physical key; Fn has its own mask.
    pub fn flag_mask(self) -> u64 {
        match self {
            Self::Fn => 0x0080_0000,       // NX_SECONDARYFNMASK
            Self::RightCmd => 0x0000_0010, // NX_DEVICERCMDKEYMASK
            Self::RightOpt => 0x0000_0040, // NX_DEVICERALTKEYMASK
            Self::RightCtrl => 0x0000_2000, // NX_DEVICERCTLKEYMASK
        }
    }
}

pub const SLOT_MAIN: usize = 0;
pub const SLOT_EDIT: usize = 1;

#[cfg(target_os = "macos")]
mod tap {
    use super::ModKey;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    pub static BINDINGS: Mutex<[Option<ModKey>; 2]> = Mutex::new([None, None]);
    static TAP_RUNNING: AtomicBool = AtomicBool::new(false);

    type CGEventRef = *mut c_void;

    #[allow(non_snake_case)]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,        // kCGSessionEventTap = 1
            place: u32,      // kCGHeadInsertEventTap = 0
            options: u32,    // kCGEventTapOptionListenOnly = 1
            events_of_interest: u64,
            callback: extern "C" fn(*const c_void, u32, CGEventRef, *mut c_void) -> CGEventRef,
            user_info: *mut c_void,
        ) -> *mut c_void; // CFMachPortRef
        fn CGEventTapEnable(tap: *mut c_void, enable: bool);
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
        fn CGEventGetFlags(event: CGEventRef) -> u64;
        fn CFMachPortCreateRunLoopSource(
            allocator: *const c_void,
            port: *mut c_void,
            order: isize,
        ) -> *mut c_void;
        fn CFRunLoopGetCurrent() -> *mut c_void;
        fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
        fn CFRunLoopRun();
        static kCFRunLoopCommonModes: *const c_void;
    }

    const K_CG_EVENT_FLAGS_CHANGED: u32 = 12;
    const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
    // Sent when the OS disables a tap (timeout or user input turnaround).
    const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    const K_CG_EVENT_TAP_DISABLED_BY_USER: u32 = 0xFFFF_FFFF;

    struct TapContext {
        handle: tauri::AppHandle,
        port: *mut c_void,
    }

    extern "C" fn tap_callback(
        _proxy: *const c_void,
        etype: u32,
        event: CGEventRef,
        user_info: *mut c_void,
    ) -> CGEventRef {
        let ctx = unsafe { &*(user_info as *const TapContext) };

        if etype == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
            || etype == K_CG_EVENT_TAP_DISABLED_BY_USER
        {
            // The OS can switch a tap off; a hotkey that silently dies is the
            // stuck-recording bug in a new costume, so switch it back on.
            log::warn!("Modifier-key tap was disabled by the OS; re-enabling");
            unsafe { CGEventTapEnable(ctx.port, true) };
            return event;
        }
        if etype != K_CG_EVENT_FLAGS_CHANGED {
            return event;
        }

        let keycode =
            unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) };
        let flags = unsafe { CGEventGetFlags(event) };

        let hit: Option<(usize, ModKey)> = {
            let bindings = BINDINGS.lock().unwrap();
            bindings
                .iter()
                .enumerate()
                .find_map(|(slot, b)| b.filter(|k| k.keycode() == keycode).map(|k| (slot, k)))
        };
        if let Some((slot, key)) = hit {
            let pressed = flags & key.flag_mask() != 0;
            // Contained, because this is a real `extern "C"` boundary: this
            // function is called straight from CoreGraphics, and since Rust
            // 1.71 an unwind across such a boundary aborts the process rather
            // than unwinding. `on_hotkey` locks a dozen mutexes, so a panic
            // anywhere else in the app that poisons one of them would turn the
            // next Fn keypress into a hard crash of the whole application,
            // where the same panic on the OS-hotkey path only kills a thread.
            // The offline transcription path already uses this pattern; here
            // the alternative to catching is an abort, not a thread death.
            let hit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::pipeline::on_hotkey(&ctx.handle, slot == super::SLOT_EDIT, pressed);
            }));
            if hit.is_err() {
                log::error!(
                    "Modifier hotkey handler panicked (slot {}); the tap stays armed",
                    slot
                );
            }
        }
        event
    }

    /// Create the tap on its own thread and confirm it exists before
    /// returning, so a permission refusal surfaces as an error in the
    /// settings UI rather than as a hotkey that saves and never fires.
    pub fn ensure_tap(handle: &tauri::AppHandle) -> Result<(), String> {
        if TAP_RUNNING.load(Ordering::SeqCst) {
            return Ok(());
        }
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let thread_handle = handle.clone();
        std::thread::spawn(move || {
            let ctx = Box::into_raw(Box::new(TapContext {
                handle: thread_handle,
                port: std::ptr::null_mut(),
            }));
            let tap = unsafe {
                CGEventTapCreate(
                    1, // session tap
                    0, // head insert
                    1, // listen-only
                    1u64 << K_CG_EVENT_FLAGS_CHANGED,
                    tap_callback,
                    ctx as *mut c_void,
                )
            };
            if tap.is_null() {
                // Leaks ctx; creation failure is once per process and tiny.
                let _ = tx.send(Err(
                    "macOS refused the keyboard event tap. Modifier-only hotkeys \
                     need the Accessibility permission (System Settings > Privacy \
                     & Security > Accessibility)."
                        .to_string(),
                ));
                return;
            }
            unsafe { (*ctx).port = tap };
            unsafe {
                let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
                CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap, true);
            }
            TAP_RUNNING.store(true, Ordering::SeqCst);
            let _ = tx.send(Ok(()));
            unsafe { CFRunLoopRun() };
            // Only reached if the run loop stops; mark dead so a later
            // binding change can rebuild instead of assuming a live tap.
            TAP_RUNNING.store(false, Ordering::SeqCst);
            log::error!("Modifier-key tap run loop exited");
        });
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "Timed out creating the keyboard event tap".to_string())?
    }
}

/// Bind (or clear) a modifier-only hotkey. Creates the event tap on first
/// use, so installs that never touch the feature never own a keyboard tap.
#[cfg(target_os = "macos")]
pub fn set_binding(
    handle: &tauri::AppHandle,
    slot: usize,
    key: Option<ModKey>,
) -> Result<(), String> {
    if key.is_some() {
        tap::ensure_tap(handle)?;
    }
    tap::BINDINGS.lock().unwrap()[slot] = key;
    match key {
        Some(k) => log::info!("Modifier hotkey armed: {} (slot {})", k.token(), slot),
        None => log::info!("Modifier hotkey cleared (slot {})", slot),
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn set_binding(
    _handle: &tauri::AppHandle,
    _slot: usize,
    key: Option<ModKey>,
) -> Result<(), String> {
    match key {
        // Windows/Linux would need a low-level keyboard hook; not built.
        Some(_) => Err("Modifier-only hotkeys are macOS-only for now.".to_string()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        for k in [ModKey::Fn, ModKey::RightCmd, ModKey::RightOpt, ModKey::RightCtrl] {
            assert_eq!(ModKey::from_token(k.token()), Some(k));
        }
        assert_eq!(ModKey::from_token("fn5"), None);
        assert_eq!(ModKey::from_token(""), None);
        assert_eq!(ModKey::from_token("cmd"), None);
    }

    /// Values from Carbon's Events.h and IOKit's IOLLEvent.h. Wrong numbers
    /// here fail silently at runtime (the tap just never matches), which is
    /// why they are pinned in a test instead of trusted.
    #[test]
    fn keycodes_and_masks_match_the_headers() {
        assert_eq!(ModKey::Fn.keycode(), 63);
        assert_eq!(ModKey::RightCmd.keycode(), 54);
        assert_eq!(ModKey::RightOpt.keycode(), 61);
        assert_eq!(ModKey::RightCtrl.keycode(), 62);
        assert_eq!(ModKey::Fn.flag_mask(), 0x0080_0000);
        assert_eq!(ModKey::RightCmd.flag_mask(), 0x0000_0010);
        assert_eq!(ModKey::RightOpt.flag_mask(), 0x0000_0040);
        assert_eq!(ModKey::RightCtrl.flag_mask(), 0x0000_2000);
    }
}
