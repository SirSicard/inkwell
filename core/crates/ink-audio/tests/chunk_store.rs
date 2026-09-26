//! The chunk store: files that are really written, read back exactly, split at gaps, repaired
//! after a crash, and a sample-rate check that reports instead of resampling.

mod common;

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::time::Duration;

use common::{MONO_16K, STEREO_48K, TempDir, at, signal, snapshot};
use ink_audio::chunk::{HEADER_LEN, chunk_file_name, parse_chunk_file_name};
use ink_audio::{
    CHUNK_DURATION, ChunkError, ChunkStore, ChunkWriter, RateVerdict, Repair, WriterSummary,
};
use ink_core::{AudioBlock, Channel, StreamFormat};

const T0: u64 = 7_000_000_000;

/// Writes `frames` of a contiguous stream in `block`-frame blocks with exact host times from `T0`.
/// Returns the samples written.
fn write_stream(
    writer: &mut ChunkWriter,
    format: StreamFormat,
    frames: usize,
    block: usize,
    seed: u64,
) -> Vec<f32> {
    let ch = usize::from(format.channels);
    let audio = signal(frames, format, seed);
    let mut f = 0;
    while f < frames {
        let n = block.min(frames - f);
        writer
            .write(
                &AudioBlock {
                    samples: &audio[f * ch..(f + n) * ch],
                    format,
                    host_time_ns: at(T0, f as u64, format.sample_rate),
                },
                0,
            )
            .unwrap();
        f += n;
    }
    audio
}

fn read_all(store: &ChunkStore, channel: Channel) -> Vec<f32> {
    store
        .chunks(channel)
        .unwrap()
        .iter()
        .flat_map(|c| store.read(c).unwrap())
        .collect()
}

fn file_len(store: &ChunkStore, channel: Channel, index: u64) -> u64 {
    std::fs::metadata(store.dir().join(chunk_file_name(channel, index)))
        .unwrap()
        .len()
}

// --- Naming ------------------------------------------------------------------------------------

#[test]
fn chunk_names_round_trip_for_every_index_including_full_64_bit() {
    assert_eq!(chunk_file_name(Channel::Mic, 0), "mic-000000.pcm");
    assert_eq!(chunk_file_name(Channel::Far, 12), "far-000012.pcm");
    assert_eq!(
        chunk_file_name(Channel::Mic, 1 << 40),
        "mic-1099511627776.pcm",
        "a 64-bit index is formatted in full, never truncated or dropped"
    );
    for channel in [Channel::Mic, Channel::Far] {
        for index in [0, 1, 999_999, 1_000_000, u64::from(u32::MAX) + 1, u64::MAX] {
            let name = chunk_file_name(channel, index);
            assert_eq!(
                parse_chunk_file_name(&name),
                Some((channel, index)),
                "{name}"
            );
        }
    }
    for junk in [
        "mic-.pcm",
        "mic-000001.wav",
        "mic-12x.pcm",
        "mic-+1.pcm",
        "mic--1.pcm",
        "other-000001.pcm",
        "mic-000001.pcm.tmp",
        ".DS_Store",
        "mic-99999999999999999999999.pcm",
    ] {
        assert_eq!(parse_chunk_file_name(junk), None, "{junk}");
    }
}

#[test]
fn writing_actually_creates_the_chunk_files_on_disk() {
    // The regression an earlier implementation had: a badly formatted name made writes produce no
    // file at all, silently. Check the directory itself, not the writer's opinion of it.
    let tmp = TempDir::new("files");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    write_stream(&mut writer, MONO_16K, 25 * 16_000, 320, 1);
    let summary = writer.finish().unwrap();

    let files: Vec<(String, u64)> = snapshot(tmp.path())
        .into_iter()
        .map(|(name, bytes)| (name, bytes.len() as u64))
        .collect();
    let chunk = |seconds: u64| HEADER_LEN as u64 + seconds * 16_000 * 4;
    assert_eq!(
        files,
        vec![
            ("mic-000000.pcm".to_string(), chunk(10)),
            ("mic-000001.pcm".to_string(), chunk(10)),
            ("mic-000002.pcm".to_string(), chunk(5)),
        ]
    );
    assert_eq!(summary.chunks, 3);
    assert_eq!(summary.frames, 25 * 16_000);
    assert_eq!(
        CHUNK_DURATION,
        Duration::from_secs(10),
        "the unit of lose-nothing"
    );
}

