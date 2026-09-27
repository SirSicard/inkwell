//! Seeds an empty data directory with a small library for looking at the screens: meetings made
//! by replaying the AMI fixture through the meeting chain (capture rings, chunks on disk, the final
//! pass, diarization, the summary and commitments), a file import of the same audio, and a few
//! dictations through the dictation worker.
//!
//! ```text
//! cargo run -p ink-ffi --example seed_library -- <empty data directory>
//! INK_DATA_DIR=<that directory> <the app>
//! ```
//!
//! `SEED_UTC_OFFSET_MINUTES` (default 0) is the offset spoken deadlines ("by Friday") resolve in:
//! set it to the Mac's, or a Friday's end lands on Saturday there. `SEED_FAR_SILENT=1` adds a
//! newest meeting whose far end kept nothing (the mic side only), for Today's needs-you banner.
//!
//! CI has no models, and neither does this: the engine, the diarizer and the language model are
//! scripted, so every word in the library is synthetic. The audio, its speech regions, the chunks,
//! the timeline and everything the chain does with them are real. It refuses a directory that
//! already holds a library, so it can never write into someone's.

#[path = "../tests/common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{MockInstaller, ROW_ID, install, start_parts, test_row};
use ink_audio::{ChunkStore, FileReplaySource, SpeechProbability, capture_ring};
use ink_core::mock::{MockClock, MockDiarizer, MockPlatform};
use ink_core::{
    AudioSource, CancelToken, Channel, Clock, EngineError, EngineInfo, Job, Llm, LlmError, LlmInfo,
    LlmRequest, LlmResponse, OfflineEngine, SpeakerId, Store, StreamFormat, TimedText,
    TranscribeOptions, Transcript,
};
use ink_engines::{EngineRow, Loader, ModelDir, Registry};
use ink_ffi::dictation::{Block, DictationInbox};
use ink_ffi::library::write_timeline;
use ink_ffi::runtime::{Core, DictationParts, Model, Parts};
use ink_pipeline::capture::SideCapture;
use ink_pipeline::chain::DictationSettings;
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::gain_stage::Vad;
use ink_pipeline::import::{ImportServices, ImportSettings, import_wav};
use ink_pipeline::meeting::{MeetingChain, MeetingServices, MeetingSettings, MeetingStart};
use ink_pipeline::speech::VadSource;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// One scripted meeting: what each side "says" per speech region, in turn, and the summary the
/// scripted model writes.
struct Script {
    mic: &'static [&'static str],
    far: &'static [&'static str],
    headline: &'static str,
    body: &'static str,
    /// (words in the line it cites, the decision)
    decisions: &'static [(&'static str, &'static str)],
    /// (words in the line it cites, the action, its owner, its deadline)
    actions: &'static [(
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
    )],
    questions: &'static [&'static str],
    /// (ms into the meeting, the note)
    notes: &'static [(u64, &'static str)],
}

const LAUNCH: Script = Script {
    mic: &[
        "Thanks for joining. Let's settle the launch date for the reading app.",
        "I'll send the revised plan and the budget sheet by Friday.",
        "Then we move the launch to the fourteenth, after the design review.",
        "I can take the accessibility pass myself this week.",
        "Everything runs on the laptop, nothing leaves it.",
        "Good, that's everything from me.",
    ],
    far: &[
        "The design review needs another week before we can sign off.",
        "Can you share the budget before our Monday call?",
        "We should keep the beta group small, maybe forty people.",
        "Agreed, the fourteenth works for us.",
        "Where do the models run? Security will ask.",
        "On the laptop, then. Nothing is uploaded.",
    ],
    headline: "Launch moves to the 14th, after the design review",
    body: "### Launch date\n\nThe design review needs **another week**, so the launch moves to the \
           *fourteenth*.\n\n### Beta\n\n- Keep the beta group small, about forty people\n- \
           Security wants it in writing that the models run on the laptop\n\n1. Revised plan \
           first\n2. Then the budget sheet",
    decisions: &[
        (
            "move the launch to the fourteenth",
            "Move the launch to the fourteenth",
        ),
        (
            "keep the beta group small",
            "Keep the beta group to about forty",
        ),
    ],
    actions: &[(
        "send the revised plan",
        "Send the revised plan and the budget sheet",
        "You",
        Some("Friday"),
    )],
    questions: &["Who signs off the design review?"],
    notes: &[
        (4_000, "Launch date: design review first"),
        (15_000, "Beta: small group"),
        (24_000, "Send plan + budget"),
    ],
};

