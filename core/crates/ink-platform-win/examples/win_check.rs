//! A hand-run check of the Windows platform layer: devices and routing, permissions, a live capture
//! of the mic and the far end with meeting detection, the hotkey, insertion and focus.
//!
//! Capture from a call, the hook and typing into other apps need the maintainer's own desktop
//! session, which an agent's SSH session and CI do not have, so they are a checklist
//! (`windows/S3.1-CHECKLIST.md`) rather than tests. It prints levels, counts and verdicts, never
//! audio and never text it did not type itself. Every capture wake runs under `assert_no_alloc`,
//! so each capture also checks I4 on real devices.
//!
//! ```text
//! cargo run -p ink-platform-win --example win_check -- MODE [OPTIONS]
//!
//!   --devices              inputs and outputs, the mic that would be recorded, the far-end plan
//!   --permissions          the four permission states (never prompts)
//!   --capture SECONDS      record the mic and the far end, with meeting detection
//!     --expect-idle-far    pass only if nothing played (the far end must stay idle)
//!     --app EXE            capture this app's far end (Zoom.exe, ms-teams.exe...) instead of all
//!                          output; its pid is looked up, as the detector would report it
//!     --device ID          record this input endpoint instead of the routed one
//!     --headset-mic        the routing setting: a classic Bluetooth headset's own mic
//!   --detect SECONDS       meeting detection only
//!   --hotkey TOKEN SECONDS print the hotkey's presses and releases (default token: right_control)
//!   --insert TEXT          after --delay seconds, insert TEXT into whatever has focus
//!     --delay SECONDS      time to click into the target first (default 5)
//!   --focus                after --delay seconds, print the focused app and the selection's length
//! ```

#[cfg(not(windows))]
fn main() {
    eprintln!("win_check runs on Windows only.");
}

