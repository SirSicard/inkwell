//! A hand-run check of Mac capture: devices and routing, permissions and the System Audio probe,
//! and a live capture of the mic and the far end with meeting detection.
//!
//! Capture needs Microphone and System Audio for the terminal app it runs in, which an agent's
//! shell and CI do not have, so it is a checklist item (`CAPTURE-CHECKLIST.md`, driven by
//! `scripts/mac-tcc-checklist.sh`) rather than a test. It prints levels, counts and verdicts, never
//! audio. Every IOProc callback runs under `assert_no_alloc`, so each capture also checks I4 on
//! real devices.
//!
//! ```text
//! cargo run -p ink-platform-mac --example capture_check -- MODE [OPTIONS]
//!
//!   --devices            inputs, the default output, and which mic would be recorded
//!   --permissions        the four permission states (never prompts)
//!     --probe            also run the System Audio tone probe (prompts if macOS never asked)
//!   --capture SECONDS    record the mic and the far end, with meeting detection
//!     --expect-idle-far  pass only if nothing played (the far end must stay idle)
//!
//!   --headset-mic        the routing setting: record the Bluetooth headset's own mic
//!   --device UID         record this input device instead of the routed one
//!   --app BUNDLE_ID      tap only this app (and its helpers) instead of all output
//! ```

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("capture_check runs on macOS only.");
}

