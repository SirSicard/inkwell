//! Raw PCM chunks on disk: the seam between capture and everything after it (architecture rule 3).
//!
//! # Why raw chunks, not WAV
//!
//! A WAV keeps its length in its header, so a crash mid-recording leaves a file whose audio is all
//! there and which reads as empty. A chunk here has nothing that depends on its length: a 64-byte
//! header written once when the chunk opens, then little-endian `f32` samples appended block by
//! block. Whatever reached the disk is readable, and [`ChunkStore::recover`] trims a torn tail to
//! whole frames instead of discarding the chunk.
//!
//! # The unit of "lose nothing"
//!
//! [`CHUNK_DURATION`] is 10 seconds of audio. Each block is written straight to the file (no
//! userspace buffer), so a killed process loses only what was still in the capture ring. When a
//! chunk closes, its data is synced, and on Unix so is the record directory, so the chunk's name
//! survives with its bytes (a new record directory's own entry is synced when it is created). On
//! Windows std cannot open a directory to sync it, and NTFS journals the entry. So a power cut
//! loses at most the open chunk, on a disk that honours flushes. Ten seconds keeps that loss small
//! while an hour stays 360 files per channel, not thousands.
//!
//! # Layout
//!
//! One directory per record. Each channel writes its own sequence: `mic-000000.pcm`,
//! `mic-000001.pcm`, …, `far-000000.pcm`, …. The index in the name is the order; the header
//! repeats the channel and index and adds the format, the host time of the chunk's first frame,
//! and flags.
//!
//! A chunk holds at most [`CHUNK_DURATION`] and closes on a block boundary: a block that would not
//! fit starts the next chunk (only a block longer than a whole chunk is split). It also closes at a
//! gap in the stream. So a header's host time is the device's own stamp for that frame, never an
//! extrapolation through the declared rate, which may be the thing that is wrong.
//!
//! Header, 64 bytes, all little-endian:
//!
//! | Bytes | Field |
//! |---|---|
//! | 0..8 | magic `INKCHUNK` |
//! | 8..10 | version (1) |
//! | 10..12 | interleaved channels |
//! | 12..16 | sample rate, Hz, as the device declared it |
//! | 16 | stream: 0 mic, 1 far end |
//! | 17 | sample encoding: 1 = `f32` little-endian |
//! | 18..20 | flags: bit 0 after a gap, bit 1 host time estimated by recovery |
//! | 20..24 | reserved, zero |
//! | 24..32 | chunk index |
//! | 32..40 | host time of the first frame, ns |
//! | 40..56 | reserved, zero |
//! | 56..64 | FNV-1a 64 of bytes 0..56 |
//!
//! The checksum tells a header from the zeros or garbage a power cut can leave behind.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use ink_core::{AudioBlock, Channel, StreamFormat};

use crate::rate::{Continuity, RateCheck, RateVerdict, frames_to_ns};

/// Audio per chunk: the unit of "lose nothing" (see the module docs).
pub const CHUNK_DURATION: Duration = Duration::from_secs(10);

/// Bytes before the first sample of a chunk file.
pub const HEADER_LEN: usize = 64;

/// The file extension of a chunk.
pub const CHUNK_EXTENSION: &str = "pcm";

const MAGIC: [u8; 8] = *b"INKCHUNK";
const VERSION: u16 = 1;
const ENCODING_F32_LE: u8 = 1;
const FLAG_AFTER_GAP: u16 = 1;
const FLAG_HOST_TIME_ESTIMATED: u16 = 1 << 1;
const FLAG_FORMAT_ESTIMATED: u16 = 1 << 2;
const SAMPLE_BYTES: u64 = 4;
const HEADER_BYTES: u64 = HEADER_LEN as u64;

/// Why a chunk operation failed. Messages name files, never audio content.
#[derive(Debug)]
#[non_exhaustive]
pub enum ChunkError {
    /// The file system refused.
    Io {
        /// What was being done.
        op: &'static str,
        /// The file or directory name (not the full path).
        file: String,
        /// The underlying error.
        source: io::Error,
    },
    /// A zero sample rate or channel count.
    InvalidFormat(StreamFormat),
    /// A block arrived in a different format than the writer's. Finish this writer and open a new
    /// one: the sequence continues, and each chunk's header states its own format.
    FormatChanged {
        /// The writer's format.
        expected: StreamFormat,
        /// The block's format.
        got: StreamFormat,
    },
    /// A chunk file whose header cannot be read. [`ChunkStore::recover`] repairs it.
    Unreadable {
        /// The file name.
        file: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// Recovery could only guess this chunk's format, so its frames cannot be read as audio
    /// without the caller deciding the format. [`ChunkStore::read_raw_samples`] returns the
    /// samples as stored.
    FormatEstimated {
        /// The file name.
        file: String,
    },
}

impl fmt::Display for ChunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { op, file, source } => write!(f, "chunk store: {op} {file}: {source}"),
            Self::InvalidFormat(format) => write!(
                f,
                "chunk store: invalid format ({} Hz, {} channels)",
                format.sample_rate, format.channels
            ),
            Self::FormatChanged { expected, got } => write!(
                f,
                "chunk store: format changed from {} Hz x{} to {} Hz x{}",
                expected.sample_rate, expected.channels, got.sample_rate, got.channels
            ),
            Self::Unreadable { file, reason } => {
                write!(f, "chunk store: {file} is unreadable: {reason}")
            }
            Self::FormatEstimated { file } => write!(
                f,
                "chunk store: {file} has a format recovery could only estimate"
            ),
        }
    }
}

