//! File import end to end on mocks: a file becomes a record through the gain stage and the
//! speech regions a meeting's final pass uses. Inkwell 0.2's import path skipped the gain stage;
//! the step's check is that an import reaches the engine at the gain target.

mod meeting_rig;

use std::path::Path;
use std::sync::Arc;

use ink_audio::gain::{TARGET_PEAK, robust_peak, to_dbfs};
use ink_core::mock::{MemStore, MockClock};
use ink_core::{CancelToken, Channel, RecordKind, RecordQuery, Store, StreamFormat};
use ink_pipeline::events::{VadUnavailable, VoiceDetection};
use ink_pipeline::import::{
    ImportError, ImportServices, ImportSettings, import_samples, import_wav,
};
use ink_pipeline::speech::VadSource;
use meeting_rig::*;

struct Setup {
    services: ImportServices,
    store: Arc<MemStore>,
    engine: Arc<Final>,
    dir: TempDir,
}

fn setup() -> Setup {
    let store = Arc::new(MemStore::new());
    let engine = Final::new(numbered());
    Setup {
        services: ImportServices {
            engine: engine.clone(),
            store: store.clone(),
            clock: Arc::new(MockClock::new(T0_NS, T0_UNIX_MS)),
        },
        store,
        engine,
        dir: TempDir::new("import"),
    }
}

/// Writes 16 kHz mono `signal` to `path` as `channels` × `rate` (a multiple of 16 kHz), in 32-bit
/// float or 16-bit PCM.
fn write_wav(path: &Path, signal: &[f32], rate: u32, channels: u16, float: bool) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: if float { 32 } else { 16 },
        sample_format: if float {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    let device = to_device(
        signal,
        StreamFormat {
            sample_rate: rate,
            channels,
        },
    );
    for s in device {
        if float {
            w.write_sample(s).unwrap();
        } else {
            w.write_sample((s * 32_767.0).round() as i16).unwrap();
        }
    }
    w.finalize().unwrap();
}

#[test]
fn file_import_reaches_the_engine_at_the_gain_target() {
    let s = setup();
    let path = s.dir.path().join("quiet memo.wav");
    let signal = join(&[silence(1.0), speech(3.0, -75.0, 81), silence(1.0)]);
    write_wav(&path, &signal, 48_000, 2, true);
    let outcome = import_wav(
        &path,
        None,
        &s.services,
        &ImportSettings::default(),
        &energy_vad(),
        &CancelToken::new(),
    )
    .unwrap();

    let calls = s.engine.mock.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].rms_dbfs > -30.0, "{:?}", calls[0]);
    assert!(calls[0].peak >= TARGET_PEAK * 0.99, "{:?}", calls[0]);
    let inputs = s.engine.inputs.lock().unwrap();
    let off = to_dbfs(robust_peak(&inputs[0].1)) - to_dbfs(TARGET_PEAK);
    assert!(off.abs() < 1.0, "{off:.2} dB off the target");

    let record = s.store.record(&outcome.record).unwrap().unwrap();
    assert_eq!(record.kind, RecordKind::FileImport);
    assert_eq!(record.title.as_deref(), Some("quiet memo"));
    assert_eq!(record.revision, 1);
    assert_eq!(record.ended_at_unix_ms, Some(T0_UNIX_MS));
    let segments = s.store.segments(&outcome.record).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].channel, Channel::Mic);
    assert!(segments[0].start_ms.abs_diff(750) <= 50, "{segments:?}");
    assert_eq!(outcome.detection, VoiceDetection::Available);
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
}

