//! The screens' commands: permissions, what is owed, a live meeting's notes, a record's speakers'
//! names, settings, modes, the model catalogue, the library's records
//! ([`library`](crate::library)), and Inkwell 0.2's data ([`import02`](crate::import02)). They
//! run on their own thread, `ink-queries`, in the order they were sent.
//!
//! Apart from the command thread on purpose: a model update holds that thread for as long as its
//! download takes, and a note typed during it, or a permission card the user is looking at, must
//! not wait minutes for it. Each of these is quick (a store call, or the system-audio probe's
//! second or so). They may overtake commands queued earlier on the command thread; nothing here
//! depends on one of those.
//!
//! Errors name what failed, never what was said: a note's or a commitment's text, or a speaker's
//! name, reaches the shell only in the event that answers the command that asked for it (I5).

use std::collections::BTreeMap;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use ink_core::{
    Channel, Commitment, CommitmentId, NoteId, Permission, PermissionProbe, PermissionState,
    PlatformError, RecordId, SpeakerId, Store,
};
use ink_engines::{ModelDir, Os, Route, RowKind};
use serde_json::{Map, Value, json};

use crate::events::{self, event};
use crate::runtime::Shared;

/// The store setting that remembers the app has asked for System Audio. Until it is set, a check
/// never runs the tone probe, because the probe would make macOS show its prompt.
pub const SYSTEM_AUDIO_ASKED_KEY: &str = "permissions.system_audio_asked";

/// The store setting holding the user's modes, as a JSON document (the shape the 0.2 import
/// writes, read by `ink_pipeline::modes::ModeStore::from_json`). Until it is set, the modes are
/// the imported ones, and without those the built-in default; the first `modes.save` or
/// `modes.delete` writes it ([`modes`](crate::modes)). The dictation chain reads the same key.
pub const MODES_KEY: &str = "dictation.modes";

/// The settings a shell may read and write through `setting.get` and `setting.set`, with the values
/// each accepts. Everything else in the store is the core's.
pub const SHELL_SETTINGS: &[(&str, &[&str])] = &[
    // The first-run state has been completed (or skipped).
    ("onboarding.done", &["true", "false"]),
    // The user's wish for dictation polish. Whether polish runs also needs a working language
    // model; the shell shows the two apart. `setting.set` takes only `off` (which withdraws the
    // consent too): polish turns on through `consent.allow` (crate::consent).
    (crate::voice::POLISH_SETTING, &["on", "off"]),
    // The dictation key and the voice-edit key (S2.7): any key this computer can watch, stored in
    // its one spelling (crate::hotkey). A change rebinds them at once. The edit key turns on with
    // its consent through `consent.allow`; `off` withdraws that consent too, and a key set without
    // one edits nothing (crate::consent).
    (crate::voice::KEY_SETTING, &[ANY_KEY]),
    (crate::voice::EDIT_KEY_SETTING, &["off", ANY_KEY]),
    // Whether the shell turns dictation on at launch (Settings > Voice). The shell reads it and
    // sends dictation.enable or not; the core does nothing with it itself.
    ("dictation.enabled", &["on", "off"]),
    // Whether a meeting's summary (with its commitments) and Ask may send its transcript to a
    // language model. As for polish, `setting.set` takes only `off` (which withdraws the consent
    // too): it turns on through `consent.allow` (crate::consent).
    (crate::consent::MEETINGS_SETTING, &["on", "off"]),
    // What happens for apps the user has not chosen for when one takes the mic for a call
    // (crate::calls): ask (the consent Drop; also when unset), always record, or never. The
    // meetings thread reads the policies again when it changes.
    (crate::calls::DEFAULT_KEY, crate::calls::DEFAULT_VALUES),
    // "Offer to record calls", which the default replaced: answered for the default until the
    // shells move to it (off is never; on over never is ask), never stored again.
    (crate::control::DETECT_KEY, &["on", "off"]),
    // Retired (crate::control::HEADSET_MIC_KEY): the core reads it nowhere; accepted until the
    // shells' Meetings switch gives way to Settings > Sound.
    (crate::control::HEADSET_MIC_KEY, &["on", "off"]),
    // The mic for dictation, meetings and the mic test (crate::devices): Automatic, or a device
    // connected when it is set (by the id audio.devices lists). A change lets go of dictation's
    // idle mic when it is another.
    (
        crate::devices::INPUT_KEY,
        &[crate::devices::AUTO, ANY_DEVICE],
    ),
    // The output a meeting's far end is to record (Windows): the default output, or a device
    // connected when it is set. Only `default` where the platform has no output picker (macOS).
    // Stored and shown; the far end follows the default until the Windows device branch pins it.
    (
        crate::devices::OUTPUT_KEY,
        &[crate::devices::DEFAULT, ANY_DEVICE],
    ),
    // Local-only mode (architecture rule 6): on unless turned off; while on, a language model
    // that is not on this machine is never called (crate::llms::PolishModel).
    (crate::llms::LOCAL_ONLY_KEY, &["on", "off"]),
    // How long the library keeps records (crate::retention): changing it sweeps at once.
    (
        crate::retention::RETENTION_KEY,
        crate::retention::RETENTION_VALUES,
    ),
    // The 0.2 import's note about the dictation key has been read (crate::phrases): said once.
    (crate::phrases::KEY_NOTE_SETTING, &["dismissed"]),
    // Appearance (Settings > Appearance). The shells read these; the core does nothing with them.
    // Unset, each is its APPEARANCE_DEFAULTS value. Light or dark, or the system's.
    ("appearance.mode", &["light", "dark", "system"]),
    // The dot colours' preset in each mode (design/tokens.json defines each preset's two colours).
    ("appearance.dots.light", DOT_PRESETS),
    ("appearance.dots.dark", DOT_PRESETS),
    // Your colour and the far end's in each mode: the preset's, or a colour of the user's own.
    ("appearance.you.light", &["preset", HEX_COLOUR]),
    ("appearance.them.light", &["preset", HEX_COLOUR]),
    ("appearance.you.dark", &["preset", HEX_COLOUR]),
    ("appearance.them.dark", &["preset", HEX_COLOUR]),
    // The glow round the window's edge while dictating or in a meeting.
    ("appearance.edge_glow", &["on", "off"]),
    // `still` draws the orb and the edge glow without motion; `system` follows the system's
    // reduce-motion setting.
    ("appearance.motion", &["system", "still"]),
    // The typing speed the Stats screen measures time saved against (crate::stats): whole words
    // a minute, 40 unless set.
    (crate::stats::TYPING_WPM_KEY, &[TYPING_WPM]),
    // Whether a milestone reached, or a best set, is celebrated (crate::stats). On unless turned
    // off.
    (crate::stats::CELEBRATE_KEY, &["on", "off"]),
    // The weekdays the streak rests on (crate::stats::rest_days): none unless set.
    (crate::stats::REST_DAYS_KEY, &["none", REST_DAYS]),
    // Whether the streak shows anywhere: shown unless hidden.
    (crate::stats::STREAK_KEY, &["shown", "hidden"]),
    // Whether the share card may carry the heatmap. Off unless turned on; the shells read it.
    (crate::stats::SHARE_HEATMAP_KEY, &["on", "off"]),
    // The week whose review the user dismissed, by its first day.
    (crate::stats::REVIEW_DISMISSED_KEY, &[DATE]),
];

