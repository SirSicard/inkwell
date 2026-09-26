//! The trait contracts, exercised through the mocks. Later crates rely on these behaviours, so a
//! mock that drifts from them fails here first.

use std::sync::{Arc, Mutex};

use ink_core::mock::{MemStore, MockDiarizer, MockEngine, MockLlm, MockPlatform, fixture_hash};
use ink_core::*;

fn assert_send_sync<T: ?Sized + Send + Sync>() {}
fn assert_send<T: ?Sized + Send>() {}

#[test]
fn traits_are_object_safe_with_the_documented_bounds() {
    assert_send_sync::<dyn Store>();
    assert_send_sync::<dyn OfflineEngine>();
    assert_send_sync::<dyn StreamingEngine>();
    assert_send_sync::<dyn Diarizer>();
    assert_send_sync::<dyn Llm>();
    assert_send_sync::<dyn CaptureControl>();
    assert_send_sync::<dyn MeetingDetector>();
    assert_send_sync::<dyn HotkeySource>();
    assert_send_sync::<dyn TextInserter>();
    assert_send_sync::<dyn FocusReader>();
    assert_send_sync::<dyn PermissionProbe>();
    assert_send_sync::<dyn Clock>();
    assert_send_sync::<Platform>();
    assert_send::<dyn AudioSource>();
    assert_send::<dyn AudioSink>();
    assert_send::<dyn EngineStream>();
}

// --- Store ---------------------------------------------------------------------------------

fn seg(channel: Channel, start_ms: u64, text: &str) -> Segment {
    Segment {
        channel,
        start_ms,
        end_ms: start_ms + 1_000,
        text: text.into(),
        speaker: None,
    }
}

fn meeting(store: &MemStore, started: i64) -> RecordId {
    store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: None,
            started_at_unix_ms: started,
            source_app: Some("com.example.meet".into()),
            audio_dir: None,
        })
        .unwrap()
}

#[test]
fn supersede_refuses_empty_and_collapsed_revisions_and_keeps_the_live_one() {
    let store = MemStore::new();
    let id = meeting(&store, 1);
    let live = [
        seg(Channel::Mic, 0, "one two three four"),
        seg(Channel::Far, 1_000, "five six seven eight"),
    ];
    store.append_segments(&id, &live).unwrap();

    assert_eq!(store.supersede(&id, &[]), Err(StoreError::EmptySupersede));
    assert_eq!(
        store.supersede(&id, &[seg(Channel::Mic, 0, "   ")]),
        Err(StoreError::EmptySupersede)
    );
    assert_eq!(
        store.supersede(&id, &[seg(Channel::Mic, 0, "one two three")]),
        Err(StoreError::SuspiciousSupersede {
            channel: Channel::Far,
            previous_words: 4,
            new_words: 0
        })
    );
    assert_eq!(store.segments(&id).unwrap(), live.to_vec());
    assert_eq!(store.record(&id).unwrap().unwrap().revision, 1);

    let offline = [
        seg(Channel::Mic, 0, "one two three four"),
        seg(Channel::Far, 1_000, "five six"),
    ];
    assert_eq!(store.supersede(&id, &offline), Ok(2));
    assert_eq!(store.segments(&id).unwrap(), offline.to_vec());
    assert_eq!(store.record(&id).unwrap().unwrap().revision, 2);
}

/// The guard is per channel: a healthy far end must not pad a mic transcript that came back empty.
#[test]
fn one_channel_collapsing_is_refused_even_when_the_total_passes() {
    let store = MemStore::new();
    let id = meeting(&store, 1);
    let ten = "w w w w w w w w w w";
    store
        .append_segments(
            &id,
            &[seg(Channel::Mic, 0, ten), seg(Channel::Far, 1_000, ten)],
        )
        .unwrap();

    let far_only = [seg(Channel::Far, 1_000, "w w w w w w w w w w w")];
    assert_eq!(
        store.supersede(&id, &far_only),
        Err(StoreError::SuspiciousSupersede {
            channel: Channel::Mic,
            previous_words: 10,
            new_words: 0
        })
    );
    assert_eq!(store.record(&id).unwrap().unwrap().revision, 1);

    // A channel the live pass never heard may appear in the offline pass.
    let other = meeting(&store, 2);
    store
        .append_segments(&other, &[seg(Channel::Mic, 0, ten)])
        .unwrap();
    assert_eq!(
        store.supersede(
            &other,
            &[seg(Channel::Mic, 0, ten), seg(Channel::Far, 9, "hi")]
        ),
        Ok(2)
    );
}