impl std::error::Error for ChunkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn io_error(op: &'static str, path: &Path) -> impl FnOnce(io::Error) -> ChunkError {
    let file = file_name(path);
    move |source| ChunkError::Io { op, file, source }
}

fn channel_tag(channel: Channel) -> &'static str {
    match channel {
        Channel::Mic => "mic",
        Channel::Far => "far",
    }
}

/// The file name of chunk `index` of `channel`.
///
/// The only place a chunk name is built. Both parts are typed, and the index is formatted by
/// Rust's type-checked formatting: an earlier implementation built names with a C-style `%d` and a
/// 64-bit value, which silently produced no file at all. The host time is not in the name; it lives
/// in the header, where it is checksummed.
pub fn chunk_file_name(channel: Channel, index: u64) -> String {
    // Six digits keep an ordinary recording sorted in a directory listing; order is always taken
    // from the parsed number, so longer indexes still sort correctly here.
    format!("{}-{index:06}.{CHUNK_EXTENSION}", channel_tag(channel))
}

/// The channel and index a chunk file name encodes, or `None` for anything else.
pub fn parse_chunk_file_name(name: &str) -> Option<(Channel, u64)> {
    let stem = name.strip_suffix(CHUNK_EXTENSION)?.strip_suffix('.')?;
    let (tag, digits) = stem.split_once('-')?;
    let channel = match tag {
        "mic" => Channel::Mic,
        "far" => Channel::Far,
        _ => return None,
    };
    // `u64::from_str` also takes a leading '+', which no name we write has.
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((channel, digits.parse().ok()?))
}

/// A chunk header. See the module docs for the byte layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Header {
    channel: Channel,
    index: u64,
    format: StreamFormat,
    host_time_ns: u64,
    flags: u16,
}

impl Header {
    fn frame_bytes(&self) -> u64 {
        u64::from(self.format.channels) * SAMPLE_BYTES
    }

    fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..8].copy_from_slice(&MAGIC);
        b[8..10].copy_from_slice(&VERSION.to_le_bytes());
        b[10..12].copy_from_slice(&self.format.channels.to_le_bytes());
        b[12..16].copy_from_slice(&self.format.sample_rate.to_le_bytes());
        b[16] = match self.channel {
            Channel::Mic => 0,
            Channel::Far => 1,
        };
        b[17] = ENCODING_F32_LE;
        b[18..20].copy_from_slice(&self.flags.to_le_bytes());
        b[24..32].copy_from_slice(&self.index.to_le_bytes());
        b[32..40].copy_from_slice(&self.host_time_ns.to_le_bytes());
        let sum = fnv1a(&b[..56]);
        b[56..64].copy_from_slice(&sum.to_le_bytes());
        b
    }

    fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let b: &[u8; HEADER_LEN] = bytes
            .get(..HEADER_LEN)
            .and_then(|h| h.try_into().ok())
            .ok_or("truncated header")?;
        if b[0..8] != MAGIC {
            return Err("not a chunk header");
        }
        if u64::from_le_bytes(le(&b[56..64])) != fnv1a(&b[..56]) {
            return Err("header checksum mismatch");
        }
        if u16::from_le_bytes(le(&b[8..10])) != VERSION {
            return Err("unsupported chunk version");
        }
        if b[17] != ENCODING_F32_LE {
            return Err("unsupported sample encoding");
        }
        let channel = match b[16] {
            0 => Channel::Mic,
            1 => Channel::Far,
            _ => return Err("unknown stream"),
        };
        let format = StreamFormat {
            sample_rate: u32::from_le_bytes(le(&b[12..16])),
            channels: u16::from_le_bytes(le(&b[10..12])),
        };
        if format.sample_rate == 0 || format.channels == 0 {
            return Err("invalid format");
        }
        Ok(Self {
            channel,
            index: u64::from_le_bytes(le(&b[24..32])),
            format,
            host_time_ns: u64::from_le_bytes(le(&b[32..40])),
            flags: u16::from_le_bytes(le(&b[18..20])),
        })
    }
}