#[cfg(target_os = "macos")]
fn main() {
    std::process::exit(mac::main());
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use assert_no_alloc::{AllocDisabler, assert_no_alloc, violation_count};
    use ink_audio::{CaptureConsumer, RealtimeGuard, capture_ring};
    use ink_core::{
        AppRef, AudioSource, CaptureControl, Channel, DeviceId, EventSink, FarEndTarget,
        MeetingDetector, MeetingSignal, Permission, PermissionProbe, StreamFormat,
    };
    use ink_platform_mac::capture::{CaptureHealth, IoStats, LevelMeter, assess};
    use ink_platform_mac::permissions::tone;
    use ink_platform_mac::{MacCapture, MacClock, MacMeetingDetector, MacPermissionProbe};

    // I4 on real devices: the counting allocator, in warn mode (it counts, never aborts).
    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    const USAGE: &str = "usage: capture_check (--devices | --permissions [--probe] | --capture \
                         SECONDS [--expect-idle-far]) [--headset-mic] [--device UID] [--app ID]";

    enum Mode {
        Devices,
        Permissions { probe: bool },
        Capture { seconds: u64, expect_idle_far: bool },
    }

    struct Options {
        mode: Mode,
        headset_mic: bool,
        device: Option<String>,
        app: Option<String>,
    }

    fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
        let mut mode = None;
        let (mut probe, mut expect_idle_far, mut headset_mic) = (false, false, false);
        let (mut device, mut app) = (None, None);
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
            match arg.as_str() {
                "--devices" => mode = Some(Mode::Devices),
                "--permissions" => mode = Some(Mode::Permissions { probe: false }),
                "--probe" => probe = true,
                "--capture" => {
                    let seconds = value("--capture")?
                        .parse()
                        .map_err(|_| "--capture takes seconds")?;
                    mode = Some(Mode::Capture {
                        seconds,
                        expect_idle_far: false,
                    });
                }
                "--expect-idle-far" => expect_idle_far = true,
                "--headset-mic" => headset_mic = true,
                "--device" => device = Some(value("--device")?),
                "--app" => app = Some(value("--app")?),
                "-h" | "--help" => return Err("capture_check: the Mac capture checklist".into()),
                other => return Err(format!("unknown argument {other:?}")),
            }
        }
        let mode = match mode.ok_or("choose a mode")? {
            Mode::Permissions { .. } => Mode::Permissions { probe },
            Mode::Capture { seconds, .. } => Mode::Capture {
                seconds,
                expect_idle_far,
            },
            other => other,
        };
        Ok(Options {
            mode,
            headset_mic,
            device,
            app,
        })
    }

    pub fn main() -> i32 {
        let options = match parse(std::env::args().skip(1)) {
            Ok(options) => options,
            Err(message) => {
                eprintln!("{message}\n{USAGE}");
                return 2;
            }
        };
        let Ok(clock) = MacClock::new() else {
            println!("FAIL clock: mach_timebase_info failed");
            return 1;
        };
        match options.mode {
            Mode::Devices => devices(clock, &options),
            Mode::Permissions { probe } => permissions(clock, probe),
            Mode::Capture {
                seconds,
                expect_idle_far,
            } => capture(clock, &options, seconds, expect_idle_far),
        }
    }

    fn devices(clock: MacClock, options: &Options) -> i32 {
        let capture = MacCapture::new(clock);
        capture.set_headset_mic(options.headset_mic);
        match capture.input_devices() {
            Ok(inputs) => {
                println!("inputs ({}):", inputs.len());
                for d in &inputs {
                    let default = if d.is_default { " [default]" } else { "" };
                    println!("  {:?} {}{} uid={}", d.transport, d.name, default, d.id.0);
                }
            }
            Err(e) => {
                println!("FAIL inputs: {e}");
                return 1;
            }
        }
        match capture.default_output() {
            Ok(Some(d)) => println!("output: {:?} {} uid={}", d.transport, d.name, d.id.0),
            Ok(None) => println!("output: none"),
            Err(e) => {
                println!("FAIL output: {e}");
                return 1;
            }
        }
        match capture.mic_route() {
            Ok(Some((device, reason))) => {
                println!(
                    "route: record {} ({:?}), because {reason:?} (headset-mic setting {})",
                    device.name, device.transport, options.headset_mic
                );
                0
            }
            Ok(None) => {
                println!("route: no input device");
                1
            }
            Err(e) => {
                println!("FAIL route: {e}");
                1
            }
        }
    }

    fn permissions(clock: MacClock, probe: bool) -> i32 {
        let probe_api = MacPermissionProbe::new(clock);
        for permission in [
            Permission::Microphone,
            Permission::SystemAudio,
            Permission::Accessibility,
            Permission::InputMonitoring,
        ] {
            println!("{permission:?}: {:?}", probe_api.check(permission));
        }
        if !probe {
            println!("(SystemAudio reads NotDetermined until the app has asked; --probe runs it)");
            return 0;
        }
        let started = Instant::now();
        match probe_api.probe_system_audio() {
            Ok(report) => {
                println!(
                    "system audio probe: {:?} -> {:?} in {} ms: {}",
                    report.verdict,
                    report.verdict.permission(),
                    started.elapsed().as_millis(),
                    tone::describe(&report)
                );
                println!(
                    "  tap callbacks {}, output callbacks {}, samples {}, tone share {:.2}, \
                     peak {:.4}",
                    report.tap_callbacks,
                    report.output_callbacks,
                    report.samples,
                    report.tone_fraction,
                    report.peak
                );
                0
            }
            Err(e) => {
                println!("FAIL system audio probe: {e}");
                1
            }
        }
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
                "{:?} {:>6.1} dBFS peak {:.3} zeros {:>5.1}% callbacks {}",
                self.channel,
                self.window.rms_dbfs(),
                self.window.peak(),
                100.0 * self.window.zero_fraction(),
                stats.callbacks
            );
            self.window = LevelMeter::new();
            text
        }

        /// The rate the audio arrived at, by host time, against the declared rate: a stream
        /// labelled with the wrong rate plays too long or too short.
        fn measured_rate(&self) -> Option<f64> {
            let span = self.last_ns.checked_sub(self.first_ns?)?;
            // Only meaningful over a few seconds of continuous audio.
            (span > 3_000_000_000).then(|| self.frames as f64 / (span as f64 / 1e9))
        }
    }

    fn capture(clock: MacClock, options: &Options, seconds: u64, expect_idle_far: bool) -> i32 {
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
        let capture = MacCapture::new(clock).with_realtime_guard(guard);
        capture.set_headset_mic(options.headset_mic);
        if let Ok(Some((device, reason))) = capture.mic_route() {
            println!(
                "route: {} ({:?}) because {reason:?}",
                device.name, device.transport
            );
        }

        let (tx, signals) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: EventSink<MeetingSignal> = Arc::new(move |signal| {
            let _ = tx.lock().map(|tx| tx.send(signal));
        });
        let detector = MacMeetingDetector::new();
        if let Err(e) = detector.start(sink) {
            println!("FAIL detector: {e}");
            return 1;
        }

        let device = options.device.clone().map(DeviceId);
        let mut mic = match capture.open_mic_source(device.as_ref()) {
            Ok(mic) => mic,
            Err(e) => {
                println!("FAIL open mic: {e}");
                return 1;
            }
        };
        let target = match &options.app {
            Some(id) => FarEndTarget::Apps(vec![AppRef {
                id: id.clone(),
                pid: None,
                name: id.clone(),
            }]),
            None => FarEndTarget::AllOutput,
        };
        let mut far = match capture.open_far_end_source(&target) {
            Ok(far) => far,
            Err(e) => {
                println!("FAIL open far end: {e}");
                return 1;
            }
        };
        println!(
            "mic: {} ({:?}) {} Hz x{}; far end: {} Hz x{}{}",
            mic.device_name(),
            mic.transport(),
            mic.format().sample_rate,
            mic.format().channels,
            far.format().sample_rate,
            far.format().channels,
            match &options.app {
                Some(id) => format!(", tapping {id} ({} processes)", far.tapped_processes()),
                None => ", all output except this process".into(),
            }
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
        println!("capturing {seconds} s...");

        let start = Instant::now();
        let mut next_report = Duration::from_secs(10);
        while start.elapsed() < Duration::from_secs(seconds) {
            thread::sleep(Duration::from_millis(50));
            for pump in &mut pumps {
                pump.drain();
            }
            while let Ok(signal) = signals.try_recv() {
                match signal {
                    MeetingSignal::MicInUse { app } => {
                        println!("detect: + {} ({})", app.id, app.name)
                    }
                    MeetingSignal::MicReleased { app } => {
                        println!("detect: - {} ({})", app.id, app.name)
                    }
                }
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

        let mut ok = true;
        for (pump, stats, stopped) in [
            (&pumps[0], mic_stats, mic_stop),
            (&pumps[1], far_stats, far_stop),
        ] {
            let health = assess(pump.channel, stats.callbacks, &pump.total);
            let overruns = pump.consumer.overruns();
            let rate = pump
                .measured_rate()
                .map_or("n/a".into(), |r| format!("{r:.0} Hz"));
            println!(
                "{:?}: {health:?}; {:.1} dBFS, zeros {:.1}%, {} frames, declared {} Hz, measured \
                 {rate}, discontinuities {}, skipped {}, untimed {}, overruns {} blocks",
                pump.channel,
                pump.total.rms_dbfs(),
                100.0 * pump.total.zero_fraction(),
                pump.frames,
                pump.format.sample_rate,
                stats.discontinuities,
                stats.skipped,
                stats.untimed,
                overruns.blocks
            );
            if let Err(e) = stopped {
                println!("FAIL {:?} stop: {e}", pump.channel);
                ok = false;
            }
            let wanted = match (pump.channel, expect_idle_far) {
                (Channel::Far, true) => CaptureHealth::Idle,
                _ => CaptureHealth::Signal,
            };
            if health == wanted {
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
            println!("I4: no IOProc callbacks ran, nothing to check");
        } else if v == 0 {
            println!("PASS I4: 0 allocations in {c} IOProc callbacks");
        } else {
            println!("FAIL I4: {v} allocations in {c} IOProc callbacks");
            ok = false;
        }
        println!(
            "detector: {} read errors, {} callback panics",
            detector.read_errors(),
            detector.callback_panics()
        );
        if ok { 0 } else { 1 }
    }
}