#[test]
fn unknown_records_are_not_found() {
    let store = MemStore::new();
    let ghost = RecordId("nope".into());
    assert_eq!(store.record(&ghost), Ok(None));
    assert_eq!(store.segments(&ghost), Err(StoreError::NotFound));
    assert_eq!(
        store.supersede(&ghost, &[seg(Channel::Mic, 0, "x")]),
        Err(StoreError::NotFound)
    );
    assert_eq!(store.delete_record(&ghost), Err(StoreError::NotFound));
    assert_eq!(store.set_title(&ghost, "x"), Err(StoreError::NotFound));
    assert_eq!(store.add_note(&ghost, 0, "x"), Err(StoreError::NotFound));
}

#[test]
fn records_segments_search_speakers_and_settings() {
    let store = MemStore::new();
    let older = meeting(&store, 100);
    let newer = meeting(&store, 200);
    let dictation = store
        .create_record(NewRecord {
            kind: RecordKind::Dictation,
            title: None,
            started_at_unix_ms: 300,
            source_app: None,
            audio_dir: Some("audio/imported-1".into()),
        })
        .unwrap();
    let ids = |q: &RecordQuery| -> Vec<RecordId> {
        store
            .records(q)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    };
    let all = RecordQuery {
        kind: None,
        started_before_unix_ms: None,
        limit: 10,
    };
    assert_eq!(
        ids(&all),
        vec![dictation.clone(), newer.clone(), older.clone()]
    );
    assert_eq!(
        ids(&RecordQuery {
            limit: 1,
            ..all.clone()
        }),
        vec![dictation.clone()]
    );
    assert_eq!(
        ids(&RecordQuery {
            kind: Some(RecordKind::Meeting),
            ..all.clone()
        }),
        vec![newer.clone(), older.clone()]
    );
    assert_eq!(
        ids(&RecordQuery {
            started_before_unix_ms: Some(200),
            ..all.clone()
        }),
        vec![older.clone()],
        "the cursor is exclusive, so paging never repeats a record"
    );
    assert_eq!(
        store
            .record(&dictation)
            .unwrap()
            .unwrap()
            .audio_dir
            .as_deref(),
        Some("audio/imported-1")
    );

    assert_eq!(store.record(&older).unwrap().unwrap().title, None);
    store.set_title(&older, "Draft review").unwrap();
    assert_eq!(
        store.record(&older).unwrap().unwrap().title.as_deref(),
        Some("Draft review")
    );

    store
        .append_segments(
            &older,
            &[
                seg(Channel::Far, 5_000, "Ship the Draft"),
                seg(Channel::Mic, 1_000, "hello"),
            ],
        )
        .unwrap();
    let starts: Vec<u64> = store
        .segments(&older)
        .unwrap()
        .iter()
        .map(|s| s.start_ms)
        .collect();
    assert_eq!(starts, vec![1_000, 5_000]);

    let hits = store.search("ship", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        (hits[0].record.clone(), hits[0].start_ms),
        (older.clone(), 5_000)
    );
    assert_eq!(hits[0].title.as_deref(), Some("Draft review"));
    assert_eq!(hits[0].started_at_unix_ms, 100);
    assert!(store.search("  ", 10).unwrap().is_empty());

    store.finish_record(&older, 900).unwrap();
    assert_eq!(
        store.record(&older).unwrap().unwrap().ended_at_unix_ms,
        Some(900)
    );

    let spk = SpeakerId("spk0".into());
    store.set_speaker_name(&older, &spk, "Guest").unwrap();
    assert_eq!(
        store.speaker_names(&older).unwrap(),
        vec![(spk, "Guest".to_string())]
    );

    let summary = Summary {
        text: "short".into(),
        model: "mock".into(),
        created_at_unix_ms: 5,
    };
    store.save_summary(&older, &summary).unwrap();
    assert_eq!(store.summary(&older).unwrap(), Some(summary));
    assert_eq!(store.summary(&newer).unwrap(), None);

    assert_eq!(store.setting("hotkey").unwrap(), None);
    store.set_setting("hotkey", "fn").unwrap();
    assert_eq!(store.setting("hotkey").unwrap().as_deref(), Some("fn"));
}

