//! Call policies: for each app that takes the microphone for a call, whether its calls are
//! recorded at once ("Always record"), offered (the consent Drop: "Ask"), or left alone ("Never"),
//! and the policy for apps the user has not chosen for.
//!
//! | Setting | Holds | Written by |
//! |---|---|---|
//! | [`DEFAULT_KEY`] | `ask` (also when unset), `always` or `never` | the shell, through `setting.set` (the queries thread); and the meetings thread once, lowering Always to Ask as a list set aside is started over |
//! | [`APPS_KEY`] | the apps seen and chosen for, as JSON | the meetings thread alone: `meetings.calls.set`, and the apps detection sees |
//!
//! Two keys, two writers: neither ever reads, changes and writes back what the other holds, but
//! for that one lowering, which reads the stored default just before it writes (with the list,
//! in one transaction) and writes Ask unless the store positively says Ask or Never. A read that
//! fails lowers too, whatever the store holds: a stored Never becomes Ask then (detection listens,
//! and offers; it never records), as a lost Always must never stand. The store has no
//! compare-and-set, so a default the user sets in the instant between the read and the write is
//! lost to Ask the same way, and nothing ever becomes Always. The thread's own default follows at
//! once, never what it last read: the store's Ask or Never, else the Ask it wrote.
//!
//! - **Keyed by identity.** An app is the identity detection reports ([`AppRef::id`]: the bundle
//!   id on the Mac, the executable on Windows), never its name, which only labels it. On Windows
//!   it is lowercased in ASCII where it enters the core ([`identity`]), as the platform matches
//!   executables (`Zoom.exe` and `zoom.exe` are one app, so a Never cannot be missed by a change
//!   of case); its name is kept as given. A stored list from before that holds one app twice in
//!   two cases is merged as read, keeping the stricter choice. An identity is 1 to
//!   [`MAX_APP_ID_BYTES`] bytes, without control or invisible format characters (bidi overrides,
//!   zero-width marks), spaces other than U+0020, or white space at either end; names lose the
//!   same characters, the spaces becoming plain ones. An identity the list refuses is never
//!   recorded by a default of Always (it is asked about), as it could never be given a Never.
//! - **Bounded.** At most [`MAX_APPS`] apps. A newly seen app beyond that takes the place of the
//!   least recently seen app the user has not chosen for; one the user chose for is never pushed
//!   out, and a choice for a new app when every place holds a choice is refused.
//! - **Fails safe.** A list that cannot be read is set aside (logged by what failed, never
//!   overwritten by what detection sees, and said in `meetings.calls`), and every app follows the
//!   default with Always lowered to Ask: nothing records without a choice the core could read. A
//!   choice then is refused (`list_unreadable`) unless it says to start the list over
//!   (`replace_unreadable`), so no choice is lost without the user agreeing. Starting over under
//!   a default of Always lowers the default to Ask, written with the new list (the meetings
//!   thread decides from the store, see above): the lost list may have held the Nevers that kept
//!   apps out of Always, so the user sets Always again knowingly.
//!   A default that cannot be read stops detection, as the switch it replaces did.
//! - **The switch it replaces.** `meetings.detect` ("Offer to record calls") is the default now:
//!   off is Never, on is Ask. [`migrate`] writes its stored value into [`DEFAULT_KEY`] once, at
//!   launch, and the default is read from it while [`DEFAULT_KEY`] is unset. Until the shells move
//!   to the default, `setting.get` and `setting.set` on it answer for the default
//!   ([`setting_value`], [`set_detect`]): on is anything but Never, and on over Never is Ask.

use std::collections::BTreeMap;

use ink_core::{AppRef, Store};
use serde_json::{Value, json};

use crate::events::event;

/// The setting: the policy for apps the user has not chosen for.
pub const DEFAULT_KEY: &str = "meetings.calls.default";

/// Its values.
pub const DEFAULT_VALUES: &[&str] = &["ask", "always", "never"];

/// The setting: the apps seen and chosen for (JSON, the meetings thread's).
pub const APPS_KEY: &str = "meetings.calls.apps";

/// The most apps the list keeps.
pub const MAX_APPS: usize = 64;

/// The longest app identity, in bytes (a Windows executable's name, or an AUMID, fits).
pub const MAX_APP_ID_BYTES: usize = 255;

/// The longest app name kept, in characters; a longer one is cut.
pub const MAX_APP_NAME_CHARS: usize = 80;

/// Whether identities are compared without case: on Windows, as it compares executables.
const FOLD_CASE: bool = cfg!(windows);

/// `app` as the core keeps an app's identity: lowercased in ASCII on Windows ([`FOLD_CASE`]),
/// where it enters the core (detection's signals, the commands naming an app), so every
/// comparison and every event says it one way. ASCII only, as the platform matches executables
/// (`ink_platform_win`'s lookups ignore ASCII case, and its detector keys apps the same way):
/// the identity still finds the app's process and its output, keeps its length, and a letter
/// outside ASCII is never folded into one the platform would tell apart.
pub fn identity(app: &str) -> String {
    fold(app, FOLD_CASE)
}

/// `app`, lowercased in ASCII when `on`.
fn fold(app: &str, on: bool) -> String {
    if on {
        app.to_ascii_lowercase()
    } else {
        app.to_owned()
    }
}

/// What happens when an app holds the microphone past detection's hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CallPolicy {
    /// Record it without asking, visibly: the same start, events and indicator as Record.
    Always,
    /// Offer to record it (the consent Drop).
    Ask,
    /// Neither offer nor record.
    Never,
}

impl CallPolicy {
    /// The policy a stored or sent value names.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "always" => Self::Always,
            "ask" => Self::Ask,
            "never" => Self::Never,
            _ => return None,
        })
    }

    /// Its value.
    pub fn name(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Ask => "ask",
            Self::Never => "never",
        }
    }
}