/// A fixed-size little-endian field. Every caller passes a range of the matching length.
fn le<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(bytes);
    out
}

/// FNV-1a, 64-bit: a checksum to recognise a header, not a defence against tampering.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// One chunk as read back from disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkInfo {
    /// Which stream.
    pub channel: Channel,
    /// Position in the channel's sequence.
    pub index: u64,
    /// The format the device declared.
    pub format: StreamFormat,
    /// Host time of the first frame, ns, on the capture clock.
    ///
    /// When [`host_time_estimated`](Self::host_time_estimated) is set this is an extrapolation
    /// that assumes the stream ran on without a gap across the torn chunk. A consumer aligning the
    /// mic with the far end must check the flag.
    pub host_time_ns: u64,
    /// Whole frames on disk. A torn partial frame at the end is not counted.
    pub frames: u64,
    /// The chunk does not continue the previous one: audio was lost between them (a ring overrun
    /// or a jump in device time), or the stream restarted with a new writer.
    pub after_gap: bool,
    /// Recovery rebuilt the header, and the host time is extrapolated from a neighbour.
    pub host_time_estimated: bool,
    /// Recovery rebuilt the header and could only guess the format (the stream may have changed
    /// format at this chunk). `format` and `frames` are that guess: [`ChunkStore::read`] refuses
    /// the chunk and the rate check leaves it out.
    pub format_estimated: bool,
    /// Where it is.
    pub path: PathBuf,
}

/// A chunk file whose header cannot be read, listed so it is never silently skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreadableChunk {
    /// Which stream.
    pub channel: Channel,
    /// Position in the channel's sequence, from the file name.
    pub index: u64,
    /// What is wrong with its header.
    pub reason: &'static str,
    /// Where it is.
    pub path: PathBuf,
}

/// A channel's chunks: the readable ones, and every file whose header cannot be read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChunkList {
    /// Readable chunks, in index order.
    pub chunks: Vec<ChunkInfo>,
    /// Unreadable chunk files, in index order. Their audio is still on disk.
    pub unreadable: Vec<UnreadableChunk>,
}

/// What a writer did, returned by [`ChunkWriter::finish`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriterSummary {
    /// Chunk files written.
    pub chunks: u64,
    /// Frames written.
    pub frames: u64,
    /// Places where the stream did not continue (overruns, jumps in device time, failed writes).
    pub gaps: u64,
    /// Frames the ring reported dropped.
    pub lost_frames: u64,
    /// The sample-rate check. A [`RateVerdict::Mismatch`] must reach the user.
    pub rate: RateVerdict,
}

/// One repair [`ChunkStore::recover`] made, or could not make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Repair {
    /// A torn partial frame was cut from the end; every whole frame was kept.
    TrimmedPartialFrame {
        /// Which stream.
        channel: Channel,
        /// Which chunk.
        index: u64,
        /// Bytes removed (less than one frame).
        bytes_removed: u64,
    },
    /// The header was missing, truncated or corrupt, and was rebuilt from a neighbouring chunk of
    /// the same channel. The host time is marked estimated.
    ///
    /// The format is certain only when both adjacent chunks survived, agree, and the next one
    /// continues this one without a gap; then a torn partial frame is trimmed. Otherwise the format
    /// is marked estimated and not a byte after the header is touched.
    RebuiltHeader {
        /// Which stream.
        channel: Channel,
        /// Which chunk.
        index: u64,
        /// The chunk whose format and timing it was rebuilt from.
        from_index: u64,
        /// Whether the format is a guess.
        format_estimated: bool,
        /// Bytes of audio after the header, all kept.
        bytes_kept: u64,
    },
    /// Nothing to rebuild from (no readable chunk in the channel). The file is left exactly as it
    /// was, never deleted.
    Unrecoverable {
        /// Which stream.
        channel: Channel,
        /// Which chunk.
        index: u64,
        /// Why.
        reason: &'static str,
    },
}

/// What [`ChunkStore::recover`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Every repair, per channel in index order.
    pub repairs: Vec<Repair>,
}

impl RecoveryReport {
    /// Whether every chunk was already intact.
    pub fn is_clean(&self) -> bool {
        self.repairs.is_empty()
    }
}

/// A chunk file as found on disk, before anything is trusted.
struct Found {
    index: u64,
    path: PathBuf,
    len: u64,
    header: Result<Header, &'static str>,
    /// Rebuilt by this recovery pass: readable now, but not evidence about its neighbours.
    rebuilt: bool,
}