const PRICING: Script = Script {
    mic: &[
        "Let's go through the pricing options for the second year.",
        "I'll draft the per-seat proposal before Tuesday.",
        "A flat fee is simpler, but per seat scales with them.",
        "Six months sounds fair for the review.",
        "I'll check the numbers with finance first.",
        "Thanks, talk on Tuesday.",
    ],
    far: &[
        "Per seat works if there is a cap in the first year.",
        "Finance will want the numbers in writing.",
        "We can revisit the cap after six months.",
        "The cap matters more to us than the rate.",
        "Send it to the whole team, please.",
        "Talk then.",
    ],
    headline: "Per-seat pricing with a first-year cap",
    body: "They prefer **per seat** with a cap in year one.\n\n- Finance wants the numbers in \
           writing\n- The cap can be revisited after six months",
    decisions: &[(
        "per seat works if there is a cap",
        "Per-seat pricing with a first-year cap",
    )],
    actions: &[(
        "draft the per-seat proposal",
        "Draft the per-seat proposal",
        "You",
        Some("Tuesday"),
    )],
    questions: &[],
    notes: &[(6_000, "Pricing: per seat + cap")],
};

const STANDUP: Script = Script {
    mic: &[
        "Quick one today. The importer is done and reviewed.",
        "I'm on the search screen next.",
        "The date sort was wrong in the old list, that's fixed.",
        "No blockers from me.",
        "Same time tomorrow.",
    ],
    far: &[
        "The build was green overnight.",
        "I'll look at the flaky test after lunch.",
        "The player needs another pass on seeking.",
        "Nothing blocking here either.",
        "Design review is Thursday.",
        "See you tomorrow.",
    ],
    headline: "Standup: importer done, search screen next",
    body: "- The importer is done and reviewed\n- The overnight build was green\n- Search \
           screen next",
    decisions: &[],
    actions: &[(
        "look at the flaky test",
        "Look at the flaky test",
        "Them",
        None,
    )],
    questions: &[],
    notes: &[],
};

const DICTATIONS: &[&str] = &[
    "Remind me to book the room for the design review on Thursday.",
    "Draft reply: thanks, the fourteenth works for us, and I'll send the plan by Friday.",
    "Note for later: the beta group should stay around forty people.",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/ami")
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Speech wherever a window is above `0` dBFS RMS: the headsets' room tone and crosstalk sit at
/// −40 to −45, their speech above −35 (as the fixture's own test finds).
struct EnergyVad(f32);

impl SpeechProbability for EnergyVad {
    fn reset(&mut self) {}
    fn probability(&mut self, window: &[f32; 512]) -> Result<f32, EngineError> {
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / 512.0).sqrt();
        Ok(if 20.0 * rms.max(1e-9).log10() > self.0 {
            1.0
        } else {
            0.0
        })
    }
}

/// The final-pass engine: each side's lines in turn, one per speech region.
struct Lines {
    script: [&'static [&'static str]; 2],
    calls: Mutex<[usize; 2]>,
}

impl OfflineEngine for Lines {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: "scripted-lines".into(),
            jobs: vec![Job::MeetingFinal],
            licence: "MIT".into(),
        }
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let side = usize::from(options.channel == Channel::Far);
        let n = {
            let mut calls = self.calls.lock().unwrap();
            calls[side] += 1;
            calls[side] - 1
        };
        let lines = self.script[side];
        Ok(Transcript {
            segments: vec![TimedText {
                start_ms: 0,
                end_ms: (audio.len() / 16) as u64,
                text: lines[n % lines.len()].into(),
            }],
        })
    }
}

/// The language model: the script's summary, citing the lines that hold its words; commitment
/// judgements for sentences that promise something; and "not the same" for every dedup pair
/// except identical tasks.
struct Writer {
    script: &'static Script,
}

impl Writer {
    /// `L{n} [..] who: text` lines of a prompt.
    fn lines(user: &str) -> Vec<(usize, String)> {
        user.lines()
            .filter_map(|l| {
                let rest = l.strip_prefix('L')?;
                let (n, rest) = rest.split_once(' ')?;
                let text = rest.split_once(": ")?.1;
                Some((n.parse().ok()?, text.to_lowercase()))
            })
            .collect()
    }