#[cfg(windows)]
fn main() {
    std::process::exit(win::main());
}

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use assert_no_alloc::{AllocDisabler, assert_no_alloc, violation_count};
    use ink_audio::{
        CaptureConsumer, CaptureHealth, LevelMeter, RealtimeGuard, assess, capture_ring,
    };
    use ink_core::{
        AppRef, AudioSource, Channel, Clock, DeviceId, EventSink, FarEndTarget, FocusReader,
        HotkeyBinding, HotkeyEvent, HotkeySource, MeetingDetector, MeetingSignal, Permission,
        PermissionProbe, StreamFormat, TextInserter,
    };
    use ink_platform_win::capture::IoStats;
    use ink_platform_win::hotkey::DEFAULT_BINDING;
    use ink_platform_win::{
        WinCapture, WinClock, WinFocusReader, WinHotkeySource, WinMeetingDetector,
        WinPermissionProbe, WinTextInserter,
    };

    // I4 on real devices: the counting allocator, in warn mode (it counts, never aborts).
    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    const USAGE: &str = "usage: win_check (--devices | --permissions | --capture SECONDS \
                         [--expect-idle-far] [--app EXE] [--device ID] [--headset-mic] | --detect \
                         SECONDS | --hotkey TOKEN SECONDS | --insert TEXT [--delay S] | --focus \
                         [--delay S])";

    enum Mode {
        Devices,
        Permissions,
        Capture { seconds: u64 },
        Detect { seconds: u64 },
        Hotkey { token: String, seconds: u64 },
        Insert { text: String },
        Focus,
    }

    struct Options {
        mode: Mode,
        expect_idle_far: bool,
        app: Option<String>,
        device: Option<String>,
        headset_mic: bool,
        delay: u64,
    }

    fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
        let mut mode = None;
        let mut options = Options {
            mode: Mode::Devices,
            expect_idle_far: false,
            app: None,
            device: None,
            headset_mic: false,
            delay: 5,
        };
        let seconds =
            |v: String, name: &str| v.parse().map_err(|_| format!("{name} takes seconds"));
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
            match arg.as_str() {
                "--devices" => mode = Some(Mode::Devices),
                "--permissions" => mode = Some(Mode::Permissions),
                "--capture" => {
                    mode = Some(Mode::Capture {
                        seconds: seconds(value("--capture")?, "--capture")?,
                    })
                }
                "--detect" => {
                    mode = Some(Mode::Detect {
                        seconds: seconds(value("--detect")?, "--detect")?,
                    })
                }
                "--hotkey" => {
                    let token = value("--hotkey")?;
                    let secs = seconds(value("--hotkey")?, "--hotkey")?;
                    mode = Some(Mode::Hotkey {
                        token,
                        seconds: secs,
                    });
                }
                "--insert" => {
                    mode = Some(Mode::Insert {
                        text: value("--insert")?,
                    })
                }
                "--focus" => mode = Some(Mode::Focus),
                "--expect-idle-far" => options.expect_idle_far = true,
                "--app" => options.app = Some(value("--app")?),
                "--device" => options.device = Some(value("--device")?),
                "--headset-mic" => options.headset_mic = true,
                "--delay" => options.delay = seconds(value("--delay")?, "--delay")?,
                "-h" | "--help" => return Err("win_check: the Windows platform checklist".into()),
                other => return Err(format!("unknown argument {other:?}")),
            }
        }
        options.mode = mode.ok_or("choose a mode")?;
        Ok(options)
    }

    pub fn main() -> i32 {
        let options = match parse(std::env::args().skip(1)) {
            Ok(options) => options,
            Err(message) => {
                eprintln!("{message}\n{USAGE}");
                return 2;
            }
        };
        let Ok(clock) = WinClock::new() else {
            println!("FAIL clock: no performance counter");
            return 1;
        };
        match &options.mode {
            Mode::Devices => devices(clock, &options),
            Mode::Permissions => permissions(),
            Mode::Capture { seconds } => capture(clock, &options, *seconds),
            Mode::Detect { seconds } => detect(*seconds),
            Mode::Hotkey { token, seconds } => hotkey(clock, token, *seconds),
            Mode::Insert { text } => insert(text, options.delay),
            Mode::Focus => focus(options.delay),
        }
    }

    fn target(app: Option<&String>) -> FarEndTarget {
        match app {
            Some(exe) => FarEndTarget::Apps(vec![AppRef {
                id: exe.clone(),
                pid: None,
                name: exe.clone(),
            }]),
            None => FarEndTarget::AllOutput,
        }
    }

    fn devices(clock: WinClock, options: &Options) -> i32 {
        let capture = WinCapture::new(clock);
        capture.set_headset_mic(options.headset_mic);
        for (what, list) in [
            ("inputs", capture.input_endpoints()),
            ("outputs", capture.output_endpoints()),
        ] {
            match list {
                Ok(list) => {
                    println!("{what} ({}):", list.len());
                    for e in &list {
                        let default = if e.info.is_default { " [default]" } else { "" };
                        println!(
                            "  {:?} {}{default} rate={} container={} id={}",
                            e.info.transport,
                            e.info.name,
                            e.rate.map_or("?".into(), |r| r.to_string()),
                            e.container.as_deref().unwrap_or("?"),
                            e.info.id.0
                        );
                    }
                }
                Err(e) => {
                    println!("FAIL {what}: {e}");
                    return 1;
                }
            }
        }
        match capture.mic_route() {
            Ok(Some((device, reason))) => println!(
                "route: record {} ({:?}), because {reason:?} (headset-mic setting {})",
                device.info.name, device.info.transport, options.headset_mic
            ),
            Ok(None) => println!("route: no input device"),
            Err(e) => {
                println!("FAIL route: {e}");
                return 1;
            }
        }
        match capture.far_end_plan(&target(options.app.as_ref())) {
            Ok(plan) => println!("far end: {plan:?}"),
            Err(e) => println!("far end: {e}"),
        }
        0
    }

    fn permissions() -> i32 {
        let probe = WinPermissionProbe::new();
        for permission in [
            Permission::Microphone,
            Permission::SystemAudio,
            Permission::Accessibility,
            Permission::InputMonitoring,
        ] {
            println!("{permission:?}: {:?}", probe.check(permission));
        }
        0
    }

    /// One channel's pump side: its ring, and the meters for the current 10 s and the whole run.
    struct Pump {
        channel: Channel,
        format: StreamFormat,
        consumer: CaptureConsumer,
        window: LevelMeter,
        total: LevelMeter,
        first_ns: Option<u64>,
        last_ns: u64,
        frames: u64,
    }

    impl Pump {
        fn new(channel: Channel, format: StreamFormat, consumer: CaptureConsumer) -> Self {
            Self {
                channel,
                format,
                consumer,
                window: LevelMeter::new(),
                total: LevelMeter::new(),
                first_ns: None,
                last_ns: 0,
                frames: 0,
            }
        }

        fn drain(&mut self) {
            while let Some(captured) = self.consumer.pop() {
                let block = captured.block;
                self.window.add(block.samples);
                self.total.add(block.samples);
                self.first_ns.get_or_insert(block.host_time_ns);
                self.frames += block.frames() as u64;
                let block_ns = block.frames() as u64 * 1_000_000_000
                    / u64::from(block.format.sample_rate.max(1));
                self.last_ns = block.host_time_ns + block_ns;
            }
        }

        fn line(&mut self, stats: IoStats) -> String {
            let text = format!(
                "{:?} {:>6.1} dBFS peak {:.3} zeros {:>5.1}% wakes {} silent packets {}",
                self.channel,
                self.window.rms_dbfs(),
                self.window.peak(),
                100.0 * self.window.zero_fraction(),
                stats.callbacks,
                stats.silent_packets
            );
            self.window = LevelMeter::new();
            text
        }

        /// The rate the audio arrived at, by host time, against the declared rate. Only for a
        /// stream that delivers continuously (the mic); loopback pauses while nothing plays.
        fn measured_rate(&self) -> Option<f64> {
            let span = self.last_ns.checked_sub(self.first_ns?)?;
            (span > 3_000_000_000).then(|| self.frames as f64 / (span as f64 / 1e9))
        }
    }

    fn print_signal(signal: MeetingSignal) -> bool {
        match signal {
            MeetingSignal::MicInUse { app } => {
                println!("detect: + {} (pid {:?})", app.id, app.pid);
                false
            }
            MeetingSignal::MicReleased { app } => {
                println!("detect: - {} (pid {:?})", app.id, app.pid);
                false
            }
            MeetingSignal::Lost { reason } => {
                println!("FAIL detect: lost: {reason}");
                true
            }
        }
    }

    fn start_detector() -> Option<(WinMeetingDetector, mpsc::Receiver<MeetingSignal>)> {
        let (tx, signals) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: EventSink<MeetingSignal> = Arc::new(move |signal| {
            let _ = tx.lock().map(|tx| tx.send(signal));
        });
        let detector = WinMeetingDetector::new();
        if let Err(e) = detector.start(sink) {
            println!("FAIL detector: {e}");
            return None;
        }
        Some((detector, signals))
    }

    fn capture(clock: WinClock, options: &Options, seconds: u64) -> i32 {
        let violations = Arc::new(AtomicU64::new(0));
        let calls = Arc::new(AtomicU64::new(0));
        let guard: RealtimeGuard = {
            let (v, c) = (violations.clone(), calls.clone());
            Arc::new(move |work: &mut dyn FnMut()| {
                let before = violation_count();
                assert_no_alloc(work);
                v.fetch_add(u64::from(violation_count() - before), Ordering::Relaxed);
                c.fetch_add(1, Ordering::Relaxed);
            })
        };
        let capture = WinCapture::new(clock).with_realtime_guard(guard);
        capture.set_headset_mic(options.headset_mic);
        if let Ok(Some((device, reason))) = capture.mic_route() {
            println!(
                "route: {} ({:?}) because {reason:?}",
                device.info.name, device.info.transport
            );
        }
        let Some((detector, signals)) = start_detector() else {
            return 1;
        };
        let device = options.device.clone().map(DeviceId);
        let mut mic = match capture.open_mic_source(device.as_ref()) {
            Ok(mic) => mic,
            Err(e) => {
                println!("FAIL open mic: {e}");
                return 1;
            }
        };
        let target = target(options.app.as_ref());
        match capture.far_end_plan(&target) {
            Ok(plan) => println!("far end plan: {plan:?}"),
            Err(e) => println!("far end plan: {e}"),
        }
        let mut far = match capture.open_far_end_source(&target) {
            Ok(far) => far,
            Err(e) => {
                println!("FAIL open far end: {e}");
                return 1;
            }
        };
        println!(
            "mic: {} ({}) {} Hz x{}; far end: {} ({}) {} Hz x{}",
            mic.device_name(),
            mic.mode(),
            mic.format().sample_rate,
            mic.format().channels,
            far.device_name(),
            far.mode(),
            far.format().sample_rate,
            far.format().channels,
        );
        let ring = |format| capture_ring(format, Duration::from_secs(2));
        let (Ok((mic_tx, mic_rx)), Ok((far_tx, far_rx))) = (ring(mic.format()), ring(far.format()))
        else {
            println!("FAIL rings");
            return 1;
        };
        let mut pumps = [
            Pump::new(Channel::Mic, mic.format(), mic_rx),
            Pump::new(Channel::Far, far.format(), far_rx),
        ];
        if let Err(e) = mic.start(Box::new(mic_tx)) {
            println!("FAIL start mic: {e}");
            return 1;
        }
        if let Err(e) = far.start(Box::new(far_tx)) {
            println!("FAIL start far end: {e}");
            return 1;
        }
        println!(
            "capturing {seconds} s (mic MMCSS {}, far MMCSS {})...",
            mic.stats().mmcss,
            far.stats().mmcss
        );
        let start = Instant::now();
        let mut next_report = Duration::from_secs(10);
        let mut detection_lost = false;
        while start.elapsed() < Duration::from_secs(seconds) {
            thread::sleep(Duration::from_millis(50));
            for pump in &mut pumps {
                pump.drain();
            }
            while let Ok(signal) = signals.try_recv() {
                detection_lost |= print_signal(signal);
            }
            if start.elapsed() >= next_report {
                let t = next_report.as_secs();
                let mic_line = pumps[0].line(mic.stats());
                let far_line = pumps[1].line(far.stats());
                println!("t={t:>3}s  {mic_line}  |  {far_line}");
                next_report += Duration::from_secs(10);
            }
        }
        let (mic_stats, far_stats) = (mic.stats(), far.stats());
        let mic_stop = mic.stop();
        let far_stop = far.stop();
        detector.stop();
        for pump in &mut pumps {
            pump.drain();
        }
        let mut ok = !detection_lost;
        for (pump, stats, stopped) in [
            (&pumps[0], mic_stats, mic_stop),
            (&pumps[1], far_stats, far_stop),
        ] {
            let health = assess(pump.channel, stats.callbacks, &pump.total);
            let rate = match pump.channel {
                Channel::Mic => pump
                    .measured_rate()
                    .map_or("n/a".into(), |r| format!("{r:.0} Hz")),
                Channel::Far => "n/a (loopback pauses when idle)".into(),
            };
            println!(
                "{:?}: {health:?}; {:.1} dBFS, zeros {:.1}%, {} frames, declared {} Hz, measured \
                 {rate}, packets {}, silent packets {}, discontinuities {}, skipped {}, untimed \
                 {}, overruns {} blocks",
                pump.channel,
                pump.total.rms_dbfs(),
                100.0 * pump.total.zero_fraction(),
                pump.frames,
                pump.format.sample_rate,
                stats.packets,
                stats.silent_packets,
                stats.discontinuities,
                stats.skipped,
                stats.untimed,
                pump.consumer.overruns().blocks,
            );
            if let Err(e) = stopped {
                println!("FAIL {:?} stop: {e}", pump.channel);
                ok = false;
            }
            let wanted = match (pump.channel, options.expect_idle_far) {
                (Channel::Far, true) => CaptureHealth::Idle,
                _ => CaptureHealth::Signal,
            };
            // Loopback with nothing playing may deliver silent packets rather than none: both are
            // an idle far end.
            let idle_far = pump.channel == Channel::Far
                && wanted == CaptureHealth::Idle
                && health == CaptureHealth::DigitalSilence
                && stats.silent_packets == stats.packets;
            if health == wanted || idle_far {
                println!("PASS {:?}: {health:?}", pump.channel);
            } else {
                println!("FAIL {:?}: {health:?}, wanted {wanted:?}", pump.channel);
                ok = false;
            }
        }
        let (v, c) = (
            violations.load(Ordering::Relaxed),
            calls.load(Ordering::Relaxed),
        );
        if c == 0 {
            println!("I4: no capture wakes ran, nothing to check");
        } else if v == 0 {
            println!("PASS I4: 0 allocations in {c} capture wakes");
        } else {
            println!("FAIL I4: {v} allocations in {c} capture wakes");
            ok = false;
        }
        println!(
            "detector: {} read errors, {} callback panics",
            detector.read_errors(),
            detector.callback_panics()
        );
        if ok { 0 } else { 1 }
    }

    fn detect(seconds: u64) -> i32 {
        let Some((detector, signals)) = start_detector() else {
            return 1;
        };
        println!("watching for {seconds} s...");
        let start = Instant::now();
        let mut lost = false;
        while start.elapsed() < Duration::from_secs(seconds) {
            if let Ok(signal) = signals.recv_timeout(Duration::from_millis(200)) {
                lost |= print_signal(signal);
            }
        }
        detector.stop();
        println!(
            "detector: {} read errors, {} callback panics",
            detector.read_errors(),
            detector.callback_panics()
        );
        if lost { 1 } else { 0 }
    }

    fn hotkey(clock: WinClock, token: &str, seconds: u64) -> i32 {
        let token = if token.is_empty() {
            DEFAULT_BINDING
        } else {
            token
        };
        let (tx, events) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: EventSink<HotkeyEvent> = Arc::new(move |event| {
            let _ = tx.lock().map(|tx| tx.send(event));
        });
        let source = WinHotkeySource::new(clock);
        if let Err(e) = source.start(&HotkeyBinding(token.into()), sink) {
            println!("FAIL hotkey {token}: {e}");
            return 1;
        }
        println!("hold {token} for {seconds} s; presses and releases print with their lag");
        let start = Instant::now();
        let mut pressed_at = None;
        while start.elapsed() < Duration::from_secs(seconds) {
            let Ok(event) = events.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            let lag_ms = |at: u64| clock.now_ns().saturating_sub(at) as f64 / 1e6;
            match event {
                HotkeyEvent::Pressed { at_ns } => {
                    pressed_at = Some(at_ns);
                    println!("pressed (reported {:.1} ms after the key)", lag_ms(at_ns));
                }
                HotkeyEvent::Released { at_ns } => {
                    let held = pressed_at.map_or(0.0, |p| at_ns.saturating_sub(p) as f64 / 1e6);
                    println!("released after {held:.0} ms held");
                }
                other => println!("{other:?}"),
            }
        }
        source.stop();
        println!(
            "stopped; callback panics {}, hook reinstalls {}",
            source.callback_panics(),
            source.hook_reinstalls()
        );
        0
    }

    fn insert(text: &str, delay: u64) -> i32 {
        println!("click into the target within {delay} s...");
        thread::sleep(Duration::from_secs(delay));
        let focus = WinFocusReader::new().focus();
        match &focus {
            Ok(info) => println!(
                "target: {} (blocked: {})",
                info.app.as_ref().map_or("none", |a| a.id.as_str()),
                info.secure_input
            ),
            Err(e) => println!("focus: {e}"),
        }
        let started = Instant::now();
        let outcome = WinTextInserter::new().insert(text);
        println!(
            "insert: {outcome:?} in {} ms",
            started.elapsed().as_millis()
        );
        if outcome.is_ok() { 0 } else { 1 }
    }

    fn focus(delay: u64) -> i32 {
        println!("click into the target (and select some text) within {delay} s...");
        thread::sleep(Duration::from_secs(delay));
        let reader = WinFocusReader::new();
        match reader.focus() {
            Ok(info) => println!(
                "focus: {} pid {:?} (blocked: {})",
                info.app.as_ref().map_or("none", |a| a.id.as_str()),
                info.app.as_ref().and_then(|a| a.pid),
                info.secure_input
            ),
            Err(e) => println!("FAIL focus: {e}"),
        }
        // The selection's length only: the checklist never prints what the user selected.
        match reader.selected_text() {
            Ok(Some(text)) => println!("selection: {} characters", text.chars().count()),
            Ok(None) => println!("selection: none"),
            Err(e) => {
                println!("FAIL selection: {e}");
                return 1;
            }
        }
        0
    }
}