impl Found {
    fn inspect(channel: Channel, index: u64, path: PathBuf) -> Result<Self, ChunkError> {
        let mut file = File::open(&path).map_err(io_error("open", &path))?;
        let len = file.metadata().map_err(io_error("stat", &path))?.len();
        let mut head = Vec::with_capacity(HEADER_LEN);
        (&mut file)
            .take(HEADER_BYTES)
            .read_to_end(&mut head)
            .map_err(io_error("read", &path))?;
        let header = Header::decode(&head).and_then(|h| {
            if h.channel == channel && h.index == index {
                Ok(h)
            } else {
                Err("header names another chunk")
            }
        });
        Ok(Self {
            index,
            path,
            len,
            header,
            rebuilt: false,
        })
    }

    /// The header as the writer left it: readable, not rebuilt, nothing estimated.
    fn original(&self) -> Option<Header> {
        self.header.ok().filter(|h| {
            !self.rebuilt && h.flags & (FLAG_HOST_TIME_ESTIMATED | FLAG_FORMAT_ESTIMATED) == 0
        })
    }

    /// Whole frames after the header, by `header`'s frame size.
    fn whole_frames(&self, header: &Header) -> u64 {
        self.len.saturating_sub(HEADER_BYTES) / header.frame_bytes()
    }
}

/// The chunks of one record.
#[derive(Clone, Debug)]
pub struct ChunkStore {
    dir: PathBuf,
    chunk_duration: Duration,
}