// --- Writing and reading -------------------------------------------------------------------

#[test]
fn chunks_read_back_exactly_what_was_written_per_channel() {
    let tmp = TempDir::new("readback");
    let store = ChunkStore::open(tmp.path())
        .unwrap()
        .with_chunk_duration(Duration::from_secs(1));
    let mut mic = store.writer(Channel::Mic, MONO_16K).unwrap();
    let mut far = store.writer(Channel::Far, STEREO_48K).unwrap();
    let mic_audio = write_stream(&mut mic, MONO_16K, 40_000, 320, 2);
    let far_audio = write_stream(&mut far, STEREO_48K, 100_000, 480, 3);
    mic.finish().unwrap();
    far.finish().unwrap();

    assert_eq!(read_all(&store, Channel::Mic), mic_audio);
    assert_eq!(read_all(&store, Channel::Far), far_audio);

    let mic_chunks = store.chunks(Channel::Mic).unwrap();
    let got: Vec<(u64, u64, u64)> = mic_chunks
        .iter()
        .map(|c| (c.index, c.frames, c.host_time_ns))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, 16_000, T0),
            (1, 16_000, at(T0, 16_000, 16_000)),
            (2, 8_000, at(T0, 32_000, 16_000)),
        ]
    );
    for c in &mic_chunks {
        assert_eq!((c.channel, c.format), (Channel::Mic, MONO_16K));
        assert!(!c.after_gap && !c.host_time_estimated);
    }
    let far_chunks = store.chunks(Channel::Far).unwrap();
    assert_eq!(far_chunks.len(), 3);
    assert!(far_chunks.iter().all(|c| c.format == STEREO_48K));
}

#[test]
fn chunks_close_on_block_boundaries_so_every_header_time_is_a_device_timestamp() {
    let tmp = TempDir::new("boundary");
    let store = ChunkStore::open(tmp.path())
        .unwrap()
        .with_chunk_duration(Duration::from_secs(1));
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    // 3000-frame blocks against 16000-frame chunks: a sixth block would not fit, so each chunk
    // closes after five and the next starts on a block the device stamped.
    let audio = write_stream(&mut writer, MONO_16K, 45_000, 3_000, 4);
    writer.finish().unwrap();

    let chunks = store.chunks(Channel::Mic).unwrap();
    let got: Vec<(u64, u64)> = chunks.iter().map(|c| (c.frames, c.host_time_ns)).collect();
    assert_eq!(
        got,
        vec![
            (15_000, T0),
            (15_000, at(T0, 15_000, 16_000)),
            (15_000, at(T0, 30_000, 16_000))
        ]
    );
    assert_eq!(read_all(&store, Channel::Mic), audio);
}

#[test]
fn a_block_longer_than_a_whole_chunk_is_split_across_chunks() {
    let tmp = TempDir::new("long-block");
    let store = ChunkStore::open(tmp.path())
        .unwrap()
        .with_chunk_duration(Duration::from_millis(10));
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    let audio = write_stream(&mut writer, MONO_16K, 500, 500, 15);
    writer.finish().unwrap();

    let chunks = store.chunks(Channel::Mic).unwrap();
    let got: Vec<(u64, u64)> = chunks.iter().map(|c| (c.frames, c.host_time_ns)).collect();
    assert_eq!(
        got,
        vec![
            (160, T0),
            (160, at(T0, 160, 16_000)),
            (160, at(T0, 320, 16_000)),
            (20, at(T0, 480, 16_000))
        ]
    );
    assert_eq!(read_all(&store, Channel::Mic), audio);
}

