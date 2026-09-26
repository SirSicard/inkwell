//! A hand-run check of the Inkwell 0.2 importer on a **copy** of a real 0.2 data directory
//! (`IMPORT-CHECKLIST.md`). It hashes every source file, prints the dry run's counts, imports into
//! a new store, counts that store again without the importer's help, and hashes the sources again.
//!
//! ```text
//! cargo run -p ink-store --example import_inkwell02 -- SOURCE_DIR NEW_STORE_FILE [OPTIONS]
//!
//!   SOURCE_DIR       a copy of ~/Library/Application Support/com.inkwell.app
//!   NEW_STORE_FILE   where to create the 1.0 store; must not exist yet
//!
//!   --dry-run        read and count only: no store is created
//!   --no-keychain    do not ask the keychain which providers have a key
//! ```
//!
//! It prints counts, file names, sizes and sha256 digests: never a transcript, a setting's value
//! or a key. The keychain is asked through ink-llm's existence check, which reads an item's
//! attributes and never its secret. Exit status 0 means every count matched and every source file
//! is byte-identical afterwards.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use ink_core::{RecordKind, RecordQuery, Store};
use ink_llm::provider::configured_providers;
use ink_llm::{KeyStore, OsKeyStore};
use ink_store::SqliteStore;
use ink_store::import::{
    APP_STYLES_KEY, Counts, DICTIONARY_KEY, Inkwell02, KeyProbe, KeyProbeError, MARKER_KEY,
    MODES_KEY, NoKeychain, SETTINGS_PREFIX, SNIPPETS_KEY, VOICE_COMMANDS_KEY,
};
use sha2::{Digest, Sha256};

const USAGE: &str = "usage: import_inkwell02 SOURCE_DIR NEW_STORE_FILE [--dry-run] [--no-keychain]";

/// The importer's view of ink-llm's key store: existence only.
struct LlmKeys(OsKeyStore);

impl KeyProbe for LlmKeys {
    fn has_key(&self, account: &str) -> Result<bool, KeyProbeError> {
        self.0.has_key(account).map_err(|_| KeyProbeError)
    }
}

struct Args {
    source: PathBuf,
    store: PathBuf,
    dry_run: bool,
    keychain: bool,
}

