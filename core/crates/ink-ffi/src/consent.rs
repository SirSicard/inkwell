//! Consent for the features that send the user's words to a language model, as the screens see
//! them (`consent.get`, `consent.allow`, and `setting.set` of a feature's switch to `off`),
//! answered on `queries`' thread.
//!
//! Dictation polish sends a dictation, voice edit the selection and the instruction, and a
//! meeting's summary and Ask the meeting's transcript, to a language model, so each runs only with
//! the user's consent for where that model sends it ([`LlmConsent`], per [`Feature`]): this
//! machine, or one named cloud provider. The core keeps each consent ([`Feature::setting_key`])
//! and is the only one that writes it:
//!
//! - **On** only through `consent.allow`, which names the feature and the destination the user
//!   agreed to. It is recorded only if it is where the model the feature would use goes now;
//!   otherwise the model changed while the user read, and the command fails so the shell can ask
//!   again. The consent and the feature's switch are written together, or neither: polish's switch
//!   (`dictation.polish` = `on`; `setting.set` refuses `on`), voice edit's key
//!   (`dictation.edit_key`, named by the command), or the meetings switch ([`MEETINGS_SETTING`] =
//!   `on`; `setting.set` refuses `on`).
//! - **Off** through `setting.set` (`dictation.polish` = `off`, `dictation.edit_key` = `off`,
//!   `meetings.llm` = `off`): the consent is withdrawn in the same write, so turning the feature
//!   on again always asks, and no stale consent can outlive the switch. A failure changes neither.
//! - **Polish holds one consent per destination** ([`Feature::per_destination`]): a dictation mode
//!   may polish on a model of its own, somewhere else than the AI setting's. `consent.allow` for
//!   polish adds one, for any destination a model polish can use goes to now (the AI setting's, or
//!   one a mode can pick: `modes.listed`'s `polish_models`), and keeps the others; `consent.revoke`
//!   takes one away (the last one taken turns polish off with it, in the same write);
//!   `consent.state` lists them (`consents`). Turning polish off withdraws them all.
//! - Whatever turned a feature on, the chain sends only where its consent covers, checked by the
//!   model each call reaches (`ink_pipeline::chain`): a model that changed destination since gets
//!   nothing until the user agrees again, and the take says so (`polish_not_allowed`, or an edit's
//!   `not_allowed`); a meeting's final pass says `summary_not_allowed`, and Ask fails asking for
//!   the user's OK (`crate::asking`).

use ink_pipeline::consent::{Feature, LlmConsent, NO_CONSENT, consents_to_setting};
use serde_json::{Value, json};

use crate::events::event;
use crate::runtime::Shared;
use crate::voice::{EDIT_KEY_SETTING, POLISH_SETTING};

/// The store setting holding the meetings switch (`on` or `off`): a meeting's summary (with its
/// commitments) and Ask send its transcript to a language model. Only `consent.allow` turns it on.
pub const MEETINGS_SETTING: &str = "meetings.llm";