#[test]
fn a_jump_in_device_time_closes_the_chunk_and_flags_the_next() {
    let tmp = TempDir::new("gap");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Far, MONO_16K).unwrap();
    let before = write_stream(&mut writer, MONO_16K, 32_000, 320, 5);
    // A process tap delivers nothing while nothing plays: the next block is 5 s later.
    let after = signal(320, MONO_16K, 6);
    let resumed = at(T0, 32_000, 16_000) + 5_000_000_000;
    writer
        .write(
            &AudioBlock {
                samples: &after,
                format: MONO_16K,
                host_time_ns: resumed,
            },
            0,
        )
        .unwrap();
    let summary = writer.finish().unwrap();

    let chunks = store.chunks(Channel::Far).unwrap();
    assert_eq!(chunks.len(), 2, "the open chunk closed at the gap");
    assert_eq!((chunks[0].frames, chunks[0].after_gap), (32_000, false));
    assert_eq!(
        (
            chunks[1].frames,
            chunks[1].after_gap,
            chunks[1].host_time_ns
        ),
        (320, true, resumed),
        "the next chunk starts at the true time of its first frame"
    );
    assert_eq!(summary.gaps, 1);
    assert_eq!(summary.lost_frames, 0, "a device gap is not a ring overrun");
    assert_eq!(read_all(&store, Channel::Far), [before, after].concat());
}

#[test]
fn a_ring_overrun_closes_the_chunk_and_flags_the_next() {
    let tmp = TempDir::new("overrun");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    write_stream(&mut writer, MONO_16K, 3_200, 320, 7);
    // The ring dropped two 320-frame blocks; the next block arrives where they would have ended.
    let next = signal(320, MONO_16K, 8);
    writer
        .write(
            &AudioBlock {
                samples: &next,
                format: MONO_16K,
                host_time_ns: at(T0, 3_840, 16_000),
            },
            640,
        )
        .unwrap();
    let summary = writer.finish().unwrap();

    let chunks = store.chunks(Channel::Mic).unwrap();
    assert_eq!(chunks.len(), 2);
    assert!(chunks[1].after_gap);
    assert_eq!(chunks[1].host_time_ns, at(T0, 3_840, 16_000));
    assert_eq!((summary.gaps, summary.lost_frames), (1, 640));
}

#[test]
fn timestamp_jitter_does_not_split_chunks() {
    let tmp = TempDir::new("jitter");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    let audio = signal(320 * 200, MONO_16K, 9);
    for i in 0..200u64 {
        let jitter = [0i64, 700_000, -400_000, 1_000_000][(i % 4) as usize];
        let t = at(T0, i * 320, 16_000).saturating_add_signed(jitter);
        let s = &audio[(i as usize) * 320..(i as usize + 1) * 320];
        writer
            .write(
                &AudioBlock {
                    samples: s,
                    format: MONO_16K,
                    host_time_ns: t,
                },
                0,
            )
            .unwrap();
    }
    let summary = writer.finish().unwrap();
    assert_eq!((summary.chunks, summary.gaps), (1, 0));
}

#[test]
fn a_block_in_another_format_is_refused_and_not_written() {
    let tmp = TempDir::new("format");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    let audio = write_stream(&mut writer, MONO_16K, 1_600, 320, 10);
    let stereo = signal(480, STEREO_48K, 11);
    let err = writer
        .write(
            &AudioBlock {
                samples: &stereo,
                format: STEREO_48K,
                host_time_ns: at(T0, 1_600, 16_000),
            },
            0,
        )
        .unwrap_err();
    assert!(
        matches!(err, ChunkError::FormatChanged { expected, got } if expected == MONO_16K && got == STEREO_48K),
        "{err}"
    );
    writer.finish().unwrap();
    assert_eq!(read_all(&store, Channel::Mic), audio);
}