/// In a value list of [`SHELL_SETTINGS`]: a device's id as `audio.devices` lists it
/// ([`crate::devices::is_device_token`]); setting it also checks that it is connected now.
pub const ANY_DEVICE: &str = "<device>";

/// In a value list of [`SHELL_SETTINGS`]: a typing speed, a whole number of words a minute in
/// [`crate::stats::TYPING_WPM_RANGE`], written plainly (`40`).
pub const TYPING_WPM: &str = "<wpm>";

/// In a value list of [`SHELL_SETTINGS`]: ISO weekdays ascending and comma-separated (`6,7`), as
/// [`crate::stats::rest_days`] reads them.
pub const REST_DAYS: &str = "<weekdays, e.g. 6,7>";

/// In a value list of [`SHELL_SETTINGS`]: a date, `YYYY-MM-DD`.
pub const DATE: &str = "<YYYY-MM-DD>";

/// In a value list of [`SHELL_SETTINGS`]: any colour written `#rrggbb`, in lowercase hex.
pub const HEX_COLOUR: &str = "#rrggbb";

/// In a value list of [`SHELL_SETTINGS`]: any key [`crate::hotkey::stored_value`] takes, which is
/// stored in its one spelling.
pub const ANY_KEY: &str = "<key>";

/// The dot colour presets, by id: the `presets` of design/tokens.json.
pub const DOT_PRESETS: &[&str] = &[
    "indigo",
    "dusk",
    "lagoon",
    "aurora",
    "citrus",
    "rosewater",
    "ink_sand",
];

/// What the shells read each appearance setting as while it is unset. `setting.value` leaves an
/// unset setting's value out, as for every other setting.
pub const APPEARANCE_DEFAULTS: &[(&str, &str)] = &[
    ("appearance.mode", "system"),
    ("appearance.dots.light", "indigo"),
    ("appearance.dots.dark", "indigo"),
    ("appearance.you.light", "preset"),
    ("appearance.them.light", "preset"),
    ("appearance.you.dark", "preset"),
    ("appearance.them.dark", "preset"),
    ("appearance.edge_glow", "on"),
    ("appearance.motion", "system"),
];

/// The longest name `speaker.name` takes, in characters: a person's name, which Ask's transcript
/// writes before each of their lines.
pub const MAX_SPEAKER_NAME_CHARS: usize = 80;

/// The most commitments `commitments.list` returns when the command names no limit.
pub const DEFAULT_COMMITMENTS_LIMIT: usize = 200;

/// The most it returns at all.
pub const MAX_COMMITMENTS_LIMIT: usize = 1_000;

/// A screen's command, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    /// `permissions.check`: every permission's state now.
    PermissionsCheck,
    /// `permission.request`: the system prompt, or the settings pane.
    PermissionRequest(Permission),
    /// `commitments.list`: the open commitments.
    CommitmentsList {
        /// At most this many.
        limit: usize,
    },
    /// `commitment.set_done`.
    CommitmentSetDone {
        /// The commitment.
        id: String,
        /// Done, or open again.
        done: bool,
    },
    /// `commitment.not_yet`: its looks-done suggestion is dismissed; it stays open.
    CommitmentNotYet {
        /// The commitment.
        id: String,
    },
    /// `note.add`.
    NoteAdd {
        /// The record it belongs to.
        record: String,
        /// Where in the record it was written, ms.
        at_ms: u64,
        /// Its text.
        text: String,
    },
    /// `note.update`.
    NoteUpdate {
        /// The note.
        note: String,
        /// Its new text.
        text: String,
    },
    /// `note.delete`.
    NoteDelete {
        /// The note.
        note: String,
    },
    /// `speaker.name`: a far-end speaker of one record named, renamed, or (`None`) cleared.
    SpeakerName {
        /// The record.
        record: String,
        /// The diarizer's label, as the record's segments carry it.
        speaker: String,
        /// The name, trimmed; `None` clears it.
        name: Option<String>,
    },
    /// `record.delete`: one record, whole ([`crate::retention::delete_one`]).
    RecordDelete {
        /// The record.
        record: String,
    },
    /// `models.list`: the catalogue's models for this OS.
    ModelsList,
    /// `engine.route`: what serves a job now. A router read, so it is here, where a model
    /// download on the command thread never delays it.
    EngineRoute(ink_core::Job),
    /// `setting.get`.
    SettingGet {
        /// One of [`SHELL_SETTINGS`].
        key: String,
    },
    /// `setting.set`.
    SettingSet {
        /// One of [`SHELL_SETTINGS`].
        key: String,
        /// One of the values it accepts.
        value: String,
    },
    /// `hotkey.check`: whether this computer can watch a key binding ([`crate::hotkey`]).
    HotkeyCheck {
        /// The binding, as the shell spelled it.
        binding: String,
    },
    /// `dictation.enable`: dictation live, or its settings read and its keys bound again.
    DictationEnable {
        /// The user's UTC offset, for `{date}` and `{time}` in snippets.
        utc_offset_minutes: Option<i32>,
    },
    /// `dictation.disable`.
    DictationDisable,
    /// `consent.get`: a feature's switch, destination and consent ([`crate::consent::state`]).
    ConsentGet(ink_pipeline::consent::Feature),
    /// `consent.allow`: the user agreed a feature may send where its model goes now.
    ConsentAllow(crate::consent::Allow),
    /// `consent.revoke`: polish may no longer send to one destination ([`crate::consent::revoke`]).
    ConsentRevoke(
        ink_pipeline::consent::Feature,
        ink_pipeline::consent::LlmConsent,
    ),
    /// The library's records, a search, one record, or counts ([`library`](crate::library)).
    Library(crate::library::LibraryQuery),
    /// Snippets, voice commands and the import's key note ([`phrases`](crate::phrases)).
    Phrases(crate::phrases::PhrasesQuery),
    /// Dictation modes: listed, saved and deleted ([`modes`](crate::modes)).
    Modes(crate::modes::ModesQuery),
    /// Own-key language model providers and their keys ([`cloud`](crate::cloud)).
    Cloud(crate::cloud::CloudQuery),
    /// Inkwell 0.2's data: looked for, or imported ([`import02`](crate::import02)).
    Import02(crate::import02::Import02Query),
    /// The Stats screen's numbers ([`stats`](crate::stats)).
    Stats(crate::stats::StatsQuery),
    /// Settings > Sound: the devices and the mic test ([`sound`](crate::sound)).
    Sound(crate::sound::SoundQuery),
}

/// A query with the command's name and id, for its events.
struct Job {
    name: String,
    id: Option<String>,
    query: Query,
}

/// The fields each query takes besides `cmd` and `id`; `None` when `name` is not a query.
fn fields(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "permissions.check" | "models.list" => &[],
        "engine.route" => &["job"],
        "permission.request" => &["permission"],
        "commitments.list" => &["limit"],
        "commitment.set_done" => &["commitment", "done"],
        "commitment.not_yet" => &["commitment"],
        "note.add" => &["record", "at_ms", "text"],
        "note.update" => &["note", "text"],
        "note.delete" => &["note"],
        "speaker.name" => &["record", "speaker", "name"],
        "record.delete" => &["record"],
        "setting.get" => &["key"],
        "setting.set" => &["key", "value"],
        "hotkey.check" => &["binding"],
        "dictation.enable" => &["utc_offset_minutes"],
        "dictation.disable" => &[],
        "consent.get" => &["feature"],
        "consent.allow" => &["feature", "to", "endpoint", "key"],
        "consent.revoke" => &["feature", "to", "endpoint"],
        _ => return None,
    })
}