/// A feature's switch: its store key, and whether a stored value means on.
pub type Switch = (&'static str, fn(Option<&str>) -> bool);

/// A feature's switch in the store. `None` for a feature this module does not switch yet (the
/// commands refuse it).
pub fn switch(feature: Feature) -> Option<Switch> {
    match feature {
        Feature::Polish => Some((POLISH_SETTING, |v| v == Some("on"))),
        Feature::Edit => Some((EDIT_KEY_SETTING, |v| v.is_some_and(|k| k != "off"))),
        Feature::Meetings => Some((MEETINGS_SETTING, |v| v == Some("on"))),
        _ => None,
    }
}

/// The switch of a feature the commands accepted (they refuse one without a switch).
fn switch_of(feature: Feature) -> Result<Switch, String> {
    switch(feature).ok_or_else(|| format!("{} has no switch", feature.name()))
}

/// The model the features would use now, if one is registered: the consent it needs and its name.
fn current(shared: &Shared) -> Option<(ink_core::LlmInfo, LlmConsent, String)> {
    shared.llms.pick().map(|llm| described(llm.info()))
}

/// A model's info, the consent it needs, and its name for the user.
pub fn described(info: ink_core::LlmInfo) -> (ink_core::LlmInfo, LlmConsent, String) {
    let needed = LlmConsent::for_model(&info);
    let name = match &needed {
        LlmConsent::Cloud { name, .. } => name.clone(),
        LlmConsent::OnDevice => info.model.clone(),
    };
    (info, needed, name)
}

/// The consent `feature` would record for `asked`: the one the model it can use now at that
/// destination needs (its destination and name as the model gives them), or `None` when no such
/// model is there. Polish can use every model a mode can pick, the AI setting's first; the
/// others only the AI setting's.
fn needed_for(shared: &Shared, feature: Feature, asked: &LlmConsent) -> Option<LlmConsent> {
    let candidates: Vec<LlmConsent> = if feature.per_destination() {
        shared
            .llms
            .choices()
            .into_iter()
            .map(|(_, info)| described(info).1)
            .collect()
    } else {
        current(shared)
            .map(|(_, needed, _)| needed)
            .into_iter()
            .collect()
    };
    candidates.into_iter().find(|c| c.same_destination(asked))
}

/// A consent as `consent.state` lists it.
fn listed(consent: &LlmConsent) -> Value {
    match consent {
        LlmConsent::OnDevice => json!({"to": "on_device"}),
        LlmConsent::Cloud { endpoint, name } => {
            json!({"to": "cloud", "name": name, "endpoint": endpoint})
        }
    }
}

/// **Queries thread.** `consent.state` for `feature`: its switch, where it would send now, and the
/// consent. A part that cannot be read is reported ("couldn't …") and counts as off, or as no
/// consent.
pub fn state(shared: &Shared, feature: Feature, reference: Option<&str>) -> Value {
    let store = shared.store.as_ref();
    let mut errors = Vec::new();
    let on = match switch(feature).map(|(key, is_on)| (key, is_on, store.setting(key))) {
        Some((_, is_on, Ok(v))) => is_on(v.as_deref()),
        Some((key, _, Err(e))) => {
            log::error!("consent: {key} could not be read: {e}");
            errors.push("the switch");
            false
        }
        None => false,
    };
    let consents = match store.setting(feature.setting_key()) {
        Ok(v) => feature.read(v.as_deref()).unwrap_or_else(|e| {
            log::error!("consent: {}: {e}", feature.name());
            errors.push("your consent");
            Vec::new()
        }),
        Err(e) => {
            log::error!("consent: {}'s could not be read: {e}", feature.name());
            errors.push("your consent");
            Vec::new()
        }
    };
    // One pick, so what is shown and whether it is allowed describe the same model.
    let now = current(shared);
    let covering = now
        .as_ref()
        .and_then(|(info, _, _)| consents.iter().find(|c| c.covers(info)));
    let allowed = covering.is_some();
    // The one covering the model now, else the first: what one consent per feature showed.
    let consent = covering.or_else(|| consents.first());
    let (to, name, endpoint) = match now {
        Some((_, needed, name)) => (
            Some(needed.kind().into()),
            Some(name.into()),
            match needed {
                LlmConsent::Cloud { endpoint, .. } => Some(endpoint.into()),
                LlmConsent::OnDevice => None,
            },
        ),
        None => (None, None, None),
    };
    event(
        "consent.state",
        &[
            ("feature", Some(feature.name().into())),
            ("on", Some(on.into())),
            ("to", to),
            ("name", name),
            ("endpoint", endpoint),
            ("allowed_to", consent.as_ref().map(|c| c.kind().into())),
            (
                "allowed_name",
                match &consent {
                    Some(LlmConsent::Cloud { name, .. }) => Some(name.clone().into()),
                    _ => None,
                },
            ),
            ("allowed", Some(allowed.into())),
            (
                "consents",
                Some(Value::Array(consents.iter().map(listed).collect())),
            ),
            (
                "error",
                (!errors.is_empty())
                    .then(|| format!("couldn't read {}", errors.join(" or ")).into()),
            ),
            ("ref", reference.map(Into::into)),
        ],
    )
}

/// What `consent.allow` asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allow {
    /// The feature.
    pub feature: Feature,
    /// The destination the shell showed (a cloud one's name is not needed to compare).
    pub asked: LlmConsent,
    /// For voice edit, the key it is turned on with, as stored ([`crate::hotkey::stored_value`]).
    pub key: Option<String>,
}

/// Reads `consent.allow`'s feature and key: voice edit needs a key, the others take none.
pub fn check_key(feature: Feature, key: Option<&str>) -> Result<Option<String>, String> {
    match (feature, key) {
        (Feature::Edit, Some(k)) if k != "off" => crate::hotkey::stored_value(k)
            .map(Some)
            .map_err(|why| format!("voice edit can't use \"{k}\": {why}")),
        (Feature::Edit, _) => Err("voice edit is turned on with its key: \"key\"".into()),
        (_, None) => Ok(None),
        (_, Some(_)) => Err(format!("{} takes no \"key\"", feature.name())),
    }
}