#[test]
fn an_invalid_format_is_refused() {
    let tmp = TempDir::new("invalid");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let zero = StreamFormat {
        sample_rate: 0,
        channels: 1,
    };
    assert!(matches!(
        store.writer(Channel::Mic, zero),
        Err(ChunkError::InvalidFormat(f)) if f == zero
    ));
}

#[test]
fn a_new_writer_continues_the_sequence_and_never_overwrites() {
    let tmp = TempDir::new("continue");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut first = store.writer(Channel::Mic, MONO_16K).unwrap();
    let a = write_stream(&mut first, MONO_16K, 1_600, 320, 12);
    first.finish().unwrap();
    let before = snapshot(tmp.path());

    // A device switch mid-meeting: a new writer, a new format, the same sequence.
    let mut second = store.writer(Channel::Mic, STEREO_48K).unwrap();
    write_stream(&mut second, STEREO_48K, 4_800, 480, 13);
    second.finish().unwrap();

    let after = snapshot(tmp.path());
    assert_eq!(after[0], before[0], "the first chunk is untouched");
    assert_eq!(after[1].0, "mic-000001.pcm");
    let chunks = store.chunks(Channel::Mic).unwrap();
    assert_eq!(
        chunks.iter().map(|c| c.format).collect::<Vec<_>>(),
        vec![MONO_16K, STEREO_48K]
    );
    assert_eq!(store.read(&chunks[0]).unwrap(), a);
}

#[test]
fn a_writer_dropped_mid_chunk_keeps_everything_it_wrote() {
    // What `kill -9` does: no finish, no sync. Blocks go straight to the file, so the open chunk
    // still holds every block written before the kill.
    let tmp = TempDir::new("killed");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    let audio = write_stream(&mut writer, MONO_16K, 56_000, 320, 14);
    drop(writer);
    assert_eq!(read_all(&store, Channel::Mic), audio);
    assert!(store.recover().unwrap().is_clean());
}

#[test]
fn a_failed_write_is_reported_and_the_next_chunk_follows_a_gap() {
    let tmp = TempDir::new("failed-write");
    let dir = tmp.join("record");
    let store = ChunkStore::open(&dir)
        .unwrap()
        .with_chunk_duration(Duration::from_secs(1));
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    // Exactly one full chunk: it is synced and closed, no file stays open.
    write_stream(&mut writer, MONO_16K, 16_000, 320, 16);
    let audio = signal(640, MONO_16K, 17);
    let block = |i: usize| AudioBlock {
        samples: &audio[i * 320..(i + 1) * 320],
        format: MONO_16K,
        host_time_ns: at(T0, 16_000 + i as u64 * 320, 16_000),
    };

    // The next chunk cannot be created: the block is lost, and the writer says so.
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(matches!(
        writer.write(&block(0), 0),
        Err(ChunkError::Io { op: "create", .. })
    ));
    std::fs::create_dir_all(&dir).unwrap();
    writer.write(&block(1), 0).unwrap();
    let summary = writer.finish().unwrap();

    let chunks = store.chunks(Channel::Mic).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].index, 1, "the failed index is not reused");
    assert!(chunks[0].after_gap, "the lost block is a gap, not silence");
    assert_eq!(summary.gaps, 1);
}

// --- Recovery ------------------------------------------------------------------------------

/// Two and a half one-second chunks of 48 kHz stereo (8-byte frames), writer dropped as in a crash.
fn crashed_store(tmp: &TempDir) -> (ChunkStore, Vec<f32>) {
    let store = ChunkStore::open(tmp.path())
        .unwrap()
        .with_chunk_duration(Duration::from_secs(1));
    let mut writer = store.writer(Channel::Far, STEREO_48K).unwrap();
    let audio = write_stream(&mut writer, STEREO_48K, 120_000, 480, 20);
    drop(writer);
    (store, audio)
}

fn chunk_path(store: &ChunkStore, index: u64) -> std::path::PathBuf {
    store.dir().join(chunk_file_name(Channel::Far, index))
}

