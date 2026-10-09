//! The global hotkey: a low-level keyboard hook (`WH_KEYBOARD_LL`).
//!
//! **Why a hook.** `RegisterHotKey` reports presses only, never releases, and cannot take a
//! modifier on its own; hold-to-talk needs both. The hook sees every key event in the session
//! before any app, needs no permission, and may swallow the hotkey's own events (the rules are in
//! `machine`). It cannot see keys typed into an app running as administrator or on the secure
//! desktop (UAC, the lock screen): a key released there is never seen, so the key's next press
//! ends such a hold (`machine`), and the core's stuck-key watchdog ends it if none comes.
//!
//! **Threads.** The hook runs on its own thread, which installs it and pumps messages: Windows
//! calls the hook on that thread. The callback reads the event, updates a `Copy` state machine
//! held in a `Cell`, calls the core's sink (which only enqueues) and returns. Windows removes a
//! hook that takes longer than its timeout (about a second) **without telling anyone**, so the
//! callback takes no locks and does no work beyond that, the thread runs at high priority, and a
//! heartbeat (`heartbeat`) notices a removed hook and installs it again; if it cannot, or has
//! had to three times in ten minutes, `Lost`. The check is sampled: only near real input.
//!
//! **Our own keys.** Events this crate injects (the paste keystroke, typed text) carry
//! [`SYNTHETIC_EVENT_MARK`] in `dwExtraInfo` and pass through untouched. Other injected input
//! (a remote-desktop client, say) is treated as typed.
//!
//! **Lost.** If the message loop fails, the source reports `Lost` once and its thread ends. Our own
//! `stop` never reports `Lost`; a hold in progress is reported as `Cancelled`.
//!
//! **Panics.** A panic in the core's sink is caught on the hook thread (it must not unwind into
//! user32), counted in [`WinHotkeySource::callback_panics`], and recovered: the hold is abandoned
//! and `Cancelled` is sent, while the key still down stays swallowed until it comes up.
//!
//! **Timestamps** are host time on the [`WinClock`] timebase: the hook's `time` (milliseconds on
//! the tick counter) gives the event's age, which is taken off the clock's `now_ns`.
//!
//! **A lone left-hand modifier's wait** (`machine`) is a thread timer on the hook thread: the
//! callback only posts [`WM_INK_ARM`] (as it posts the mask key's message), and the message loop
//! sets the timer and, when it fires, asks the machine whether the hold starts. The loop then
//! injects the mask key and reports the press, stamped at the key's press. For the length of the
//! wait only, a low-level mouse hook notes a click or the wheel (Ctrl+click, Ctrl+wheel), which
//! the keyboard hook cannot see; it does nothing else, and is gone when the wait ends, so no
//! mouse move passes through this thread outside a wait.
#![cfg(windows)]

pub(crate) mod binding;
mod heartbeat;
mod machine;

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{Clock, EventSink, HotkeyBinding, HotkeyEvent, HotkeySource, PlatformError};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::Accessibility::FILTERKEYS;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, KillTimer, MSG, PM_NOREMOVE,
    PeekMessageW, PostThreadMessageW, SPI_GETFILTERKEYS, SPI_GETKEYBOARDDELAY,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetTimer, SetWindowsHookExW, SystemParametersInfoW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_MBUTTONDOWN, WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_TIMER, WM_XBUTTONDOWN,
};

use crate::clock::WinClock;
use binding::{Binding, modifier, vk};
use heartbeat::{Heartbeat, ReinstallBudget};
use machine::{Edge, HoldMachine, HookInput};

pub use binding::{DEFAULT_BINDING, KEYS};

/// Whether the hook can watch `token` (a dictation or edit key the user chose): its canonical
/// spelling ([`Binding::canonical`]), to store and compare, or why not, in the parser's own words.
/// A [`WinHotkeySource`] binds exactly the tokens this accepts.
pub fn check(token: &str) -> Result<String, &'static str> {
    match Binding::parse(token) {
        Ok(binding) => Ok(binding.canonical()),
        Err(PlatformError::Unsupported(why)) => Err(why),
        // The parser refuses only as Unsupported; anything else is still a refusal.
        Err(_) => Err(binding::refusal::UNKNOWN_KEY),
    }
}