/// **Queries thread.** `consent.allow`: records the user's consent for the feature and turns it
/// on, if the destination asked for is where a model the feature can use goes now (for polish,
/// any model a mode can pick; for the others, the AI setting's). What is recorded is the model's
/// own destination and name. Polish keeps its consents for other destinations; the others hold
/// this one only. Answers `consent.state`, or an error for `command.failed`: no model to agree
/// to, the model changed while the user read (the shell asks again), or the store refused (then
/// nothing is saved).
pub fn allow(shared: &Shared, allow: &Allow, reference: Option<&str>) -> Result<Value, String> {
    if let (Feature::Edit, Some(key)) = (allow.feature, allow.key.as_deref()) {
        crate::hotkey::unique_setting(shared.store.as_ref(), crate::voice::EDIT_KEY_SETTING, key)?;
    }
    let feature = allow.feature;
    if shared.llms.choices().is_empty() {
        return Err(format!(
            "no language model is set up for {}",
            feature.name()
        ));
    }
    let Some(needed) = needed_for(shared, feature, &allow.asked) else {
        // Nothing recorded: the user agreed to something that is no longer where it goes.
        shared.events.emit(state(shared, feature, None));
        return Err(format!(
            "{}'s model changed before you agreed; look again",
            feature.name()
        ));
    };
    let (switch_key, _) = switch_of(feature)?;
    let on = match (feature, allow.key.as_deref()) {
        (Feature::Edit, Some(key)) => key,
        _ => "on",
    };
    let value = if feature.per_destination() {
        // Added to the others; one for the same destination is replaced (its name may be newer).
        // Ones that cannot be read are dropped: they never counted.
        let mut consents = stored_or_none(shared, feature);
        consents.retain(|c| !c.same_destination(&needed));
        consents.push(needed);
        consents_to_setting(&consents)
    } else {
        needed.to_setting()
    };
    // The consent and the switch together, or neither: a consent saved without the switch (or
    // the reverse) is a state the user never chose.
    shared
        .store
        .set_settings(&[(feature.setting_key(), &value), (switch_key, on)])
        .map_err(|e| {
            log::error!(
                "consent: {}'s consent and switch could not be saved: {e}",
                feature.name()
            );
            format!(
                "couldn't save your consent, so {} stays off",
                feature.name()
            )
        })?;
    shared
        .events
        .emit(crate::queries::setting_value(switch_key, Some(on.into())));
    crate::voice::settings_changed(shared);
    Ok(state(shared, feature, reference))
}

/// A per-destination feature's consents as stored; none when they cannot be read (logged).
fn stored_or_none(shared: &Shared, feature: Feature) -> Vec<LlmConsent> {
    ink_pipeline::consent::stored(shared.store.as_ref(), feature)
}

/// **Queries thread.** `consent.revoke`: takes polish's consent for the destination `asked` away;
/// the others stay. Taking the last one turns polish off in the same write, as turning it off
/// withdraws them all. A destination without a consent is no failure. Answers `consent.state`, or
/// an error for `command.failed`: the consents cannot be read (nothing changes: the user could not
/// see what they revoke), or the store refused.
pub fn revoke(
    shared: &Shared,
    feature: Feature,
    asked: &LlmConsent,
    reference: Option<&str>,
) -> Result<Value, String> {
    if !feature.per_destination() {
        return Err(format!(
            "{} has one consent: turn it off to withdraw it",
            feature.name()
        ));
    }
    let (switch_key, _) = switch_of(feature)?;
    let stored = shared
        .store
        .setting(feature.setting_key())
        .map_err(|e| e.to_string())
        .and_then(|v| feature.read(v.as_deref()).map_err(|e| e.to_string()))
        .map_err(|e| {
            log::error!(
                "consent: {}'s consents could not be read: {e}",
                feature.name()
            );
            format!("couldn't read your consents for {}", feature.name())
        })?;
    let kept: Vec<LlmConsent> = stored
        .iter()
        .filter(|c| !c.same_destination(asked))
        .cloned()
        .collect();
    if kept.len() == stored.len() {
        return Ok(state(shared, feature, reference));
    }
    let value = consents_to_setting(&kept);
    let mut writes: Vec<(&str, &str)> = vec![(feature.setting_key(), &value)];
    if kept.is_empty() {
        writes.push((switch_key, "off"));
    }
    shared.store.set_settings(&writes).map_err(|e| {
        log::error!(
            "consent: {}'s consent could not be revoked: {e}",
            feature.name()
        );
        format!(
            "couldn't revoke the consent, so {} is as it was",
            feature.name()
        )
    })?;
    if kept.is_empty() {
        shared.events.emit(crate::queries::setting_value(
            switch_key,
            Some("off".into()),
        ));
    }
    crate::voice::settings_changed(shared);
    Ok(state(shared, feature, reference))
}

/// The feature whose switch `key` is, if turning it off withdraws a consent.
pub fn feature_switched_by(key: &str) -> Option<Feature> {
    Feature::ALL
        .into_iter()
        .find(|f| switch(*f).is_some_and(|(k, _)| k == key))
}

/// **Queries thread.** `setting.set` of a feature's switch to `off`: the switch off and the
/// consent withdrawn in one write, so turning it on again always asks. A failure changes neither,
/// and is the command's to report.
pub fn turn_off(shared: &Shared, feature: Feature) -> Result<(), String> {
    let (switch_key, _) = switch_of(feature)?;
    shared
        .store
        .set_settings(&[(switch_key, "off"), (feature.setting_key(), NO_CONSENT)])
        .map_err(|e| {
            log::error!("consent: {} could not be turned off: {e}", feature.name());
            format!("couldn't turn {} off: {e}", feature.name())
        })
}