#[test]
fn torn_chunk_with_a_partial_frame_is_trimmed_to_whole_frames_and_kept() {
    let tmp = TempDir::new("partial");
    let (store, audio) = crashed_store(&tmp);
    // The crash cut the last frame: 3 of its 8 bytes are missing.
    let last = chunk_path(&store, 2);
    let len = std::fs::metadata(&last).unwrap().len();
    OpenOptions::new()
        .write(true)
        .open(&last)
        .unwrap()
        .set_len(len - 3)
        .unwrap();

    let report = store.recover().unwrap();
    assert_eq!(
        report.repairs,
        vec![Repair::TrimmedPartialFrame {
            channel: Channel::Far,
            index: 2,
            bytes_removed: 5,
        }]
    );
    assert_eq!(file_len(&store, Channel::Far, 2), len - 8);
    let kept = read_all(&store, Channel::Far);
    assert_eq!(kept, audio[..audio.len() - 2], "every whole frame kept");
    assert!(store.recover().unwrap().is_clean(), "idempotent");
}

#[test]
fn torn_chunk_with_a_truncated_header_is_rebuilt_from_the_previous_chunk() {
    let tmp = TempDir::new("truncated");
    let (store, _) = crashed_store(&tmp);
    let before = store.chunks(Channel::Far).unwrap();
    // The crash hit while the chunk's header was being written.
    let last = chunk_path(&store, 2);
    OpenOptions::new()
        .write(true)
        .open(&last)
        .unwrap()
        .set_len(20)
        .unwrap();
    assert!(matches!(
        store.chunks(Channel::Far),
        Err(ChunkError::Unreadable { .. })
    ));
    let intact = snapshot(tmp.path());

    let report = store.recover().unwrap();
    assert_eq!(
        report.repairs,
        vec![Repair::RebuiltHeader {
            channel: Channel::Far,
            index: 2,
            from_index: 1,
            frames_kept: 0,
        }]
    );
    let after = store.chunks(Channel::Far).unwrap();
    assert_eq!(after.len(), 3);
    assert_eq!(after[..2], before[..2]);
    assert_eq!(
        snapshot(tmp.path())[..2],
        intact[..2],
        "earlier chunks untouched"
    );
    let rebuilt = &after[2];
    assert_eq!(rebuilt.format, STEREO_48K);
    assert_eq!(rebuilt.frames, 0);
    assert!(rebuilt.host_time_estimated);
    assert_eq!(rebuilt.host_time_ns, at(T0, 96_000, 48_000));
    assert!(store.recover().unwrap().is_clean(), "idempotent");
}

#[test]
fn torn_chunk_with_a_zeroed_header_keeps_its_audio() {
    // After a power cut a file can have its length but zeros where the header was.
    let tmp = TempDir::new("zeroed");
    let (store, audio) = crashed_store(&tmp);
    let last = chunk_path(&store, 2);
    let mut file = OpenOptions::new().write(true).open(&last).unwrap();
    file.write_all(&[0u8; HEADER_LEN]).unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(&[1, 2, 3]).unwrap(); // and a partial frame
    drop(file);

    let report = store.recover().unwrap();
    assert_eq!(
        report.repairs,
        vec![Repair::RebuiltHeader {
            channel: Channel::Far,
            index: 2,
            from_index: 1,
            frames_kept: 24_000,
        }]
    );
    assert_eq!(read_all(&store, Channel::Far), audio, "no audio lost");
}

#[test]
fn a_torn_first_chunk_is_rebuilt_from_the_next_one() {
    let tmp = TempDir::new("first");
    let (store, audio) = crashed_store(&tmp);
    // Flip one header byte of chunk 0: the checksum no longer matches.
    let first = chunk_path(&store, 0);
    let mut bytes = std::fs::read(&first).unwrap();
    bytes[33] ^= 0xFF;
    std::fs::write(&first, &bytes).unwrap();

    let report = store.recover().unwrap();
    assert_eq!(
        report.repairs,
        vec![Repair::RebuiltHeader {
            channel: Channel::Far,
            index: 0,
            from_index: 1,
            frames_kept: 48_000,
        }]
    );
    let chunks = store.chunks(Channel::Far).unwrap();
    assert_eq!(chunks[0].host_time_ns, T0, "extrapolated back from chunk 1");
    assert!(chunks[0].host_time_estimated);
    assert_eq!(read_all(&store, Channel::Far), audio);
}

