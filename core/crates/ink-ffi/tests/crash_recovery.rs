//! `kill -9` mid-meeting (S2.8's Verify): a process recording a meeting is killed outright, and the
//! next launch finishes the meeting from what is on disk. At most one chunk of audio may be lost
//! (S1.2a's unit, `CHUNK_DURATION`, 10 s); in practice only what was still in the capture ring.
//!
//! The meeting is a real-time replay into a real SQLite library in a scratch directory, recorded
//! by a child process: this test binary run again with [`CHILD_ENV`] set, so the kill is a real
//! `SIGKILL` (`TerminateProcess` on Windows) of a real core, nothing simulated. The parent then:
//!
//! 1. reads the chunks as the crash left them (the open chunk torn mid-write, as a crash leaves it);
//! 2. starts a core on the same directory and sends `meetings.recover`;
//! 3. checks the audio on disk is exactly the fixture's first samples, and short of what was
//!    captured by at most one chunk (the measured loss is printed);
//! 4. checks the record was ended and superseded by the final pass, and the marker is gone.

mod common;

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use common::*;
use ink_audio::{CHUNK_DURATION, ChunkStore};
use ink_core::{Channel, RecordId};
use ink_engines::{ModelDir, Registry};
use ink_ffi::runtime::{Core, MeetingPlatform, Parts};

/// Set in the child: the scratch directory it records into.
const CHILD_ENV: &str = "INK_CRASH_CHILD_DIR";

/// How long the child records before it is killed: past the first chunk, into the second.
const RECORD_FOR: Duration = Duration::from_millis(12_500);

/// The fixture's length: longer than the child lives.
const FIXTURE_S: f64 = 40.0;

fn parts(dir: &Path) -> Parts {
    let models = ModelDir::new(dir.join("models"));
    let row = test_row(ROW_ID);
    install(&models, &row);
    let loader = MockLoader::new(Behaviour::Say("words from the final pass".into()));
    Parts {
        store: Arc::new(ink_store::SqliteStore::open(dir.join("library.sqlite")).unwrap()),
        clock: clock(),
        registry: Registry::new(vec![row]).unwrap(),
        models,
        loader: loader.clone(),
        installer: Arc::new(MockInstaller {
            generation: loader.generation.clone(),
            gate: None,
            installs: AtomicUsize::new(0),
        }),
        data_dir: dir.to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: MeetingPlatform::default(),
    }
}

/// The child: records a real-time replay into `CHILD_ENV`'s directory, printing every event, until
/// it is killed. Without `CHILD_ENV` (an ordinary `--ignored` run) it returns at once.
#[test]
#[ignore = "the child half of kill_nine_mid_meeting_loses_at_most_one_chunk; that test runs it"]
fn crash_child() {
    let Ok(dir) = std::env::var(CHILD_ENV) else {
        return;
    };
    let dir = Path::new(&dir);
    let out = Box::new(|json: &str| {
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "EVENT {json}");
        let _ = stdout.flush();
    });
    let core = Core::start(parts(dir), out).unwrap();
    core.command(&format!(
        r#"{{"cmd":"replay_meeting","mic":{:?},"far":{:?},"title":"Killed mid-meeting","pacing":"realtime"}}"#,
        dir.join("mic.wav").to_str().unwrap(),
        dir.join("far.wav").to_str().unwrap()
    ))
    .unwrap();
    // Recording, until the parent kills this process.
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Every sample of `channel` on disk, in chunk order.
fn on_disk(chunks: &ChunkStore, channel: Channel) -> Vec<f32> {
    let list = chunks.chunks(channel).unwrap();
    list.chunks
        .iter()
        .flat_map(|c| chunks.read(c).unwrap())
        .collect()
}

/// The fixture's samples as the replay delivers them (16-bit PCM, full scale 2^15).
fn fixture(path: &Path) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).unwrap();
    reader
        .samples::<i32>()
        .map(|s| s.unwrap() as f32 / 32_768.0)
        .collect()
}

#[test]
fn kill_nine_mid_meeting_loses_at_most_one_chunk_and_the_next_launch_finishes_it() {
    let dir = TempDir::new("kill-nine");
    speech_wav(&dir.path().join("mic.wav"), FIXTURE_S, 41);
    speech_wav(&dir.path().join("far.wav"), FIXTURE_S, 42);

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut record = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    while record.is_none() && Instant::now() < deadline {
        let Some(Ok(line)) = lines.next() else { break };
        let Some(json) = line.strip_prefix("EVENT ") else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        if v["type"] == "meeting.started" {
            record = Some(RecordId(v["record"].as_str().unwrap().to_owned()));
        }
    }
    let record = record.expect("the child started its meeting");
    let started = Instant::now();
    std::thread::sleep(RECORD_FOR);
    child.kill().unwrap();
    let recorded_for = started.elapsed();
    let _ = child.wait();
    drop(lines);

    // As the crash left it: the meeting's directory with its marker, and its chunks.
    let meetings: Vec<_> = std::fs::read_dir(dir.path().join("meetings"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(meetings.len(), 1);
    let audio = &meetings[0];
    assert!(
        audio.join(ink_ffi::recovery::LIVE_FILE).is_file(),
        "marked live"
    );

    // The next launch.
    let (core, events) = start_parts(parts(dir.path()));
    let store = core.shared().store.clone();
    let before = store.record(&record).unwrap().expect("the record survived");
    assert_eq!(before.ended_at_unix_ms, None, "the crash never ended it");
    core.command(r#"{"cmd":"meetings.recover","id":"r"}"#)
        .unwrap();
    let recovered = events.wait_type("meeting.recovered", Duration::from_secs(30));
    assert_eq!(recovered["record"], record.0.as_str());
    let finished = events.wait_type("meeting.finished", Duration::from_secs(60));
    assert_eq!(finished["record"], record.0.as_str());
    assert_eq!(finished["revision"], 2, "the final pass ran over the audio");
    events.wait_type("meetings.recovered", Duration::from_secs(30));
    let after = store.record(&record).unwrap().unwrap();
    let ended = after.ended_at_unix_ms.expect("ended");
    assert!(ended > after.started_at_unix_ms);
    assert!(
        store
            .segments(&record)
            .unwrap()
            .iter()
            .any(|s| s.text == "words from the final pass")
    );
    assert!(!audio.join(ink_ffi::recovery::LIVE_FILE).exists(), "done");
    events.assert_valid();
    core.shutdown();

    // The audio: exactly the fixture's first samples, and short of what was captured by at most
    // one chunk.
    let chunks = ChunkStore::open(audio).unwrap();
    let rate = 16_000.0;
    for (channel, file) in [(Channel::Mic, "mic.wav"), (Channel::Far, "far.wav")] {
        let kept = on_disk(&chunks, channel);
        let source = fixture(&dir.path().join(file));
        assert!(!kept.is_empty(), "{channel:?}");
        assert!(
            kept.iter()
                .zip(&source)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{channel:?}: what is on disk is what was captured, sample for sample"
        );
        let kept_s = kept.len() as f64 / rate;
        let lost_s = recorded_for.as_secs_f64() - kept_s;
        eprintln!(
            "kill -9 after {:.2} s: {channel:?} kept {kept_s:.2} s on disk; lost {lost_s:.3} s \
             (the ring and the time to start capture; not a measurement)",
            recorded_for.as_secs_f64()
        );
        assert!(
            lost_s <= CHUNK_DURATION.as_secs_f64(),
            "{channel:?} lost {lost_s:.2} s, more than one chunk"
        );
    }
}