/// A command read as a query: its name, its `id`, and the query.
pub type Read = (String, Option<String>, Query);

/// Reads a command's JSON as a query: `Ok(None)` when it is some other command (or one the command
/// thread's parser refuses), else the query or why it cannot be read.
pub fn read(json: &str) -> Result<Option<Read>, String> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Ok(None);
    };
    let Some(name) = v.get("cmd").and_then(Value::as_str) else {
        return Ok(None);
    };
    let id = match v.get("id") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("command: \"id\" must be a string".into()),
    };
    match parse(name, &v) {
        None => Ok(None),
        Some(query) => Ok(Some((name.to_owned(), id, query?))),
    }
}

/// Reads `v` as the query named `name`: `None` when `name` is not one of these commands, else the
/// query or why it cannot be read. Unknown fields are refused, as for every other command.
pub fn parse(name: &str, v: &Value) -> Option<Result<Query, String>> {
    if let Some(query) = crate::library::parse(name, v) {
        return Some(query.map(Query::Library));
    }
    if let Some(query) = crate::phrases::parse(name, v) {
        return Some(query.map(Query::Phrases));
    }
    if let Some(query) = crate::modes::parse(name, v) {
        return Some(query.map(Query::Modes));
    }
    if let Some(query) = crate::cloud::parse(name, v) {
        return Some(query.map(Query::Cloud));
    }
    if let Some(query) = crate::import02::parse(name, v) {
        return Some(query.map(Query::Import02));
    }
    if let Some(query) = crate::stats::parse(name, v) {
        return Some(query.map(Query::Stats));
    }
    if let Some(query) = crate::sound::parse(name, v) {
        return Some(query.map(Query::Sound));
    }
    let allowed = fields(name)?;
    Some(parse_known(name, allowed, v))
}

fn parse_known(name: &str, allowed: &[&str], v: &Value) -> Result<Query, String> {
    let obj = v.as_object().ok_or("command: not an object")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["cmd", "id"].contains(&k.as_str()) && !allowed.contains(&k.as_str()))
    {
        return Err(format!("{name}: unknown field \"{k}\""));
    }
    let text = |k: &str| -> Result<String, String> {
        obj.get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{name}: needs a string \"{k}\""))
    };
    Ok(match name {
        "permissions.check" => Query::PermissionsCheck,
        "permission.request" => Query::PermissionRequest(
            parse_permission(&text("permission")?)
                .ok_or_else(|| format!("{name}: unknown permission"))?,
        ),
        "commitments.list" => Query::CommitmentsList {
            limit: match obj.get("limit") {
                None => DEFAULT_COMMITMENTS_LIMIT,
                Some(n) => n
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| (1..=MAX_COMMITMENTS_LIMIT).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "{name}: \"limit\" is a whole number from 1 to {MAX_COMMITMENTS_LIMIT}"
                        )
                    })?,
            },
        },
        "commitment.set_done" => Query::CommitmentSetDone {
            id: text("commitment")?,
            done: obj
                .get("done")
                .and_then(Value::as_bool)
                .ok_or_else(|| format!("{name}: needs \"done\", true or false"))?,
        },
        "commitment.not_yet" => Query::CommitmentNotYet {
            id: text("commitment")?,
        },
        "note.add" => Query::NoteAdd {
            record: text("record")?,
            at_ms: obj
                .get("at_ms")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("{name}: needs \"at_ms\", a whole number of ms"))?,
            text: text("text")?,
        },
        "note.update" => Query::NoteUpdate {
            note: text("note")?,
            text: text("text")?,
        },
        "note.delete" => Query::NoteDelete {
            note: text("note")?,
        },
        "speaker.name" => Query::SpeakerName {
            record: text("record")?,
            speaker: Some(text("speaker")?)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    format!("{name}: \"speaker\" is the diarizer's label, never empty")
                })?,
            name: speaker_name(name, &text("name")?)?,
        },
        "record.delete" => Query::RecordDelete {
            record: text("record")?,
        },
        "models.list" => Query::ModelsList,
        "engine.route" => Query::EngineRoute(
            events::parse_job(&text("job")?).ok_or_else(|| format!("{name}: unknown job"))?,
        ),
        "setting.get" => Query::SettingGet {
            key: shell_setting(name, &text("key")?)?,
        },
        "setting.set" => {
            let key = shell_setting(name, &text("key")?)?;
            let mut value = text("value")?;
            if key == crate::voice::KEY_SETTING
                || (key == crate::voice::EDIT_KEY_SETTING && value != "off")
            {
                // A key is judged by the platform's own parser and stored in its one spelling;
                // the refusal says why, in its words.
                value = crate::hotkey::stored_value(&value)
                    .map_err(|why| format!("{name}: \"{key}\" can't be \"{value}\": {why}"))?;
            }
            let accepted = SHELL_SETTINGS
                .iter()
                .find(|(k, _)| *k == key)
                .map_or(&[][..], |(_, values)| *values);
            if !accepted.iter().any(|allowed| accepts(allowed, &value)) {
                return Err(format!(
                    "{name}: \"{key}\" takes one of: {}",
                    accepted.join(", ")
                ));
            }
            if [
                crate::voice::POLISH_SETTING,
                crate::consent::MEETINGS_SETTING,
            ]
            .contains(&key.as_str())
                && value != "off"
            {
                // Only the user's consent turns polish, or summaries and Ask, on (crate::consent).
                return Err(format!(
                    "{name}: \"{key}\" turns on only through consent.allow, once the user agreed where it sends"
                ));
            }
            Query::SettingSet { key, value }
        }
        "hotkey.check" => Query::HotkeyCheck {
            binding: text("binding")?,
        },
        "dictation.enable" => Query::DictationEnable {
            utc_offset_minutes: match obj.get("utc_offset_minutes") {
                None => None,
                Some(n) => Some(
                    n.as_i64()
                        .and_then(|n| i32::try_from(n).ok())
                        .filter(|n| (-14 * 60..=14 * 60).contains(n))
                        .ok_or_else(|| {
                            format!("{name}: \"utc_offset_minutes\" is minutes from -840 to 840")
                        })?,
                ),
            },
        },
        "dictation.disable" => Query::DictationDisable,
        "consent.get" => Query::ConsentGet(feature(name, &text("feature")?)?),
        "consent.revoke" => Query::ConsentRevoke(
            feature(name, &text("feature")?)?,
            destination(name, obj, &text)?,
        ),
        "consent.allow" => {
            let feature = feature(name, &text("feature")?)?;
            let asked = destination(name, obj, &text)?;
            let key = match obj.get("key") {
                None => None,
                Some(_) => Some(text("key")?),
            };
            Query::ConsentAllow(crate::consent::Allow {
                feature,
                asked,
                key: crate::consent::check_key(feature, key.as_deref())
                    .map_err(|e| format!("{name}: {e}"))?,
            })
        }
        _ => unreachable!("fields() lists every query"),
    })
}

