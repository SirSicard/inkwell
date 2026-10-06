//! The models on disk, as the screens manage them: the catalogue's free space and the suggested
//! language model size.

use ink_engines::{EngineRow, Os, RowKind, suggested_language};

use crate::runtime::Shared;

/// **Worker.** The bytes free to this user on the volume models are installed on, or `None` when
/// the OS cannot say (logged). Asked of the models root, or, before it exists, of the nearest
/// directory above it that does.
pub fn free_bytes(shared: &Shared) -> Option<u64> {
    let root = shared.models.root();
    let Some(existing) = root.ancestors().find(|p| p.is_dir()) else {
        log::warn!("models: no directory above the models root exists to ask for free space");
        return None;
    };
    match shared.system.free_disk_bytes(existing) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            log::warn!("models: the free space could not be read ({e})");
            None
        }
    }
}

/// **Worker.** The machine's memory, or `None` when the OS cannot say (logged).
fn memory(shared: &Shared) -> Option<u64> {
    match shared.system.total_memory_bytes() {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            log::warn!("models: the machine's memory could not be read ({e})");
            None
        }
    }
}

/// **Worker.** The language row the core suggests for this machine, if this OS has any: the
/// Default, or the Small one with under 12 GB of memory.
pub fn suggested(shared: &Shared) -> Option<&EngineRow> {
    let os = Os::current()?;
    let rows = shared.registry.rows();
    // Memory is read only where there is a size to suggest (never on the Mac).
    rows.iter()
        .any(|r| r.runs_on(os) && is_language(r))
        .then(|| suggested_language(rows, os, memory(shared)))
        .flatten()
}

/// Whether `row` is a language model.
pub fn is_language(row: &EngineRow) -> bool {
    matches!(row.kind, RowKind::Language(_))
}
