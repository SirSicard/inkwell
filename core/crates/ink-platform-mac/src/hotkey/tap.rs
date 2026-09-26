//! The event tap's thread, run loop and callback. The FFI lives here; every decision lives in
//! [`machine`](super::machine), which is tested without a tap.
#![cfg(target_os = "macos")]

use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, Ordering};
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
use super::machine::{Edge, HoldMachine, TapInput};
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
    /// Starts the tap thread and waits until the tap exists or has been refused.
    pub(super) fn spawn(
        binding: Binding,
        sink: EventSink<HotkeyEvent>,
        clock: MacClock,
    ) -> Result<Self, PlatformError> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("ink-hotkey-tap".into())
            .spawn({
                let sink = sink.clone();
                let stop = Arc::clone(&stop);
                move || run(binding, sink, clock, stop, &ready_tx)
            })
            .map_err(|e| {
                PlatformError::Failed(format!("could not start the hotkey thread: {e}"))
            })?;
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(control)) => Ok(Self {
                thread,
                control,
                sink,
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
        } = self;
        control.halt();
        drop(control);
        // A tap thread that panicked outside the callback's guard leaves the hold state
        // unknown; reporting a cancel is the safe side, since the core ignores it when idle.
        let held = thread.join().unwrap_or(true);
        if held {
            sink(HotkeyEvent::Cancelled);
        }
    }
}

/// What the callback needs. It lives on the tap thread's stack and is touched only by that
/// thread, which is why plain `Cell`s are enough: the callback never locks.
struct Context {
    machine: Cell<HoldMachine>,
    sink: EventSink<HotkeyEvent>,
    clock: MacClock,
    port: Cell<*const CFMachPort>,
}

impl Context {
    /// Decides one event and reports its edge. Returns whether to swallow it.
    fn handle(&self, event_type: CGEventType, event: &CGEvent) -> bool {
        let input = if event_type == CGEventType::TapDisabledByTimeout
            || event_type == CGEventType::TapDisabledByUserInput
        {
            TapInput::Disabled
        } else if event_type == CGEventType::FlagsChanged
            || event_type == CGEventType::KeyDown
            || event_type == CGEventType::KeyUp
        {
            let field = |f| CGEvent::integer_value_field(Some(event), f);
            if field(CGEventField::EventSourceUserData) == SYNTHETIC_EVENT_MARK {
                return false;
            }
            // Keycodes are 16-bit; anything else matches no binding.
            let keycode =
                u16::try_from(field(CGEventField::KeyboardEventKeycode)).unwrap_or(u16::MAX);
            let flags = CGEvent::flags(Some(event)).0;
            if event_type == CGEventType::FlagsChanged {
                TapInput::FlagsChanged { keycode, flags }
            } else if event_type == CGEventType::KeyDown {
                TapInput::KeyDown {
                    keycode,
                    flags,
                    autorepeat: field(CGEventField::KeyboardEventAutorepeat) != 0,
                }
            } else {
                TapInput::KeyUp { keycode }
            }
        } else {
            return false;
        };

        let mut machine = self.machine.get();
        let verdict = machine.on(input);
        self.machine.set(machine);

        if verdict.reenable {
            // SAFETY: `port` is set by `run` before the run loop first runs, and points at the
            // port `run` keeps retained until after the run loop has returned for good; this
            // callback only runs inside that run loop.
            if let Some(port) = unsafe { self.port.get().as_ref() } {
                CGEvent::tap_enable(port, true);
            }
        }
        if let Some(edge) = verdict.edge {
            let stamp = || {
                event_time_ns(
                    CGEvent::timestamp(Some(event)),
                    self.clock.timebase(),
                    self.clock.now_ns(),
                )
            };
            (self.sink)(match edge {
                Edge::Pressed => HotkeyEvent::Pressed { at_ns: stamp() },
                Edge::Released => HotkeyEvent::Released { at_ns: stamp() },
                Edge::Cancelled => HotkeyEvent::Cancelled,
            });
        }
        verdict.swallow
    }
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
    // An unwind must never cross into CoreGraphics. A panicking sink costs one event; the tap
    // stays armed and the event passes through.
    match catch_unwind(AssertUnwindSafe(|| context.handle(event_type, event_ref))) {
        Ok(true) => ptr::null_mut(),
        _ => pass,
    }
}

/// The tap thread: create the tap, report, run until stopped. Returns whether a hold was still
/// in progress, so `shutdown` can report its cancellation.
fn run(
    binding: Binding,
    sink: EventSink<HotkeyEvent>,
    clock: MacClock,
    stop: Arc<AtomicBool>,
    ready: &mpsc::SyncSender<Result<Control, PlatformError>>,
) -> bool {
    let context = Context {
        machine: Cell::new(HoldMachine::new(binding)),
        sink,
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

    let mut ended_by_stop = true;
    while !stop.load(Ordering::Acquire) {
        // Blocks until the run loop is stopped or its only source is gone. Finished means the
        // port was invalidated: by `shutdown`, or by the system.
        if CFRunLoop::run_in_mode(default, 1.0e10, false) == CFRunLoopRunResult::Finished {
            ended_by_stop = stop.load(Ordering::Acquire);
            break;
        }
    }
    port.invalidate();
    run_loop.remove_source(Some(&source), common);

    let held = context.machine.get().is_held();
    if held && !ended_by_stop {
        // The system took the tap away. This thread is joined only when the app next stops or
        // rebinds the hotkey, which may be much later, so report the lost hold now rather than
        // leave the core holding a key that is not down.
        (context.sink)(HotkeyEvent::Cancelled);
        return false;
    }
    held
}
