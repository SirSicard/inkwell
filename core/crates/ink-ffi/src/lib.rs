//! The C ABI the native shells link: `include/inkwell.h`, implemented here.
//!
//! | Module | Holds |
//! |---|---|
//! | this file | the `extern "C"` functions: argument checks, the panic boundary, the one running core |
//! | [`runtime`] | the core at run time: config, shared state, commands, the ordered shutdown |
//! | [`hub`] | the event thread |
//! | [`events`] | events as JSON, per `schema/events.schema.json` |
//! | [`schema`] | the schema's model, its validator and the Swift generator |
//! | [`external`] | engines the shell registers (`InkEngineVTable`): offline, live streams, language models; their completion calls and stream events |
//! | [`llms`] | the language models the shell registered, and the polish model built on them |
//! | [`gate`] | exclusive holds on models during updates, and the engine every chain calls |
//! | [`mailbox`] | the bounded queue from the pump to a chain's worker |
//! | [`meeting`] | a meeting run: capture, the pump, the meeting worker |
//! | [`consent`] | the user's consent for each feature that sends words to a language model (polish, voice edit), and their switches, as the screens see them |
//! | [`queries`] | the screens' commands (permissions, owed, notes, settings, modes, models), on their own thread |
//! | [`dictation`] | the dictation worker |
//! | [`voice`] | dictation, live: the keys, the mic, the worker and the engine's warm-up |
//! | [`library`] | the library as the screens read it (records, search, a record, counts), answered on `queries`' thread |
//! | [`logging`] | the only logger and `tracing` subscriber, with both privacy filters |
//! | [`control`] | meetings started, stopped and detected, on their own thread ([`detection`] decides) |
//! | [`capture`] | a meeting's mic and far end from this machine's devices |
//! | [`engines`] | what a meeting runs on besides speech: the VAD, the diarizer, the language model |
//! | [`asking`] | Ask: questions about the live meeting, on their own thread |
//! | [`recovery`] | finishing a meeting a crash interrupted |
//! | [`retention`] | the retention setting's sweep |
//!
//! The threading contract is the header's (THREADS). In short: events reach the shell on one
//! core thread, engines are called on worker threads and answer through a completion call, and
//! nothing here calls into or waits on the shell's main thread.
//!
//! Every function catches panics at the boundary (a panic never unwinds into the shell) and
//! returns `INK_ERR_PANIC` instead.

#![deny(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![warn(missing_docs)]

pub mod asking;
pub mod capture;
pub mod consent;
pub mod control;
pub mod detection;
pub mod dictation;
pub mod engines;
pub mod events;
#[allow(unsafe_code)]
pub mod external;
pub mod gate;
pub mod hub;
pub mod library;
pub mod llms;
pub mod logging;
pub mod mailbox;
pub mod meeting;
pub mod phrases;
pub mod queries;
pub mod recovery;
pub mod retention;
pub mod runtime;
pub mod schema;
mod vad;
pub mod voice;

use std::ffi::{c_char, c_void};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock, PoisonError, RwLock};

use ink_audio::{BandsReader, BandsWriter, bands_channel};

use crate::external::{CompleteError, InkEngineVTable, Registration, StreamEventError};
use crate::runtime::{Config, Core, Parts};

/// `INK_OK`.
pub const INK_OK: i32 = 0;
/// `INK_ERR_NOT_INITIALIZED`.
pub const INK_ERR_NOT_INITIALIZED: i32 = -1;
/// `INK_ERR_ALREADY_INITIALIZED`.
pub const INK_ERR_ALREADY_INITIALIZED: i32 = -2;
/// `INK_ERR_INVALID_ARGUMENT`.
pub const INK_ERR_INVALID_ARGUMENT: i32 = -3;
/// `INK_ERR_FAILED`.
pub const INK_ERR_FAILED: i32 = -4;
/// `INK_ERR_UNKNOWN_CALL`.
pub const INK_ERR_UNKNOWN_CALL: i32 = -5;
/// `INK_ERR_PANIC`.
pub const INK_ERR_PANIC: i32 = -6;

/// `InkEventCallback`.
pub type InkEventCallback = unsafe extern "C" fn(ctx: *mut c_void, json: *const c_char, len: usize);

/// `InkBands`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InkBands {
    /// 80-500 Hz.
    pub low: f32,
    /// 500 Hz-2 kHz.
    pub mid: f32,
    /// 2-8 kHz.
    pub high: f32,
    /// Bands published so far.
    pub published: u64,
}