#[test]
fn commitments_merge_complete_and_go_with_their_record() {
    let store = MemStore::new();
    let id = meeting(&store, 1);
    let said = |text: &str, at: u64| NewCommitment {
        text: text.into(),
        owner: Some("Guest".into()),
        due: None,
        due_at_unix_ms: None,
        provenance: vec![Span {
            channel: Channel::Far,
            start_ms: at,
            end_ms: at + 500,
        }],
    };
    let ids = store
        .add_commitments(
            &id,
            &[
                said("send the deck", 1_000),
                said("send over the deck", 9_000),
            ],
        )
        .unwrap();
    assert_eq!(ids.len(), 2);

    store.merge_commitment(&ids[1], &ids[0]).unwrap();
    assert!(matches!(
        store.merge_commitment(&ids[0], &ids[0]),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        store.merge_commitment(&ids[0], &CommitmentId("gone".into())),
        Err(StoreError::NotFound)
    );
    store.set_commitment_done(&ids[0], true).unwrap();

    let all = store.commitments(&id).unwrap();
    assert_eq!(all.len(), 2, "merged commitments are kept for audit");
    assert!(all[0].done);
    assert_eq!(all[1].merged_into.as_ref(), Some(&ids[0]));
    assert_eq!(all[1].provenance[0].start_ms, 9_000);

    store.delete_record(&id).unwrap();
    assert_eq!(
        store.set_commitment_done(&ids[0], false),
        Err(StoreError::NotFound)
    );
}

/// The Owed and Today screens read open commitments across every record, soonest due first.
#[test]
fn open_commitments_span_records_and_skip_done_and_merged() {
    let store = MemStore::new();
    let a = meeting(&store, 1);
    let b = meeting(&store, 2);
    let owe = |text: &str, due_at: Option<i64>| NewCommitment {
        text: text.into(),
        owner: Some("Guest".into()),
        due: due_at.map(|_| "as said".into()),
        due_at_unix_ms: due_at,
        provenance: vec![],
    };
    let in_a = store
        .add_commitments(
            &a,
            &[
                owe("undated", None),
                owe("later", Some(900)),
                owe("done", Some(1)),
            ],
        )
        .unwrap();
    let in_b = store
        .add_commitments(
            &b,
            &[owe("soonest", Some(100)), owe("repeat of later", Some(900))],
        )
        .unwrap();
    store.set_commitment_done(&in_a[2], true).unwrap();
    store.merge_commitment(&in_b[1], &in_a[1]).unwrap();

    let open: Vec<String> = store
        .open_commitments(10)
        .unwrap()
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(open, vec!["soonest", "later", "undated"]);
    assert_eq!(store.open_commitments(1).unwrap().len(), 1);
}

/// Notes carry their own time on the record, for the timestamp chips.
#[test]
fn notes_are_kept_in_time_order_and_go_with_their_record() {
    let store = MemStore::new();
    let id = meeting(&store, 1);
    let late = store.add_note(&id, 9_000, "follow up on pricing").unwrap();
    let early = store.add_note(&id, 1_000, "agenda").unwrap();
    store
        .update_note(&late, "follow up on pricing, Friday")
        .unwrap();

    let notes = store.notes(&id).unwrap();
    assert_eq!(
        notes
            .iter()
            .map(|n| (n.id.clone(), n.at_ms))
            .collect::<Vec<_>>(),
        vec![(early.clone(), 1_000), (late.clone(), 9_000)]
    );
    assert_eq!(notes[1].text, "follow up on pricing, Friday");
    assert_eq!(notes[0].record, id);

    store.delete_note(&early).unwrap();
    assert_eq!(store.delete_note(&early), Err(StoreError::NotFound));
    assert_eq!(store.update_note(&early, "x"), Err(StoreError::NotFound));
    store.delete_record(&id).unwrap();
    assert_eq!(store.update_note(&late, "x"), Err(StoreError::NotFound));
}

// --- Engines -------------------------------------------------------------------------------

/// A 440 Hz sine at `rms_dbfs`, 16 kHz, one second.
fn tone(rms_dbfs: f64) -> Vec<f32> {
    let amplitude = 2f64.sqrt() * 10f64.powf(rms_dbfs / 20.0);
    (0..16_000)
        .map(|n| {
            (amplitude * (2.0 * std::f64::consts::PI * 440.0 * f64::from(n) / 16_000.0).sin())
                as f32
        })
        .collect()
}

fn transcript(text: &str) -> Transcript {
    Transcript {
        segments: vec![TimedText {
            start_ms: 0,
            end_ms: 1_000,
            text: text.into(),
        }],
    }
}

fn options(channel: Channel) -> TranscribeOptions {
    TranscribeOptions {
        channel,
        context: None,
        cancel: CancelToken::new(),
    }
}

#[test]
fn fixture_hash_is_the_documented_fnv1a_over_little_endian_bits() {
    // Cross-checked against an independent implementation (Python, struct.pack('<f')).
    assert_eq!(fixture_hash(&[]), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fixture_hash(&[0.0, 1.0, -0.5, 0.25]), FIXTURE_HASH_GOLDEN);
    assert_ne!(fixture_hash(&[0.0]), fixture_hash(&[-0.0]));
}

const FIXTURE_HASH_GOLDEN: u64 = 0x9ff6_6d3e_055f_db2b;

#[test]
fn mock_engine_answers_fixtures_and_records_what_it_heard() {
    let quiet = tone(-75.0);
    let engine = MockEngine::new("mock-asr", &[Job::DictationFinal])
        .with_fixture(&quiet, transcript("quick brown fox"));

    let out = engine.transcribe(&quiet, &options(Channel::Mic)).unwrap();
    assert_eq!(out.text(), "quick brown fox");

    let calls = engine.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].hash, fixture_hash(&quiet));
    assert_eq!(calls[0].frames, 16_000);
    assert!(
        (calls[0].rms_dbfs + 75.0).abs() < 0.05,
        "{}",
        calls[0].rms_dbfs
    );
    assert_eq!(calls[0].channel, Channel::Mic);
}

