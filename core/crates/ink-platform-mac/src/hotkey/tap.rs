//! The event tap's thread, run loop and callback. The FFI lives here; the decisions live in
//! [`machine`](super::machine) and in [`Hold`], both tested without a tap.
#![cfg(target_os = "macos")]

use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{Clock, EventSink, HotkeyEvent, Permission, PlatformError};
use objc2_core_foundation::{
    CFMachPort, CFRetained, CFRunLoop, CFRunLoopRunResult, kCFRunLoopCommonModes,
    kCFRunLoopDefaultMode,
};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType,
};

use super::binding::Binding;
use super::machine::{Edge, HoldMachine, TapInput, Verdict};
use super::{SYNTHETIC_EVENT_MARK, event_time_ns};
use crate::ax;
use crate::clock::MacClock;

/// How long `start` waits for the tap thread to create its tap.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// A running tap: its thread, and what another thread needs to stop it.
pub(super) struct Tap {
    thread: JoinHandle<bool>,
    control: Control,
    sink: EventSink<HotkeyEvent>,
    panics: Arc<AtomicU64>,
}

/// The tap's port and run loop, as the thread that stops the tap sees them.
struct Control {
    port: CFRetained<CFMachPort>,
    run_loop: CFRetained<CFRunLoop>,
    stop: Arc<AtomicBool>,
}

impl Control {
    /// Makes the tap thread's run loop return for good, from any thread.
    fn halt(&self) {
        // The flag first, then the invalidation: a run loop that has not started yet finds no
        // source and returns at once, and one that is running is stopped and woken. Either way
        // the thread sees the flag, so no stop is lost.
        self.stop.store(true, Ordering::Release);
        self.port.invalidate();
        self.run_loop.stop();
        self.run_loop.wake_up();
    }
}

// SAFETY: another thread uses `Control` only to stop the tap, through `CFMachPortInvalidate`,
// `CFRunLoopStop` and `CFRunLoopWakeUp`, which Core Foundation supports from any thread (stopping
// another thread's run loop is their documented purpose), and to release them, which is atomic.
unsafe impl Send for Control {}

impl Tap {
    /// Starts the tap thread and waits until the tap exists or has been refused. Every panic
    /// the tap thread catches is counted in `panics`.
    pub(super) fn spawn(
        binding: Binding,
        sink: EventSink<HotkeyEvent>,
        clock: MacClock,
        panics: Arc<AtomicU64>,
    ) -> Result<Self, PlatformError> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("ink-hotkey-tap".into())
            .spawn({
                let hold = (binding, sink.clone(), Arc::clone(&panics));
                let stop = Arc::clone(&stop);
                move || run(hold, clock, stop, &ready_tx)
            })
            .map_err(|e| {
                PlatformError::Failed(format!("could not start the hotkey thread: {e}"))
            })?;
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(control)) => Ok(Self {
                thread,
                control,
                sink,
                panics,
            }),
            Ok(Err(error)) => {
                // The thread has already returned; joining only collects it.
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                // The flag first: a thread that reports after the drain below sees it before it
                // ever runs its loop. One that reported in between is parked in its run loop
                // already, and the drain finds its control and halts it.
                stop.store(true, Ordering::Release);
                if let Ok(Ok(control)) = ready_rx.try_recv() {
                    control.halt();
                }
                Err(PlatformError::Failed(
                    "the hotkey tap did not start within 5 s".into(),
                ))
            }
        }
    }

    /// Stops the run loop, joins the thread and reports `Cancelled` if a hold was in progress.
    pub(super) fn shutdown(self) {
        let Self {
            thread,
            control,
            sink,
            panics,
        } = self;
        control.halt();
        drop(control);
        // A tap thread that panicked outside the callback's guard leaves the hold state
        // unknown; reporting a cancel is the safe side, since the core ignores it when idle.
        let held = thread.join().unwrap_or(true);
        if held {
            deliver(&sink, &panics, HotkeyEvent::Cancelled);
        }
    }
}

/// Calls the core's sink. A panic in it is caught (it must never unwind into CoreGraphics or out
/// of a `stop`), counted, and reported as `false`: never silent.
fn deliver(sink: &EventSink<HotkeyEvent>, panics: &AtomicU64, event: HotkeyEvent) -> bool {
    let delivered = catch_unwind(AssertUnwindSafe(|| sink(event))).is_ok();
    if !delivered {
        panics.fetch_add(1, Ordering::Relaxed);
    }
    delivered
}

