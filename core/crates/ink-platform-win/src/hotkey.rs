//! The global hotkey: a low-level keyboard hook (`WH_KEYBOARD_LL`).
//!
//! **Why a hook.** `RegisterHotKey` reports presses only, never releases, and cannot take a
//! modifier on its own; hold-to-talk needs both. The hook sees every key event in the session
//! before any app, needs no permission, and may swallow the hotkey's own events (the rules are in
//! `machine`). It cannot see keys typed into an app running as administrator or on the secure
//! desktop (UAC, the lock screen): a key released there is never seen, so the core's stuck-key
//! watchdog ends such a hold.
//!
//! **Threads.** The hook runs on its own thread, which installs it and pumps messages: Windows
//! calls the hook on that thread. The callback reads the event, updates a `Copy` state machine
//! held in a `Cell`, calls the core's sink (which only enqueues) and returns. Windows removes a
//! hook that takes longer than its timeout (about a second) **without telling anyone**, so the
//! callback takes no locks and does no work beyond that.
//!
//! **Our own keys.** Events this crate injects (the paste keystroke, typed text) carry
//! [`SYNTHETIC_EVENT_MARK`] in `dwExtraInfo` and pass through untouched. Other injected input
//! (a remote-desktop client, say) is treated as typed.
//!
//! **Lost.** If the message loop fails, the source reports `Lost` once and its thread ends. Our own
//! `stop` never reports `Lost`; a hold in progress is reported as `Cancelled`.
//!
//! **Panics.** A panic in the core's sink is caught on the hook thread (it must not unwind into
//! user32), counted in [`WinHotkeySource::callback_panics`], and recovered: the hold starts over
//! and `Cancelled` is sent.
//!
//! **Timestamps** are host time on the [`WinClock`] timebase: the hook's `time` (milliseconds on
//! the tick counter) gives the event's age, which is taken off the clock's `now_ns`.
#![cfg(windows)]

pub(crate) mod binding;
mod machine;

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{Clock, EventSink, HotkeyBinding, HotkeyEvent, HotkeySource, PlatformError};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::clock::WinClock;
use binding::{Binding, modifier, vk};
use machine::{Edge, HoldMachine, HookInput};

pub use binding::{DEFAULT_BINDING, KEYS};

/// The marker in `dwExtraInfo` on every key event this crate injects, so its own hook lets them
/// through. Arbitrary; ASCII for "inkw".
pub(crate) const SYNTHETIC_EVENT_MARK: usize = 0x696E_6B77;

/// How long `start` waits for the hook to be installed.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// The host time of a hook event: `now_ns` less the event's age on the tick counter (which wraps
/// every 49.7 days, hence the wrapping subtraction). An age over a second is not believed; the
/// event is stamped `now_ns`.
pub(crate) fn event_time_ns(now_ns: u64, now_tick_ms: u32, event_tick_ms: u32) -> u64 {
    let age_ms = now_tick_ms.wrapping_sub(event_tick_ms);
    if age_ms > 1_000 {
        return now_ns;
    }
    now_ns.saturating_sub(u64::from(age_ms) * 1_000_000)
}

/// The chord modifiers down now. **Hook thread.**
fn modifiers_down() -> u8 {
    // SAFETY: GetAsyncKeyState takes any virtual key and only reads state.
    let down = |key: u32| unsafe { GetAsyncKeyState(key as i32) } < 0;
    let mut bits = 0;
    if down(vk::CONTROL) {
        bits |= modifier::CTRL;
    }
    if down(vk::SHIFT) {
        bits |= modifier::SHIFT;
    }
    if down(vk::MENU) {
        bits |= modifier::ALT;
    }
    if down(vk::LWIN) || down(vk::RWIN) {
        bits |= modifier::WIN;
    }
    bits
}

/// What the hook callback needs, kept on the hook thread.
struct HookContext {
    sink: EventSink<HotkeyEvent>,
    clock: WinClock,
    panics: Arc<AtomicU64>,
}

thread_local! {
    /// The hold state. `Copy`, in a `Cell`: the callback never borrows across the sink call.
    static MACHINE: Cell<Option<HoldMachine>> = const { Cell::new(None) };
    /// Set once before the hook is installed, read by the callback.
    static CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) };
}

/// Calls the sink; a panic is caught, counted and recovered (the hold is reset, `Cancelled` sent).
fn emit(context: &HookContext, event: HotkeyEvent) {
    if catch_unwind(AssertUnwindSafe(|| (context.sink)(event))).is_ok() {
        return;
    }
    context.panics.fetch_add(1, Ordering::Relaxed);
    MACHINE.with(|m| {
        if let Some(mut machine) = m.get() {
            machine.reset();
            m.set(Some(machine));
        }
    });
    let _ = catch_unwind(AssertUnwindSafe(|| (context.sink)(HotkeyEvent::Cancelled)));
}