#[test]
fn mock_engine_never_answers_unknown_audio_with_an_empty_transcript() {
    let engine = MockEngine::new("mock-asr", &[Job::MeetingFinal]);
    let audio = tone(-20.0);
    let err = engine
        .transcribe(&audio, &options(Channel::Far))
        .unwrap_err();
    let EngineError::Failed(msg) = err else {
        panic!("expected Failed, got {err:?}")
    };
    assert!(
        msg.contains(&format!("{:016x}", fixture_hash(&audio))),
        "{msg}"
    );
}

#[test]
fn cancelled_calls_stop_before_any_work() {
    let engine = MockEngine::new("mock-asr", &[Job::MeetingFinal]);
    let opts = options(Channel::Far);
    opts.cancel.cancel();
    assert_eq!(
        engine.transcribe(&tone(-20.0), &opts),
        Err(EngineError::Cancelled)
    );
    assert!(engine.calls().is_empty());

    let diarizer = MockDiarizer::new(vec![]);
    assert_eq!(
        diarizer.diarize(&[], &opts.cancel),
        Err(EngineError::Cancelled)
    );
    let llm = MockLlm::new(Endpoint::InProcess, "ok");
    let request = LlmRequest {
        system: String::new(),
        user: String::new(),
        max_tokens: 16,
        temperature: 0.0,
        json_schema: None,
    };
    assert_eq!(
        llm.complete(&request, &opts.cancel),
        Err(LlmError::Cancelled)
    );
    assert_eq!(
        llm.complete(&request, &CancelToken::new()).unwrap().text,
        "ok"
    );
    assert_eq!(llm.calls(), 1);
}