fn args() -> Result<Args, String> {
    let mut paths = Vec::new();
    let (mut dry_run, mut keychain) = (false, true);
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("--dry-run") => dry_run = true,
            Some("--no-keychain") => keychain = false,
            Some(flag) if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    let [source, store]: [PathBuf; 2] = paths.try_into().map_err(|_| USAGE.to_string())?;
    Ok(Args {
        source,
        store,
        dry_run,
        keychain,
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => {
            println!("\nRESULT: PASS");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("\nRESULT: FAIL (see the lines marked MISMATCH or CHANGED)");
            ExitCode::FAILURE
        }
        Err(message) => {
            eprintln!("import_inkwell02: {message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool, String> {
    let args = args()?;
    if !args.source.is_dir() {
        return Err("SOURCE_DIR is not a directory".into());
    }
    if !args.dry_run {
        if args.store.exists() {
            return Err("NEW_STORE_FILE exists already; give a path for a new store".into());
        }
        // The store must not be written inside the directory being checked.
        let source = std::fs::canonicalize(&args.source).map_err(|e| e.to_string())?;
        let parent = args
            .store
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = std::fs::canonicalize(parent)
            .map_err(|_| "NEW_STORE_FILE's directory does not exist".to_string())?;
        if parent.starts_with(&source) {
            return Err("NEW_STORE_FILE must be outside SOURCE_DIR".into());
        }
    }

    println!("Inkwell 0.2 import check\n\nSource files before (sha256):");
    let before = hash_dir(&args.source)?;
    print_hashes(&before);

    let keys = if args.keychain {
        match OsKeyStore::native() {
            Ok(store) => {
                println!("\nKeychain: asked which providers have a key (attributes only).");
                Some(LlmKeys(store))
            }
            Err(e) => {
                println!("\nKeychain: unavailable ({e:?}); keys left unchecked.");
                None
            }
        }
    } else {
        println!("\nKeychain: not asked (--no-keychain).");
        None
    };
    let probe: &dyn KeyProbe = match &keys {
        Some(keys) => keys,
        None => &NoKeychain,
    };

    let source = Inkwell02::read(&args.source, probe).map_err(|e| e.to_string())?;
    let counts = source.counts();
    println!("\nDry run, counted in the source:");
    for (kind, n) in counts.by_kind() {
        println!("  {kind:<20} {n:>7}");
    }
    let report = source.report();
    println!("\nLeft behind in the source (not imported):");
    println!(
        "  rows whose raw text differs from the pasted text: {}",
        report.raw_text_differs
    );
    println!("  style and model names of every row");
    if report.secret_fields_skipped.is_empty() {
        println!("  plaintext key fields in settings.json: none");
    } else {
        println!(
            "  plaintext key fields in settings.json: {} (values neither imported nor printed; \
             consider rotating those keys)",
            report.secret_fields_skipped.join(", ")
        );
    }
    println!(
        "  other settings.json fields: {}",
        report.unknown_fields_skipped
    );
    println!(
        "  providers not checked in the keychain: {}",
        report.keys_unchecked
    );

    let mut pass = true;
    if !args.dry_run {
        let store = SqliteStore::open(&args.store).map_err(|e| e.to_string())?;
        let started = Instant::now();
        let written = store.import_inkwell02(&source).map_err(|e| e.to_string())?;
        println!(
            "\nImported in {:.2} s. Counted again in the new store:",
            started.elapsed().as_secs_f64()
        );
        let recount = recount(&store, &args.store, keys.as_ref())?;
        println!(
            "  {:<20} {:>7} {:>7} {:>7}",
            "kind", "source", "written", "store"
        );
        for ((kind, source_n), ((_, written_n), (_, store_n))) in counts
            .by_kind()
            .into_iter()
            .zip(written.by_kind().into_iter().zip(recount.by_kind()))
        {
            let ok = source_n == written_n && written_n == store_n;
            pass &= ok;
            let mark = if ok { "ok" } else { "MISMATCH" };
            println!("  {kind:<20} {source_n:>7} {written_n:>7} {store_n:>7}  {mark}");
        }
    }

    println!("\nSource files after (sha256):");
    let after = hash_dir(&args.source)?;
    print_hashes(&after);
    let identical = before == after;
    pass &= identical;
    for name in before
        .keys()
        .chain(after.keys().filter(|k| !before.contains_key(*k)))
    {
        if before.get(name) != after.get(name) {
            println!("  CHANGED: {name}");
        }
    }
    println!(
        "Sources byte-identical: {}",
        if identical { "yes" } else { "NO" }
    );
    Ok(pass)
}

/// Counts every kind in the store without the importer: the records, the documents' items, the
/// settings rows, and the providers ink-llm finds a key for now.
fn recount(store: &SqliteStore, path: &Path, keys: Option<&LlmKeys>) -> Result<Counts, String> {
    let dictations = store
        .records(&RecordQuery {
            kind: Some(RecordKind::Dictation),
            before: None,
            limit: usize::MAX,
        })
        .map_err(|e| e.to_string())?
        .len();
    let items = |key: &str, pointer: &str| -> Result<usize, String> {
        let Some(text) = store.setting(key).map_err(|e| e.to_string())? else {
            return Ok(0);
        };
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        value
            .pointer(pointer)
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .ok_or_else(|| format!("{key} is not the expected document"))
    };
    // The trait cannot list settings, so the rows are counted on a second, read-only connection.
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    let settings: i64 = conn
        .query_row(
            "SELECT count(*) FROM setting WHERE substr(key, 1, length(?1)) = ?1",
            [SETTINGS_PREFIX],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    // With a keychain, ask ink-llm itself which providers it finds; without one, the marker's
    // list is all there is.
    let linked_keys = match keys {
        Some(keys) => configured_providers(&keys.0).len(),
        None => {
            let marker = store
                .setting(MARKER_KEY)
                .map_err(|e| e.to_string())?
                .ok_or("the import wrote no marker")?;
            let marker: serde_json::Value =
                serde_json::from_str(&marker).map_err(|e| e.to_string())?;
            marker["linked_keys"].as_array().map_or(0, Vec::len)
        }
    };
    Ok(Counts {
        dictations,
        dictionary_entries: items(DICTIONARY_KEY, "")?,
        snippets: items(SNIPPETS_KEY, "")?,
        modes: items(MODES_KEY, "/modes")?,
        settings: usize::try_from(settings).map_err(|e| e.to_string())?,
        voice_commands: items(VOICE_COMMANDS_KEY, "/commands")?,
        app_style_rules: items(APP_STYLES_KEY, "/rules")?,
        linked_keys,
    })
}

/// Every file directly in `dir` (subdirectories such as `models/` are skipped), by name, with its
/// size and sha256.
fn hash_dir(dir: &Path) -> Result<BTreeMap<String, (u64, String)>, String> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        out.insert(
            entry.file_name().to_string_lossy().into_owned(),
            (bytes.len() as u64, digest),
        );
    }
    Ok(out)
}

fn print_hashes(files: &BTreeMap<String, (u64, String)>) {
    for (name, (size, digest)) in files {
        println!("  {name:<28} {size:>10} bytes  {digest}");
    }
}