/// The running core. Commands take it shared; init and shutdown swap it.
static CORE: RwLock<Option<Core>> = RwLock::new(None);
/// Held by `ink_init` and `ink_shutdown` for their whole run, so one waits for the other, while
/// `CORE`'s own lock is only ever held briefly (a command from the event thread during shutdown
/// then finds no core rather than a deadlock).
static LIFECYCLE: Mutex<()> = Mutex::new(());
/// The ink's bands: one writer, lent to each core in turn, and one reader for the process.
static BANDS_WRITER: Mutex<Option<BandsWriter>> = Mutex::new(None);
static BANDS_READER: OnceLock<BandsReader> = OnceLock::new();
/// The far end's, likewise (`ink_far_bands_read`).
static FAR_BANDS_WRITER: Mutex<Option<BandsWriter>> = Mutex::new(None);
static FAR_BANDS_READER: OnceLock<BandsReader> = OnceLock::new();

fn guard(f: impl FnOnce() -> i32) -> i32 {
    panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        // The payload is not logged: it could hold what was said (I5).
        log::error!("a panic reached the C ABI boundary");
        INK_ERR_PANIC
    })
}

/// `INK_MAX_JSON`: the longest JSON string the core reads, in bytes without the NUL.
pub const INK_MAX_JSON: usize = 1 << 20;

/// A string argument: `None` for NULL, bad UTF-8, or longer than [`INK_MAX_JSON`].
///
/// # Safety
///
/// `s` is NULL or a NUL-terminated string valid for this call.
#[allow(unsafe_code)]
unsafe fn arg<'a>(s: *const c_char) -> Option<&'a str> {
    // SAFETY: forwarded from this function's own contract.
    unsafe { bounded_str(s, INK_MAX_JSON) }
}

/// Reads a NUL-terminated string of at most `max` bytes: `None` for NULL, a longer string, or bad
/// UTF-8. It looks for the NUL within `max + 1` bytes instead of scanning the whole string, so an
/// oversized argument is refused without being read to its end.
///
/// # Safety
///
/// `s` is NULL or a NUL-terminated string valid for this call.
#[allow(unsafe_code)]
pub(crate) unsafe fn bounded_str<'a>(s: *const c_char, max: usize) -> Option<&'a str> {
    if s.is_null() {
        return None;
    }
    let mut len = 0;
    loop {
        // SAFETY: every byte up to and including the NUL belongs to the string, and this stops
        // at the first NUL, so it never reads past it.
        if unsafe { *s.add(len) } == 0 {
            break;
        }
        if len == max {
            return None;
        }
        len += 1;
    }
    // SAFETY: the `len` bytes before the NUL, just scanned, valid for this call.
    let bytes = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len) };
    std::str::from_utf8(bytes).ok()
}

/// The shell's callback and context, moved to the event thread.
struct Callback {
    cb: InkEventCallback,
    ctx: *mut c_void,
}

// SAFETY: the header requires `ctx` to be usable from the event thread until `ink_shutdown`
// returns, which is when the event thread has ended; the function pointer is plain code.
#[allow(unsafe_code)]
unsafe impl Send for Callback {}

impl Callback {
    #[allow(unsafe_code)]
    fn call(&self, buf: &[u8]) {
        // SAFETY: `buf` ends with a NUL the length leaves out, and lives for the call; the shell
        // registered `cb` for exactly this signature.
        unsafe { (self.cb)(self.ctx, buf.as_ptr().cast(), buf.len() - 1) };
    }
}