/// A speaker's name as `speaker.name` takes it: trimmed, `None` when nothing is left (cleared).
/// Refused over two lines or more, or with any other control character (Ask's transcript writes
/// it before each of their lines, one line per turn), or longer than [`MAX_SPEAKER_NAME_CHARS`]. The error never quotes it.
fn speaker_name(command: &str, raw: &str) -> Result<Option<String>, String> {
    let name = raw.trim();
    // A line or paragraph separator breaks a line as surely as a line feed does.
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(format!(
            "{command}: \"name\" is one line, without control characters"
        ));
    }
    if name.chars().count() > MAX_SPEAKER_NAME_CHARS {
        return Err(format!(
            "{command}: \"name\" is at most {MAX_SPEAKER_NAME_CHARS} characters"
        ));
    }
    Ok((!name.is_empty()).then(|| name.to_owned()))
}

/// The destination a consent command names: `"to":"on_device"`, or `"to":"cloud"` with the
/// `"endpoint"` `consent.state` (or `modes.listed`) gave. Its name is not needed to compare: what
/// is recorded is the model's own.
fn destination(
    name: &str,
    obj: &serde_json::Map<String, Value>,
    text: &dyn Fn(&str) -> Result<String, String>,
) -> Result<ink_pipeline::consent::LlmConsent, String> {
    match text("to")?.as_str() {
        "on_device" if !obj.contains_key("endpoint") => {
            Ok(ink_pipeline::consent::LlmConsent::OnDevice)
        }
        "cloud" => Ok(ink_pipeline::consent::LlmConsent::Cloud {
            endpoint: text("endpoint")?,
            name: String::new(),
        }),
        _ => Err(format!(
            "{name}: \"to\" is on_device, or cloud with the \"endpoint\" consent.state gave"
        )),
    }
}

/// A feature the consent commands serve: one with a switch ([`crate::consent::switch`]).
fn feature(name: &str, feature: &str) -> Result<ink_pipeline::consent::Feature, String> {
    ink_pipeline::consent::Feature::parse(feature)
        .filter(|f| crate::consent::switch(*f).is_some())
        .ok_or_else(|| format!("{name}: \"feature\" is polish, edit or meetings"))
}

/// Whether `allowed`, one entry of a value list in [`SHELL_SETTINGS`], accepts `value`.
fn accepts(allowed: &str, value: &str) -> bool {
    if allowed == ANY_DEVICE {
        crate::devices::is_device_token(value)
    } else if allowed == ANY_KEY {
        crate::hotkey::stored_value(value).is_ok()
    } else if allowed == TYPING_WPM {
        // Digits only, so the stored text reads back as the number it is ("040" and "+40" are
        // refused rather than kept in two spellings).
        !value.starts_with('0')
            && value.bytes().all(|b| b.is_ascii_digit())
            && value
                .parse::<u32>()
                .is_ok_and(|w| crate::stats::TYPING_WPM_RANGE.contains(&w))
    } else if allowed == REST_DAYS {
        crate::stats::rest_days(value).is_some()
    } else if allowed == DATE {
        crate::stats::Calendar::parse_date(value).is_some()
    } else if allowed == HEX_COLOUR {
        // `#` and six lowercase hex digits; the pattern itself is not a colour.
        value.len() == 7
            && value.starts_with('#')
            && value[1..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    } else {
        allowed == value
    }
}

/// `key` if the shell may use it.
fn shell_setting(name: &str, key: &str) -> Result<String, String> {
    if SHELL_SETTINGS.iter().any(|(k, _)| *k == key) {
        Ok(key.to_owned())
    } else {
        Err(format!(
            "{name}: \"{key}\" is not a setting the shell may use"
        ))
    }
}

/// A permission for its schema name.
pub fn parse_permission(name: &str) -> Option<Permission> {
    Some(match name {
        "microphone" => Permission::Microphone,
        "system_audio" => Permission::SystemAudio,
        "accessibility" => Permission::Accessibility,
        "input_monitoring" => Permission::InputMonitoring,
        _ => return None,
    })
}

/// A permission's schema name.
fn permission(p: Permission) -> &'static str {
    match p {
        Permission::Microphone => "microphone",
        Permission::SystemAudio => "system_audio",
        Permission::Accessibility => "accessibility",
        Permission::InputMonitoring => "input_monitoring",
        // Non-exhaustive: a permission this build cannot name is never parsed, so never asked.
        _ => "unknown",
    }
}

/// A permission state's schema name.
fn state(s: PermissionState) -> &'static str {
    match s {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        PermissionState::NotDetermined => "not_determined",
        PermissionState::Unknown => "unknown",
    }
}

/// The permission probe for platforms without one: every state is unknown, and nothing can be
/// asked for. Nothing is made up.
pub struct NoPermissionProbe;

impl PermissionProbe for NoPermissionProbe {
    fn check(&self, _: Permission) -> PermissionState {
        PermissionState::Unknown
    }

    fn request(&self, _: Permission) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported(
            "this platform has no permission probe yet",
        ))
    }
}

/// Names a far-end speaker of `record`, or clears its name. A name goes only to a label the
/// record's current transcript gives the far end: the mic is the user, never renamed, and a label a
/// later pass dropped names nobody. A clear needs no such label, so a stale name can still go.
/// Errors name the record and the label, never the name.
fn name_speaker(
    store: &dyn Store,
    record: &str,
    speaker: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let id = RecordId(record.to_owned());
    let label = SpeakerId(speaker.to_owned());
    let Some(name) = name else {
        return store
            .clear_speaker_name(&id, &label)
            .map_err(|e| e.to_string());
    };
    let said = store.segments(&id).map_err(|e| e.to_string())?;
    if !said
        .iter()
        .any(|s| s.channel == Channel::Far && s.speaker.as_ref() == Some(&label))
    {
        return Err(format!(
            "record {record} has no far-end speaker {speaker} in its transcript"
        ));
    }
    store
        .set_speaker_name(&id, &label, name)
        .map_err(|e| e.to_string())
}

/// The thread that runs the queries.
pub struct QueryWorker {
    tx: Sender<Job>,
    thread: JoinHandle<()>,
}

impl QueryWorker {
    /// Starts `ink-queries`. It holds `shared` until [`stop`](Self::stop).
    pub fn start(
        shared: Arc<Shared>,
        probe: Arc<dyn PermissionProbe>,
        models: ModelDir,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = thread::Builder::new()
            .name("ink-queries".into())
            .spawn(move || {
                let ctx = Ctx {
                    shared: &shared,
                    probe: probe.as_ref(),
                    models: &models,
                };
                while let Ok(job) = rx.recv() {
                    ctx.guarded(job);
                }
            })?;
        Ok(Self { tx, thread })
    }

    /// Queues a query.
    pub fn send(&self, name: String, id: Option<String>, query: Query) -> Result<(), String> {
        self.tx
            .send(Job { name, id, query })
            .map_err(|_| "the queries thread has stopped".to_owned())
    }

    /// Runs what is queued, then ends the thread. Its events are emitted before this returns.
    pub fn stop(self) {
        drop(self.tx);
        if self.thread.join().is_err() {
            log::error!("the queries thread panicked outside its per-query boundary");
        }
    }
}

struct Ctx<'a> {
    shared: &'a Arc<Shared>,
    probe: &'a dyn PermissionProbe,
    models: &'a ModelDir,
}