#[test]
fn a_stream_reports_partials_then_its_finals() {
    let first = tone(-30.0);
    let second = tone(-31.0);
    let whole: Vec<f32> = first.iter().chain(&second).copied().collect();
    let engine = MockEngine::new("mock-live", &[Job::LivePartials])
        .with_fixture(&whole, transcript("settled text"));

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut stream = engine
        .open_stream(
            Channel::Mic,
            Arc::new(move |e| sink.lock().unwrap().push(e)),
        )
        .unwrap();
    stream.push(&first).unwrap();
    stream.push(&second).unwrap();
    stream.finish().unwrap();

    let events = events.lock().unwrap();
    assert_eq!(events.len(), 3);
    assert!(matches!(events[0], AsrEvent::Partial { .. }));
    assert!(matches!(events[1], AsrEvent::Partial { .. }));
    assert_eq!(
        events[2],
        AsrEvent::Final(transcript("settled text").segments[0].clone())
    );
}

#[test]
fn diarizer_streams_and_offline_calls_return_its_turns() {
    let turns = vec![SpeakerTurn {
        speaker: SpeakerId("spk0".into()),
        start_ms: 0,
        end_ms: 2_000,
    }];
    let diarizer = MockDiarizer::new(turns.clone());
    assert_eq!(diarizer.diarize(&[], &CancelToken::new()).unwrap(), turns);
    assert_eq!(diarizer.calls(), 1);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let mut stream = diarizer
        .open_stream(Arc::new(move |t| sink.lock().unwrap().push(t)))
        .unwrap();
    stream.push(&[0.0; 160]).unwrap();
    stream.finish().unwrap();
    assert_eq!(*seen.lock().unwrap(), turns);
}

// --- Platform ------------------------------------------------------------------------------

/// Records (host time, frames, format) per block, the way the capture ring would see them.
struct Recorder(Arc<Mutex<Vec<(u64, usize, StreamFormat)>>>);

impl AudioSink for Recorder {
    fn push(&mut self, block: &AudioBlock<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((block.host_time_ns, block.frames(), block.format));
    }
}

#[test]
fn capture_delivers_only_between_start_and_stop() {
    let mock = Arc::new(MockPlatform::new());
    let platform = mock.platform();
    let mut far = platform
        .capture
        .open_far_end(&FarEndTarget::AllOutput)
        .unwrap();
    assert_eq!(far.channel(), Channel::Far);
    assert_eq!(mock.far_targets(), vec![FarEndTarget::AllOutput]);

    let stereo = vec![0.1f32; 960];
    assert!(!mock.feed(Channel::Far, &stereo, 1), "not started yet");

    let blocks = Arc::new(Mutex::new(Vec::new()));
    far.start(Box::new(Recorder(blocks.clone()))).unwrap();
    assert!(
        far.start(Box::new(Recorder(blocks.clone()))).is_err(),
        "a second start is refused"
    );
    assert!(mock.feed(Channel::Far, &stereo, 10));
    assert!(mock.feed(Channel::Far, &stereo, 20));
    let stats = far.stop().unwrap();
    assert!(!mock.feed(Channel::Far, &stereo, 30), "stopped");

    assert_eq!(stats.frames, 960);
    assert_eq!(far.stop().unwrap(), SourceStats::default());
    let blocks = blocks.lock().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0], (10, 480, far.format()));
    assert_eq!(far.format().channels, 2);
}