/// One app in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallApp {
    /// Its identity.
    pub id: String,
    /// Its name as detection last saw it, when it has been seen.
    pub name: Option<String>,
    /// The user's choice; `None` follows the default.
    pub policy: Option<CallPolicy>,
    /// When detection last saw it hold the microphone past the hold, Unix ms.
    pub seen_unix_ms: Option<i64>,
}

/// What [`CallPolicies::seen`] changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// A new app in the list.
    New,
    /// An app already there, under a new name.
    Renamed,
    /// An app already there: only when it was seen changed.
    Again,
    /// Not kept: the list is set aside, the identity is not one the list takes, or every place
    /// holds a choice.
    NotKept,
}

/// The default, and the apps seen and chosen for. Pure: [`load`] reads it, the meetings thread
/// writes it ([`to_json`](Self::to_json)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallPolicies {
    default: CallPolicy,
    apps: BTreeMap<String, CallApp>,
    /// Why the stored list could not be read, while it is set aside.
    unreadable: Option<String>,
    /// Identities kept and compared in lowercase ([`FOLD_CASE`]; tests set it either way).
    fold_case: bool,
}

impl Default for CallPolicies {
    /// Ask, for every app: the consent Drop, as before per-app choices.
    fn default() -> Self {
        Self::new(CallPolicy::Ask)
    }
}

impl CallPolicies {
    /// `default` for every app, and no apps.
    pub fn new(default: CallPolicy) -> Self {
        Self {
            default,
            apps: BTreeMap::new(),
            unreadable: None,
            fold_case: FOLD_CASE,
        }
    }

    /// As [`new`](Self::new), with identities compared without case or not, whatever the OS.
    #[cfg(test)]
    pub(crate) fn folding_case(mut self, fold: bool) -> Self {
        self.fold_case = fold;
        self
    }

    /// `app` as the list keys it. Checked after ([`check_app`]), so the check is of what is kept.
    fn key(&self, app: &str) -> String {
        fold(app, self.fold_case)
    }

    /// `default` for every app, with the stored list set aside because of `why`.
    pub fn set_aside(default: CallPolicy, why: String) -> Self {
        Self {
            unreadable: Some(why),
            ..Self::new(default)
        }
    }

    /// The policy for apps not chosen for, as set.
    pub fn default_policy(&self) -> CallPolicy {
        self.default
    }

    /// What happens for `app` now: the user's choice, else the default; Always is lowered to Ask
    /// while the list is set aside, and for an identity the list refuses (it could never be
    /// listed, nor given a Never).
    pub fn policy(&self, app: &str) -> CallPolicy {
        let key = self.key(app);
        let policy = self
            .apps
            .get(&key)
            .and_then(|a| a.policy)
            .unwrap_or(self.default);
        if (self.unreadable.is_some() || check_app(&key).is_err()) && policy == CallPolicy::Always {
            CallPolicy::Ask
        } else {
            policy
        }
    }

    /// Whether any app could be offered or recorded: detection listens only then.
    pub fn listens(&self) -> bool {
        self.default != CallPolicy::Never
            || self
                .apps
                .values()
                .any(|a| matches!(a.policy, Some(CallPolicy::Always | CallPolicy::Ask)))
    }

    /// Why the stored list is set aside, if it is.
    pub fn unreadable(&self) -> Option<&str> {
        self.unreadable.as_deref()
    }