impl Ctx<'_> {
    /// One query behind a panic boundary, as the command thread runs commands.
    fn guarded(&self, job: Job) {
        let (name, id) = (job.name.clone(), job.id.clone());
        if panic::catch_unwind(AssertUnwindSafe(|| self.run(job))).is_err() {
            // The payload is not logged: it could hold a note's words (I5).
            log::error!("command {name} panicked; the next one still runs");
            self.shared.events.emit(events::command_failed(
                &name,
                id.as_deref(),
                "a bug in the core stopped this command",
            ));
        }
    }

    fn run(&self, job: Job) {
        let Job { name, id, query } = job;
        let emit = |e: Value| self.shared.events.emit(e);
        let fail_coded = |message: String, code: Option<&str>| {
            log::warn!("command {name} failed: {message}");
            emit(events::command_failed_coded(
                &name,
                id.as_deref(),
                &message,
                code,
            ));
        };
        let fail = |message: String| fail_coded(message, None);
        let store = self.shared.store.as_ref();
        match query {
            Query::PermissionsCheck => emit(self.permissions()),
            Query::PermissionRequest(p) => match self.probe.request(p) {
                Ok(()) => {
                    if p == Permission::SystemAudio
                        && let Err(e) = store.set_setting(SYSTEM_AUDIO_ASKED_KEY, "true")
                    {
                        // The probe remembers for this run; the next launch would treat System
                        // Audio as never asked (and show it as such) until the next request.
                        log::error!("could not remember that system audio was asked for: {e}");
                    }
                    emit(event(
                        "permission.requested",
                        &[("permission", Some(permission(p).into()))],
                    ));
                }
                Err(e) => fail(e.to_string()),
            },
            Query::CommitmentsList { limit } => match commitments(store, limit) {
                Ok(e) => emit(e),
                Err(e) => fail(e),
            },
            Query::CommitmentSetDone {
                id: commitment,
                done,
            } => match store.set_commitment_done(&CommitmentId(commitment.clone()), done) {
                Ok(()) => emit(event(
                    "commitment.updated",
                    &[
                        ("commitment", Some(commitment.into())),
                        ("done", Some(done.into())),
                    ],
                )),
                Err(e) => fail(e.to_string()),
            },
            Query::CommitmentNotYet { id: commitment } => {
                match store.set_done_evidence(&CommitmentId(commitment.clone()), None) {
                    Ok(()) => emit(event(
                        "commitment.updated",
                        &[
                            ("commitment", Some(commitment.into())),
                            ("done", Some(false.into())),
                        ],
                    )),
                    Err(e) => fail(e.to_string()),
                }
            }
            Query::NoteAdd {
                record,
                at_ms,
                text,
            } => match store.add_note(&RecordId(record.clone()), at_ms, &text) {
                Ok(note) => emit(event(
                    "note.added",
                    &[
                        ("record", Some(record.into())),
                        ("note", Some(note.0.into())),
                        ("at_ms", Some(at_ms.into())),
                        ("ref", id.clone().map(Into::into)),
                    ],
                )),
                Err(e) => fail(e.to_string()),
            },
            Query::NoteUpdate { note, text } => {
                match store.update_note(&NoteId(note.clone()), &text) {
                    Ok(()) => emit(event(
                        "note.updated",
                        &[
                            ("note", Some(note.into())),
                            ("ref", id.clone().map(Into::into)),
                        ],
                    )),
                    Err(e) => fail(e.to_string()),
                }
            }
            Query::NoteDelete { note } => match store.delete_note(&NoteId(note.clone())) {
                Ok(()) => emit(event(
                    "note.deleted",
                    &[
                        ("note", Some(note.into())),
                        ("ref", id.clone().map(Into::into)),
                    ],
                )),
                Err(e) => fail(e.to_string()),
            },
            Query::SpeakerName {
                record,
                speaker,
                name,
            } => match name_speaker(store, &record, &speaker, name.as_deref()) {
                Ok(()) => emit(event(
                    "speaker.named",
                    &[
                        ("record", Some(record.into())),
                        ("speaker", Some(speaker.into())),
                        ("named", Some(name.is_some().into())),
                        ("ref", id.clone().map(Into::into)),
                    ],
                )),
                Err(e) => fail(e),
            },
            Query::RecordDelete { record } => {
                match crate::retention::delete_one(self.shared, &RecordId(record.clone())) {
                    Ok(deleted) => emit(event(
                        "record.deleted",
                        &[
                            ("record", Some(record.into())),
                            ("kind", Some(crate::library::kind_name(deleted.kind).into())),
                            ("audio_left", Some(deleted.audio_left.into())),
                            ("scrubbed", Some(deleted.scrubbed.into())),
                            ("ref", id.clone().map(Into::into)),
                        ],
                    )),
                    Err(e) => fail(e),
                }
            }
            Query::ModelsList => emit(self.catalogue(id.as_deref())),
            Query::EngineRoute(job) => emit(routed(self.shared, job)),
            Query::SettingGet { key } => match if crate::calls::is_calls_setting(&key) {
                crate::calls::setting_value(store, &key)
            } else {
                store.setting(&key).map_err(|e| e.to_string())
            } {
                Ok(value) => emit(setting(&key, value)),
                Err(e) => fail(e),
            },
            Query::SettingSet { key, value } => {
                match match crate::consent::feature_switched_by(&key) {
                    // A feature's switch turned off: with its consent withdrawn, in one write. (Polish
                    // is only ever set to off: the parser refuses on.)
                    Some(feature) if value == "off" => {
                        crate::consent::turn_off(self.shared, feature)
                    }
                    // A device is checked against those connected now, and remembered.
                    _ if [crate::devices::INPUT_KEY, crate::devices::OUTPUT_KEY]
                        .contains(&key.as_str()) =>
                    {
                        crate::sound::set_choice(self.shared, &key, &value)
                    }
                    _ if key == crate::control::DETECT_KEY => {
                        crate::calls::set_detect(store, &value)
                    }
                    _ => store.set_setting(&key, &value).map_err(|e| e.to_string()),
                } {
                    Ok(()) => {
                        if key == crate::llms::LOCAL_ONLY_KEY {
                            self.shared.local_only.set(value != "off");
                        }
                        let calls = crate::calls::is_calls_setting(&key);
                        let sweep = key == crate::retention::RETENTION_KEY;
                        let switched = crate::consent::feature_switched_by(&key);
                        emit(setting(&key, Some(value)));
                        if calls {
                            if key == crate::control::DETECT_KEY {
                                // The default it set, for a screen that shows the default.
                                let default = crate::calls::DEFAULT_KEY;
                                match crate::calls::setting_value(store, default) {
                                    Ok(v) => emit(setting(default, v)),
                                    Err(e) => log::warn!("call policies: the default: {e}"),
                                }
                            }
                            self.shared
                                .tell_meetings(crate::control::Msg::Calls { announce: true });
                        }
                        if let Some(feature) = switched {
                            emit(crate::consent::state(self.shared, feature, None));
                        }
                        if sweep {
                            self.shared.sweep_soon();
                        }
                        if crate::voice::DICTATION_SETTINGS.contains(&key.as_str()) {
                            crate::voice::settings_changed(self.shared);
                        }
                        if [crate::devices::INPUT_KEY, crate::devices::OUTPUT_KEY]
                            .contains(&key.as_str())
                        {
                            crate::sound::choice_changed(self.shared, &key);
                        }
                    }
                    Err(e) => fail(e),
                }
            }
            Query::HotkeyCheck { binding } => emit(crate::hotkey::checked(
                &binding,
                crate::hotkey::check(&binding),
                id.as_deref(),
            )),
            Query::DictationEnable { utc_offset_minutes } => {
                crate::voice::enable(self.shared, self.models, utc_offset_minutes, id.as_deref())
            }
            Query::DictationDisable => crate::voice::disable(self.shared, id.as_deref()),
            Query::ConsentGet(feature) => {
                emit(crate::consent::state(self.shared, feature, id.as_deref()))
            }
            Query::ConsentAllow(asked) => {
                match crate::consent::allow(self.shared, &asked, id.as_deref()) {
                    Ok(e) => emit(e),
                    Err(e) => fail(e),
                }
            }
            Query::ConsentRevoke(feature, asked) => {
                match crate::consent::revoke(self.shared, feature, &asked, id.as_deref()) {
                    Ok(e) => emit(e),
                    Err(e) => fail(e),
                }
            }
            Query::Library(query) => {
                match crate::library::answer(self.shared, query, id.as_deref()) {
                    Ok(e) => emit(e),
                    Err(e) => fail(e),
                }
            }
            Query::Phrases(query) => {
                let saves = query.saves();
                match crate::phrases::answer(store, query, id.as_deref()) {
                    Ok(e) => {
                        emit(e);
                        // A running dictation takes the new list at once.
                        if saves {
                            crate::voice::settings_changed(self.shared);
                        }
                    }
                    Err(e) => fail_coded(e.message, e.code),
                }
            }
            Query::Modes(query) => {
                let saves = query.saves();
                match crate::modes::answer(self.shared, query, id.as_deref()) {
                    Ok(e) => {
                        emit(e);
                        // A running dictation takes the change at once (a deleted mode's pin too).
                        if saves {
                            crate::voice::settings_changed(self.shared);
                        }
                    }
                    Err(e) => fail_coded(e.message, e.code),
                }
            }
            Query::Cloud(query) => match crate::cloud::answer(self.shared, query, id.as_deref()) {
                Ok(Some(e)) => emit(e),
                // llm.test: the test thread answers.
                Ok(None) => {}
                Err(e) => fail(e),
            },
            Query::Stats(query) => match crate::stats::answer(self.shared, query, id.as_deref()) {
                Ok(e) => emit(e),
                Err(e) => fail(e),
            },
            Query::Sound(query) => match crate::sound::answer(self.shared, query, id.as_deref()) {
                Ok(Some(e)) => emit(e),
                // audio.test, audio.test_stop: the sound thread answers.
                Ok(None) => {}
                Err(e) => fail(e),
            },
            // A store call and a read of 0.2's files: about a second for a long history, and
            // apart from the command thread, which a model download can hold for minutes.
            Query::Import02(query) => {
                let import = self.shared.import02.get();
                match crate::import02::answer(import, query, id.as_deref()) {
                    Ok(e) => {
                        // Dictations are all that milestones count of an import.
                        let dictations = e
                            .get("counts")
                            .and_then(|c| c.get("dictations"))
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        // The imported words are history: the next milestone check notes what
                        // they reach without celebrating it. Set before the answer goes out, so a
                        // check the shell sends on it finds the flag; only when words came over,
                        // so an empty import never swallows a milestone reached since. Not in the
                        // import's transaction: a failure here (logged) only costs a celebration
                        // the import did not earn.
                        if query.imports()
                            && dictations > 0
                            && let Err(err) =
                                store.set_setting(crate::stats::MILESTONES_AFRESH_KEY, "yes")
                        {
                            log::warn!("import: milestones could not be noted afresh: {err}");
                        }
                        emit(e);
                        if query.imports() {
                            // A running dictation takes the imported key and lists at once.
                            crate::voice::settings_changed(self.shared);
                        }
                    }
                    Err(e) => fail(e),
                }
            }
        }
    }

    /// `permissions.checked`. Never prompts (the probe's contract); System Audio takes about a
    /// second once it has been asked for.
    fn permissions(&self) -> Value {
        let check = |p: Permission| Some(Value::from(state(self.probe.check(p))));
        event(
            "permissions.checked",
            &[
                ("microphone", check(Permission::Microphone)),
                ("system_audio", check(Permission::SystemAudio)),
                ("accessibility", check(Permission::Accessibility)),
                ("input_monitoring", check(Permission::InputMonitoring)),
            ],
        )
    }

    /// `models.listed`: every registry model this OS runs, with whether it is installed, and the
    /// free space where models go.
    fn catalogue(&self, reference: Option<&str>) -> Value {
        catalogue(self.shared, self.models, reference)
    }
}