#[test]
fn denied_permissions_fail_the_calls_that_need_them() {
    let mock = Arc::new(MockPlatform::new());
    let p = mock.platform();
    assert_eq!(
        p.permissions.check(Permission::Microphone),
        PermissionState::NotDetermined
    );

    // The hotkey is an active (blocking) event tap, which needs Accessibility; Input Monitoring
    // is only for a listen-only tap, so denying it alone blocks nothing.
    mock.set_permission(Permission::InputMonitoring, PermissionState::Denied);
    assert_eq!(
        p.hotkeys
            .start(&HotkeyBinding("fn".into()), Arc::new(|_| {})),
        Ok(())
    );
    p.hotkeys.stop();

    mock.set_permission(Permission::Microphone, PermissionState::Denied);
    mock.set_permission(Permission::SystemAudio, PermissionState::Denied);
    mock.set_permission(Permission::Accessibility, PermissionState::Denied);
    assert_eq!(
        p.capture.open_mic(None).err(),
        Some(PlatformError::PermissionDenied(Permission::Microphone))
    );
    assert_eq!(
        p.capture.open_far_end(&FarEndTarget::AllOutput).err(),
        Some(PlatformError::PermissionDenied(Permission::SystemAudio))
    );
    assert_eq!(
        p.inserter.insert("x"),
        Err(PlatformError::PermissionDenied(Permission::Accessibility))
    );
    assert_eq!(
        p.hotkeys
            .start(&HotkeyBinding("fn".into()), Arc::new(|_| {})),
        Err(PlatformError::PermissionDenied(Permission::Accessibility))
    );
    assert!(mock.inserted().is_empty());

    p.permissions.request(Permission::Microphone).unwrap();
    assert_eq!(mock.requested(), vec![Permission::Microphone]);
}

#[test]
fn unknown_input_devices_are_refused() {
    let mock = Arc::new(MockPlatform::new());
    let p = mock.platform();
    let devices = p.capture.input_devices().unwrap();
    assert_eq!(devices.len(), 1);
    assert!(p.capture.open_mic(Some(&devices[0].id)).is_ok());
    assert!(matches!(
        p.capture.open_mic(Some(&DeviceId("unplugged".into()))),
        Err(PlatformError::Device(_))
    ));
    assert_eq!(
        p.capture.default_output().unwrap().map(|d| d.transport),
        Some(Transport::BuiltIn)
    );
}

#[test]
fn hotkey_events_carry_host_time_and_stop_when_stopped() {
    let mock = Arc::new(MockPlatform::new());
    let p = mock.platform();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    assert!(matches!(
        p.hotkeys
            .start(&HotkeyBinding(" ".into()), Arc::new(|_| {})),
        Err(PlatformError::Unsupported(_))
    ));
    p.hotkeys
        .start(
            &HotkeyBinding("fn".into()),
            Arc::new(move |e| sink.lock().unwrap().push(e)),
        )
        .unwrap();
    assert_eq!(mock.hotkey_binding(), Some(HotkeyBinding("fn".into())));

    mock.clock().advance_ns(5_000_000);
    assert!(mock.press());
    mock.clock().advance_ns(250_000_000);
    assert!(mock.release());
    assert!(mock.cancel_hotkey());
    p.hotkeys.stop();
    assert!(!mock.press());

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            HotkeyEvent::Pressed { at_ns: 5_000_000 },
            HotkeyEvent::Released { at_ns: 255_000_000 },
            HotkeyEvent::Cancelled,
        ]
    );
    assert_eq!(p.clock.unix_ms(), 255);
}

/// Wall time is derived from host time, so sub-millisecond steps add up instead of rounding away.
#[test]
fn mock_clock_wall_time_follows_host_time_without_drift() {
    let clock = ink_core::mock::MockClock::new(0, 1_000);
    for _ in 0..1_000 {
        clock.advance_ns(999_999);
    }
    assert_eq!(clock.now_ns(), 999_999_000);
    assert_eq!(clock.unix_ms(), 1_000 + 999);
}

#[test]
fn insertion_is_blocked_under_secure_input() {
    let mock = Arc::new(MockPlatform::new());
    let p = mock.platform();
    assert_eq!(p.inserter.insert("hello"), Ok(InsertOutcome::Pasted));
    mock.set_insert_outcome(InsertOutcome::Typed);
    assert_eq!(p.inserter.insert("again"), Ok(InsertOutcome::Typed));

    mock.set_focus(FocusInfo {
        app: Some(AppRef {
            id: "com.example.terminal".into(),
            pid: Some(42),
            name: "Terminal".into(),
        }),
        secure_input: true,
    });
    assert_eq!(p.inserter.insert("secret"), Ok(InsertOutcome::Blocked));
    assert_eq!(
        mock.inserted(),
        vec!["hello".to_string(), "again".to_string()]
    );
    assert_eq!(p.focus.focus().unwrap().app.unwrap().pid, Some(42));

    assert_eq!(p.focus.selected_text().unwrap(), None);
    mock.set_selection(Some("fix this"));
    assert_eq!(
        p.focus.selected_text().unwrap().as_deref(),
        Some("fix this")
    );
}

