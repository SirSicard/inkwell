//! The core's one logger: a `log` logger and a `tracing` subscriber, installed once per process,
//! each applying both privacy filters.
//!
//! Two crates in the core log through facades whose debug output is unsafe to keep:
//!
//! | Path | Who | Unsafe at debug | Filter |
//! |---|---|---|---|
//! | `log` | ureq (ink-llm, ink-engines) | request headers, with `x-api-key` in clear | [`ink_llm::log_record_allowed`] |
//! | `tracing` | llama.cpp, ggml, mtmd (through llama-cpp-2) | lines quoting generated text | `ink_engines::llama::log_allowed` |
//!
//! Each filter belongs to its path, and both are applied here so no other logger is ever needed:
//! the header tells the shells not to install one for the core's targets. Two more rules keep a
//! record from slipping between them:
//!
//! - A llama.cpp target on the `log` path is dropped at every level. llama-cpp-2 logs through
//!   `tracing`, and `tracing`'s `log` feature is off in this tree; if a dependency ever turned it
//!   on, those lines would arrive here, where llama's filter could not see them.
//! - Without the `engine-llama` feature there is no llama.cpp and no llama filter to call; the
//!   subscriber then drops llama.cpp's targets outright.
//!
//! The `tracing` subscriber is written here rather than taken from `tracing-subscriber`: it keeps
//! no spans and formats one line per event, which is all the core needs.
//!
//! **Threads:** any but realtime (nothing on a realtime thread logs). A log line never holds a
//! transcript (I5): that is each crate's rule; the filters here stop what other people's crates
//! would otherwise write.

use std::fmt::{self, Write as _};
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber};

/// One line as a sink receives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// `error`, `warn`, `info`, `debug` or `trace`.
    pub level: &'static str,
    /// The record's target.
    pub target: String,
    /// The message, with a `tracing` event's other fields appended as `name=value`.
    pub message: String,
}

/// Where lines go besides stderr (tests capture them).
pub type Sink = Arc<dyn Fn(&Line) + Send + Sync>;

/// The most verbose level written, as `log`'s levels count: 0 off, 1 error ... 5 trace.
static MAX: AtomicUsize = AtomicUsize::new(3);
static STDERR: AtomicBool = AtomicBool::new(true);
static SINK: RwLock<Option<Sink>> = RwLock::new(None);
static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();

/// Parses a level name from the config.
pub fn parse_level(name: &str) -> Option<log::LevelFilter> {
    Some(match name {
        "off" => log::LevelFilter::Off,
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        _ => return None,
    })
}

/// Installs the logger and the subscriber for this process (the first call; later calls only
/// report how that went), then applies `level` and `stderr`. Fails when another logger or
/// subscriber was installed first: the filters could then not be guaranteed.
pub fn install(level: log::LevelFilter, stderr: bool) -> Result<(), String> {
    INSTALLED
        .get_or_init(|| {
            log::set_logger(&CoreLogger).map_err(|_| {
                "another `log` logger is installed; the core must install the only one".to_owned()
            })?;
            tracing::subscriber::set_global_default(CoreSubscriber).map_err(|_| {
                "another `tracing` subscriber is installed; the core must install the only one"
                    .to_owned()
            })
        })
        .clone()?;
    MAX.store(level as usize, Ordering::Relaxed);
    STDERR.store(stderr, Ordering::Relaxed);
    // The `log` macros skip formatting above this; the filters still run on everything below.
    log::set_max_level(level);
    // `tracing` caches whether each call site is wanted; the level may have changed.
    tracing::callsite::rebuild_interest_cache();
    Ok(())
}

/// Sets (or clears) the sink every written line also goes to.
pub fn set_sink(sink: Option<Sink>) {
    *SINK.write().unwrap_or_else(PoisonError::into_inner) = sink;
}

fn level_allows(level: usize) -> bool {
    level <= MAX.load(Ordering::Relaxed)
}

/// A llama.cpp target: its own name (`llama-cpp-2`), the binding's modules, the sys crate.
fn llama_target(target: &str) -> bool {
    target.starts_with("llama-cpp") || target.starts_with("llama_cpp")
}

/// Whether a `log` record may be written.
pub fn log_allowed(metadata: &log::Metadata<'_>) -> bool {
    level_allows(metadata.level() as usize)
        && ink_llm::log_record_allowed(metadata)
        && !llama_target(metadata.target())
}

fn tracing_level(level: &tracing::Level) -> usize {
    match *level {
        tracing::Level::ERROR => 1,
        tracing::Level::WARN => 2,
        tracing::Level::INFO => 3,
        tracing::Level::DEBUG => 4,
        tracing::Level::TRACE => 5,
    }
}

/// Whether a `tracing` event or span may be written.
pub fn tracing_allowed(metadata: &Metadata<'_>) -> bool {
    level_allows(tracing_level(metadata.level())) && llama_filter(metadata)
}

#[cfg(feature = "engine-llama")]
fn llama_filter(metadata: &Metadata<'_>) -> bool {
    ink_engines::llama::log_allowed(metadata)
}

#[cfg(not(feature = "engine-llama"))]
fn llama_filter(metadata: &Metadata<'_>) -> bool {
    // No llama.cpp in this build, so none of its lines are expected; any that came would have no
    // filter to pass.
    !llama_target(metadata.target())
}

fn write(line: &Line) {
    if STDERR.load(Ordering::Relaxed) {
        // A failed write to stderr has nowhere to be reported.
        let _ = writeln!(
            std::io::stderr().lock(),
            "[{} {}] {}",
            line.level,
            line.target,
            line.message
        );
    }
    let sink = SINK.read().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(sink) = sink {
        sink(line);
    }
}

struct CoreLogger;

impl log::Log for CoreLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        log_allowed(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        // The `log` macros never call `enabled`: the filter must run here.
        if !log_allowed(record.metadata()) {
            return;
        }
        write(&Line {
            level: level_name(record.level() as usize),
            target: record.target().to_owned(),
            message: record.args().to_string(),
        });
    }

    fn flush(&self) {}
}

fn level_name(level: usize) -> &'static str {
    match level {
        1 => "error",
        2 => "warn",
        3 => "info",
        4 => "debug",
        _ => "trace",
    }
}

struct CoreSubscriber;

/// Collects an event's fields into one message.
#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, "{}={value:?}", field.name());
        }
    }
}

impl Subscriber for CoreSubscriber {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        // "sometimes": asked again per event, since the level can change at run time.
        if llama_filter(metadata) {
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        tracing_allowed(metadata)
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        // Spans are not kept: every span is the same placeholder.
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let metadata = event.metadata();
        if !tracing_allowed(metadata) {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        write(&Line {
            level: level_name(tracing_level(metadata.level())),
            target: metadata.target().to_owned(),
            message: message.0,
        });
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llama_targets_are_recognised() {
        for t in [
            "llama-cpp-2",
            "llama_cpp_2",
            "llama_cpp_2::log",
            "llama_cpp_sys_2",
        ] {
            assert!(llama_target(t), "{t}");
        }
        assert!(!llama_target("ink_pipeline"));
        assert!(!llama_target("ureq"));
    }

    #[test]
    fn level_names_round_trip() {
        for name in ["error", "warn", "info", "debug", "trace"] {
            let level = parse_level(name).unwrap();
            assert_eq!(level_name(level as usize), name);
        }
        assert_eq!(parse_level("loud"), None);
    }
}