#[test]
fn an_unreadable_chunk_with_nothing_to_rebuild_from_is_left_in_place() {
    let tmp = TempDir::new("lone");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    write_stream(&mut writer, MONO_16K, 1_600, 320, 21);
    drop(writer);
    let path = store.dir().join(chunk_file_name(Channel::Mic, 0));
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[..HEADER_LEN].fill(0);
    std::fs::write(&path, &bytes).unwrap();

    let report = store.recover().unwrap();
    assert!(matches!(
        report.repairs.as_slice(),
        [Repair::Unrecoverable {
            channel: Channel::Mic,
            index: 0,
            ..
        }]
    ));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "never deleted or changed"
    );

    // Capture that resumes into this record does not overwrite it.
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    write_stream(&mut writer, MONO_16K, 320, 320, 22);
    writer.finish().unwrap();
    assert!(store.dir().join(chunk_file_name(Channel::Mic, 1)).exists());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn recovery_of_intact_chunks_changes_nothing() {
    let tmp = TempDir::new("intact");
    let (store, _) = crashed_store(&tmp);
    let before = snapshot(tmp.path());
    assert!(store.recover().unwrap().is_clean());
    assert_eq!(snapshot(tmp.path()), before);
}

// --- Sample-rate mismatch ------------------------------------------------------------------

/// Writes `seconds` of blocks that declare `declared` but arrive at `real_hz` frames per second of
/// host time.
fn write_mislabelled(writer: &mut ChunkWriter, declared: StreamFormat, real_hz: u32, seconds: u32) {
    let block = (real_hz / 50) as usize; // 20 ms of real time
    let audio = signal(block, declared, 30);
    for i in 0..u64::from(seconds * 50) {
        writer
            .write(
                &AudioBlock {
                    samples: &audio,
                    format: declared,
                    host_time_ns: T0 + i * 20_000_000,
                },
                0,
            )
            .unwrap();
    }
}

#[test]
fn rate_mismatch_48k_audio_declared_as_16k_is_reported_not_resampled() {
    // The incident: a mic captured at 48 kHz into a stream that declared 16 kHz.
    let tmp = TempDir::new("mismatch-up");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    write_mislabelled(&mut writer, MONO_16K, 48_000, 3);
    let expected = RateVerdict::Mismatch {
        declared_hz: 16_000,
        measured_hz: 48_000,
    };
    assert_eq!(writer.rate(), expected, "visible during the recording");
    let summary = writer.finish().unwrap();
    assert_eq!(summary.rate, expected);
    assert_eq!(
        summary.frames,
        3 * 48_000,
        "every sample kept, none resampled"
    );
    assert_eq!(summary.gaps, 0, "a wrong rate is not a gap");
    let chunks = store.chunks(Channel::Mic).unwrap();
    assert!(
        chunks.iter().all(|c| c.format == MONO_16K),
        "the declared rate stays in the header"
    );
}

#[test]
fn rate_mismatch_16k_audio_declared_as_48k_is_reported() {
    let tmp = TempDir::new("mismatch-down");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let declared = StreamFormat {
        sample_rate: 48_000,
        channels: 1,
    };
    let mut writer = store.writer(Channel::Far, declared).unwrap();
    write_mislabelled(&mut writer, declared, 16_000, 3);
    let summary = writer.finish().unwrap();
    assert_eq!(
        summary.rate,
        RateVerdict::Mismatch {
            declared_hz: 48_000,
            measured_hz: 16_000
        }
    );
    assert_eq!(summary.gaps, 0, "late-looking blocks are not gaps");
}