/// An insertion whose clipboard could not be put back is still an insertion: the text is in, so it
/// is an `Ok` outcome the pipeline never retries, never an error.
#[test]
fn an_insertion_that_could_not_restore_the_clipboard_is_still_an_insertion() {
    let mock = Arc::new(MockPlatform::new());
    let p = mock.platform();
    mock.set_insert_outcome(InsertOutcome::InsertedClipboardNotRestored);
    assert_eq!(
        p.inserter.insert("hello"),
        Ok(InsertOutcome::InsertedClipboardNotRestored)
    );
    assert_eq!(mock.inserted(), vec!["hello".to_string()]);
}

/// `Lost` ends the binding: the OS removed the hotkey, nothing arrives until `start` again.
#[test]
fn a_lost_hotkey_stays_lost_until_started_again() {
    let mock = Arc::new(MockPlatform::new());
    let p = mock.platform();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let on_event: EventSink<HotkeyEvent> = Arc::new(move |e| sink.lock().unwrap().push(e));
    p.hotkeys
        .start(&HotkeyBinding("fn".into()), on_event.clone())
        .unwrap();
    assert!(mock.press());
    assert!(mock.lose_hotkey());
    assert!(!mock.release(), "nothing is bound after a loss");
    assert!(!mock.lose_hotkey());
    assert_eq!(mock.hotkey_binding(), None);

    p.hotkeys
        .start(&HotkeyBinding("fn".into()), on_event)
        .unwrap();
    assert!(mock.press());
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            HotkeyEvent::Pressed { at_ns: 0 },
            HotkeyEvent::Cancelled,
            HotkeyEvent::Lost,
            HotkeyEvent::Pressed { at_ns: 0 },
        ]
    );
}

#[test]
fn meeting_signals_reach_the_detector_until_it_stops() {
    let mock = Arc::new(MockPlatform::new());
    let p = mock.platform();
    let app = AppRef {
        id: "com.example.meet".into(),
        pid: None,
        name: "Meet".into(),
    };
    assert!(!mock.emit_meeting(MeetingSignal::MicInUse { app: app.clone() }));

    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    p.meetings
        .start(Arc::new(move |s| sink.lock().unwrap().push(s)))
        .unwrap();
    assert!(mock.emit_meeting(MeetingSignal::MicInUse { app: app.clone() }));
    p.meetings.stop();
    assert!(!mock.emit_meeting(MeetingSignal::MicReleased { app: app.clone() }));
    assert_eq!(*seen.lock().unwrap(), vec![MeetingSignal::MicInUse { app }]);
}

/// Losing the hotkey mid-hold ends the hold first, exactly as the Mac tap does: `Cancelled`, then
/// `Lost`. Losing it while idle sends `Lost` alone.
#[test]
fn losing_the_hotkey_mid_hold_cancels_the_hold_first() {
    let mock = Arc::new(MockPlatform::new());
    let p = mock.platform();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let record: EventSink<HotkeyEvent> = Arc::new(move |e| sink.lock().unwrap().push(e));
    p.hotkeys
        .start(&HotkeyBinding("fn".into()), record.clone())
        .unwrap();
    assert!(mock.press());
    assert!(mock.lose_hotkey());
    assert_eq!(
        events.lock().unwrap().split_off(1),
        vec![HotkeyEvent::Cancelled, HotkeyEvent::Lost]
    );

    events.lock().unwrap().clear();
    p.hotkeys
        .start(&HotkeyBinding("fn".into()), record)
        .unwrap();
    assert!(mock.press());
    assert!(mock.release());
    assert!(mock.lose_hotkey());
    assert_eq!(events.lock().unwrap().last(), Some(&HotkeyEvent::Lost));
    assert_eq!(
        events.lock().unwrap().len(),
        3,
        "idle loss sends Lost alone"
    );
}