    fn cite(lines: &[(usize, String)], words: &str) -> Option<usize> {
        lines
            .iter()
            .find(|(_, t)| t.contains(&words.to_lowercase()))
            .map(|(n, _)| *n)
    }

    fn summary(&self, user: &str) -> String {
        let lines = Self::lines(user);
        let decisions: Vec<serde_json::Value> = self
            .script
            .decisions
            .iter()
            .filter_map(|(words, text)| {
                let line = Self::cite(&lines, words)?;
                Some(serde_json::json!({"text": text, "line": line, "quote": words}))
            })
            .collect();
        let actions: Vec<serde_json::Value> = self
            .script
            .actions
            .iter()
            .filter_map(|(words, text, owner, due)| {
                let line = Self::cite(&lines, words)?;
                Some(serde_json::json!({
                    "text": text, "owner": owner, "due": due, "line": line, "quote": words
                }))
            })
            .collect();
        serde_json::json!({
            "headline": self.script.headline,
            "body": self.script.body,
            "decisions": decisions,
            "actions": actions,
            "open_questions": self.script.questions,
            "not_found": [],
        })
        .to_string()
    }

    fn judge(user: &str) -> String {
        let sentence = user
            .rsplit_once("## The sentence to classify\n")
            .map_or("", |(_, s)| s.trim().trim_matches('"'));
        let promise = sentence
            .split_once("I'll ")
            .map(|(_, rest)| rest.trim_end_matches('.'));
        match promise {
            Some(rest) => {
                let (task, due) = match rest.rsplit_once(" by ").or(rest.rsplit_once(" before ")) {
                    Some((task, due)) => (task, Some(due)),
                    None => (rest, None),
                };
                let mut task = task.to_owned();
                if let Some(first) = task.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                let quote: String = format!("I'll {rest}")
                    .split_whitespace()
                    .take(4)
                    .collect::<Vec<_>>()
                    .join(" ");
                serde_json::json!({
                    "class": "commitment", "confidence": 0.9, "task": task, "due": due,
                    "quote": quote,
                })
            }
            None => serde_json::json!({
                "class": "option_discussion", "confidence": 0.8, "task": null, "due": null,
                "quote": sentence.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
            }),
        }
        .to_string()
    }

    fn dedup(user: &str) -> String {
        let quoted: Vec<&str> = user.split('"').skip(1).step_by(2).collect();
        let same = quoted.len() == 2
            && quoted[0]
                .to_lowercase()
                .contains(&quoted[1].to_lowercase()[..quoted[1].len().min(12)]);
        serde_json::json!({"same": same, "keep": "B", "why": "the same promise"}).to_string()
    }
}

impl Llm for Writer {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "scripted".into(),
            model: "seed".into(),
            endpoint: ink_core::Endpoint::InProcess,
        }
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        let text = if request.system.contains("meeting record") {
            self.summary(&request.user)
        } else if request.system.contains("classify ONE sentence") {
            Self::judge(&request.user)
        } else {
            Self::dedup(&request.user)
        };
        Ok(LlmResponse { text })
    }
}