/// The marker in `dwExtraInfo` on every key event this crate injects, so its own hook lets them
/// through. Arbitrary; ASCII for "inkw".
pub(crate) const SYNTHETIC_EVENT_MARK: usize = 0x696E_6B77;

/// The marker on the heartbeat key (`heartbeat`): the hook swallows it and notes that it arrived.
/// ASCII for "inkh".
pub(crate) const HEARTBEAT_MARK: usize = 0x696E_6B68;

/// The mask key: an unassigned virtual key (`0xE8`), injected while a swallowed chord's modifiers
/// are down so their release is not a lone tap (see `machine`). It carries
/// [`SYNTHETIC_EVENT_MARK`] and passes through the hook, because Windows has to see it; apps
/// ignore it.
pub(crate) const MASK_VK: u16 = 0xE8;

/// The hook thread's message asking it to inject the mask key (posted from the hook callback,
/// which does not inject from inside itself).
const WM_INK_MASK: u32 = WM_APP + 2;

/// The hook thread's message asking it to end a lone modifier's wait in `wParam` milliseconds
/// (posted from the hook callback, which sets no timer itself). A newer one replaces it.
const WM_INK_ARM: u32 = WM_APP + 3;

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

/// The chord modifiers still down once `released` is up. The hook runs before the key state takes
/// its event in, so the key coming up still reads as down: each modifier counts while a key of it
/// other than `released` is down (left Ctrl let go of with right Ctrl held: still Ctrl). Reads key
/// state only: no allocation, no lock.
fn modifiers_after_release(released: u32) -> u8 {
    // SAFETY: GetAsyncKeyState takes any virtual key and only reads state.
    let down = |key: u32| key != released && unsafe { GetAsyncKeyState(key as i32) } < 0;
    let mut bits = 0;
    if down(vk::LCONTROL) || down(vk::RCONTROL) {
        bits |= modifier::CTRL;
    }
    if down(vk::LSHIFT) || down(vk::RSHIFT) {
        bits |= modifier::SHIFT;
    }
    if down(vk::LMENU) || down(vk::RMENU) {
        bits |= modifier::ALT;
    }
    if down(vk::LWIN) || down(vk::RWIN) {
        bits |= modifier::WIN;
    }
    bits
}

/// Whether the key state reads `key` as down before the event the hook is deciding (the hook runs
/// before the state takes the event in): the OS saw the key go down and has not seen it come up.
/// A key the hook swallows reads as up at its repeats and key-up too (see `machine`). Reads key
/// state only. **Hook thread.**
fn reads_down(key: u32) -> bool {
    // SAFETY: GetAsyncKeyState takes any virtual key and only reads state.
    let state = unsafe { GetAsyncKeyState(key as i32) };
    state < 0
}

/// FilterKeys is on (`FKF_FILTERKEYSON`, which windows-rs does not bind).
const FKF_FILTERKEYSON: u32 = 0x1;