#[test]
fn rate_mismatch_44k1_against_48k_is_caught() {
    let tmp = TempDir::new("mismatch-441");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let declared = StreamFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    let mut writer = store.writer(Channel::Far, declared).unwrap();
    write_mislabelled(&mut writer, declared, 44_100, 2);
    assert_eq!(
        writer.finish().unwrap().rate,
        RateVerdict::Mismatch {
            declared_hz: 48_000,
            measured_hz: 44_100
        }
    );
}

#[test]
fn a_consistent_stream_with_clock_drift_and_jitter_is_not_a_mismatch() {
    let tmp = TempDir::new("drift");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    let audio = signal(320, MONO_16K, 31);
    // The device clock runs 0.1 % slow against the host clock, with ±0.8 ms of callback jitter.
    for i in 0..500u64 {
        let ideal = T0 + i * 20_020_000;
        let jitter = [0i64, 800_000, -800_000, 300_000][(i % 4) as usize];
        writer
            .write(
                &AudioBlock {
                    samples: &audio,
                    format: MONO_16K,
                    host_time_ns: ideal.saturating_add_signed(jitter),
                },
                0,
            )
            .unwrap();
    }
    let summary = writer.finish().unwrap();
    assert!(
        matches!(summary.rate, RateVerdict::Consistent { measured_hz } if (15_950..=16_000).contains(&measured_hz)),
        "{:?}",
        summary.rate
    );
}

#[test]
fn a_silent_stretch_on_a_tap_is_a_gap_not_a_rate_mismatch() {
    // Delivered duration is far below wall-clock duration here, and that is fine: the tap sent no
    // callbacks for 10 s. Comparing totals would call it a mismatch; comparing runs does not.
    let tmp = TempDir::new("silent");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Far, MONO_16K).unwrap();
    let audio = signal(320, MONO_16K, 32);
    for (offset, blocks) in [(0u64, 75u64), (11_500_000_000, 75)] {
        for i in 0..blocks {
            writer
                .write(
                    &AudioBlock {
                        samples: &audio,
                        format: MONO_16K,
                        host_time_ns: T0 + offset + i * 20_000_000,
                    },
                    0,
                )
                .unwrap();
        }
    }
    let summary = writer.finish().unwrap();
    assert_eq!(summary.gaps, 1);
    assert_eq!(
        summary.rate,
        RateVerdict::Consistent {
            measured_hz: 16_000
        }
    );
}

#[test]
fn rate_is_pending_until_a_second_of_host_time() {
    let tmp = TempDir::new("pending");
    let store = ChunkStore::open(tmp.path()).unwrap();
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    write_stream(&mut writer, MONO_16K, 8_000, 320, 33);
    assert_eq!(writer.rate(), RateVerdict::Pending);
    let WriterSummary { rate, .. } = writer.finish().unwrap();
    assert_eq!(rate, RateVerdict::Pending);
}

#[test]
fn rate_mismatch_is_found_from_the_headers_after_a_crash() {
    // The writer's own verdict dies with the process; the headers are enough to recompute it.
    let tmp = TempDir::new("mismatch-crash");
    let store = ChunkStore::open(tmp.path())
        .unwrap()
        .with_chunk_duration(Duration::from_secs(1));
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    write_mislabelled(&mut writer, MONO_16K, 48_000, 4);
    drop(writer);
    assert_eq!(
        store.rate_check(Channel::Mic).unwrap(),
        RateVerdict::Mismatch {
            declared_hz: 16_000,
            measured_hz: 48_000
        }
    );

    // And a healthy record reads as consistent.
    let tmp = TempDir::new("healthy-crash");
    let store = ChunkStore::open(tmp.path())
        .unwrap()
        .with_chunk_duration(Duration::from_secs(1));
    let mut writer = store.writer(Channel::Mic, MONO_16K).unwrap();
    write_stream(&mut writer, MONO_16K, 64_000, 320, 34);
    drop(writer);
    assert_eq!(
        store.rate_check(Channel::Mic).unwrap(),
        RateVerdict::Consistent {
            measured_hz: 16_000
        }
    );
}