/// See `inkwell.h`.
///
/// # Safety
///
/// `config_json` is NULL or a NUL-terminated string; `cb` is a valid function for the whole run
/// and `ctx` what it expects, usable from the event thread until `ink_shutdown` returns.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ink_init(
    config_json: *const c_char,
    cb: Option<InkEventCallback>,
    ctx: *mut c_void,
) -> i32 {
    // SAFETY: forwarded from this function's own contract.
    let config = unsafe { arg(config_json) };
    guard(|| {
        let (Some(config), Some(cb)) = (config, cb) else {
            return INK_ERR_INVALID_ARGUMENT;
        };
        let _lifecycle = LIFECYCLE.lock().unwrap_or_else(PoisonError::into_inner);
        if CORE
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
        {
            return INK_ERR_ALREADY_INITIALIZED;
        }
        let config = match Config::parse(config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ink_init: {e}");
                return INK_ERR_INVALID_ARGUMENT;
            }
        };
        if let Err(e) = logging::install(config.log_level, config.log_stderr) {
            eprintln!("ink_init: {e}");
            return INK_ERR_FAILED;
        }
        let parts = match Parts::production(&config) {
            Ok(p) => p,
            Err(e) => {
                log::error!("ink_init: {e}");
                return INK_ERR_FAILED;
            }
        };
        let callback = Callback { cb, ctx };
        let mut buf = Vec::new();
        let out = Box::new(move |json: &str| {
            buf.clear();
            buf.extend_from_slice(json.as_bytes());
            buf.push(0);
            callback.call(&buf);
        });
        match Core::start(parts, out) {
            Ok(core) => {
                let mut slot = BANDS_WRITER.lock().unwrap_or_else(PoisonError::into_inner);
                let writer = slot.take().unwrap_or_else(|| {
                    // The first start in this process makes the one pair; its reader serves every
                    // later `ink_bands_read`. The writer is lent only to a core that started, and
                    // comes back at shutdown, so it is never lost.
                    let (writer, reader) = bands_channel();
                    let _ = BANDS_READER.set(reader);
                    writer
                });
                drop(slot);
                core.lend_bands(writer);
                let mut slot = FAR_BANDS_WRITER
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let far = slot.take().unwrap_or_else(|| {
                    let (writer, reader) = bands_channel();
                    let _ = FAR_BANDS_READER.set(reader);
                    writer
                });
                drop(slot);
                core.lend_far_bands(far);
                // Dictation's platform; `dictation.enable` answers `dictation.off` without it.
                #[cfg(target_os = "macos")]
                match voice::VoicePlatform::mac() {
                    Ok(platform) => core.set_voice_platform(platform),
                    Err(e) => log::error!("ink_init: dictation has no platform: {e}"),
                }
                #[cfg(windows)]
                match voice::VoicePlatform::win() {
                    Ok(platform) => core.set_voice_platform(platform),
                    Err(e) => log::error!("ink_init: dictation has no platform: {e}"),
                }
                *CORE.write().unwrap_or_else(PoisonError::into_inner) = Some(core);
                INK_OK
            }
            Err(e) => {
                log::error!("ink_init: a core thread did not start: {e}");
                INK_ERR_FAILED
            }
        }
    })
}

/// See `inkwell.h`.
///
/// # Safety
///
/// `command_json` is NULL or a NUL-terminated string valid for this call.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ink_command(command_json: *const c_char) -> i32 {
    // SAFETY: forwarded from this function's own contract.
    let json = unsafe { arg(command_json) };
    guard(|| {
        let Some(json) = json else {
            return INK_ERR_INVALID_ARGUMENT;
        };
        let core = CORE.read().unwrap_or_else(PoisonError::into_inner);
        let Some(core) = core.as_ref() else {
            return INK_ERR_NOT_INITIALIZED;
        };
        match core.command(json) {
            Ok(()) => INK_OK,
            Err(e) => {
                log::warn!("ink_command: {e}");
                INK_ERR_INVALID_ARGUMENT
            }
        }
    })
}

/// See `inkwell.h`. Lock-free and allocation-free: a render thread may call it.
///
/// # Safety
///
/// `out` is NULL or points to writable memory for one `InkBands`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ink_bands_read(out: *mut InkBands) -> i32 {
    // SAFETY: forwarded from this function's own contract.
    unsafe { read_bands(&BANDS_READER, out) }
}

/// See `inkwell.h`: [`ink_bands_read`] for the far end. Lock-free and allocation-free.
///
/// # Safety
///
/// `out` is NULL or points to writable memory for one `InkBands`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ink_far_bands_read(out: *mut InkBands) -> i32 {
    // SAFETY: forwarded from this function's own contract.
    unsafe { read_bands(&FAR_BANDS_READER, out) }
}

/// Copies the latest bands from `reader` into `out`.
///
/// # Safety
///
/// `out` is NULL or points to writable memory for one `InkBands`.
#[allow(unsafe_code)]
unsafe fn read_bands(reader: &OnceLock<BandsReader>, out: *mut InkBands) -> i32 {
    if out.is_null() {
        return INK_ERR_INVALID_ARGUMENT;
    }
    // No panic boundary: nothing here can panic, and `catch_unwind` itself costs nothing to
    // leave out on a path a render loop calls.
    let bands = reader
        .get()
        .map(BandsReader::read)
        .map_or_else(InkBands::default, |s| InkBands {
            low: s.bands.low,
            mid: s.bands.mid,
            high: s.bands.high,
            published: s.published,
        });
    // SAFETY: non-null, and the caller guarantees it is writable for one `InkBands`.
    unsafe { out.write(bands) };
    INK_OK
}