impl ChunkStore {
    /// Opens the record directory `dir`, creating it if needed.
    ///
    /// **Worker.** After a crash, call [`recover`](Self::recover) before opening writers.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, ChunkError> {
        let dir = dir.into();
        let existed = dir.is_dir();
        fs::create_dir_all(&dir).map_err(io_error("create directory", &dir))?;
        if !existed {
            // A new record's own entry must survive a power cut too, or its chunks are unreachable.
            if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
                sync_dir(parent)?;
            }
        }
        Ok(Self {
            dir,
            chunk_duration: CHUNK_DURATION,
        })
    }

    /// Overrides [`CHUNK_DURATION`] for writers opened from now on. For tests and measurements;
    /// the product uses the default.
    pub fn with_chunk_duration(mut self, duration: Duration) -> Self {
        self.chunk_duration = duration;
        self
    }

    /// The record directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Chunk files of `channel` by index, whatever their state. Other files are not ours and are
    /// ignored.
    fn list(&self, channel: Channel) -> Result<Vec<(u64, PathBuf)>, ChunkError> {
        let mut found = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(io_error("list", &self.dir))? {
            let entry = entry.map_err(io_error("list", &self.dir))?;
            let Some((c, index)) = entry.file_name().to_str().and_then(parse_chunk_file_name)
            else {
                continue;
            };
            let is_file = entry
                .file_type()
                .map_err(io_error("stat", &entry.path()))?
                .is_file();
            if c == channel && is_file {
                found.push((index, entry.path()));
            }
        }
        found.sort_unstable_by_key(|(index, _)| *index);
        Ok(found)
    }

    /// A writer for `channel`, continuing after any chunks already on disk.
    ///
    /// **Pump.** One writer per channel at a time.
    pub fn writer(
        &self,
        channel: Channel,
        format: StreamFormat,
    ) -> Result<ChunkWriter, ChunkError> {
        if format.sample_rate == 0 || format.channels == 0 {
            return Err(ChunkError::InvalidFormat(format));
        }
        let chunk_frames =
            u128::from(format.sample_rate) * self.chunk_duration.as_nanos() / 1_000_000_000;
        let chunk_frames = u64::try_from(chunk_frames).unwrap_or(u64::MAX).max(1);
        // After every existing file, readable or not: a writer never reuses an index.
        let next_index = self
            .list(channel)?
            .last()
            .map_or(0, |(index, _)| index.saturating_add(1));
        Ok(ChunkWriter {
            dir: self.dir.clone(),
            channel,
            format,
            chunk_frames,
            next_index,
            open: None,
            pending_gap: false,
            resumes_sequence: next_index > 0,
            rate: RateCheck::new(format.sample_rate),
            bytes: Vec::new(),
            summary: WriterSummary {
                chunks: 0,
                frames: 0,
                gaps: 0,
                lost_frames: 0,
                rate: RateVerdict::Pending,
            },
        })
    }

    /// The chunks of `channel`, in order: the readable ones, and every file whose header cannot
    /// be read, so one torn chunk never hides the chunks after it.
    ///
    /// **Worker or pump.** A chunk still being written is listed with its whole frames so far.
    /// [`recover`](Self::recover) repairs unreadable chunks where it can. Only a failure to list
    /// or open files is an error.
    pub fn chunks(&self, channel: Channel) -> Result<ChunkList, ChunkError> {
        let mut list = ChunkList::default();
        for (index, path) in self.list(channel)? {
            let found = Found::inspect(channel, index, path)?;
            let header = match found.header {
                Ok(header) => header,
                Err(reason) => {
                    list.unreadable.push(UnreadableChunk {
                        channel,
                        index,
                        reason,
                        path: found.path,
                    });
                    continue;
                }
            };
            list.chunks.push(ChunkInfo {
                channel,
                index,
                format: header.format,
                host_time_ns: header.host_time_ns,
                frames: found.whole_frames(&header),
                after_gap: header.flags & FLAG_AFTER_GAP != 0,
                host_time_estimated: header.flags & FLAG_HOST_TIME_ESTIMATED != 0,
                format_estimated: header.flags & FLAG_FORMAT_ESTIMATED != 0,
                path: found.path,
            });
        }
        Ok(list)
    }

    /// Every whole sample after the header, as stored, with no frame alignment: for a chunk whose
    /// format recovery could only estimate, where the caller decides what the samples are.
    pub fn read_raw_samples(&self, chunk: &ChunkInfo) -> Result<Vec<f32>, ChunkError> {
        let (_, bytes) = self.load(chunk)?;
        Ok(decode_samples(&bytes[HEADER_LEN..]))
    }

    /// A chunk's whole frames as they are on disk now, interleaved.
    ///
    /// A chunk whose format recovery could only estimate is refused with
    /// [`ChunkError::FormatEstimated`]: reading it as frames would assert a format nobody knows.
    pub fn read(&self, chunk: &ChunkInfo) -> Result<Vec<f32>, ChunkError> {
        let (header, bytes) = self.load(chunk)?;
        if header.flags & FLAG_FORMAT_ESTIMATED != 0 {
            return Err(ChunkError::FormatEstimated {
                file: file_name(&chunk.path),
            });
        }
        let data = &bytes[HEADER_LEN..];
        let whole = data.len() - data.len() % (header.frame_bytes() as usize);
        Ok(decode_samples(&data[..whole]))
    }

    /// The file's bytes and its verified header.
    fn load(&self, chunk: &ChunkInfo) -> Result<(Header, Vec<u8>), ChunkError> {
        let bytes = fs::read(&chunk.path).map_err(io_error("read", &chunk.path))?;
        let unreadable = |reason| ChunkError::Unreadable {
            file: file_name(&chunk.path),
            reason,
        };
        let header = Header::decode(&bytes).map_err(unreadable)?;
        if header.channel != chunk.channel || header.index != chunk.index {
            return Err(unreadable("header names another chunk"));
        }
        Ok((header, bytes))
    }

    /// Repairs what a crash left. Never deletes a file, never trims on a guess, and is idempotent.
    ///
    /// - A readable chunk with a torn partial frame is trimmed to whole frames.
    /// - An unreadable header is rebuilt from a neighbouring chunk. Its format is certain only when
    ///   both adjacent chunks survived, agree, and the next continues this one without a gap; then
    ///   a torn frame is trimmed as above. Otherwise the header says the format is estimated, every
    ///   byte after it is kept, and readers treat the chunk as such (see
    ///   [`ChunkInfo::format_estimated`]).
    /// - With no readable chunk in the channel, the file is left as it is and reported.
    ///
    /// **Worker.** Run it before opening writers on the directory.
    pub fn recover(&self) -> Result<RecoveryReport, ChunkError> {
        let mut report = RecoveryReport::default();
        for channel in [Channel::Mic, Channel::Far] {
            self.recover_channel(channel, &mut report.repairs)?;
        }
        Ok(report)
    }

    fn recover_channel(
        &self,
        channel: Channel,
        repairs: &mut Vec<Repair>,
    ) -> Result<(), ChunkError> {
        let mut found = Vec::new();
        for (index, path) in self.list(channel)? {
            found.push(Found::inspect(channel, index, path)?);
        }

        // Readable chunks first: a torn tail loses its partial frame, nothing more. A chunk whose
        // format is only estimated is never trimmed: its frame size is a guess.
        for f in &mut found {
            let Ok(header) = f.header else { continue };
            if header.flags & FLAG_FORMAT_ESTIMATED != 0 {
                continue;
            }
            let partial = f.len.saturating_sub(HEADER_BYTES) % header.frame_bytes();
            if partial > 0 {
                set_len(&f.path, f.len - partial)?;
                f.len -= partial;
                repairs.push(Repair::TrimmedPartialFrame {
                    channel,
                    index: f.index,
                    bytes_removed: partial,
                });
            }
        }

        // Then unreadable headers, in index order, so a rebuilt chunk can serve the one after it.
        for i in 0..found.len() {
            if found[i].header.is_ok() {
                continue;
            }
            let index = found[i].index;
            let data_bytes = found[i].len.saturating_sub(HEADER_BYTES);

            // Evidence: the adjacent chunks as their writer left them. The writer flags the first
            // chunk after any gap or restart (and a format change always restarts it), so a next
            // chunk without that flag was written by the same writer, straight after this one.
            let adjacent = |j: usize, want: u64| {
                found
                    .get(j)
                    .filter(|f| f.index == want)
                    .and_then(Found::original)
            };
            let previous = i
                .checked_sub(1)
                .and_then(|j| adjacent(j, index.wrapping_sub(1)));
            let continuing =
                adjacent(i + 1, index.wrapping_add(1)).filter(|h| h.flags & FLAG_AFTER_GAP == 0);
            // Certain only when both neighbours survived, agree, and the next one continues this
            // chunk. Anything less is a guess, and a guess never trims a byte.
            let certain =
                matches!((previous, continuing), (Some(p), Some(n)) if p.format == n.format);

            // The source of the format guess and the time estimate: the chunk that continues this
            // one if there is one, else the nearest readable chunk before it, else after it.
            let readable = |f: &Found| f.header.ok().map(|h| (f.index, h, f.whole_frames(&h)));
            let source = match continuing {
                Some(h) => Some(Source::After(index.wrapping_add(1), h)),
                None => found[..i]
                    .iter()
                    .rev()
                    .find_map(readable)
                    .map(|(j, h, frames)| Source::Before(j, h, frames))
                    .or_else(|| {
                        found[i + 1..]
                            .iter()
                            .find_map(readable)
                            .map(|(j, h, _)| Source::After(j, h))
                    }),
            };
            let Some(source) = source else {
                repairs.push(Repair::Unrecoverable {
                    channel,
                    index,
                    reason: "no readable chunk in this channel to rebuild its header from",
                });
                continue;
            };
            let (from_index, format, host_time_ns) = match source {
                // Where the previous chunk ends, if the stream continued without a gap.
                Source::Before(j, h, frames) => (
                    j,
                    h.format,
                    h.host_time_ns
                        .saturating_add(frames_to_ns(frames, h.format.sample_rate)),
                ),
                // Back from the chunk after it, by this chunk's frames in that chunk's format.
                Source::After(j, h) => (
                    j,
                    h.format,
                    h.host_time_ns.saturating_sub(frames_to_ns(
                        data_bytes / h.frame_bytes(),
                        h.format.sample_rate,
                    )),
                ),
            };
            let header = Header {
                channel,
                index,
                format,
                host_time_ns,
                flags: if certain {
                    FLAG_HOST_TIME_ESTIMATED
                } else {
                    FLAG_HOST_TIME_ESTIMATED | FLAG_FORMAT_ESTIMATED
                },
            };
            // A certain format may trim a torn partial frame, as for any readable chunk. A guessed
            // one keeps every byte; a file shorter than a header only grows to hold one.
            let len = if certain {
                HEADER_BYTES + data_bytes / header.frame_bytes() * header.frame_bytes()
            } else {
                found[i].len.max(HEADER_BYTES)
            };
            rewrite_header(&found[i].path, &header, len)?;
            found[i].header = Ok(header);
            found[i].rebuilt = true;
            found[i].len = len;
            repairs.push(Repair::RebuiltHeader {
                channel,
                index,
                from_index,
                format_estimated: !certain,
                bytes_kept: len - HEADER_BYTES,
            });
        }
        Ok(())
    }

    /// The sample-rate check recomputed from the chunk headers alone, for a record whose writer
    /// died with the process. It runs on the readable chunks; unreadable ones are listed by
    /// [`chunks`](Self::chunks). Runs are split where a header says the chunk follows a gap, or the
    /// format changes; chunks whose time or format recovery estimated are left out. The first
    /// mismatch found wins.
    pub fn rate_check(&self, channel: Channel) -> Result<RateVerdict, ChunkError> {
        let mut verdict = RateVerdict::Pending;
        let mut current: Option<(StreamFormat, RateCheck)> = None;
        let mut break_next = false;
        for chunk in self.chunks(channel)?.chunks {
            // Estimated times and formats are exactly what this check must not trust.
            if chunk.host_time_estimated || chunk.format_estimated {
                break_next = true;
                continue;
            }
            let same_format = matches!(&current, Some((f, _)) if *f == chunk.format);
            if !same_format {
                if let Some((_, check)) = current.take() {
                    verdict = combine(verdict, check.verdict());
                }
                current = Some((chunk.format, RateCheck::new(chunk.format.sample_rate)));
            }
            if let Some((_, check)) = &mut current {
                let starts_run = !same_format || chunk.after_gap || break_next;
                check.observe_run(chunk.host_time_ns, chunk.frames, starts_run);
            }
            break_next = false;
        }
        if let Some((_, check)) = current {
            verdict = combine(verdict, check.verdict());
        }
        Ok(verdict)
    }
}