/// The hook. **Hook thread.**
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let swallow = if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points at the event's KBDLLHOOKSTRUCT for the call.
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // The whole decision is guarded: nothing may unwind into user32.
        catch_unwind(AssertUnwindSafe(|| decide(event, wparam.0 as u32))).unwrap_or(false)
    } else {
        false
    };
    if swallow {
        return LRESULT(1);
    }
    // SAFETY: passes the event on unchanged, as a hook must when it does not swallow it.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// One event: update the machine, report an edge, say whether to swallow.
fn decide(event: &KBDLLHOOKSTRUCT, message: u32) -> bool {
    if event.dwExtraInfo == SYNTHETIC_EVENT_MARK {
        return false;
    }
    let input = match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => HookInput::KeyDown {
            vk: event.vkCode,
            modifiers: modifiers_down(),
        },
        WM_KEYUP | WM_SYSKEYUP => HookInput::KeyUp { vk: event.vkCode },
        _ => return false,
    };
    let Some(mut machine) = MACHINE.with(Cell::get) else {
        return false;
    };
    let verdict = machine.on(input);
    MACHINE.with(|m| m.set(Some(machine)));
    if let Some(edge) = verdict.edge {
        CONTEXT.with(|c| {
            if let Some(context) = c.borrow().as_ref() {
                // SAFETY: no arguments; reads the tick counter.
                let now_tick = unsafe { GetTickCount() };
                let at_ns = event_time_ns(context.clock.now_ns(), now_tick, event.time);
                emit(
                    context,
                    match edge {
                        Edge::Pressed => HotkeyEvent::Pressed { at_ns },
                        Edge::Released => HotkeyEvent::Released { at_ns },
                    },
                );
            }
        });
    }
    verdict.swallow
}

/// The hook thread: install, pump, uninstall. A hold in progress at the end is `Cancelled`.
fn run(
    binding: Binding,
    context: HookContext,
    ready: mpsc::SyncSender<Result<u32, PlatformError>>,
) {
    // Make sure this thread has a message queue before anyone posts to it.
    let mut msg = MSG::default();
    // SAFETY: a live MSG; PM_NOREMOVE leaves the queue as it is.
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    MACHINE.with(|m| m.set(Some(HoldMachine::new(binding))));
    CONTEXT.with(|c| *c.borrow_mut() = Some(context));
    // SAFETY: this module's handle (the hook procedure lives in it) and a valid hook procedure.
    let hook = unsafe {
        GetModuleHandleW(None).and_then(|module| {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(module.into()), 0)
        })
    };
    let hook = match hook {
        Ok(hook) => hook,
        Err(e) => {
            let _ = ready.send(Err(PlatformError::Failed(format!(
                "the keyboard hook was refused: {e}"
            ))));
            return;
        }
    };
    // SAFETY: no arguments.
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    let mut lost = false;
    loop {
        // SAFETY: a live MSG; any window of this thread.
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        match got.0 {
            0 => break, // WM_QUIT: our stop
            -1 => {
                lost = true;
                break;
            }
            _ => {} // nothing of ours to dispatch; the hook runs inside GetMessageW
        }
    }
    // SAFETY: installed above on this thread, removed once.
    let _ = unsafe { UnhookWindowsHookEx(hook) };
    let held = MACHINE.with(|m| m.take()).is_some_and(|m| m.is_held());
    if let Some(context) = CONTEXT.with(|c| c.borrow_mut().take()) {
        if held {
            emit(&context, HotkeyEvent::Cancelled);
        }
        if lost {
            emit(&context, HotkeyEvent::Lost);
        }
    }
}

/// A running hook thread.
struct Hook {
    thread: JoinHandle<()>,
    thread_id: u32,
}