/// See `inkwell.h`.
///
/// # Safety
///
/// `vtable` is NULL or points to a readable `InkEngineVTable` of the size it states, whose
/// functions and `ctx` honour the header's contract until `release` is called.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ink_register_engine(vtable: *const InkEngineVTable) -> i32 {
    guard(|| {
        let core = CORE.read().unwrap_or_else(PoisonError::into_inner);
        let Some(core) = core.as_ref() else {
            return INK_ERR_NOT_INITIALIZED;
        };
        // SAFETY: forwarded from this function's own contract.
        let engine =
            match unsafe { Registration::from_table(vtable, core.shared().shutdown.clone()) } {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("ink_register_engine: {}", e.0);
                    return INK_ERR_INVALID_ARGUMENT;
                }
            };
        match core.register(engine) {
            Ok(_) => INK_OK,
            Err(e) => {
                log::warn!("ink_register_engine: {e}");
                INK_ERR_FAILED
            }
        }
    })
}

/// See `inkwell.h`. Any thread.
///
/// # Safety
///
/// `result_json` is NULL or a NUL-terminated string valid for this call.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ink_engine_complete(call: u64, result_json: *const c_char) -> i32 {
    // SAFETY: forwarded from this function's own contract.
    let json = unsafe { arg(result_json) };
    // A missing, unreadable or oversized answer still answers the call, as a failure, so the
    // worker waiting on it is released; the engine is told its answer was refused.
    guard(|| match external::complete(call, json.unwrap_or("")) {
        Ok(()) => INK_OK,
        Err(CompleteError::Unknown) => INK_ERR_UNKNOWN_CALL,
        Err(CompleteError::Malformed) => INK_ERR_INVALID_ARGUMENT,
    })
}

/// See `inkwell.h`. Any thread; it only queues.
///
/// # Safety
///
/// `event_json` is NULL or a NUL-terminated string valid for this call.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ink_stream_event(stream: u64, event_json: *const c_char) -> i32 {
    // SAFETY: forwarded from this function's own contract.
    let json = unsafe { arg(event_json) };
    guard(|| {
        let Some(json) = json else {
            return INK_ERR_INVALID_ARGUMENT;
        };
        match external::stream_event(stream, json) {
            Ok(()) => INK_OK,
            Err(StreamEventError::Unknown) => INK_ERR_UNKNOWN_CALL,
            Err(StreamEventError::Malformed) => INK_ERR_INVALID_ARGUMENT,
        }
    })
}

/// See `inkwell.h`.
#[allow(unsafe_code)] // `no_mangle` only: the function itself is safe.
#[unsafe(no_mangle)]
pub extern "C" fn ink_shutdown() -> i32 {
    guard(|| {
        let _lifecycle = LIFECYCLE.lock().unwrap_or_else(PoisonError::into_inner);
        // Taken out under a brief lock: shutting down joins the event thread, which may be
        // calling ink_command, which must then find no core instead of waiting on this lock.
        let core = CORE.write().unwrap_or_else(PoisonError::into_inner).take();
        let Some(core) = core else {
            return INK_ERR_NOT_INITIALIZED;
        };
        let stopped = core.shutdown();
        if let Some(writer) = stopped.bands {
            *BANDS_WRITER.lock().unwrap_or_else(PoisonError::into_inner) = Some(writer);
        }
        if let Some(writer) = stopped.far_bands {
            *FAR_BANDS_WRITER
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(writer);
        }
        INK_OK
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::*;

    #[test]
    #[allow(unsafe_code)]
    fn strings_are_read_up_to_the_cap_and_no_further() {
        let at = CString::new("a".repeat(16)).unwrap();
        let over = CString::new("a".repeat(17)).unwrap();
        let bad = CString::new(vec![0xff, 0xfe]).unwrap();
        // SAFETY: valid NUL-terminated strings, and NULL.
        unsafe {
            assert_eq!(bounded_str(at.as_ptr(), 16).map(str::len), Some(16));
            assert_eq!(bounded_str(over.as_ptr(), 16), None);
            assert_eq!(bounded_str(bad.as_ptr(), 16), None);
            assert_eq!(bounded_str(std::ptr::null(), 16), None);
            assert_eq!(bounded_str(c"".as_ptr(), 0), Some(""));
        }
    }
}
