//! Call policies: for each app that takes the microphone for a call, whether its calls are
//! recorded at once ("Always record"), offered (the consent Drop: "Ask"), or left alone ("Never"),
//! and the policy for apps the user has not chosen for.
//!
//! | Setting | Holds | Written by |
//! |---|---|---|
//! | [`DEFAULT_KEY`] | `ask` (also when unset), `always` or `never` | the shell, through `setting.set` (the queries thread) |
//! | [`APPS_KEY`] | the apps seen and chosen for, as JSON | the meetings thread alone: `meetings.calls.set`, and the apps detection sees |
//!
//! Two keys, two writers: neither ever reads, changes and writes back what the other holds.
//!
//! - **Keyed by identity.** An app is the identity detection reports ([`AppRef::id`]: the bundle
//!   id on the Mac, the executable on Windows), never its name, which only labels it. An identity
//!   is 1 to [`MAX_APP_ID_BYTES`] bytes, without control or invisible format characters (bidi
//!   overrides, zero-width marks) or white space at either end; names lose the same characters.
//! - **Bounded.** At most [`MAX_APPS`] apps. A newly seen app beyond that takes the place of the
//!   least recently seen app the user has not chosen for; one the user chose for is never pushed
//!   out, and a choice for a new app when every place holds a choice is refused.
//! - **Fails safe.** A list that cannot be read is set aside (logged by what failed, never
//!   overwritten by what detection sees, and said in `meetings.calls`), and every app follows the
//!   default with Always lowered to Ask: nothing records without a choice the core could read. A
//!   choice then is refused (`list_unreadable`) unless it says to start the list over
//!   (`replace_unreadable`), so no choice is lost without the user agreeing. A default that cannot
//!   be read stops detection, as the switch it replaces did.
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
        }
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
    /// while the list is set aside.
    pub fn policy(&self, app: &str) -> CallPolicy {
        let policy = self
            .apps
            .get(app)
            .and_then(|a| a.policy)
            .unwrap_or(self.default);
        if self.unreadable.is_some() && policy == CallPolicy::Always {
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
    /// aside is started over with this choice. Refused for an identity the list does not take,
    /// and for a new app when every place holds a choice.
    pub fn choose(&mut self, app: &str, policy: Option<CallPolicy>) -> Result<(), String> {
        check_app(app)?;
        if self.unreadable.take().is_some() {
            log::warn!("call policies: the unreadable list was started over by a choice");
            self.apps.clear();
        }
        if let Some(entry) = self.apps.get_mut(app) {
            entry.policy = policy;
            return Ok(());
        }
        if policy.is_none() {
            // Nothing chosen, and nothing to clear.
            return Ok(());
        }
        if !self.make_room() {
            return Err(format!(
                "the list of apps is full ({MAX_APPS} apps, each with a choice): set one of them to follow the default first"
            ));
        }
        self.apps.insert(
            app.to_owned(),
            CallApp {
                id: app.to_owned(),
                name: None,
                policy,
                seen_unix_ms: None,
            },
        );
        Ok(())
    }

    /// Detection saw `app` hold the microphone past its hold, at `now_unix_ms`.
    pub fn seen(&mut self, app: &AppRef, now_unix_ms: i64) -> Seen {
        if self.unreadable.is_some() || check_app(&app.id).is_err() {
            return Seen::NotKept;
        }
        let name = clean_name(&app.name);
        if let Some(entry) = self.apps.get_mut(&app.id) {
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
            app.id.clone(),
            CallApp {
                id: app.id.clone(),
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
        let v: Value =
            serde_json::from_str(text).map_err(|_| "the stored list is not JSON".to_owned())?;
        let entries = v
            .get("apps")
            .and_then(Value::as_array)
            .ok_or("the stored list has no apps array")?;
        if entries.len() > MAX_APPS {
            return Err(format!("the stored list holds more than {MAX_APPS} apps"));
        }
        let mut apps = BTreeMap::new();
        for entry in entries {
            let id = entry
                .get("app")
                .and_then(Value::as_str)
                .ok_or("a stored app has no identity")?;
            check_app(id).map_err(|e| format!("a stored app: {e}"))?;
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
                id: id.to_owned(),
                name,
                policy,
                seen_unix_ms,
            };
            if apps.insert(id.to_owned(), app).is_some() {
                return Err("the stored list names an app twice".into());
            }
        }
        Ok(Self {
            default,
            apps,
            unreadable: None,
        })
    }
}

/// Whether `app` is an identity the list takes. The refusal never quotes it.
pub fn check_app(app: &str) -> Result<(), String> {
    if app.is_empty() || app.len() > MAX_APP_ID_BYTES {
        return Err(format!(
            "an app's identity is 1 to {MAX_APP_ID_BYTES} bytes"
        ));
    }
    if app.chars().any(hidden) || app.trim() != app {
        return Err(
            "an app's identity has no control or invisible characters and no white space at either end"
                .into(),
        );
    }
    Ok(())
}

/// A character that reads as nothing or reorders what follows: a control character, a line or
/// paragraph separator, a bidi mark or override, a zero-width mark, a soft hyphen. Kept out of
/// identities and names, so two apps never look alike in the list.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
        )
}

/// A name as the list keeps it: one line, trimmed, at most [`MAX_APP_NAME_CHARS`]; `None` when
/// nothing is left.
fn clean_name(name: &str) -> Option<String> {
    let one_line: String = name.chars().filter(|c| !hidden(*c)).collect();
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
        // A choice starts the list over.
        p.choose("zoom", Some(CallPolicy::Always)).unwrap();
        assert!(p.unreadable().is_none());
        assert_eq!(p.policy("zoom"), CallPolicy::Always);
        assert_eq!(p.policy("chat"), CallPolicy::Always, "the default again");
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
}
