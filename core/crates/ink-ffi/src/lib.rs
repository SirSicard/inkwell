//! The C ABI the native shells link: `include/inkwell.h`, implemented here.
//!
//! | Module | Holds |
//! |---|---|
//! | this file | the `extern "C"` functions: argument checks, the panic boundary, the one running core |
//! | [`runtime`] | the core at run time: config, shared state, commands, the ordered shutdown |
//! | [`hub`] | the event thread |
//! | [`events`] | events as JSON, per `schema/events.schema.json` |
//! | [`schema`] | the schema's model, its validator and the Swift generator |
//! | [`external`] | engines the shell registers (`InkEngineVTable`) and their completion calls |
//! | [`gate`] | exclusive holds on models during updates, and the engine every chain calls |
//! | [`mailbox`] | the bounded queue from the pump to a chain's worker |
//! | [`meeting`] | a meeting run: capture, the pump, the meeting worker |
//! | [`dictation`] | the dictation worker |
//! | [`logging`] | the only logger and `tracing` subscriber, with both privacy filters |
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

pub mod events;
pub mod schema;