/// The hold as the tap thread keeps it: the state machine, the sink, and the rule that the tap and
/// the core never disagree about whether a key is down. When the core may have missed an edge
/// (its sink panicked), the hold starts over and the core is told `Cancelled`. Plain `Cell`s:
/// only the tap thread touches it, and the callback never locks.
struct Hold {
    machine: Cell<HoldMachine>,
    sink: EventSink<HotkeyEvent>,
    panics: Arc<AtomicU64>,
    lost: Cell<bool>,
}

impl Hold {
    fn new(binding: Binding, sink: EventSink<HotkeyEvent>, panics: Arc<AtomicU64>) -> Self {
        Self {
            machine: Cell::new(HoldMachine::new(binding)),
            sink,
            panics,
            lost: Cell::new(false),
        }
    }

    /// Decides one event and reports its edge, stamped by `at_ns`. Returns the tap's verdict.
    fn on(&self, input: TapInput, at_ns: impl Fn() -> u64) -> Verdict {
        let mut machine = self.machine.get();
        let verdict = machine.on(input);
        self.machine.set(machine);
        if let Some(edge) = verdict.edge {
            let event = match edge {
                Edge::Pressed => HotkeyEvent::Pressed { at_ns: at_ns() },
                Edge::Released => HotkeyEvent::Released { at_ns: at_ns() },
                Edge::Cancelled => HotkeyEvent::Cancelled,
            };
            // A failed cancel is counted and left there: the hold is already reset, and trying
            // again would only panic again.
            if !deliver(&self.sink, &self.panics, event) && edge != Edge::Cancelled {
                self.start_over();
            }
        }
        verdict
    }

    /// The OS took the hotkey away: reset and report `Lost`, once.
    fn lose(&self) {
        if !self.lost.replace(true) {
            self.reset();
            deliver(&self.sink, &self.panics, HotkeyEvent::Lost);
        }
    }

    /// After a panic the callback's guard caught outside the sink: counted, and handled like a
    /// sink panic, because the machine may have moved without the core hearing about it.
    fn recover_from_panic(&self) {
        self.panics.fetch_add(1, Ordering::Relaxed);
        self.start_over();
    }

    fn start_over(&self) {
        self.reset();
        deliver(&self.sink, &self.panics, HotkeyEvent::Cancelled);
    }

    fn reset(&self) {
        let mut machine = self.machine.get();
        machine.reset();
        self.machine.set(machine);
    }

    fn is_held(&self) -> bool {
        self.machine.get().is_held()
    }

    fn is_lost(&self) -> bool {
        self.lost.get()
    }
}

/// What the callback needs. It lives on the tap thread's stack and is touched only by that
/// thread.
struct Context {
    hold: Hold,
    clock: MacClock,
    port: Cell<*const CFMachPort>,
}

impl Context {
    /// Decides one event and reports its edge. Returns whether to swallow it.
    fn handle(&self, event_type: CGEventType, event: &CGEvent) -> bool {
        let Some(input) = decode(event_type, event) else {
            return false;
        };
        let stamp = || {
            event_time_ns(
                CGEvent::timestamp(Some(event)),
                self.clock.timebase(),
                self.clock.now_ns(),
            )
        };
        let verdict = self.hold.on(input, stamp);
        if verdict.reenable {
            // SAFETY: `port` is set by `run` before the run loop first runs, and points at the
            // port `run` keeps retained until after the run loop has returned for good; this
            // callback only runs inside that run loop.
            if let Some(port) = unsafe { self.port.get().as_ref() } {
                CGEvent::tap_enable(port, true);
                if !CGEvent::tap_is_enabled(port) {
                    // The OS will not take the tap back (Accessibility revoked, say). The tap is
                    // disabled, so no event waits on this: report the loss and end the thread.
                    self.hold.lose();
                    if let Some(run_loop) = CFRunLoop::current() {
                        run_loop.stop();
                    }
                }
            }
        }
        verdict.swallow
    }
}

/// Reduces a tap event to what the decision needs. `None` for anything the hotkey ignores,
/// including events this crate posted itself.
fn decode(event_type: CGEventType, event: &CGEvent) -> Option<TapInput> {
    if event_type == CGEventType::TapDisabledByTimeout
        || event_type == CGEventType::TapDisabledByUserInput
    {
        return Some(TapInput::Disabled);
    }
    if event_type != CGEventType::FlagsChanged
        && event_type != CGEventType::KeyDown
        && event_type != CGEventType::KeyUp
    {
        return None;
    }
    let field = |f| CGEvent::integer_value_field(Some(event), f);
    if field(CGEventField::EventSourceUserData) == SYNTHETIC_EVENT_MARK {
        return None;
    }
    // Keycodes are 16-bit; anything else matches no binding.
    let keycode = u16::try_from(field(CGEventField::KeyboardEventKeycode)).unwrap_or(u16::MAX);
    let flags = CGEvent::flags(Some(event)).0;
    Some(if event_type == CGEventType::FlagsChanged {
        TapInput::FlagsChanged { keycode, flags }
    } else if event_type == CGEventType::KeyDown {
        TapInput::KeyDown {
            keycode,
            flags,
            autorepeat: field(CGEventField::KeyboardEventAutorepeat) != 0,
        }
    } else {
        TapInput::KeyUp { keycode }
    })
}