/// Where recovery takes a rebuilt header's format and time from.
#[derive(Clone, Copy)]
enum Source {
    /// A chunk before the torn one: (index, header, whole frames).
    Before(u64, Header, u64),
    /// A chunk after it: (index, header).
    After(u64, Header),
}

/// Little-endian `f32` samples; a trailing partial sample is left out.
fn decode_samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(SAMPLE_BYTES as usize)
        .map(|s| f32::from_le_bytes(le(s)))
        .collect()
}

/// Keeps the first mismatch; otherwise the latest consistent verdict; otherwise pending.
fn combine(so_far: RateVerdict, next: RateVerdict) -> RateVerdict {
    match (so_far, next) {
        (RateVerdict::Mismatch { .. }, _) => so_far,
        (_, RateVerdict::Pending) => so_far,
        _ => next,
    }
}

fn set_len(path: &Path, len: u64) -> Result<(), ChunkError> {
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(io_error("open", path))?;
    file.set_len(len).map_err(io_error("truncate", path))?;
    file.sync_all().map_err(io_error("sync", path))
}

fn rewrite_header(path: &Path, header: &Header, len: u64) -> Result<(), ChunkError> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(io_error("open", path))?;
    file.set_len(len).map_err(io_error("truncate", path))?;
    file.seek(SeekFrom::Start(0))
        .map_err(io_error("seek", path))?;
    file.write_all(&header.encode())
        .map_err(io_error("write header", path))?;
    file.sync_all().map_err(io_error("sync", path))
}