/// **Worker.** `models.listed`: every registry model this OS runs, with whether it is installed
/// in `models`, what it is for (a language model with its name and whether it is the suggested
/// size), and the free space where models go.
pub(crate) fn catalogue(shared: &Shared, models: &ModelDir, reference: Option<&str>) -> Value {
    let os = Os::current();
    let suggested = crate::models::suggested(shared).map(|row| row.id.clone());
    let list: Vec<Value> = shared
        .registry
        .rows()
        .iter()
        .filter(|row| os.is_some_and(|os| row.runs_on(os)))
        .map(|row| {
            let mut entry = json!({
                "id": row.id,
                "kind": if crate::models::is_language(row) { "language" } else { "speech" },
                "licence": row.licence,
                "size_bytes": row.total_size(),
                "installed": models.is_installed(row),
                "jobs": row
                    .scores
                    .iter()
                    .map(|s| json!({"job": events::job(s.job), "wer": s.wer}))
                    .collect::<Vec<_>>(),
            });
            if let RowKind::Language(language) = &row.kind {
                entry["name"] = language.name.as_str().into();
                entry["suggested"] = (suggested.as_deref() == Some(row.id.as_str())).into();
            }
            entry
        })
        .collect();
    event(
        "models.listed",
        &[
            ("models", Some(Value::Array(list))),
            (
                "free_bytes",
                crate::models::free_bytes(shared).map(Into::into),
            ),
            ("ref", reference.map(Into::into)),
        ],
    )
}

/// `setting.value`.
pub(crate) fn setting_value(key: &str, value: Option<String>) -> Value {
    setting(key, value)
}

fn setting(key: &str, value: Option<String>) -> Value {
    event(
        "setting.value",
        &[("key", Some(key.into())), ("value", value.map(Into::into))],
    )
}

/// `engine.routed`: what serves `job` now: a model downloaded from the registry, an engine the
/// shell registered (such as a fallback while that model downloads), or nothing installed.
fn routed(shared: &Shared, job: ink_core::Job) -> Value {
    let (id, source) = match shared.router.route(job) {
        Ok(Route::Model(row)) => (Some(row.id.clone()), Some("registry")),
        Ok(Route::External { id, .. }) => (Some(id), Some("shell")),
        Err(_) => (None, None),
    };
    event(
        "engine.routed",
        &[
            ("job", Some(events::job(job).into())),
            ("id", id.map(Into::into)),
            ("source", source.map(Into::into)),
        ],
    )
}

/// Puts `value` under `key` when there is one: optional fields are left out, never null.
fn put(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        map.insert(key.into(), value);
    }
}