#[test]
fn an_import_sends_only_its_speech_to_the_engine() {
    let s = setup();
    let path = s.dir.path().join("two remarks.wav");
    let signal = join(&[speech(1.0, -30.0, 82), silence(8.0), speech(1.0, -30.0, 83)]);
    write_wav(&path, &signal, 16_000, 1, false);
    let outcome = import_wav(
        &path,
        Some("Remarks".into()),
        &s.services,
        &ImportSettings::default(),
        &energy_vad(),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(outcome.pass.regions, 2);
    let sent: usize = s
        .engine
        .inputs
        .lock()
        .unwrap()
        .iter()
        .map(|(_, a)| a.len())
        .sum();
    assert!(sent <= 3 * RATE + RATE / 5, "{sent} samples of 10 s");
    let segments = s.store.segments(&outcome.record).unwrap();
    assert_eq!(segments.len(), 2);
    assert!(segments[1].start_ms.abs_diff(8_750) <= 50, "{segments:?}");
    assert_eq!(
        s.store
            .record(&outcome.record)
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Remarks")
    );
}

#[test]
fn an_import_with_no_speech_makes_no_record() {
    let s = setup();
    let path = s.dir.path().join("room tone.wav");
    write_wav(&path, &silence(5.0), 16_000, 1, false);
    let err = import_wav(
        &path,
        None,
        &s.services,
        &ImportSettings::default(),
        &energy_vad(),
        &CancelToken::new(),
    )
    .unwrap_err();
    assert!(matches!(err, ImportError::NothingHeard { .. }), "{err}");
    assert!(
        s.engine.mock.calls().is_empty(),
        "silence reaches no engine"
    );
    let all = s
        .store
        .records(&RecordQuery {
            kind: None,
            before: None,
            limit: 10,
        })
        .unwrap();
    assert!(all.is_empty(), "no empty record in the library");
}

#[test]
fn a_region_that_comes_back_empty_is_reported_by_an_import() {
    let mut s = setup();
    let empty: Answer = Arc::new(|_, _, audio| Ok(words(" ", audio.len())));
    s.engine = Final::new(empty);
    s.services.engine = s.engine.clone();
    let signal = join(&[silence(0.5), speech(2.0, -30.0, 84), silence(0.5)]);
    let err = import_samples(
        &signal,
        StreamFormat::CANONICAL,
        None,
        &s.services,
        &ImportSettings::default(),
        &energy_vad(),
        &CancelToken::new(),
    )
    .unwrap_err();
    let ImportError::NothingHeard { pass, warnings } = err else {
        panic!("{err}")
    };
    assert_eq!((pass.regions, pass.empty_regions), (1, 1));
    assert!(matches!(
        warnings[..],
        [
            ink_pipeline::meeting::events::MeetingWarning::EmptySpeechRegion {
                channel: Channel::Mic,
                ..
            }
        ]
    ));
}

/// Samples the shell decoded: a voice panned hard right is kept (channels are averaged).
#[test]
fn decoded_samples_become_a_record_and_a_one_sided_voice_is_kept() {
    let s = setup();
    let voice = join(&[silence(0.5), speech(2.0, -30.0, 85), silence(0.5)]);
    let stereo: Vec<f32> = voice.iter().flat_map(|&v| [0.0, v]).collect();
    let outcome = import_samples(
        &stereo,
        StreamFormat {
            sample_rate: 16_000,
            channels: 2,
        },
        Some("Panned".into()),
        &s.services,
        &ImportSettings::default(),
        &energy_vad(),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(outcome.pass.regions, 1);
    assert_eq!(s.store.segments(&outcome.record).unwrap().len(), 1);
}

#[test]
fn an_import_without_a_vad_uses_the_fallback_and_says_so() {
    let s = setup();
    let signal = join(&[silence(0.5), speech(2.0, -60.0, 86), silence(0.5)]);
    let outcome = import_samples(
        &signal,
        StreamFormat::CANONICAL,
        None,
        &s.services,
        &ImportSettings::default(),
        &VadSource::Unavailable(VadUnavailable::Downloading),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(
        outcome.detection,
        VoiceDetection::Unavailable(VadUnavailable::Downloading)
    );
    // Still lifted to the target: the fallback levels too.
    let calls = s.engine.mock.calls();
    assert!(calls[0].peak >= TARGET_PEAK * 0.99, "{:?}", calls[0]);
}

#[test]
fn a_vad_that_fails_during_an_import_is_reported_and_the_fallback_finishes() {
    let s = setup();
    let vad = vad_source(|_| {
        Box::new(FailAfter {
            inner: EnergyVad(-50.0),
            windows: 0,
        })
    });
    let signal = join(&[silence(0.5), speech(2.0, -30.0, 87), silence(0.5)]);
    let outcome = import_samples(
        &signal,
        StreamFormat::CANONICAL,
        None,
        &s.services,
        &ImportSettings::default(),
        &vad,
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(
        outcome.detection,
        VoiceDetection::Unavailable(VadUnavailable::Failed)
    );
    assert!(matches!(
        outcome.warnings[..],
        [ink_pipeline::meeting::events::MeetingWarning::VadFailed { .. }]
    ));
}

#[test]
fn a_cancelled_import_makes_no_record() {
    let s = setup();
    let cancel = CancelToken::new();
    cancel.cancel();
    let signal = join(&[silence(0.5), speech(2.0, -30.0, 88), silence(0.5)]);
    let err = import_samples(
        &signal,
        StreamFormat::CANONICAL,
        None,
        &s.services,
        &ImportSettings::default(),
        &energy_vad(),
        &cancel,
    )
    .unwrap_err();
    assert!(matches!(err, ImportError::Cancelled), "{err}");
    assert!(s.engine.mock.calls().is_empty());
}

#[test]
fn a_file_that_is_not_audio_is_refused_by_name() {
    let s = setup();
    let path = s.dir.path().join("notes.wav");
    std::fs::write(&path, b"not a wav file").unwrap();
    let err = import_wav(
        &path,
        None,
        &s.services,
        &ImportSettings::default(),
        &energy_vad(),
        &CancelToken::new(),
    )
    .unwrap_err();
    let ImportError::Read(msg) = &err else {
        panic!("{err}")
    };
    assert!(msg.starts_with("notes.wav"), "{msg}");
    assert!(
        !msg.contains(&*s.dir.path().to_string_lossy()),
        "no full path: {msg}"
    );
}
