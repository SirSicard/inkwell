//! A hand-run timer for dictation as the user feels it: from the dictation key coming up to the
//! text arriving in the focused app (TextEdit, say), over N takes.
//!
//! It watches the key with a **listen-only** tap at the HID level, which sees the key before
//! Inkwell's own (active, session-level) tap swallows it, and never swallows anything itself. At
//! each key-up it polls the focused element's length in characters through Accessibility every
//! millisecond until the length changes: that moment is the text arriving. It prints lengths and
//! times only, never text.
//!
//! Needs, for the terminal app it runs in: **Input Monitoring** (the listen-only tap) and
//! **Accessibility** (reading TextEdit's length). It never prompts; without them it says which is
//! missing. Run it after Inkwell is running, with the caret in an empty TextEdit document.
//!
//! ```text
//! cargo run -p ink-platform-mac --example dictation_timing -- [--key fn] [--runs 10]
//!
//!   --key TOKEN   the dictation key as Settings sets it: fn, right_option, right_command,
//!                 right_control, right_shift (default fn)
//!   --runs N      takes to time (default 10); then p50, p95 and max
//! ```
//!
//! What it measures ends where the user sees text; what `ink-bench latency` measures ends at the
//! insertion request. The difference is the capture path, the event tap, the paste and the target
//! app drawing it.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("dictation_timing runs on macOS only.");
}