/// Replays the fixture as one meeting that started at `started_unix_ms`, into `dir`'s library.
fn meeting(
    dir: &Path,
    store: &Arc<dyn Store>,
    script: &'static Script,
    started_unix_ms: i64,
    names: &[(&str, &str)],
    far: bool,
) -> String {
    const T0_NS: u64 = 5_000_000_000;
    let clock = Arc::new(MockClock::new(T0_NS, started_unix_ms));
    let name = format!("{started_unix_ms}-seed");
    let chunks = ChunkStore::open(dir.join("meetings").join(&name)).expect("chunk directory");
    let diarizer = Arc::new(MockDiarizer::new(
        [
            ("spk0", 0, 10_000),
            ("spk1", 10_000, 20_000),
            ("spk2", 20_000, 30_000),
        ]
        .iter()
        .map(|&(s, a, b)| ink_core::SpeakerTurn {
            speaker: SpeakerId(s.into()),
            start_ms: a,
            end_ms: b,
        })
        .collect(),
    ));
    let vad: VadSource = VadSource::Installed(Arc::new(|| {
        Ok(Box::new(EnergyVad(-35.0)) as Box<dyn SpeechProbability>)
    }));
    let mut chain = MeetingChain::start(
        MeetingServices {
            live: None,
            offline: Arc::new(Lines {
                script: [script.mic, script.far],
                calls: Mutex::default(),
            }),
            diarizer: Some(diarizer),
            store: store.clone(),
            clock: clock.clone(),
            llm: Some(Arc::new(Writer { script })),
        },
        MeetingSettings {
            utc_offset_minutes: std::env::var("SEED_UTC_OFFSET_MINUTES")
                .ok()
                .and_then(|m| m.parse().ok())
                .unwrap_or(0),
            ..MeetingSettings::default()
        },
        vad,
        Arc::new(|_| {}),
        MeetingStart {
            title: None,
            source_app: Some("Zoom".into()),
            audio_dir: Some(format!("meetings/{name}")),
            routing: Default::default(),
        },
    )
    .expect("the meeting starts");
    write_timeline(chunks.dir(), chain.start_ns()).expect("timeline");
    let record = chain.record().clone();
    let mut sides = Vec::new();
    let replayed: &[(Channel, &str)] = if far {
        &[
            (Channel::Mic, "IS1009a-mic.wav"),
            (Channel::Far, "IS1009a-far.wav"),
        ]
    } else {
        &[(Channel::Mic, "IS1009a-mic.wav")]
    };
    for &(channel, file) in replayed {
        let (tx, rx) = capture_ring(StreamFormat::CANONICAL, Duration::from_secs(31)).unwrap();
        let mut source =
            FileReplaySource::open(fixtures().join(file), channel, clock.clone()).unwrap();
        source.start(Box::new(tx)).unwrap();
        source.wait().unwrap();
        sides.push(SideCapture::new(channel, rx, chunks.clone()));
    }
    for side in sides {
        let summary = side.finish(
            &mut |b| chain.push_audio(b.channel, &b.samples, b.host_time_ns, b.dropped_frames),
            &mut |issue| eprintln!("capture: {issue}"),
        );
        chain.capture_ended(summary);
    }
    // The fixture is 30 s long: the meeting ends then.
    clock.advance_ns(30_000_000_000);
    let ended = chain.stop();
    let outcome = ended
        .finalize(&chunks, &CancelToken::new())
        .expect("the final pass");
    assert!(outcome.superseded, "the final pass replaced the live one");
    for (speaker, person) in names {
        store
            .set_speaker_name(&record, &SpeakerId((*speaker).into()), person)
            .unwrap();
    }
    for (at, note) in script.notes {
        store.add_note(&record, *at, note).unwrap();
    }
    record.0
}

/// Loads models that say the dictations in turn, one per take.
struct Dictated(Arc<AtomicUsize>);

impl Loader<Model> for Dictated {
    fn load(&self, row: &EngineRow) -> Result<Model, EngineError> {
        Ok(Box::new(Takes {
            id: row.id.clone(),
            next: self.0.clone(),
        }))
    }
}

struct Takes {
    id: String,
    next: Arc<AtomicUsize>,
}

impl OfflineEngine for Takes {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: self.id.clone(),
            jobs: vec![Job::DictationFinal],
            licence: "MIT".into(),
        }
    }

    fn transcribe(&self, audio: &[f32], _: &TranscribeOptions) -> Result<Transcript, EngineError> {
        let n = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Transcript {
            segments: vec![TimedText {
                start_ms: 0,
                end_ms: (audio.len() / 16) as u64,
                text: DICTATIONS[n % DICTATIONS.len()].into(),
            }],
        })
    }
}

/// A real-time clock whose wall time is the machine's.
struct WallClock(Instant, i64);

impl Clock for WallClock {
    fn now_ns(&self) -> u64 {
        1_000_000_000 + self.0.elapsed().as_nanos() as u64
    }
    fn unix_ms(&self) -> i64 {
        self.1 + self.0.elapsed().as_millis() as i64
    }
}