/// The tap's callback. Returning null swallows the event; returning it passes it on.
unsafe extern "C-unwind" fn callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    let pass = event.as_ptr();
    // SAFETY: `user_info` is the `Context` that `run` gave `CGEventTapCreate`. The callback runs
    // only inside that thread's run loop, which returns before `run` drops the context.
    let Some(context) = (unsafe { user_info.cast::<Context>().as_ref() }) else {
        return pass;
    };
    // SAFETY: CoreGraphics passes an event that is valid for the duration of the callback.
    let event_ref = unsafe { event.as_ref() };
    // An unwind must never cross into CoreGraphics. Sink panics are caught inside `Hold`; this
    // guard is the backstop for this crate's own code, and recovers the same way.
    match catch_unwind(AssertUnwindSafe(|| context.handle(event_type, event_ref))) {
        Ok(true) => ptr::null_mut(),
        Ok(false) => pass,
        Err(_) => {
            context.hold.recover_from_panic();
            pass
        }
    }
}

/// The tap thread: create the tap, report, run until stopped. Returns whether a hold was still
/// in progress, so `shutdown` can report its cancellation.
fn run(
    (binding, sink, panics): (Binding, EventSink<HotkeyEvent>, Arc<AtomicU64>),
    clock: MacClock,
    stop: Arc<AtomicBool>,
    ready: &mpsc::SyncSender<Result<Control, PlatformError>>,
) -> bool {
    let context = Context {
        hold: Hold::new(binding, sink, panics),
        clock,
        port: Cell::new(ptr::null()),
    };
    // SAFETY: `callback` has exactly the `CGEventTapCallBack` signature. `user_info` points at
    // `context`, which outlives the tap: the port is invalidated and the run loop has returned
    // before this function returns and drops it.
    let port = unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            binding.event_mask(),
            Some(callback),
            ptr::from_ref(&context).cast_mut().cast(),
        )
    };
    let Some(port) = port else {
        // An active tap is refused without Accessibility. If the process is trusted, the refusal
        // has some other cause, and saying "permission" would send the user the wrong way.
        let error = if ax::is_process_trusted() {
            PlatformError::Failed("macOS refused the keyboard event tap".into())
        } else {
            PlatformError::PermissionDenied(Permission::Accessibility)
        };
        let _ = ready.send(Err(error));
        return false;
    };
    context.port.set(ptr::from_ref(&*port));

    let source = CFMachPort::new_run_loop_source(None, Some(&port), 0);
    let (Some(source), Some(run_loop)) = (source, CFRunLoop::current()) else {
        port.invalidate();
        let _ = ready.send(Err(PlatformError::Failed(
            "could not attach the event tap to a run loop".into(),
        )));
        return false;
    };
    // SAFETY: both are immutable `CFString` constants that Core Foundation exports for the life
    // of the process.
    let (common, default) = unsafe { (kCFRunLoopCommonModes, kCFRunLoopDefaultMode) };
    run_loop.add_source(Some(&source), common);
    CGEvent::tap_enable(&port, true);

    let control = Control {
        port: port.clone(),
        run_loop: run_loop.clone(),
        stop: Arc::clone(&stop),
    };
    if ready.send(Ok(control)).is_err() {
        // `start` gave up waiting; nobody will ever stop this tap, so stop now.
        stop.store(true, Ordering::Release);
    }

    let mut invalidated_by_system = false;
    while !stop.load(Ordering::Acquire) && !context.hold.is_lost() {
        // Blocks until the run loop is stopped or its only source is gone. Finished means the
        // port was invalidated: by `shutdown` (the flag is set first), or by the system.
        if CFRunLoop::run_in_mode(default, 1.0e10, false) == CFRunLoopRunResult::Finished {
            invalidated_by_system = !stop.load(Ordering::Acquire);
            break;
        }
    }
    port.invalidate();
    run_loop.remove_source(Some(&source), common);

    if invalidated_by_system {
        // Not our stop: the OS removed the tap. This thread is joined only when the app next
        // stops or rebinds the hotkey, which may be much later, so report it now.
        context.hold.lose();
    }
    // A lost hotkey was reported and reset already; only our own stop leaves a hold to cancel.
    !context.hold.is_lost() && context.hold.is_held()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    use super::super::binding::{flag, keycode};
    use super::*;

    fn fn_key(down: bool) -> TapInput {
        TapInput::FlagsChanged {
            keycode: keycode::FUNCTION,
            flags: if down { flag::SECONDARY_FN } else { 0 },
        }
    }

    /// A sink that records what it gets and panics when `panics_on` says so. The panic goes
    /// through `resume_unwind`, which skips the panic hook, so the test output stays clean.
    fn sink(
        panics_on: impl Fn(HotkeyEvent) -> bool + Send + Sync + 'static,
    ) -> (EventSink<HotkeyEvent>, Arc<Mutex<Vec<HotkeyEvent>>>) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&got);
        let sink: EventSink<HotkeyEvent> = Arc::new(move |event| {
            if panics_on(event) {
                std::panic::resume_unwind(Box::new("scripted sink panic"));
            }
            record.lock().expect("unpoisoned").push(event);
        });
        (sink, got)
    }

    fn hold(sink: EventSink<HotkeyEvent>) -> (Hold, Arc<AtomicU64>) {
        let panics = Arc::new(AtomicU64::new(0));
        let binding = Binding::parse("fn").expect("valid");
        (Hold::new(binding, sink, Arc::clone(&panics)), panics)
    }

    const AT: fn() -> u64 = || 42;

    #[test]
    fn a_panicking_sink_is_counted_and_the_next_press_starts_clean() {
        let first = Arc::new(AtomicBool::new(true));
        let (sink, got) = sink(move |_| first.swap(false, Ordering::SeqCst));
        let (hold, panics) = hold(sink);

        hold.on(fn_key(true), AT);
        assert_eq!(panics.load(Ordering::SeqCst), 1);
        assert!(
            !hold.is_held(),
            "the tap must not hold a key the core never heard about"
        );
        // The release of the lost press is not the core's any more.
        assert_eq!(hold.on(fn_key(false), AT).edge, None);
        hold.on(fn_key(true), AT);
        assert_eq!(
            *got.lock().expect("unpoisoned"),
            [HotkeyEvent::Cancelled, HotkeyEvent::Pressed { at_ns: 42 }]
        );
    }

    #[test]
    fn a_sink_that_always_panics_never_leaves_the_tap_holding() {
        let (sink, got) = sink(|_| true);
        let (hold, panics) = hold(sink);
        hold.on(fn_key(true), AT);
        // The press, then the cancel that tried to undo it.
        assert_eq!(panics.load(Ordering::SeqCst), 2);
        assert!(!hold.is_held());
        hold.on(fn_key(false), AT);
        assert_eq!(panics.load(Ordering::SeqCst), 2, "no edge, no call");
        assert!(got.lock().expect("unpoisoned").is_empty());
    }

    #[test]
    fn a_failed_cancel_is_counted_and_not_retried() {
        let (sink, got) = sink(|event| event == HotkeyEvent::Cancelled);
        let (hold, panics) = hold(sink);
        hold.on(fn_key(true), AT);
        hold.on(TapInput::Disabled, AT);
        assert_eq!(panics.load(Ordering::SeqCst), 1);
        assert!(!hold.is_held());
        assert_eq!(
            *got.lock().expect("unpoisoned"),
            [HotkeyEvent::Pressed { at_ns: 42 }]
        );
    }

    #[test]
    fn a_lost_hotkey_is_reported_once_and_ends_the_hold() {
        let (sink, got) = sink(|_| false);
        let (hold, panics) = hold(sink);
        hold.on(fn_key(true), AT);
        hold.lose();
        hold.lose();
        assert!(hold.is_lost());
        assert!(!hold.is_held());
        assert_eq!(panics.load(Ordering::SeqCst), 0);
        assert_eq!(
            *got.lock().expect("unpoisoned"),
            [HotkeyEvent::Pressed { at_ns: 42 }, HotkeyEvent::Lost]
        );
    }

    /// A panic outside the sink (in this crate's own callback code) is caught by the callback's
    /// guard; recovering from it resets the hold and tells the core, like a sink panic.
    #[test]
    fn recovering_from_a_callback_panic_resets_and_cancels() {
        let (sink, got) = sink(|_| false);
        let (hold, panics) = hold(sink);
        hold.on(fn_key(true), AT);
        hold.recover_from_panic();
        assert_eq!(panics.load(Ordering::SeqCst), 1);
        assert!(!hold.is_held());
        assert_eq!(
            *got.lock().expect("unpoisoned"),
            [HotkeyEvent::Pressed { at_ns: 42 }, HotkeyEvent::Cancelled]
        );
    }
}