#[cfg(target_os = "macos")]
fn main() {
    mac::main();
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::process::exit;
    use std::ptr::{self, NonNull};
    use std::sync::mpsc::{self, Sender};
    use std::thread;
    use std::time::{Duration, Instant};

    use objc2_core_foundation::{CFMachPort, CFRunLoop, kCFRunLoopCommonModes};
    use objc2_core_graphics::{
        CGEvent, CGEventField, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventTapProxy, CGEventType,
    };

    const USAGE: &str = "usage: dictation_timing [--key TOKEN] [--runs N]";
    /// Longest wait for text after a key-up.
    const GIVE_UP: Duration = Duration::from_secs(5);

    /// `kCGEventFlagsChanged`, for the tap's mask.
    const FLAGS_CHANGED: u64 = 12;

    /// A modifier held on its own: its keycode and the flag bit that says it is down (as
    /// ink-platform-mac's bindings; kept here because an example sees only the public API).
    fn key(token: &str) -> Option<(i64, u64)> {
        Some(match token {
            "fn" => (0x3F, 0x0080_0000),
            "right_option" => (0x3D, 0x0000_0040),
            "right_command" => (0x36, 0x0000_0010),
            "right_control" => (0x3E, 0x0000_2000),
            "right_shift" => (0x3C, 0x0000_0004),
            _ => return None,
        })
    }

    enum Edge {
        Down,
        Up(Instant),
    }

    struct Context {
        keycode: i64,
        mask: u64,
        down: std::cell::Cell<bool>,
        edges: Sender<Edge>,
    }

    unsafe extern "C-unwind" fn callback(
        _proxy: CGEventTapProxy,
        event_type: CGEventType,
        event: NonNull<CGEvent>,
        user_info: *mut c_void,
    ) -> *mut CGEvent {
        let pass = event.as_ptr();
        // SAFETY: `user_info` is the `Context` `main` gave the tap; it lives for the process.
        let Some(context) = (unsafe { user_info.cast::<Context>().as_ref() }) else {
            return pass;
        };
        if event_type != CGEventType::FlagsChanged {
            return pass;
        }
        // SAFETY: CoreGraphics passes an event valid for the callback.
        let event_ref = unsafe { event.as_ref() };
        let keycode =
            CGEvent::integer_value_field(Some(event_ref), CGEventField::KeyboardEventKeycode);
        if keycode != context.keycode {
            return pass;
        }
        let now_down = CGEvent::flags(Some(event_ref)).0 & context.mask != 0;
        if now_down != context.down.get() {
            context.down.set(now_down);
            let _ = context.edges.send(if now_down {
                Edge::Down
            } else {
                Edge::Up(Instant::now())
            });
        }
        // Listen-only: the event goes on whatever this returns.
        pass
    }

    pub fn main() {
        let mut token = "fn".to_owned();
        let mut runs = 10usize;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--key" => token = args.next().unwrap_or_default(),
                "--runs" => runs = args.next().and_then(|n| n.parse().ok()).unwrap_or(0),
                _ => {
                    eprintln!("{USAGE}");
                    exit(2);
                }
            }
        }
        let Some((keycode, mask)) = key(&token) else {
            eprintln!("unknown key {token:?}\n{USAGE}");
            exit(2);
        };
        if runs == 0 {
            eprintln!("--runs takes a number above 0\n{USAGE}");
            exit(2);
        }
        if ink_platform_mac::focused_character_count().is_none() {
            eprintln!(
                "note: the focused element reports no length yet. Put the caret in a TextEdit \
                 document, and give this terminal Accessibility if it has none."
            );
        }
        let (tx, rx) = mpsc::channel();
        // Leaked: the tap uses it for the life of the process.
        let context: &'static Context = Box::leak(Box::new(Context {
            keycode,
            mask,
            down: std::cell::Cell::new(false),
            edges: tx,
        }));
        thread::spawn(move || time(&rx, runs));
        // SAFETY: `callback` has the `CGEventTapCallBack` signature; `context` lives forever.
        let port = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::HIDEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                1 << FLAGS_CHANGED,
                Some(callback),
                ptr::from_ref(context).cast_mut().cast(),
            )
        };
        let Some(port) = port else {
            eprintln!(
                "FAIL: macOS refused the listen-only tap. Give this terminal Input Monitoring \
                 (System Settings > Privacy & Security > Input Monitoring), then run again."
            );
            exit(1);
        };
        let Some(source) = CFMachPort::new_run_loop_source(None, Some(&port), 0) else {
            eprintln!("FAIL: could not attach the tap to a run loop");
            exit(1);
        };
        let Some(run_loop) = CFRunLoop::current() else {
            eprintln!("FAIL: no run loop");
            exit(1);
        };
        // SAFETY: an immutable CFString constant exported for the life of the process.
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        CGEvent::tap_enable(&port, true);
        println!(
            "Timing {runs} takes on {token}: hold it, say a sentence into Inkwell, let go. \
             Lengths and times only are printed."
        );
        CFRunLoop::run();
    }

    fn time(edges: &mpsc::Receiver<Edge>, runs: usize) {
        let mut times = Vec::new();
        let mut before = None;
        while times.len() < runs {
            match edges.recv() {
                Ok(Edge::Down) => before = ink_platform_mac::focused_character_count(),
                Ok(Edge::Up(up)) => {
                    let start = before.or_else(ink_platform_mac::focused_character_count);
                    let mut arrived = None;
                    while up.elapsed() < GIVE_UP {
                        let now = ink_platform_mac::focused_character_count();
                        if now.is_some() && now != start {
                            arrived = Some((up.elapsed(), now));
                            break;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    match arrived {
                        Some((took, now)) => {
                            let ms = took.as_secs_f64() * 1_000.0;
                            times.push(ms);
                            println!(
                                "take {}: key-up to text {ms:.0} ms (length {} -> {})",
                                times.len(),
                                start.map_or("?".into(), |n| n.to_string()),
                                now.map_or("?".into(), |n| n.to_string())
                            );
                        }
                        None => println!(
                            "a take with no text within {} s (not counted): too short, or the \
                             caret is not in a text element that reports its length",
                            GIVE_UP.as_secs()
                        ),
                    }
                }
                Err(_) => exit(1),
            }
        }
        times.sort_by(f64::total_cmp);
        let at =
            |p: f64| times[((p * times.len() as f64).ceil() as usize).clamp(1, times.len()) - 1];
        println!(
            "{} takes: p50 {:.0} ms, p95 {:.0} ms, max {:.0} ms (key-up to text in the app)",
            times.len(),
            at(0.5),
            at(0.95),
            times[times.len() - 1]
        );
        exit(0);
    }
}
