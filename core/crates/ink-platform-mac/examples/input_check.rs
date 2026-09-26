//! A hand-run check of the Mac input layer: the hotkey tap, insertion, focus and selection.
//!
//! It needs Accessibility for the terminal app it runs in, which an agent's shell and CI do not
//! have, so it is a checklist item (`INPUT-CHECKLIST.md`) rather than a test. It never prompts for
//! the permission, and it prints no clipboard or selection content, only counts and fingerprints.
//!
//! ```text
//! cargo run -p ink-platform-mac --example input_check -- [MODE] [OPTIONS]
//!
//!   (no mode)          hold the hotkey; on release, insert the test text into the focused app
//!   --timed SECONDS    no hotkey: wait, then insert into whatever is focused
//!   --probe            print permissions, the clock and the main-thread check; touch nothing
//!
//!   --hotkey TOKEN     the binding, default "fn" (e.g. right_option, ctrl+shift+space, f13)
//!   --cycles N         stop after N releases, default 3
//!   --text TEXT        what to insert, default "Inkwell input check."
//! ```

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("input_check runs on macOS only.");
}

#[cfg(target_os = "macos")]
fn main() {
    mac::main();
}

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::process::exit;
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use dispatch2::DispatchQueue;
    use ink_core::{
        Clock, EventSink, FocusReader, HotkeyBinding, HotkeyEvent, HotkeySource, InsertOutcome,
        TextInserter,
    };
    use ink_platform_mac::{MacClock, MacFocusReader, MacHotkeySource, MacTextInserter};
    use objc2_app_kit::NSPasteboard;
    use objc2_core_foundation::CFRunLoop;
    use objc2_core_graphics::CGPreflightPostEventAccess;

    const USAGE: &str = "usage: input_check [--timed SECONDS | --probe] [--hotkey TOKEN] \
                         [--cycles N] [--text TEXT]";

    enum Mode {
        Hotkey,
        Timed(u64),
        Probe,
    }

    struct Options {
        mode: Mode,
        hotkey: String,
        cycles: u32,
        text: String,
    }

    // SAFETY: `Boolean AXIsProcessTrusted(void)` from `HIServices/AXUIElement.h`.
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        /// The non-prompting trust check.
        safe fn AXIsProcessTrusted() -> u8;
    }

    pub fn main() {
        let options = match parse(std::env::args().skip(1)) {
            Ok(options) => options,
            Err(message) => {
                eprintln!("{message}\n{USAGE}");
                exit(2);
            }
        };
        // As in the app: the pasteboard promise and the pasteboard writes are served on the main
        // run loop, so the checks run on a worker while the main thread runs its loop.
        thread::spawn(move || exit(run(&options)));
        loop {
            CFRunLoop::run();
        }
    }

    fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
        let mut options = Options {
            mode: Mode::Hotkey,
            hotkey: "fn".into(),
            cycles: 3,
            text: "Inkwell input check.".into(),
        };
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
            match arg.as_str() {
                "--timed" => {
                    let seconds = value("--timed")?;
                    options.mode =
                        Mode::Timed(seconds.parse().map_err(|_| "--timed takes seconds")?);
                }
                "--probe" => options.mode = Mode::Probe,
                "--hotkey" => options.hotkey = value("--hotkey")?,
                "--cycles" => {
                    options.cycles = value("--cycles")?
                        .parse()
                        .map_err(|_| "--cycles takes a number")?;
                }
                "--text" => options.text = value("--text")?,
                "-h" | "--help" => return Err("input_check: the Mac input checklist binary".into()),
                other => return Err(format!("unknown argument {other:?}")),
            }
        }
        Ok(options)
    }

    fn run(options: &Options) -> i32 {
        let Ok(clock) = MacClock::new() else {
            eprintln!("FAIL clock: mach_timebase_info failed");
            return 1;
        };
        probe(&clock);
        match options.mode {
            Mode::Probe => 0,
            Mode::Timed(seconds) => timed(options, seconds),
            Mode::Hotkey => hotkey(options, clock),
        }
    }

    fn probe(clock: &MacClock) {
        let yes_no = |b: bool| if b { "granted" } else { "MISSING" };
        println!(
            "permissions (checked without prompting): accessibility {}, post events {}",
            yes_no(AXIsProcessTrusted() != 0),
            yes_no(CGPreflightPostEventAccess()),
        );
        let a = clock.now_ns();
        thread::sleep(Duration::from_millis(100));
        let elapsed_ms = (clock.now_ns() - a) as f64 / 1e6;
        println!(
            "clock: now_ns {a}, unix_ms {}, a 100 ms sleep measured {elapsed_ms:.1} ms",
            clock.unix_ms()
        );
        let (tx, rx) = mpsc::channel();
        DispatchQueue::main().exec_async(move || {
            let _ = tx.send(());
        });
        let serving = rx.recv_timeout(Duration::from_secs(1)).is_ok();
        println!(
            "main run loop: {}",
            if serving {
                "serving (the pasteboard promise can be answered)"
            } else {
                "NOT serving: insertion will fall back or fail"
            }
        );
    }

    fn timed(options: &Options, seconds: u64) -> i32 {
        println!("inserting in {seconds} s: click into the target field now");
        thread::sleep(Duration::from_secs(seconds));
        report_focus(&MacFocusReader::new());
        insert(&options.text)
    }

    fn hotkey(options: &Options, clock: MacClock) -> i32 {
        let source = MacHotkeySource::new(clock);
        let (tx, rx) = mpsc::channel();
        let sink: EventSink<HotkeyEvent> = Arc::new(move |event| {
            let _ = tx.send(event);
        });
        if let Err(error) = source.start(&HotkeyBinding(options.hotkey.clone()), sink) {
            eprintln!("FAIL hotkey {:?}: {error}", options.hotkey);
            return 1;
        }
        println!(
            "hotkey {:?}: hold it, release to insert; {} cycles (Ctrl+C quits)",
            options.hotkey, options.cycles
        );
        let focus = MacFocusReader::new();
        let mut pressed_at = None;
        let mut failures = 0;
        let mut done = 0;
        while done < options.cycles {
            let Ok(event) = rx.recv() else {
                eprintln!("FAIL the hotkey sink was dropped");
                return 1;
            };
            // How long the event took to reach this thread, from its own timestamp. A few ms is
            // right; seconds or a huge number means the timestamp is on the wrong timebase.
            let lag_ms = |at_ns: u64| clock.now_ns().saturating_sub(at_ns) as f64 / 1e6;
            match event {
                HotkeyEvent::Pressed { at_ns } => {
                    println!("pressed  (stamped {:.1} ms ago)", lag_ms(at_ns));
                    report_focus(&focus);
                    pressed_at = Some(at_ns);
                }
                HotkeyEvent::Released { at_ns } => {
                    let held = pressed_at
                        .take()
                        .map(|p| at_ns.saturating_sub(p) as f64 / 1e9);
                    println!(
                        "released (stamped {:.1} ms ago), held {}",
                        lag_ms(at_ns),
                        held.map_or("?".into(), |s| format!("{s:.2} s"))
                    );
                    failures += insert(&options.text);
                    done += 1;
                }
                HotkeyEvent::Cancelled => {
                    println!("cancelled (the OS disabled the tap mid-hold, or it was stopped)");
                    pressed_at = None;
                }
                HotkeyEvent::Lost => {
                    println!(
                        "lost: the OS removed the hotkey (Accessibility revoked?); \
                         nothing more arrives until it is started again"
                    );
                    return 1;
                }
            }
        }
        source.stop();
        if source.callback_panics() > 0 {
            println!("FAIL the tap caught {} panics", source.callback_panics());
            return 1;
        }
        i32::from(failures > 0)
    }

    fn report_focus(focus: &MacFocusReader) {
        match focus.focus() {
            Ok(info) => {
                let app = info.app.map_or("none".into(), |app| {
                    format!(
                        "{} (pid {}, \"{}\")",
                        app.id,
                        app.pid.map_or("?".into(), |p| p.to_string()),
                        app.name
                    )
                });
                println!("focus: {app}, secure input {}", info.secure_input);
            }
            Err(error) => println!("focus: error {error}"),
        }
        match focus.selected_text() {
            Ok(Some(text)) => println!("selection: {} characters", text.chars().count()),
            Ok(None) => println!("selection: none (or Accessibility missing)"),
            Err(error) => println!("selection: error {error}"),
        }
    }

    /// Inserts and reports; 1 on failure.
    fn insert(text: &str) -> i32 {
        let before = clipboard_fingerprint();
        let started = Instant::now();
        let outcome = MacTextInserter::new().insert(text);
        let took_ms = started.elapsed().as_secs_f64() * 1e3;
        let after = clipboard_fingerprint();
        println!(
            "clipboard: {} (change count {} -> {})",
            if before.1 == after.1 {
                "same items as before"
            } else {
                "DIFFERENT from before (unless you copied something meanwhile)"
            },
            before.0,
            after.0
        );
        match outcome {
            Ok(InsertOutcome::PastedClipboardNotRestored) => {
                println!(
                    "insert: PastedClipboardNotRestored in {took_ms:.0} ms: the text is in, the \
                     previous clipboard is not (fully) back"
                );
                1
            }
            Ok(outcome) => {
                println!("insert: {outcome:?} in {took_ms:.0} ms");
                0
            }
            Err(error) => {
                println!("insert: FAILED in {took_ms:.0} ms: {error}");
                1
            }
        }
    }

    /// The change count, and a hash of every item's types and bytes. Nothing is printed from
    /// the contents.
    fn clipboard_fingerprint() -> (isize, u64) {
        let pasteboard = NSPasteboard::generalPasteboard();
        let mut hasher = DefaultHasher::new();
        if let Some(items) = pasteboard.pasteboardItems() {
            for item in items.to_vec() {
                for ty in item.types().to_vec() {
                    ty.to_string().hash(&mut hasher);
                    if let Some(data) = item.dataForType(&ty) {
                        data.to_vec().hash(&mut hasher);
                    }
                }
            }
        }
        (pasteboard.changeCount(), hasher.finish())
    }
}
