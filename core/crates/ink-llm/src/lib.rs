//! Language models for the core: bring-your-own-key providers, the local-only guard, API keys in
//! the OS keychain, and the tasks that use a model (polish, voice editing, meeting summaries,
//! commitments).
//!
//! # Threads, and why there is no async runtime
//!
//! [`Llm::complete`](ink_core::Llm::complete) is a **worker**-thread call that blocks until the
//! answer arrives. The HTTP client is [`ureq`], which is blocking too, so nothing here needs an
//! async runtime: a worker thread makes the request and waits for it. Inkwell 0.2 built a Tokio
//! runtime and an HTTP client for every call; here one client ([`UreqTransport::shared`]) is
//! built once per process and every provider shares it, with its connection pool.
//!
//! # What goes where
//!
//! | Module | Holds |
//! |---|---|
//! | [`guard`] | The local-only guard (architecture rule 6): which endpoints are on this machine. |
//! | [`transport`] | The HTTP seam ([`Transport`]) and its one real implementation. |
//! | [`keys`] | API keys in the OS keychain, behind [`KeyStore`]. |
//! | [`provider`] | [`ByokLlm`]: OpenAI, Groq, Anthropic, OpenRouter and any OpenAI-compatible server. |
//! | [`tasks`] | Polish, voice editing, summaries, commitments and their deduplication, each generic over `&dyn Llm`. |
//!
//! # Privacy
//!
//! Prompts, transcripts, keys and answers never reach a log or an error (I5). This crate does not
//! log at all. Errors name what failed (a status code, a missing field), never what was said, and
//! an HTTP error body is never read into an error, because providers echo the request back in it.
//!
//! [`Llm`]: ink_core::Llm

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod guard;
pub mod keys;
pub mod provider;
pub mod tasks;
pub mod transport;

mod json;

pub use guard::{EndpointError, EndpointUrl, GuardedLlm, LocalOnly};
pub use keys::{ApiKey, KEYRING_SERVICE, KeyStore, KeyStoreError, OsKeyStore};
pub use provider::{ByokConfig, ByokLlm, Provider};
pub use transport::{HttpRequest, HttpResponse, Transport, TransportError, UreqTransport};