/// One take through the dictation worker, as the pump and the hotkey deliver it.
fn take(core: &Core, inbox: &DictationInbox, seed: u64) {
    const BLOCK: usize = 160;
    const BLOCK_NS: u64 = 10_000_000;
    let mut t = core.shared().clock.now_ns();
    let hotkey = inbox.hotkey_sink();
    let audio = |samples: &[f32], t: &mut u64| {
        for block in samples.chunks(BLOCK) {
            inbox.push_audio(Block {
                samples: block.to_vec(),
                host_time_ns: *t,
                dropped_frames: 0,
            });
            *t += BLOCK_NS;
        }
    };
    audio(&[0.0; 8_000], &mut t);
    hotkey(ink_core::HotkeyEvent::Pressed { at_ns: t });
    audio(&ink_audio::synth::speech_like(2.0, -30.0, seed), &mut t);
    hotkey(ink_core::HotkeyEvent::Released { at_ns: t });
    audio(&[0.0; 9_600], &mut t);
}

fn dictations(dir: &Path) {
    let installer = Arc::new(MockInstaller {
        generation: Arc::default(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let models = ModelDir::new(dir.join("seed-models"));
    let row = test_row(ROW_ID);
    install(&models, &row);
    let (core, events) = start_parts(Parts {
        store: Arc::new(ink_store::SqliteStore::open(dir.join("library.sqlite")).unwrap()),
        clock: Arc::new(WallClock(Instant::now(), now_unix_ms())),
        registry: Registry::new(vec![row]).unwrap(),
        models,
        loader: Arc::new(Dictated(Arc::default())),
        installer,
        data_dir: dir.to_owned(),
        permissions: None,
    });
    let platform = Arc::new(MockPlatform::new());
    let inbox = core
        .start_dictation(DictationParts {
            inserter: platform.clone(),
            focus: platform.clone(),
            llm: None,
            settings: DictationSettings::default(),
            vad: Vad::Unavailable(VadUnavailable::ModelMissing),
        })
        .unwrap();
    for n in 0..DICTATIONS.len() {
        take(&core, &inbox, n as u64 + 1);
        assert!(
            events.wait_count("dictation.inserted", n + 1, Duration::from_secs(20)),
            "dictation {n}: {:?}",
            events.types()
        );
    }
    core.shutdown();
    let _ = std::fs::remove_dir_all(dir.join("seed-models"));
}

fn main() {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: seed_library <empty data directory>");
    assert!(
        dir.is_absolute(),
        "the data directory must be an absolute path"
    );
    std::fs::create_dir_all(&dir).unwrap();
    assert!(
        !dir.join("library.sqlite").exists(),
        "{} already holds a library; seed an empty directory",
        dir.display()
    );
    let store: Arc<dyn Store> =
        Arc::new(ink_store::SqliteStore::open(dir.join("library.sqlite")).unwrap());
    let now = now_unix_ms();
    // Out of order on purpose: the list must sort by start, not by when a record was written.
    let names = [("spk0", "Alex"), ("spk1", "Robin"), ("spk2", "Sam")];
    let days_ago = |d: i64, hour: i64| now - d * DAY_MS - (now % DAY_MS) + hour * HOUR_MS;
    let standup = meeting(&dir, &store, &STANDUP, days_ago(4, 7), &names, true);
    let launch = meeting(&dir, &store, &LAUNCH, now - 2 * HOUR_MS, &names, true);
    let pricing = meeting(&dir, &store, &PRICING, days_ago(1, 13), &names, true);
    if std::env::var("SEED_FAR_SILENT").as_deref() == Ok("1") {
        meeting(&dir, &store, &STANDUP, now - HOUR_MS, &names, false);
    }
    // A file import of the mic side (S1.5b's import path): titled by its file name.
    let imported = import_wav(
        &fixtures().join("IS1009a-mic.wav"),
        None,
        &ImportServices {
            engine: Arc::new(Lines {
                script: [STANDUP.mic, STANDUP.mic],
                calls: Mutex::default(),
            }),
            store: store.clone(),
            clock: Arc::new(MockClock::new(1_000_000_000, days_ago(2, 9))),
        },
        &ImportSettings::default(),
        &VadSource::Installed(Arc::new(|| {
            Ok(Box::new(EnergyVad(-35.0)) as Box<dyn SpeechProbability>)
        })),
        &CancelToken::new(),
    )
    .expect("the import");
    drop(store);
    dictations(&dir);
    println!(
        "seeded {}: meetings {launch}, {pricing}, {standup}; import {}",
        dir.display(),
        imported.record.0
    );
}