/// `commitments.listed`: the open commitments (not done, not merged into another), soonest due
/// first, each with its record's title and start and how many others were merged into it.
///
/// Merges are counted within the records that hold an open commitment. A commitment merged into
/// one from another record that holds no open commitment of its own is not counted: the store
/// has no reverse lookup, and reading every record for it would grow with the whole library.
fn commitments(store: &dyn Store, limit: usize) -> Result<Value, String> {
    let open = store.open_commitments(limit).map_err(|e| e.to_string())?;
    let mut records: BTreeMap<RecordId, (Option<String>, i64)> = BTreeMap::new();
    let mut merged: BTreeMap<CommitmentId, u64> = BTreeMap::new();
    for c in &open {
        if records.contains_key(&c.record) {
            continue;
        }
        let Some(record) = store.record(&c.record).map_err(|e| e.to_string())? else {
            // Deleting a record deletes its commitments; one listed without its record is a
            // race with that delete, and it is gone by the next list.
            continue;
        };
        records.insert(c.record.clone(), (record.title, record.started_at_unix_ms));
        for other in store.commitments(&c.record).map_err(|e| e.to_string())? {
            if let Some(into) = other.merged_into {
                *merged.entry(into).or_default() += 1;
            }
        }
    }
    let items: Vec<Value> = open
        .iter()
        .filter_map(|c| {
            let (title, started) = records.get(&c.record)?;
            Some(owed(
                store,
                c,
                title.as_deref(),
                *started,
                merged.get(&c.id).copied(),
            ))
        })
        .collect();
    Ok(event(
        "commitments.listed",
        &[("items", Some(Value::Array(items)))],
    ))
}