    /// The apps, most recently seen first (never seen last), then by identity.
    pub fn apps(&self) -> Vec<&CallApp> {
        let mut apps: Vec<&CallApp> = self.apps.values().collect();
        apps.sort_by(|a, b| {
            b.seen_unix_ms
                .cmp(&a.seen_unix_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        apps
    }

    /// The user chose `policy` for `app` (`None`: it follows the default again). A list set
    /// aside is started over with this choice (the caller asked the user first): true then, and
    /// the caller decides from the store whether the default is lowered with it
    /// ([`set_default`](Self::set_default)). Refused for an identity the list does not take,
    /// and for a new app when every place holds a choice.
    pub fn choose(&mut self, app: &str, policy: Option<CallPolicy>) -> Result<bool, String> {
        let app = self.key(app);
        check_app(&app)?;
        let started_over = self.unreadable.take().is_some();
        if started_over {
            log::warn!("call policies: the unreadable list was started over by a choice");
            self.apps.clear();
        }
        if let Some(entry) = self.apps.get_mut(&app) {
            entry.policy = policy;
            return Ok(started_over);
        }
        if policy.is_none() {
            // Nothing chosen, and nothing to clear.
            return Ok(started_over);
        }
        if !self.make_room() {
            return Err(format!(
                "the list of apps is full ({MAX_APPS} apps, each with a choice): set one of them to follow the default first"
            ));
        }
        self.apps.insert(
            app.clone(),
            CallApp {
                id: app,
                name: None,
                policy,
                seen_unix_ms: None,
            },
        );
        Ok(started_over)
    }

    /// The default from now, as the store says it (a list started over: the caller writes Ask
    /// when it lowers Always).
    pub fn set_default(&mut self, default: CallPolicy) {
        self.default = default;
    }

    /// Detection saw `app` hold the microphone past its hold, at `now_unix_ms`.
    pub fn seen(&mut self, app: &AppRef, now_unix_ms: i64) -> Seen {
        let id = self.key(&app.id);
        if self.unreadable.is_some() || check_app(&id).is_err() {
            return Seen::NotKept;
        }
        let name = clean_name(&app.name);
        if let Some(entry) = self.apps.get_mut(&id) {
            entry.seen_unix_ms = Some(now_unix_ms);
            if name.is_some() && entry.name != name {
                entry.name = name;
                return Seen::Renamed;
            }
            return Seen::Again;
        }
        if !self.make_room() {
            log::info!("call policies: the list is full of choices; a new app is not kept");
            return Seen::NotKept;
        }
        self.apps.insert(
            id.clone(),
            CallApp {
                id,
                name,
                policy: None,
                seen_unix_ms: Some(now_unix_ms),
            },
        );
        Seen::New
    }

    /// Room for one more app: there is, or the least recently seen app without a choice goes.
    fn make_room(&mut self) -> bool {
        if self.apps.len() < MAX_APPS {
            return true;
        }
        let oldest = self
            .apps
            .values()
            .filter(|a| a.policy.is_none())
            .min_by(|a, b| {
                a.seen_unix_ms
                    .cmp(&b.seen_unix_ms)
                    .then_with(|| a.id.cmp(&b.id))
            })
            .map(|a| a.id.clone());
        match oldest {
            Some(id) => {
                self.apps.remove(&id);
                true
            }
            None => false,
        }
    }

    /// The list as [`APPS_KEY`] stores it: `{"apps": [{"app", "name"?, "policy"?,
    /// "seen_unix_ms"?}]}`, by identity.
    pub fn to_json(&self) -> String {
        let apps: Vec<Value> = self
            .apps
            .values()
            .map(|a| {
                let mut v = json!({"app": a.id});
                if let Some(name) = &a.name {
                    v["name"] = name.as_str().into();
                }
                if let Some(policy) = a.policy {
                    v["policy"] = policy.name().into();
                }
                if let Some(seen) = a.seen_unix_ms {
                    v["seen_unix_ms"] = seen.into();
                }
                v
            })
            .collect();
        json!({ "apps": apps }).to_string()
    }

    /// Reads [`APPS_KEY`]'s value, with `default`. Any entry that is not one the list takes (an
    /// identity refused, a policy unknown, a duplicate, more than [`MAX_APPS`]) refuses it all: a
    /// list read in part could drop a Never and record what the user said not to.
    pub fn from_json(default: CallPolicy, text: &str) -> Result<Self, String> {
        Self::from_json_in(Self::new(default), text)
    }

    /// [`from_json`](Self::from_json) into `empty`, which says how identities compare.
    fn from_json_in(empty: Self, text: &str) -> Result<Self, String> {
        let v: Value =
            serde_json::from_str(text).map_err(|_| "the stored list is not JSON".to_owned())?;
        let entries = v
            .get("apps")
            .and_then(Value::as_array)
            .ok_or("the stored list has no apps array")?;
        if entries.len() > MAX_APPS {
            return Err(format!("the stored list holds more than {MAX_APPS} apps"));
        }
        let mut apps: BTreeMap<String, CallApp> = BTreeMap::new();
        let mut stored = std::collections::BTreeSet::new();
        for entry in entries {
            let given = entry
                .get("app")
                .and_then(Value::as_str)
                .ok_or("a stored app has no identity")?;
            if !stored.insert(given) {
                return Err("the stored list names an app twice".into());
            }
            let id = empty.key(given);
            check_app(&id).map_err(|e| format!("a stored app: {e}"))?;
            let policy = match entry.get("policy") {
                None => None,
                Some(p) => Some(
                    p.as_str()
                        .and_then(CallPolicy::parse)
                        .ok_or("a stored app's policy is unknown")?,
                ),
            };
            let name = match entry.get("name") {
                None => None,
                Some(n) => clean_name(n.as_str().ok_or("a stored app's name is not text")?),
            };
            let seen_unix_ms = match entry.get("seen_unix_ms") {
                None => None,
                Some(t) => Some(t.as_i64().ok_or("a stored app's time is not a number")?),
            };
            let app = CallApp {
                id: id.clone(),
                name,
                policy,
                seen_unix_ms,
            };
            // Written before identities were lowercased: one app in two cases. Merged, keeping
            // the stricter choice, so a Never in either is kept.
            let app = match apps.remove(&id) {
                Some(other) => {
                    log::warn!("call policies: one app was stored in two cases; merged");
                    merged(other, app, empty.default)
                }
                None => app,
            };
            apps.insert(id, app);
        }
        Ok(Self { apps, ..empty })
    }
}

/// One app stored twice (in two cases): the stricter choice, by what it does under `default`
/// (Never, then Ask, then Always; a choice over following the default when they do the same),
/// the name last seen, and the later sighting.
fn merged(a: CallApp, b: CallApp, default: CallPolicy) -> CallApp {
    let strictness = |p: Option<CallPolicy>| {
        let rank = match p.unwrap_or(default) {
            CallPolicy::Never => 2,
            CallPolicy::Ask => 1,
            CallPolicy::Always => 0,
        };
        (rank, p.is_some())
    };
    let policy = if strictness(b.policy) > strictness(a.policy) {
        b.policy
    } else {
        a.policy
    };
    let (seen_unix_ms, name) = if b.seen_unix_ms > a.seen_unix_ms {
        (b.seen_unix_ms, b.name.or(a.name))
    } else {
        (a.seen_unix_ms, a.name.or(b.name))
    };
    CallApp {
        id: a.id,
        name,
        policy,
        seen_unix_ms,
    }
}

/// Whether `app` is an identity the list takes. The refusal never quotes it.
pub fn check_app(app: &str) -> Result<(), String> {
    if app.is_empty() || app.len() > MAX_APP_ID_BYTES {
        return Err(format!(
            "an app's identity is 1 to {MAX_APP_ID_BYTES} bytes"
        ));
    }
    if app.chars().any(|c| hidden(c) || odd_space(c)) || app.trim() != app {
        return Err(
            "an app's identity has no control or invisible characters, no spaces but plain ones, and no white space at either end"
                .into(),
        );
    }
    Ok(())
}

/// A character that reads as nothing or reorders what follows: a control character, a line or
/// paragraph separator, any of Unicode's format characters (category Cf: bidi marks and
/// overrides, zero-width marks and joiners, the soft hyphen, the interlinear annotation marks,
/// tags, the Arabic and other prepended number marks), and what draws as nothing besides: the
/// grapheme joiner, the variation selectors, the Hangul fillers, the blank Braille pattern.
/// Refused in identities and dropped from names, so two apps never look alike in the list. By
/// hand, as of Unicode 16: no crate in the lock gives the categories.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            // Cf.
            '\u{00AD}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061C}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08E2}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{13430}'..='\u{1343F}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0001}'
                | '\u{E0020}'..='\u{E007F}'
                // Zl and Zp.
                | '\u{2028}'..='\u{2029}'
                // Not Cf, and drawn as nothing: the grapheme joiner, the Khmer inherent vowels,
                // the variation selectors (and the tag block's unassigned rest), the Hangul
                // fillers, blank Braille.
                | '\u{034F}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180D}'
                | '\u{180F}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{E0100}'..='\u{E01EF}'
                | '\u{E0000}'
                | '\u{E0002}'..='\u{E001F}'
                | '\u{115F}'..='\u{1160}'
                | '\u{3164}'
                | '\u{FFA0}'
                | '\u{2800}'
        )
}

/// A space other than U+0020 (Unicode's Zs): a no-break, ogham, en to hair, narrow no-break,
/// medium mathematical or ideographic space. Refused in identities; a plain space in names, so
/// two names never differ by a space that looks the same.
fn odd_space(c: char) -> bool {
    matches!(
        c,
        '\u{00A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// A name as the list keeps it, and as the shell is shown it (the consent Drop, the recording):
/// one line, its spaces plain, trimmed, at most [`MAX_APP_NAME_CHARS`]; `None` when nothing is
/// left.
pub(crate) fn clean_name(name: &str) -> Option<String> {
    let one_line: String = name
        .chars()
        .filter(|c| !hidden(*c))
        .map(|c| if odd_space(c) { ' ' } else { c })
        .collect();
    let trimmed: String = one_line.trim().chars().take(MAX_APP_NAME_CHARS).collect();
    let trimmed = trimmed.trim_end();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The default as stored, else as the switch it replaces left it (`meetings.detect` off is Never),
/// else Ask. A stored value this build does not know (only [`DEFAULT_VALUES`] can be set) is Ask,
/// and logged: never Always on a guess.
pub fn read_default(store: &dyn Store) -> Result<CallPolicy, String> {
    match store.setting(DEFAULT_KEY).map_err(|e| e.to_string())? {
        Some(value) => Ok(CallPolicy::parse(&value).unwrap_or_else(|| {
            log::error!("call policies: the stored default is not one this build knows; asking");
            CallPolicy::Ask
        })),
        None => Ok(legacy_default(store)?.unwrap_or(CallPolicy::Ask)),
    }
}

/// The default `meetings.detect` stands for, when it is stored.
fn legacy_default(store: &dyn Store) -> Result<Option<CallPolicy>, String> {
    Ok(store
        .setting(crate::control::DETECT_KEY)
        .map_err(|e| e.to_string())?
        .map(|v| match v.as_str() {
            "off" => CallPolicy::Never,
            _ => CallPolicy::Ask,
        }))
}

/// **Worker** (launch). Writes `meetings.detect`'s stored value into [`DEFAULT_KEY`] while that is
/// unset: off is Never, anything else Ask. A failure is logged; the default is read the same way
/// from the old switch until a later launch writes it ([`read_default`]).
pub fn migrate(store: &dyn Store) {
    let migrated = (|| -> Result<(), String> {
        if store
            .setting(DEFAULT_KEY)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Ok(());
        }
        if let Some(policy) = legacy_default(store)? {
            store
                .set_setting(DEFAULT_KEY, policy.name())
                .map_err(|e| e.to_string())?;
            log::info!(
                "call policies: the detection switch became the default ({})",
                policy.name()
            );
        }
        Ok(())
    })();
    if let Err(e) = migrated {
        log::warn!("call policies: the detection switch could not be migrated: {e}");
    }
}

/// **Worker.** Every app's policy and the list, from the store. `Err` when the default cannot be
/// read (detection then stays off); a list that cannot be read is set aside (see the module docs).
pub fn load(store: &dyn Store) -> Result<CallPolicies, String> {
    let default = read_default(store)?;
    Ok(match store.setting(APPS_KEY) {
        Ok(None) => CallPolicies::new(default),
        Ok(Some(text)) => CallPolicies::from_json(default, &text).unwrap_or_else(|why| {
            log::warn!("call policies: {why}; set aside");
            CallPolicies::set_aside(default, format!("couldn't read the apps' choices: {why}"))
        }),
        Err(e) => {
            log::warn!("call policies: the list could not be read: {e}; set aside");
            CallPolicies::set_aside(default, format!("couldn't read the apps' choices: {e}"))
        }
    })
}

/// Whether `key` is a setting this module answers for in `setting.get` and `setting.set`.
pub fn is_calls_setting(key: &str) -> bool {
    key == DEFAULT_KEY || key == crate::control::DETECT_KEY
}

/// `setting.get` for [`DEFAULT_KEY`] and `meetings.detect`: the default as the core reads it
/// ([`read_default`]); no value while neither has ever been set (the shell's default, Ask, is the
/// core's). `meetings.detect` answers `off` for Never and `on` for the rest.
pub fn setting_value(store: &dyn Store, key: &str) -> Result<Option<String>, String> {
    let stored = store.setting(DEFAULT_KEY).map_err(|e| e.to_string())?;
    if stored.is_none() && legacy_default(store)?.is_none() {
        return Ok(None);
    }
    let policy = read_default(store)?;
    Ok(Some(if key == DEFAULT_KEY {
        policy.name().to_owned()
    } else if policy == CallPolicy::Never {
        "off".to_owned()
    } else {
        "on".to_owned()
    }))
}

/// `setting.set meetings.detect`: off sets the default to Never; on sets it to Ask when it is
/// Never, and leaves Ask or Always as they are.
pub fn set_detect(store: &dyn Store, value: &str) -> Result<(), String> {
    let policy = match value {
        "off" => CallPolicy::Never,
        _ if read_default(store)? == CallPolicy::Never => CallPolicy::Ask,
        _ => return Ok(()),
    };
    store
        .set_setting(DEFAULT_KEY, policy.name())
        .map_err(|e| e.to_string())
}

/// A `meetings.calls.set` policy: `always`, `ask` or `never`, or `default` (`None`: the app follows
/// the default again).
pub fn parse_choice(value: &str) -> Result<Option<CallPolicy>, String> {
    match value {
        "default" => Ok(None),
        other => CallPolicy::parse(other)
            .map(Some)
            .ok_or_else(|| "\"policy\" is always, ask, never or default".to_owned()),
    }
}

/// `meetings.calls`: the default and every app in the list, in answer to `meetings.calls.list`
/// or `meetings.calls.set` (with its id as `ref`), and unasked when the list or the default
/// changed.
pub fn listing(policies: &CallPolicies, reference: Option<&str>) -> Value {
    let apps: Vec<Value> = policies
        .apps()
        .into_iter()
        .map(|a| {
            let mut v = json!({
                "app": a.id,
                "policy": policies.policy(&a.id).name(),
                "chosen": a.policy.is_some(),
            });
            if let Some(name) = &a.name {
                v["app_name"] = name.as_str().into();
            }
            if let Some(seen) = a.seen_unix_ms {
                v["seen_unix_ms"] = seen.into();
            }
            v
        })
        .collect();
    event(
        "meetings.calls",
        &[
            ("default", Some(policies.default_policy().name().into())),
            ("apps", Some(Value::Array(apps))),
            ("message", policies.unreadable().map(Into::into)),
            ("ref", reference.map(Into::into)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::mock::MemStore;

    fn app(id: &str, name: &str) -> AppRef {
        AppRef {
            id: id.into(),
            pid: None,
            name: name.into(),
        }
    }

    #[test]
    fn a_choice_wins_over_the_default_and_clearing_it_follows_the_default_again() {
        let mut p = CallPolicies::new(CallPolicy::Ask);
        assert_eq!(p.policy("zoom"), CallPolicy::Ask);
        p.choose("zoom", Some(CallPolicy::Always)).unwrap();
        p.choose("chat", Some(CallPolicy::Never)).unwrap();
        assert_eq!(p.policy("zoom"), CallPolicy::Always);
        assert_eq!(p.policy("chat"), CallPolicy::Never);
        assert_eq!(p.policy("other"), CallPolicy::Ask);
        p.choose("zoom", None).unwrap();
        assert_eq!(p.policy("zoom"), CallPolicy::Ask);
        assert_eq!(p.apps().len(), 2, "a cleared app stays listed");
        p.choose("never-listed", None).unwrap();
        assert_eq!(p.apps().len(), 2, "clearing nothing adds nothing");
    }

    #[test]
    fn the_default_applies_to_apps_not_chosen_for() {
        for default in [CallPolicy::Always, CallPolicy::Ask, CallPolicy::Never] {
            let mut p = CallPolicies::new(default);
            p.seen(&app("zoom", "Zoom"), 1);
            assert_eq!(p.policy("zoom"), default, "a seen app follows the default");
            assert_eq!(p.policy("unseen"), default);
        }
    }

    #[test]
    fn detection_listens_only_while_some_app_could_be_offered_or_recorded() {
        let mut p = CallPolicies::new(CallPolicy::Never);
        assert!(!p.listens(), "Never for all is the old switch off");
        p.choose("chat", Some(CallPolicy::Never)).unwrap();
        assert!(!p.listens());
        p.choose("zoom", Some(CallPolicy::Always)).unwrap();
        assert!(p.listens(), "an Always app is watched for");
        assert!(CallPolicies::new(CallPolicy::Ask).listens());
    }

    #[test]
    fn identities_are_validated_and_refusals_never_quote_them() {
        let mut p = CallPolicies::default();
        for bad in [
            "",
            " zoom",
            "zoom\n",
            "a\u{7}b",
            "zo\u{202E}om",
            "zo\u{200B}om",
            "zo\u{200D}om",
            "zo\u{FE0F}om",
            "zo\u{E0041}om",
            "zo\u{0600}om",
            "zo\u{1D173}om",
            "zo\u{E0100}om",
            "zo\u{2029}om",
            "\u{3164}",
            &"x".repeat(256),
        ] {
            let e = p.choose(bad, Some(CallPolicy::Always)).unwrap_err();
            if !bad.trim().is_empty() {
                assert!(!e.contains(bad.trim()), "{e}");
            }
            assert_eq!(p.seen(&app(bad, "Bad"), 1), Seen::NotKept);
        }
        assert!(p.apps().is_empty());
        for good in [
            "us.zoom.xos",
            "Zoom.exe",
            "Microsoft.Teams_8wekyb3d8bbwe!MSTeams",
            "My App.exe",
            &"x".repeat(255),
        ] {
            p.choose(good, Some(CallPolicy::Ask)).unwrap();
        }
        assert_eq!(parse_choice("default"), Ok(None));
        assert_eq!(parse_choice("always"), Ok(Some(CallPolicy::Always)));
        assert!(parse_choice("Always").is_err());
    }

    #[test]
    fn names_are_one_trimmed_line_and_cut_to_the_limit() {
        let mut p = CallPolicies::default();
        p.seen(&app("a", "  Zo\nom\u{2028}\u{202E}\u{200B} "), 1);
        p.seen(&app("b", &"n".repeat(200)), 1);
        p.seen(&app("c", " \t "), 1);
        let names: Vec<Option<String>> = p.apps().iter().map(|a| a.name.clone()).collect();
        assert_eq!(
            names,
            [Some("Zoom".into()), Some("n".repeat(80)), None],
            "by id among equal times"
        );
    }

    #[test]
    fn a_full_list_pushes_out_the_oldest_unchosen_app_and_never_a_choice() {
        let mut p = CallPolicies::default();
        for i in 0..MAX_APPS {
            p.seen(&app(&format!("app{i:02}"), "A"), i as i64);
        }
        p.choose("app00", Some(CallPolicy::Never)).unwrap();
        assert_eq!(p.seen(&app("new1", "N"), 1_000), Seen::New);
        assert!(p.apps().iter().any(|a| a.id == "app00"), "a choice is kept");
        assert!(
            !p.apps().iter().any(|a| a.id == "app01"),
            "the oldest unchosen goes"
        );
        assert_eq!(p.apps().len(), MAX_APPS);

        let mut full = CallPolicies::default();
        for i in 0..MAX_APPS {
            full.choose(&format!("app{i:02}"), Some(CallPolicy::Ask))
                .unwrap();
        }
        assert_eq!(full.seen(&app("new", "N"), 1), Seen::NotKept);
        assert!(full.choose("new", Some(CallPolicy::Always)).is_err());
        full.choose("app03", Some(CallPolicy::Never)).unwrap();
        assert_eq!(full.policy("new"), CallPolicy::Ask);
    }

    #[test]
    fn seeing_an_app_again_updates_its_time_and_says_a_new_name() {
        let mut p = CallPolicies::default();
        assert_eq!(p.seen(&app("zoom", "Zoom"), 1), Seen::New);
        assert_eq!(p.seen(&app("zoom", "Zoom"), 2), Seen::Again);
        assert_eq!(p.seen(&app("zoom", "Zoom Workplace"), 3), Seen::Renamed);
        assert_eq!(
            p.seen(&app("zoom", ""), 4),
            Seen::Again,
            "no name keeps the old"
        );
        let a = p.apps()[0].clone();
        assert_eq!(a.name.as_deref(), Some("Zoom Workplace"));
        assert_eq!(a.seen_unix_ms, Some(4));
    }

    #[test]
    fn the_list_round_trips_and_any_bad_entry_sets_it_all_aside() {
        let mut p = CallPolicies::new(CallPolicy::Never);
        p.seen(&app("zoom", "Zoom"), 5);
        p.choose("zoom", Some(CallPolicy::Always)).unwrap();
        p.choose("chat", Some(CallPolicy::Never)).unwrap();
        let back = CallPolicies::from_json(CallPolicy::Never, &p.to_json()).unwrap();
        assert_eq!(back, p);
        for bad in [
            "not json",
            r#"{"apps": 3}"#,
            r#"{"apps": [{"app": "zoom", "policy": "sometimes"}]}"#,
            r#"{"apps": [{"app": "zoom"}, {"app": "zoom"}]}"#,
            r#"{"apps": [{"app": " zoom"}]}"#,
            r#"{"apps": [{"policy": "never"}]}"#,
            r#"{"apps": [{"app": "zoom", "seen_unix_ms": "today"}]}"#,
        ] {
            assert!(
                CallPolicies::from_json(CallPolicy::Ask, bad).is_err(),
                "{bad}"
            );
        }
        let many: Vec<Value> = (0..=MAX_APPS)
            .map(|i| json!({"app": format!("a{i}")}))
            .collect();
        assert!(
            CallPolicies::from_json(CallPolicy::Ask, &json!({"apps": many}).to_string()).is_err()
        );
    }

    #[test]
    fn a_list_set_aside_never_records_and_is_not_overwritten_by_what_is_seen() {
        let store = MemStore::new();
        store.set_setting(DEFAULT_KEY, "always").unwrap();
        store
            .set_setting(
                APPS_KEY,
                r#"{"apps": [{"app": "chat", "policy": "bogus"}]}"#,
            )
            .unwrap();
        let mut p = load(&store).unwrap();
        assert!(p.unreadable().is_some());
        assert_eq!(p.policy("zoom"), CallPolicy::Ask, "Always lowered to Ask");
        assert_eq!(p.policy("chat"), CallPolicy::Ask);
        assert_eq!(p.seen(&app("zoom", "Zoom"), 1), Seen::NotKept);
        assert!(listing(&p, None)["message"].is_string());
        // A choice starts the list over (and the meetings thread lowers the default with it,
        // from what the store says: control's test).
        assert_eq!(
            p.choose("zoom", Some(CallPolicy::Always)),
            Ok(true),
            "started over"
        );
        assert!(p.unreadable().is_none());
        assert_eq!(p.policy("zoom"), CallPolicy::Always);
        p.set_default(CallPolicy::Ask);
        assert_eq!(
            p.policy("chat"),
            CallPolicy::Ask,
            "never Always for all by a start over"
        );
    }

    #[test]
    fn the_old_switch_migrates_into_the_default_once() {
        // Off is Never.
        let store = MemStore::new();
        store
            .set_setting(crate::control::DETECT_KEY, "off")
            .unwrap();
        assert_eq!(
            read_default(&store),
            Ok(CallPolicy::Never),
            "read before migrating"
        );
        migrate(&store);
        assert_eq!(
            store.setting(DEFAULT_KEY).unwrap().as_deref(),
            Some("never")
        );
        // Once: a default set later is never overwritten by the old switch.
        store.set_setting(DEFAULT_KEY, "always").unwrap();
        migrate(&store);
        assert_eq!(read_default(&store), Ok(CallPolicy::Always));

        // On is Ask; never set leaves the default unset (Ask).
        let store = MemStore::new();
        store.set_setting(crate::control::DETECT_KEY, "on").unwrap();
        migrate(&store);
        assert_eq!(store.setting(DEFAULT_KEY).unwrap().as_deref(), Some("ask"));
        let store = MemStore::new();
        migrate(&store);
        assert_eq!(store.setting(DEFAULT_KEY).unwrap(), None);
        assert_eq!(read_default(&store), Ok(CallPolicy::Ask));
        // A stored value this build does not know never becomes Always.
        store.set_setting(DEFAULT_KEY, "sometimes").unwrap();
        assert_eq!(read_default(&store), Ok(CallPolicy::Ask));
    }

    #[test]
    fn the_old_switch_answers_for_the_default() {
        let store = MemStore::new();
        assert_eq!(setting_value(&store, crate::control::DETECT_KEY), Ok(None));
        set_detect(&store, "on").unwrap();
        assert_eq!(
            store.setting(DEFAULT_KEY).unwrap(),
            None,
            "on over Ask: unchanged"
        );
        set_detect(&store, "off").unwrap();
        assert_eq!(read_default(&store), Ok(CallPolicy::Never));
        assert_eq!(
            setting_value(&store, crate::control::DETECT_KEY),
            Ok(Some("off".into()))
        );
        set_detect(&store, "on").unwrap();
        assert_eq!(read_default(&store), Ok(CallPolicy::Ask));
        store.set_setting(DEFAULT_KEY, "always").unwrap();
        set_detect(&store, "on").unwrap();
        assert_eq!(read_default(&store), Ok(CallPolicy::Always), "Always stays");
        assert_eq!(
            setting_value(&store, crate::control::DETECT_KEY),
            Ok(Some("on".into()))
        );
        assert_eq!(
            setting_value(&store, DEFAULT_KEY),
            Ok(Some("always".into()))
        );
    }

    #[test]
    fn the_listing_names_each_apps_policy_and_whether_it_was_chosen() {
        let mut p = CallPolicies::new(CallPolicy::Never);
        p.seen(&app("zoom", "Zoom"), 10);
        p.seen(&app("chat", "Chat"), 20);
        p.choose("zoom", Some(CallPolicy::Always)).unwrap();
        let v = listing(&p, Some("l1"));
        assert_eq!(v["type"], "meetings.calls");
        assert_eq!(v["default"], "never");
        assert_eq!(v["ref"], "l1");
        assert_eq!(
            v["apps"],
            json!([
                {"app": "chat", "app_name": "Chat", "policy": "never", "chosen": false, "seen_unix_ms": 20},
                {"app": "zoom", "app_name": "Zoom", "policy": "always", "chosen": true, "seen_unix_ms": 10},
            ])
        );
        assert!(v.get("message").is_none());
    }

    #[test]
    fn on_windows_an_identity_is_one_app_whatever_its_case() {
        let mut p = CallPolicies::new(CallPolicy::Always).folding_case(true);
        p.choose("Zoom.exe", Some(CallPolicy::Never)).unwrap();
        assert_eq!(p.policy("zoom.exe"), CallPolicy::Never);
        assert_eq!(
            p.policy("ZOOM.EXE"),
            CallPolicy::Never,
            "a Never is never missed"
        );
        assert_eq!(p.seen(&app("zoom.EXE", "Zoom Workplace"), 1), Seen::Renamed);
        let apps = p.apps();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "zoom.exe", "kept in lowercase");
        assert_eq!(
            apps[0].name.as_deref(),
            Some("Zoom Workplace"),
            "the name as given"
        );
        let stored = CallPolicies::new(CallPolicy::Ask).folding_case(true);
        let back = CallPolicies::from_json_in(stored, &p.to_json()).unwrap();
        assert_eq!(back.policy("ZOOM.exe"), CallPolicy::Never);
        // The Mac's bundle ids keep their case, as the platform reports them.
        let mut mac = CallPolicies::new(CallPolicy::Ask).folding_case(false);
        mac.choose("us.zoom.xos", Some(CallPolicy::Never)).unwrap();
        assert_eq!(mac.policy("US.ZOOM.XOS"), CallPolicy::Ask);
        assert_eq!(fold("Zoom.EXE", true), "zoom.exe");
        assert_eq!(fold("Zoom.EXE", false), "Zoom.EXE");
    }

    /// The fold is ASCII, as the platform matches executables: it never changes an identity's
    /// length (so the limit, checked on what is kept, is the limit as given), and letters outside
    /// ASCII keep their case (the Kelvin sign is not 'k', 'İ' is not "i̇"), so two apps the
    /// platform tells apart are never folded into one.
    #[test]
    fn identities_fold_in_ascii_as_the_platform_matches_them() {
        assert_eq!(fold("Zoom.EXE", true), "zoom.exe");
        assert_eq!(fold("\u{212A}ZOOM.exe", true), "\u{212A}zoom.exe");
        assert_eq!(fold("\u{130}.exe", true), "\u{130}.exe");
        assert_eq!(fold("ΣΑ.exe", true), "ΣΑ.exe");
        let longest = format!("{}.EXE", "\u{130}".repeat((MAX_APP_ID_BYTES - 4) / 2));
        assert!(longest.len() <= MAX_APP_ID_BYTES);
        assert_eq!(fold(&longest, true).len(), longest.len());
        let mut win = CallPolicies::new(CallPolicy::Ask).folding_case(true);
        win.choose(&longest, Some(CallPolicy::Never)).unwrap();
        let back = CallPolicies::from_json_in(
            CallPolicies::new(CallPolicy::Ask).folding_case(true),
            &win.to_json(),
        )
        .unwrap();
        assert_eq!(back.policy(&longest), CallPolicy::Never, "read back whole");
        assert!(win.choose(&"x".repeat(MAX_APP_ID_BYTES + 1), None).is_err());
    }

    /// A list written before identities were lowercased may hold one app in two cases: merged
    /// as read, keeping the stricter choice, never set aside as a whole for it.
    #[test]
    fn one_app_stored_in_two_cases_is_merged_keeping_the_stricter_choice() {
        let read = |default, text: &str| {
            CallPolicies::from_json_in(CallPolicies::new(default).folding_case(true), text).unwrap()
        };
        let both = r#"{"apps": [
            {"app": "Zoom.exe", "policy": "always", "name": "Zoom", "seen_unix_ms": 5},
            {"app": "zoom.exe", "policy": "never", "seen_unix_ms": 9}
        ]}"#;
        let p = read(CallPolicy::Always, both);
        assert!(p.unreadable().is_none());
        assert_eq!(p.apps().len(), 1);
        assert_eq!(p.policy("ZOOM.EXE"), CallPolicy::Never, "the Never is kept");
        assert_eq!(p.apps()[0].name.as_deref(), Some("Zoom"));
        assert_eq!(p.apps()[0].seen_unix_ms, Some(9));
        // Following the default (Never) is stricter than Always chosen.
        let chosen_and_not =
            r#"{"apps": [{"app": "Zoom.exe", "policy": "always"}, {"app": "zoom.exe"}]}"#;
        assert_eq!(
            read(CallPolicy::Never, chosen_and_not).policy("zoom.exe"),
            CallPolicy::Never
        );
        assert_eq!(
            read(CallPolicy::Ask, chosen_and_not).policy("zoom.exe"),
            CallPolicy::Ask
        );
        // Ask chosen against an Ask default: the choice is kept.
        let asks = r#"{"apps": [{"app": "zoom.exe"}, {"app": "Zoom.exe", "policy": "ask"}]}"#;
        assert_eq!(
            read(CallPolicy::Ask, asks).apps()[0].policy,
            Some(CallPolicy::Ask)
        );
        // The same identity twice, as stored, is still a list that cannot be read.
        let twice = r#"{"apps": [{"app": "zoom.exe"}, {"app": "zoom.exe"}]}"#;
        assert!(
            CallPolicies::from_json_in(
                CallPolicies::new(CallPolicy::Ask).folding_case(true),
                twice
            )
            .is_err()
        );
    }

    /// An identity the list refuses could never be listed or given a Never: a default of Always
    /// asks about it instead of recording it.
    #[test]
    fn an_identity_the_list_refuses_is_asked_about_never_recorded_by_the_default() {
        let p = CallPolicies::new(CallPolicy::Always);
        for bad in ["zo\u{200B}om", " zoom", &"x".repeat(MAX_APP_ID_BYTES + 1)] {
            assert_eq!(p.policy(bad), CallPolicy::Ask, "{:?}", bad.len());
        }
        assert_eq!(p.policy("zoom"), CallPolicy::Always);
        assert_eq!(
            CallPolicies::new(CallPolicy::Never).policy("zo\u{200B}om"),
            CallPolicy::Never
        );
    }

    #[test]
    fn spaces_other_than_plain_ones_are_refused_in_identities_and_plain_in_names() {
        let mut p = CallPolicies::default();
        for space in [
            '\u{00A0}', '\u{1680}', '\u{2007}', '\u{202F}', '\u{205F}', '\u{3000}',
        ] {
            assert!(
                check_app(&format!("zo{space}om")).is_err(),
                "{:X}",
                space as u32
            );
        }
        assert!(check_app("zo om").is_ok());
        p.seen(&app("a", "Micro\u{00A0}soft\u{3000}Teams\u{17B4}"), 1);
        assert_eq!(p.apps()[0].name.as_deref(), Some("Micro soft Teams"));
        assert!(check_app("zo\u{17B5}om").is_err());
    }

    #[test]
    fn a_choice_says_when_it_started_the_list_over_and_lowering_is_the_callers() {
        let mut p = CallPolicies::set_aside(CallPolicy::Always, "unreadable".into());
        assert_eq!(p.choose("zoom", Some(CallPolicy::Always)), Ok(true));
        assert_eq!(p.default_policy(), CallPolicy::Always, "not by the choice");
        p.set_default(CallPolicy::Ask);
        assert_eq!(p.default_policy(), CallPolicy::Ask);
        assert_eq!(
            p.policy("other"),
            CallPolicy::Ask,
            "never Always for all by a start over"
        );
        assert_eq!(p.policy("zoom"), CallPolicy::Always, "the choice made");
        let mut p = CallPolicies::set_aside(CallPolicy::Never, "unreadable".into());
        assert_eq!(p.choose("zoom", Some(CallPolicy::Ask)), Ok(true));
        assert_eq!(p.default_policy(), CallPolicy::Never);
        let mut readable = CallPolicies::new(CallPolicy::Always);
        assert_eq!(readable.choose("zoom", Some(CallPolicy::Never)), Ok(false));
        assert_eq!(readable.default_policy(), CallPolicy::Always);
    }

    #[test]
    fn names_drop_joiners_and_blanks_that_draw_as_nothing() {
        let mut p = CallPolicies::default();
        p.seen(&app("a", "Te\u{200D}am\u{FE0F}s\u{3164}\u{2800}"), 1);
        assert_eq!(p.apps()[0].name.as_deref(), Some("Teams"));
    }
}