/// Writes one channel's blocks into chunks.
///
/// **Pump.** Dropping a writer without [`finish`](Self::finish) is what a crash does: the open
/// chunk keeps every block written so far, unsynced.
pub struct ChunkWriter {
    dir: PathBuf,
    channel: Channel,
    format: StreamFormat,
    chunk_frames: u64,
    next_index: u64,
    open: Option<OpenChunk>,
    /// Audio was lost since the last block written (an overrun, or a failed write).
    pending_gap: bool,
    /// Chunks of this channel existed before this writer: its first chunk does not continue them.
    resumes_sequence: bool,
    rate: RateCheck,
    bytes: Vec<u8>,
    summary: WriterSummary,
}

struct OpenChunk {
    file: File,
    path: PathBuf,
    frames: u64,
}

/// Syncs a finished chunk to disk, then its directory entry, and closes it: from here a power cut
/// cannot take it.
fn sync(chunk: OpenChunk) -> Result<(), ChunkError> {
    chunk
        .file
        .sync_data()
        .map_err(io_error("sync", &chunk.path))?;
    match chunk.path.parent() {
        Some(dir) => sync_dir(dir),
        None => Ok(()),
    }
}

/// Makes a directory's entries durable: a new file's name, not only its bytes.
///
/// On Unix the directory itself is synced. On Windows std cannot open a directory to sync it, and
/// NTFS journals the entry, so there is nothing to do.
fn sync_dir(dir: &Path) -> Result<(), ChunkError> {
    #[cfg(unix)]
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(io_error("sync directory", dir))?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

impl ChunkWriter {
    /// The format this writer accepts.
    pub fn format(&self) -> StreamFormat {
        self.format
    }

    /// Appends a block. `lost_frames_before` is audio the ring dropped just before it
    /// ([`CapturedBlock::dropped_frames_before`](crate::ring::CapturedBlock::dropped_frames_before)).
    ///
    /// A gap (lost audio, or device time that jumps) closes the open chunk, so the next chunk's
    /// header carries the true time of its first frame. A full chunk is synced and closed. Only
    /// whole frames are written: a trailing partial frame, which [`AudioBlock`]'s contract rules
    /// out, would misalign every frame after it.
    ///
    /// After an I/O error the open chunk is abandoned (its torn tail is what
    /// [`ChunkStore::recover`] trims) and the next block starts a new chunk after a gap.
    pub fn write(
        &mut self,
        block: &AudioBlock<'_>,
        lost_frames_before: u64,
    ) -> Result<(), ChunkError> {
        if block.format != self.format {
            return Err(ChunkError::FormatChanged {
                expected: self.format,
                got: block.format,
            });
        }
        if lost_frames_before > 0 {
            self.pending_gap = true;
            self.summary.lost_frames = self.summary.lost_frames.saturating_add(lost_frames_before);
        }
        let frames = block.samples.len() / usize::from(self.format.channels);
        if frames == 0 {
            return Ok(());
        }
        let lost_before = std::mem::take(&mut self.pending_gap);
        let result = self.append(block, frames, lost_before);
        if result.is_err() {
            // Whatever part of the block did not reach a chunk is lost: the next chunk starts
            // after a gap, never as if the stream had continued.
            self.pending_gap = true;
        }
        result
    }

    fn append(
        &mut self,
        block: &AudioBlock<'_>,
        frames: usize,
        lost_before: bool,
    ) -> Result<(), ChunkError> {
        let channels = usize::from(self.format.channels);
        let continuity = self
            .rate
            .observe(block.host_time_ns, frames as u64, lost_before);
        let gap = lost_before || continuity == Continuity::Gap;
        if gap {
            self.summary.gaps += 1;
            self.close()?;
        }

        let mut offset = 0usize;
        let mut after_gap = gap;
        while offset < frames {
            let remaining = (frames - offset) as u64;
            // Close before a block that would not fit, so the next chunk starts on a block the
            // device stamped. Only a block longer than a whole chunk is split.
            if self
                .open
                .as_ref()
                .is_some_and(|o| o.frames > 0 && o.frames + remaining > self.chunk_frames)
            {
                self.close()?;
            }
            // Held by value while writing; put back only while it has room.
            let mut open = match self.open.take() {
                Some(open) => open,
                None => {
                    let host_time_ns = block
                        .host_time_ns
                        .saturating_add(frames_to_ns(offset as u64, self.format.sample_rate));
                    let flagged = after_gap || std::mem::take(&mut self.resumes_sequence);
                    after_gap = false;
                    self.open_chunk(host_time_ns, flagged)?
                }
            };
            let n = remaining.min(self.chunk_frames - open.frames) as usize;
            let samples = &block.samples[offset * channels..(offset + n) * channels];
            self.bytes.clear();
            self.bytes.reserve(samples.len() * SAMPLE_BYTES as usize);
            for s in samples {
                self.bytes.extend_from_slice(&s.to_le_bytes());
            }
            if let Err(source) = open.file.write_all(&self.bytes) {
                // The chunk is dropped here, not put back: it may end in a partial frame now.
                return Err(ChunkError::Io {
                    op: "write",
                    file: file_name(&open.path),
                    source,
                });
            }
            open.frames += n as u64;
            self.summary.frames += n as u64;
            offset += n;
            if open.frames >= self.chunk_frames {
                sync(open)?;
            } else {
                self.open = Some(open);
            }
        }
        Ok(())
    }

    fn open_chunk(&mut self, host_time_ns: u64, after_gap: bool) -> Result<OpenChunk, ChunkError> {
        let index = self.next_index;
        let path = self.dir.join(chunk_file_name(self.channel, index));
        // `create_new`: an existing chunk is never overwritten, whatever the caller did.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io_error("create", &path))?;
        // The index is spent once the file exists, even if the header write below fails.
        self.next_index = index.saturating_add(1);
        self.summary.chunks += 1;
        let header = Header {
            channel: self.channel,
            index,
            format: self.format,
            host_time_ns,
            flags: if after_gap { FLAG_AFTER_GAP } else { 0 },
        };
        file.write_all(&header.encode())
            .map_err(io_error("write header", &path))?;
        Ok(OpenChunk {
            file,
            path,
            frames: 0,
        })
    }

    fn close(&mut self) -> Result<(), ChunkError> {
        self.open.take().map_or(Ok(()), sync)
    }

    /// The sample-rate check so far. The pump polls it so a mismatch shows during the recording,
    /// not after it.
    pub fn rate(&self) -> RateVerdict {
        self.rate.verdict()
    }

    /// Syncs and closes the open chunk.
    pub fn finish(mut self) -> Result<WriterSummary, ChunkError> {
        self.close()?;
        Ok(WriterSummary {
            rate: self.rate.verdict(),
            ..self.summary
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Header {
        Header {
            channel: Channel::Far,
            index: 1 << 40,
            format: StreamFormat {
                sample_rate: 44_100,
                channels: 2,
            },
            host_time_ns: u64::MAX - 7,
            flags: FLAG_AFTER_GAP,
        }
    }

    #[test]
    fn header_round_trips() {
        let h = header();
        assert_eq!(Header::decode(&h.encode()), Ok(h));
    }

    #[test]
    fn any_damaged_header_byte_is_detected() {
        let good = header().encode();
        for i in 0..HEADER_LEN {
            let mut bad = good;
            bad[i] ^= 0x01;
            assert!(Header::decode(&bad).is_err(), "byte {i}");
        }
        assert_eq!(Header::decode(&good[..63]), Err("truncated header"));
        assert_eq!(
            Header::decode(&[0u8; HEADER_LEN]),
            Err("not a chunk header")
        );
    }
}