fn owed(
    store: &dyn Store,
    c: &Commitment,
    title: Option<&str>,
    started: i64,
    merged: Option<u64>,
) -> Value {
    let mut item = Map::new();
    item.insert("id".into(), c.id.0.clone().into());
    item.insert("record".into(), c.record.0.clone().into());
    put(&mut item, "record_title", title.map(Into::into));
    item.insert("record_started_at_unix_ms".into(), started.into());
    item.insert("text".into(), c.text.clone().into());
    put(&mut item, "owner", c.owner.clone().map(Into::into));
    put(&mut item, "recipient", c.recipient.clone().map(Into::into));
    put(&mut item, "due", c.due.clone().map(Into::into));
    put(
        &mut item,
        "due_at_unix_ms",
        c.due_at_unix_ms.map(Into::into),
    );
    if let Some(span) = c.provenance.iter().min_by_key(|s| s.start_ms) {
        item.insert("said_at_ms".into(), span.start_ms.into());
        item.insert("channel".into(), events::channel(span.channel).into());
    }
    item.insert("merged".into(), merged.unwrap_or(0).into());
    put(
        &mut item,
        "looks_done",
        c.looks_done
            .as_ref()
            .map(|e| crate::library::done_evidence(store, e)),
    );
    Value::Object(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_parse_and_refuse_what_they_cannot_read() {
        let p = |json: &str| {
            let v: Value = serde_json::from_str(json).unwrap();
            let name = v["cmd"].as_str().unwrap().to_owned();
            parse(&name, &v)
        };
        assert!(p(r#"{"cmd":"model.warm","job":"dictation_final"}"#).is_none());
        assert_eq!(
            p(r#"{"cmd":"permissions.check","id":"c"}"#),
            Some(Ok(Query::PermissionsCheck))
        );
        assert_eq!(
            p(r#"{"cmd":"permission.request","permission":"system_audio"}"#),
            Some(Ok(Query::PermissionRequest(Permission::SystemAudio)))
        );
        assert_eq!(
            p(r#"{"cmd":"commitments.list"}"#),
            Some(Ok(Query::CommitmentsList {
                limit: DEFAULT_COMMITMENTS_LIMIT
            }))
        );
        assert_eq!(
            p(r#"{"cmd":"note.add","record":"r","at_ms":1200,"text":"hi"}"#),
            Some(Ok(Query::NoteAdd {
                record: "r".into(),
                at_ms: 1200,
                text: "hi".into()
            }))
        );
        assert_eq!(
            p(r#"{"cmd":"speaker.name","record":"r","speaker":"spk1","name":" Robin\t"}"#),
            Some(Ok(Query::SpeakerName {
                record: "r".into(),
                speaker: "spk1".into(),
                name: Some("Robin".into())
            }))
        );
        assert_eq!(
            p(r#"{"cmd":"speaker.name","record":"r","speaker":"spk1","name":"  "}"#),
            Some(Ok(Query::SpeakerName {
                record: "r".into(),
                speaker: "spk1".into(),
                name: None
            })),
            "nothing left once trimmed: cleared"
        );
        let longest = "é".repeat(MAX_SPEAKER_NAME_CHARS);
        assert!(
            matches!(
                parse(
                    "speaker.name",
                    &json!({"cmd": "speaker.name", "record": "r", "speaker": "spk1", "name": longest})
                ),
                Some(Ok(_))
            ),
            "counted in characters, not bytes"
        );
        assert_eq!(
            p(r#"{"cmd":"setting.set","key":"dictation.polish","value":"off"}"#),
            Some(Ok(Query::SettingSet {
                key: "dictation.polish".into(),
                value: "off".into()
            }))
        );
        use crate::consent::Allow;
        use ink_pipeline::consent::{Feature, LlmConsent};
        assert_eq!(
            p(r#"{"cmd":"consent.allow","feature":"polish","to":"on_device","id":"a"}"#),
            Some(Ok(Query::ConsentAllow(Allow {
                feature: Feature::Polish,
                asked: LlmConsent::OnDevice,
                key: None
            })))
        );
        assert_eq!(
            p(
                r#"{"cmd":"consent.allow","feature":"edit","to":"cloud","endpoint":"shell engine x","key":"right_command"}"#
            ),
            Some(Ok(Query::ConsentAllow(Allow {
                feature: Feature::Edit,
                asked: LlmConsent::Cloud {
                    endpoint: "shell engine x".into(),
                    name: String::new()
                },
                key: Some("right_command".into())
            })))
        );
        assert_eq!(
            p(r#"{"cmd":"consent.get","feature":"edit"}"#),
            Some(Ok(Query::ConsentGet(Feature::Edit)))
        );
        assert_eq!(
            p(
                r#"{"cmd":"consent.revoke","feature":"polish","to":"cloud","endpoint":"shell engine x"}"#
            ),
            Some(Ok(Query::ConsentRevoke(
                Feature::Polish,
                LlmConsent::Cloud {
                    endpoint: "shell engine x".into(),
                    name: String::new()
                }
            )))
        );
        assert_eq!(
            p(r#"{"cmd":"consent.revoke","feature":"polish","to":"on_device"}"#),
            Some(Ok(Query::ConsentRevoke(
                Feature::Polish,
                LlmConsent::OnDevice
            )))
        );
        for bad in [
            r#"{"cmd":"consent.revoke","feature":"polish"}"#,
            r#"{"cmd":"consent.revoke","feature":"polish","to":"cloud"}"#,
            r#"{"cmd":"consent.revoke","feature":"polish","to":"on_device","endpoint":"x"}"#,
            r#"{"cmd":"consent.revoke","feature":"polish","to":"on_device","key":"fn"}"#,
            r#"{"cmd":"consent.revoke","feature":"summary","to":"on_device"}"#,
        ] {
            assert!(matches!(p(bad), Some(Err(_))), "{bad} must be refused");
        }
        assert_eq!(
            p(r#"{"cmd":"consent.allow","feature":"meetings","to":"on_device"}"#),
            Some(Ok(Query::ConsentAllow(Allow {
                feature: Feature::Meetings,
                asked: LlmConsent::OnDevice,
                key: None
            })))
        );
        assert_eq!(
            p(r#"{"cmd":"setting.set","key":"meetings.llm","value":"off"}"#),
            Some(Ok(Query::SettingSet {
                key: "meetings.llm".into(),
                value: "off".into()
            }))
        );
        assert_eq!(
            p(r#"{"cmd":"engine.route","job":"live_partials","id":"r"}"#),
            Some(Ok(Query::EngineRoute(ink_core::Job::LivePartials)))
        );
        for bad in [
            r#"{"cmd":"engine.route"}"#,
            r#"{"cmd":"engine.route","job":"typing"}"#,
            r#"{"cmd":"engine.route","job":"live_partials","engine":"x"}"#,
            // Only the user's consent turns polish on: consent.allow, never setting.set.
            r#"{"cmd":"setting.set","key":"dictation.polish","value":"on"}"#,
            r#"{"cmd":"setting.set","key":"llm.consent.polish","value":"{\"to\":\"on_device\"}"}"#,
            r#"{"cmd":"setting.set","key":"llm.consent.edit","value":"none"}"#,
            // Nor summaries and Ask.
            r#"{"cmd":"setting.set","key":"meetings.llm","value":"on"}"#,
            r#"{"cmd":"setting.set","key":"llm.consent.meetings","value":"{\"to\":\"on_device\"}"}"#,
            r#"{"cmd":"consent.allow","feature":"meetings","to":"on_device","key":"fn"}"#,
            r#"{"cmd":"consent.get"}"#,
            r#"{"cmd":"consent.get","feature":"summary"}"#,
            r#"{"cmd":"consent.allow","feature":"polish"}"#,
            r#"{"cmd":"consent.allow","feature":"polish","to":"cloud"}"#,
            r#"{"cmd":"consent.allow","feature":"polish","to":"everywhere"}"#,
            r#"{"cmd":"consent.allow","feature":"polish","to":"on_device","endpoint":"https://api.example.com"}"#,
            r#"{"cmd":"consent.allow","feature":"polish","to":"on_device","key":"fn"}"#,
            r#"{"cmd":"consent.allow","feature":"edit","to":"on_device"}"#,
            r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"off"}"#,
            r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"left_shift"}"#,
            r#"{"cmd":"permissions.check","deep":true}"#,
            r#"{"cmd":"permission.request","permission":"camera"}"#,
            r#"{"cmd":"permission.request"}"#,
            r#"{"cmd":"commitments.list","limit":0}"#,
            r#"{"cmd":"commitments.list","limit":"5"}"#,
            r#"{"cmd":"commitment.set_done","commitment":"c","done":"yes"}"#,
            r#"{"cmd":"note.add","record":"r","at_ms":-1,"text":"hi"}"#,
            r#"{"cmd":"note.add","record":"r","text":"hi"}"#,
            r#"{"cmd":"note.update","note":"n"}"#,
            r#"{"cmd":"speaker.name","record":"r","speaker":"spk1"}"#,
            r#"{"cmd":"speaker.name","record":"r","name":"A"}"#,
            r#"{"cmd":"speaker.name","speaker":"spk1","name":"A"}"#,
            r#"{"cmd":"speaker.name","record":"r","speaker":"","name":"A"}"#,
            r#"{"cmd":"speaker.name","record":"r","speaker":"spk1","name":null}"#,
            r#"{"cmd":"speaker.name","record":"r","speaker":"spk1","name":"A\nB"}"#,
            r#"{"cmd":"speaker.name","record":"r","speaker":"spk1","name":"A\u0000"}"#,
            r#"{"cmd":"speaker.name","record":"r","speaker":"spk1","name":"A\u2028L9 [00:00] You: B"}"#,
            r#"{"cmd":"speaker.name","record":"r","speaker":"spk1","name":"A\u2029B"}"#,
            r#"{"cmd":"setting.get","key":"permissions.system_audio_asked"}"#,
            r#"{"cmd":"setting.set","key":"dictation.polish","value":"maybe"}"#,
            r#"{"cmd":"setting.set","key":"library.path","value":"/x"}"#,
        ] {
            assert!(matches!(p(bad), Some(Err(_))), "{bad} must be refused");
        }
    }

    #[test]
    fn appearance_settings_take_their_values_and_default_to_one_of_them() {
        let set = |key: &str, value: &str| {
            parse(
                "setting.set",
                &json!({"cmd": "setting.set", "key": key, "value": value}),
            )
        };
        for (key, value) in [
            ("appearance.mode", "dark"),
            ("appearance.mode", "system"),
            ("appearance.dots.light", "indigo"),
            ("appearance.dots.dark", "ink_sand"),
            ("appearance.you.light", "preset"),
            ("appearance.them.light", "#ffa34d"),
            ("appearance.you.dark", "#0a1b2c"),
            ("appearance.them.dark", "preset"),
            ("appearance.edge_glow", "off"),
            ("appearance.motion", "still"),
        ] {
            assert_eq!(
                set(key, value),
                Some(Ok(Query::SettingSet {
                    key: key.into(),
                    value: value.into()
                })),
                "{key} = {value}"
            );
        }
        for (key, value) in [
            ("appearance.mode", "auto"),
            ("appearance.dots.light", "Indigo"),
            ("appearance.dots.light", "#6b5cff"),
            ("appearance.dots.dark", "preset"),
            // Lowercase hex only, exactly six digits after the #.
            ("appearance.you.light", "#6B5CFF"),
            ("appearance.you.light", "#6b5cf"),
            ("appearance.you.light", "#6b5cff0"),
            ("appearance.you.light", "6b5cff"),
            ("appearance.you.light", "#6b5cfg"),
            ("appearance.them.dark", "#rrggbb"),
            ("appearance.them.dark", "indigo"),
            ("appearance.edge_glow", "true"),
            ("appearance.motion", "reduce"),
            ("appearance.accent", "preset"),
        ] {
            assert!(
                matches!(set(key, value), Some(Err(_))),
                "{key} = {value} must be refused"
            );
        }
        // Every appearance setting has a default, and it is one of the setting's values.
        let appearance: Vec<_> = SHELL_SETTINGS
            .iter()
            .filter(|(key, _)| key.starts_with("appearance."))
            .collect();
        assert_eq!(appearance.len(), APPEARANCE_DEFAULTS.len());
        for (key, values) in appearance {
            let (_, default) = APPEARANCE_DEFAULTS
                .iter()
                .find(|(k, _)| k == key)
                .unwrap_or_else(|| panic!("{key} has no default"));
            assert!(
                values.iter().any(|allowed| accepts(allowed, default)),
                "{key}'s default {default}"
            );
        }
    }

    #[test]
    fn the_dot_presets_are_the_design_tokens_presets() {
        let tokens: Value =
            serde_json::from_str(include_str!("../../../../design/tokens.json")).unwrap();
        let ids: Vec<&str> = tokens["presets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|preset| preset["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, DOT_PRESETS);
    }
}