/// [`machine::repeat_gap_ms`] from the keyboard settings now, or the fallback if they cannot be
/// read. **Hook thread**, outside the callback.
fn repeat_gap_now() -> u32 {
    let mut setting = 0u32;
    // SAFETY: SPI_GETKEYBOARDDELAY writes one integer to the live variable passed.
    let delay = unsafe {
        SystemParametersInfoW(
            SPI_GETKEYBOARDDELAY,
            0,
            Some((&raw mut setting).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map(|()| setting);
    let mut keys = FILTERKEYS {
        cbSize: size_of::<FILTERKEYS>() as u32,
        ..Default::default()
    };
    // SAFETY: SPI_GETFILTERKEYS fills the live struct passed, its size set.
    let filter = unsafe {
        SystemParametersInfoW(
            SPI_GETFILTERKEYS,
            keys.cbSize,
            Some((&raw mut keys).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map(|()| {
        (keys.dwFlags & FKF_FILTERKEYSON != 0).then_some((keys.iDelayMSec, keys.iRepeatMSec))
    });
    match (delay, filter) {
        (Ok(delay), Ok(filter)) => machine::repeat_gap_ms(delay, filter),
        _ => machine::FALLBACK_REPEAT_GAP_MS,
    }
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

/// Whether a mouse button is down now: a modifier held with it is a click or a drag (Ctrl+click,
/// Shift+drag), not a dictation. Reads key state only. **Hook thread.**
fn pointer_down() -> bool {
    [
        vk::LBUTTON,
        vk::RBUTTON,
        vk::MBUTTON,
        vk::XBUTTON1,
        vk::XBUTTON2,
    ]
    .into_iter()
    // SAFETY: GetAsyncKeyState takes any virtual key and only reads state.
    .any(|key| unsafe { GetAsyncKeyState(key as i32) } < 0)
}

/// What the hook callback needs, kept on the hook thread.
struct HookContext {
    sink: EventSink<HotkeyEvent>,
    clock: WinClock,
    panics: Arc<AtomicU64>,
    reinstalls: Arc<AtomicU64>,
}

thread_local! {
    /// The hold state. `Copy`, in a `Cell`: the callback never borrows across the sink call.
    static MACHINE: Cell<Option<HoldMachine>> = const { Cell::new(None) };
    /// Set once before the hook is installed, read by the callback.
    static CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) };
    /// Set by the callback when the heartbeat key reaches it.
    static HEARTBEAT_SEEN: Cell<bool> = const { Cell::new(false) };
    /// Every call of the callback, for telling a removed hook from a heartbeat another hook ate.
    static CALLBACKS: Cell<u64> = const { Cell::new(0) };
    /// A click or the wheel during a lone modifier's wait, set by the mouse hook.
    static POINTER_USED: Cell<bool> = const { Cell::new(false) };
}

/// The mouse hook, installed only while a lone modifier waits: notes a button press or the wheel,
/// and passes every event on. **Hook thread.**
unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && matches!(
            wparam.0 as u32,
            WM_LBUTTONDOWN
                | WM_RBUTTONDOWN
                | WM_MBUTTONDOWN
                | WM_XBUTTONDOWN
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
        )
    {
        POINTER_USED.set(true);
    }
    // SAFETY: passes the event on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Installs the mouse hook on this thread.
fn install_mouse() -> windows::core::Result<HHOOK> {
    // SAFETY: this module's handle (the hook procedure lives in it) and a valid hook procedure.
    unsafe {
        GetModuleHandleW(None).and_then(|module| {
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(module.into()), 0)
        })
    }
}

/// Calls the sink; a panic is caught, counted and recovered (the hold is abandoned, `Cancelled`
/// sent). Whether the sink took the event.
fn emit(context: &HookContext, event: HotkeyEvent) -> bool {
    if catch_unwind(AssertUnwindSafe(|| (context.sink)(event))).is_ok() {
        return true;
    }
    context.panics.fetch_add(1, Ordering::Relaxed);
    MACHINE.with(|m| {
        if let Some(mut machine) = m.get() {
            machine.reset();
            m.set(Some(machine));
        }
    });
    let _ = catch_unwind(AssertUnwindSafe(|| (context.sink)(HotkeyEvent::Cancelled)));
    false
}

/// The hook. **Hook thread.**
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    CALLBACKS.set(CALLBACKS.get().wrapping_add(1));
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
    if event.dwExtraInfo == HEARTBEAT_MARK {
        HEARTBEAT_SEEN.set(true);
        return true; // ours alone: no app sees it
    }
    let Some(mut machine) = MACHINE.with(Cell::get) else {
        return false;
    };
    // The key state is read for the hotkey's own key only: the callback runs on every keystroke.
    let ours = event.vkCode == machine.key_vk();
    let input = match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => HookInput::KeyDown {
            vk: event.vkCode,
            // A lone modifier's own press counts the other modifiers only, its sibling included.
            modifiers: if ours && machine.waits() {
                modifiers_after_release(event.vkCode)
            } else {
                modifiers_down()
            },
            reads_down: ours && reads_down(event.vkCode),
            at_ms: event.time,
        },
        WM_KEYUP | WM_SYSKEYUP => HookInput::KeyUp {
            vk: event.vkCode,
            modifiers: modifiers_after_release(event.vkCode),
            reads_down: ours && reads_down(event.vkCode),
        },
        _ => return false,
    };
    let verdict = machine.on(input);
    MACHINE.with(|m| m.set(Some(machine)));
    if verdict.mask {
        // SAFETY: posts to this thread's own queue; the loop injects the key after the callback.
        let _ =
            unsafe { PostThreadMessageW(GetCurrentThreadId(), WM_INK_MASK, WPARAM(0), LPARAM(0)) };
    }
    if let Some(ms) = verdict.arm_ms {
        // SAFETY: posts to this thread's own queue; the loop sets the timer after the callback.
        let _ = unsafe {
            PostThreadMessageW(
                GetCurrentThreadId(),
                WM_INK_ARM,
                WPARAM(ms as usize),
                LPARAM(0),
            )
        };
    }
    if let Some(edge) = verdict.edge {
        report(edge, verdict.at_ms.unwrap_or(event.time));
    }
    verdict.swallow
}

/// Reports an edge that happened at `at_tick_ms` on the tick counter. **Hook thread**, in the
/// callback or the loop.
fn report(edge: Edge, at_tick_ms: u32) {
    CONTEXT.with(|c| {
        if let Some(context) = c.borrow().as_ref() {
            // SAFETY: no arguments; reads the tick counter.
            let now_tick = unsafe { GetTickCount() };
            let at_ns = event_time_ns(context.clock.now_ns(), now_tick, at_tick_ms);
            match edge {
                Edge::Pressed => {
                    emit(context, HotkeyEvent::Pressed { at_ns });
                }
                Edge::Released => {
                    emit(context, HotkeyEvent::Released { at_ns });
                }
                Edge::ReleasedThenPressed => {
                    // If the release panicked, the key now trails (`emit`): no press after it.
                    if emit(context, HotkeyEvent::Released { at_ns }) {
                        emit(context, HotkeyEvent::Pressed { at_ns });
                    }
                }
            }
        }
    });
}

/// A lone modifier's wait ended (its timer fired): the machine decides, the mask key goes in
/// while the modifier is still down, then the press is reported. Returns a further wait, if the
/// timer fired early. **Hook thread**, in the loop.
fn wait_ended() -> Option<u32> {
    // SAFETY: no arguments; reads the tick counter.
    let now = unsafe { GetTickCount() };
    let verdict = MACHINE.with(|m| {
        let mut machine = m.get()?;
        // The key itself reading up means its key-up was lost: no hold on a key nobody holds.
        let interrupted = POINTER_USED.take() || pointer_down() || !reads_down(machine.key_vk());
        let verdict = machine.on_timer(now, interrupted);
        m.set(Some(machine));
        Some(verdict)
    })?;
    if verdict.mask {
        crate::insert::send_mask_key();
    }
    if let Some(edge) = verdict.edge {
        report(edge, verdict.at_ms.unwrap_or(now));
    }
    verdict.arm_ms
}

/// The hook thread: install, pump, uninstall. A hold in progress at the end is `Cancelled`.
fn run(
    binding: Binding,
    context: HookContext,
    ready: mpsc::SyncSender<Result<u32, PlatformError>>,
    cancelled: &AtomicBool,
) {
    // Make sure this thread has a message queue before anyone posts to it.
    let mut msg = MSG::default();
    // SAFETY: a live MSG; PM_NOREMOVE leaves the queue as it is.
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    MACHINE.with(|m| m.set(Some(HoldMachine::new(binding, repeat_gap_now()))));
    CONTEXT.with(|c| *c.borrow_mut() = Some(context));
    // The hook's deadline is wall time: a busy machine must not starve this thread into it.
    // SAFETY: the pseudo-handle of this thread.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) };
    let forget = || {
        MACHINE.with(Cell::take);
        CONTEXT.with(|c| c.borrow_mut().take());
    };
    if cancelled.load(Ordering::Acquire) {
        return forget();
    }
    let hook = match install() {
        Ok(hook) => hook,
        Err(e) => {
            let _ = ready.send(Err(PlatformError::Failed(format!(
                "the keyboard hook was refused: {e}"
            ))));
            return forget();
        }
    };
    // A shortcut recorded while hooks were suspended may still be physically held on resume.
    // Before ready (and before pumping callbacks), let that initial hold finish without an
    // action. The OS saw its down, so its repeats and release must continue reaching the app.
    MACHINE.with(|slot| {
        if let Some(mut machine) = slot.get() {
            machine.wait_for_initial_release(reads_down(machine.key_vk()));
            slot.set(Some(machine));
        }
    });
    // `start` gave up waiting (it said so to its caller): the hook must not outlive that answer.
    // SAFETY: no arguments.
    if cancelled.load(Ordering::Acquire) || ready.send(Ok(unsafe { GetCurrentThreadId() })).is_err()
    {
        // SAFETY: installed above on this thread, removed once.
        let _ = unsafe { UnhookWindowsHookEx(hook) };
        return forget();
    }
    let (hook, lost) = pump(hook);
    // SAFETY: installed on this thread, removed once.
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

/// Installs the hook on this thread.
fn install() -> windows::core::Result<HHOOK> {
    // SAFETY: this module's handle (the hook procedure lives in it) and a valid hook procedure.
    unsafe {
        GetModuleHandleW(None).and_then(|module| {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(module.into()), 0)
        })
    }
}

/// The tick of the last user input, from `GetLastInputInfo`.
fn last_input_tick() -> Option<u32> {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: a live struct with its size set.
    unsafe { GetLastInputInfo(&mut info) }
        .as_bool()
        .then_some(info.dwTime)
}

/// The hook thread's message loop: the hook runs inside `GetMessageW`; the loop injects the mask
/// key when asked and runs the heartbeat. Returns the hook (it may have been reinstalled) and
/// whether the hotkey was lost.
fn pump(mut hook: HHOOK) -> (HHOOK, bool) {
    let mut msg = MSG::default();
    let mut heartbeat = Heartbeat::default();
    let mut budget = ReinstallBudget::default();
    // SAFETY: a thread timer (no window, no callback); it posts WM_TIMER to this thread.
    let tick_timer = unsafe { SetTimer(None, 0, heartbeat::INTERVAL_MS, None) };
    let mut check_timer = 0usize;
    // A lone modifier's wait: one at a time, a newer press replacing it, with the mouse hook on.
    let mut wait_timer = 0usize;
    let mut mouse: Option<HHOOK> = None;
    let lost = loop {
        // SAFETY: a live MSG; any window of this thread.
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        match got.0 {
            0 => break false, // WM_QUIT: our stop
            -1 => break true,
            _ => {}
        }
        match msg.message {
            WM_INK_MASK => crate::insert::send_mask_key(),
            WM_INK_ARM => {
                POINTER_USED.set(false);
                if mouse.is_none() {
                    // Without it a click in the wait goes unseen; the wait itself still works.
                    mouse = install_mouse().ok();
                }
                if !arm(&mut wait_timer, msg.wParam.0 as u32) {
                    abandon_wait(&mut mouse);
                }
            }
            WM_TIMER if wait_timer != 0 && msg.wParam.0 == wait_timer => {
                // SAFETY: the timer made above on this thread.
                let _ = unsafe { KillTimer(None, wait_timer) };
                wait_timer = 0;
                match wait_ended() {
                    Some(ms) if arm(&mut wait_timer, ms) => {}
                    Some(_) => abandon_wait(&mut mouse),
                    None => unhook_mouse(&mut mouse),
                }
            }
            WM_TIMER if tick_timer != 0 && msg.wParam.0 == tick_timer => {
                // The keyboard settings may have changed (FilterKeys switched on, say).
                let gap = repeat_gap_now();
                MACHINE.with(|m| {
                    if let Some(mut machine) = m.get() {
                        machine.set_repeat_gap(gap);
                        m.set(Some(machine));
                    }
                });
                // SAFETY: no arguments.
                let now = unsafe { GetTickCount() };
                // Not while an elevated window is in front: it would drop the heartbeat (UIPI).
                let due = last_input_tick().is_some_and(|last| {
                    heartbeat
                        .should_send(now, last, || !crate::integrity::foreground_blocks_input())
                });
                if due && check_timer == 0 {
                    HEARTBEAT_SEEN.set(false);
                    if crate::insert::send_heartbeat() {
                        // SAFETY: no arguments.
                        heartbeat.sent(unsafe { GetTickCount() }, CALLBACKS.get());
                        // SAFETY: a one-shot thread timer, killed when it fires.
                        check_timer = unsafe { SetTimer(None, 0, heartbeat::DEADLINE_MS, None) };
                    }
                }
            }
            WM_TIMER if check_timer != 0 && msg.wParam.0 == check_timer => {
                // SAFETY: the timer made above on this thread.
                let _ = unsafe { KillTimer(None, check_timer) };
                check_timer = 0;
                if HEARTBEAT_SEEN.take() {
                    heartbeat.seen();
                }
                if heartbeat.missed(CALLBACKS.get()) {
                    // Windows removed the hook, most likely. Past the cap, report it rather than
                    // churn (a hook that keeps timing out, or one we keep misjudging).
                    // SAFETY: no arguments.
                    if !budget.allow(unsafe { GetTickCount() }) {
                        break true;
                    }
                    // Unhooking the stale handle may fail; either way a new one goes in.
                    // SAFETY: the handle this thread installed.
                    let _ = unsafe { UnhookWindowsHookEx(hook) };
                    match install() {
                        Ok(fresh) => {
                            hook = fresh;
                            reinstalled();
                        }
                        Err(_) => break true,
                    }
                }
            }
            _ => {}
        }
    };
    for timer in [tick_timer, check_timer, wait_timer] {
        if timer != 0 {
            // SAFETY: timers made on this thread.
            let _ = unsafe { KillTimer(None, timer) };
        }
    }
    unhook_mouse(&mut mouse);
    (hook, lost)
}

/// Sets the lone modifier's wait timer to `ms`, replacing the one in `timer`. Whether Windows gave
/// a timer. **Hook thread**, in the loop.
fn arm(timer: &mut usize, ms: u32) -> bool {
    if *timer != 0 {
        // SAFETY: a timer this thread made.
        let _ = unsafe { KillTimer(None, *timer) };
    }
    // SAFETY: a one-shot thread timer, killed when it fires or is replaced.
    *timer = unsafe { SetTimer(None, 0, ms, None) };
    *timer != 0
}

/// No timer for the wait: it ends as a shortcut (see `machine`), and the mouse hook goes.
fn abandon_wait(mouse: &mut Option<HHOOK>) {
    MACHINE.with(|m| {
        if let Some(mut machine) = m.get() {
            machine.abandon_wait();
            m.set(Some(machine));
        }
    });
    unhook_mouse(mouse);
}

/// Removes the mouse hook, if it is in.
fn unhook_mouse(mouse: &mut Option<HHOOK>) {
    if let Some(hook) = mouse.take() {
        // SAFETY: installed on this thread, removed once.
        let _ = unsafe { UnhookWindowsHookEx(hook) };
    }
}

/// After a reinstall: counted. A hold in progress is kept, because the miss may have been false
/// (another hook ate the heartbeat); if its release really was lost, the key's next press ends it,
/// or the core's stuck-hold watchdog does.
fn reinstalled() {
    CONTEXT.with(|c| {
        if let Some(context) = c.borrow().as_ref() {
            context.reinstalls.fetch_add(1, Ordering::Relaxed);
        }
    });
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
        reinstalls: Arc<AtomicU64>,
    ) -> Result<Self, PlatformError> {
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let context = HookContext {
            sink,
            clock,
            panics,
            reinstalls,
        };
        let thread = thread::Builder::new()
            .name("ink-hotkey".into())
            .spawn({
                let cancelled = Arc::clone(&cancelled);
                move || run(binding, context, ready_tx, &cancelled)
            })
            .map_err(|e| {
                PlatformError::Failed(format!("could not start the hotkey thread: {e}"))
            })?;
        match ready.recv_timeout(START_TIMEOUT) {
            Ok(Ok(thread_id)) => Ok(Self { thread, thread_id }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                // The thread unhooks (or never hooks) when it sees the flag or the dropped
                // receiver; if it reported ready in between, it is stopped here.
                if let Some(thread_id) = crate::com::abandon_start(&cancelled, &ready) {
                    Self { thread, thread_id }.shutdown();
                }
                Err(PlatformError::Failed(
                    "the keyboard hook did not start in time".into(),
                ))
            }
        }
    }

    fn shutdown(self) {
        if self.thread.is_finished() {
            // It ended on its own (Lost); nothing to post to.
            let _ = self.thread.join();
            return;
        }
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
    reinstalls: Arc<AtomicU64>,
}

impl WinHotkeySource {
    /// A source that stamps events on `clock`'s timebase. No hook exists until
    /// [`start`](HotkeySource::start).
    pub fn new(clock: WinClock) -> Self {
        Self {
            clock,
            hook: Mutex::new(None),
            panics: Arc::new(AtomicU64::new(0)),
            reinstalls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Panics caught on the hook thread since this source was created. Each was recovered: the
    /// hold was abandoned and `Cancelled` sent. A non-zero count is a bug to report.
    pub fn callback_panics(&self) -> u64 {
        self.panics.load(Ordering::Relaxed)
    }

    /// Times the heartbeat found the hook removed by Windows and installed it again. Each is a
    /// callback that ran past Windows' deadline; a rising count is worth reporting.
    pub fn hook_reinstalls(&self) -> u64 {
        self.reinstalls.load(Ordering::Relaxed)
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
            Arc::clone(&self.reinstalls),
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

    /// `start` timed out: the thread must not keep a global hook. Runs anywhere; where a hook can
    /// be installed it is removed again at once, and the thread ends either way.
    #[test]
    fn a_hook_thread_whose_start_was_abandoned_ends_without_a_hook() {
        for cancel_first in [false, true] {
            let (ready_tx, ready) = mpsc::sync_channel(1);
            drop(ready); // `start` stopped waiting
            let cancelled = Arc::new(AtomicBool::new(cancel_first));
            let called = Arc::new(AtomicU64::new(0));
            let sink: EventSink<HotkeyEvent> = {
                let called = Arc::clone(&called);
                Arc::new(move |_| {
                    called.fetch_add(1, Ordering::Relaxed);
                })
            };
            let context = HookContext {
                sink,
                clock: WinClock::new().unwrap(),
                panics: Arc::default(),
                reinstalls: Arc::default(),
            };
            let thread = thread::spawn({
                let cancelled = Arc::clone(&cancelled);
                move || {
                    run(
                        Binding::parse(DEFAULT_BINDING).unwrap(),
                        context,
                        ready_tx,
                        &cancelled,
                    )
                }
            });
            let start = std::time::Instant::now();
            while !thread.is_finished() {
                assert!(
                    start.elapsed() < Duration::from_secs(3),
                    "the abandoned hook thread kept running"
                );
                thread::sleep(Duration::from_millis(10));
            }
            thread.join().unwrap();
            assert_eq!(
                called.load(Ordering::Relaxed),
                0,
                "its sink is never called"
            );
        }
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
