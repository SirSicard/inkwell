//! Tauri command surface, split by domain.
//!
//! Everything is re-exported flat, so `commands::<name>` still resolves and the
//! `generate_handler!` list in lib.rs is unchanged by the split. The audit that
//! keeps them honest: every #[tauri::command] defined here must appear there.

mod history;
mod model;
mod settings;
mod text;
mod transcribe;

pub use history::*;
pub use model::*;
pub use settings::*;
pub use text::*;
pub use transcribe::*;