impl Hook {
    fn spawn(
        binding: Binding,
        sink: EventSink<HotkeyEvent>,
        clock: WinClock,
        panics: Arc<AtomicU64>,
    ) -> Result<Self, PlatformError> {
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let context = HookContext {
            sink,
            clock,
            panics,
        };
        let thread = thread::Builder::new()
            .name("ink-hotkey".into())
            .spawn(move || run(binding, context, ready_tx))
            .map_err(|e| {
                PlatformError::Failed(format!("could not start the hotkey thread: {e}"))
            })?;
        match ready.recv_timeout(START_TIMEOUT) {
            Ok(Ok(thread_id)) => Ok(Self { thread, thread_id }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(PlatformError::Failed(
                "the keyboard hook did not start in time".into(),
            )),
        }
    }

    fn shutdown(self) {
        // SAFETY: posts WM_QUIT to the hook thread's queue, which exists (made before `ready`).
        let posted = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if posted.is_ok() {
            let _ = self.thread.join();
        }
        // If the post failed, the thread has already left its loop (it ended on its own); it is
        // not joined, so a stuck thread cannot hang `stop`.
    }
}

/// [`HotkeySource`] on a low-level keyboard hook.
pub struct WinHotkeySource {
    clock: WinClock,
    hook: Mutex<Option<Hook>>,
    panics: Arc<AtomicU64>,
}

impl WinHotkeySource {
    /// A source that stamps events on `clock`'s timebase. No hook exists until
    /// [`start`](HotkeySource::start).
    pub fn new(clock: WinClock) -> Self {
        Self {
            clock,
            hook: Mutex::new(None),
            panics: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Panics caught on the hook thread since this source was created. Each was recovered: the
    /// hold was reset and `Cancelled` sent. A non-zero count is a bug to report.
    pub fn callback_panics(&self) -> u64 {
        self.panics.load(Ordering::Relaxed)
    }
}

impl HotkeySource for WinHotkeySource {
    /// Parses the token first, so an unsupported one leaves the current binding running. Then
    /// stops the old hook (reporting `Cancelled` to the old sink if a hold was in progress) and
    /// installs a new one, waiting until it exists.
    fn start(
        &self,
        binding: &HotkeyBinding,
        on_event: EventSink<HotkeyEvent>,
    ) -> Result<(), PlatformError> {
        let parsed = Binding::parse(&binding.0)?;
        let mut slot = self.hook.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(old) = slot.take() {
            old.shutdown();
        }
        *slot = Some(Hook::spawn(
            parsed,
            on_event,
            self.clock,
            Arc::clone(&self.panics),
        )?);
        Ok(())
    }

    /// Stops and joins the hook thread. A hold in progress is reported as `Cancelled` before it
    /// returns; after that the sink is never called again. Never call it from the sink itself.
    fn stop(&self) {
        let old = self
            .hook
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(old) = old {
            old.shutdown();
        }
    }
}

impl Drop for WinHotkeySource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_is_stamped_by_its_age_on_the_tick_counter() {
        let now = 5_000_000_000_000;
        assert_eq!(event_time_ns(now, 10_000, 9_997), now - 3_000_000);
        assert_eq!(event_time_ns(now, 10_000, 10_000), now);
        // The counter wrapped between the event and now.
        assert_eq!(event_time_ns(now, 2, u32::MAX - 1), now - 4_000_000);
    }

    #[test]
    fn an_implausible_age_is_stamped_now() {
        let now = 42_000_000_000;
        assert_eq!(event_time_ns(now, 10_000, 5_000), now, "5 s old");
        assert_eq!(event_time_ns(now, 10_000, 10_001), now, "from the future");
        assert_eq!(event_time_ns(0, 10, 5), 0, "saturates");
    }

    #[test]
    fn our_marker_is_recognisable_ascii() {
        assert_eq!(&(SYNTHETIC_EVENT_MARK as u32).to_be_bytes(), b"inkw");
    }

    #[test]
    fn an_unsupported_token_is_refused_before_any_hook_exists() {
        let source = WinHotkeySource::new(WinClock::new().unwrap());
        let sink: EventSink<HotkeyEvent> = Arc::new(|_| {});
        assert!(matches!(
            source.start(&HotkeyBinding("fn".into()), sink),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(source.hook.lock().unwrap().is_none());
    }

    /// Installs and removes a real hook. It needs a desktop session (a hook in a service session
    /// sees nothing, and CI runners vary), so it runs by hand:
    /// `cargo test -p ink-platform-win -- --ignored`, from a terminal on the desktop.
    #[test]
    #[ignore = "installs a global keyboard hook"]
    fn a_hook_starts_rebinds_and_stops() {
        let source = WinHotkeySource::new(WinClock::new().unwrap());
        let sink: EventSink<HotkeyEvent> = Arc::new(|_| {});
        source
            .start(&HotkeyBinding(DEFAULT_BINDING.into()), sink.clone())
            .expect("hook");
        source
            .start(&HotkeyBinding("ctrl+shift+space".into()), sink)
            .expect("rebinding replaces the hook");
        source.stop();
        source.stop();
    }
}
