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
//! - Whatever turned a feature on, the chain sends only where its consent covers, checked by the
//!   model each call reaches (`ink_pipeline::chain`): a model that changed destination since gets
//!   nothing until the user agrees again, and the take says so (`polish_not_allowed`, or an edit's
//!   `not_allowed`); a meeting's final pass says `summary_not_allowed`, and Ask fails asking for
//!   the user's OK (`crate::asking`).

use ink_pipeline::consent::{Feature, LlmConsent, NO_CONSENT};
use serde_json::Value;

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
    shared.llms.pick().map(|llm| {
        let info = llm.info();
        let needed = LlmConsent::for_model(&info);
        let name = match &needed {
            LlmConsent::Cloud { name, .. } => name.clone(),
            LlmConsent::OnDevice => info.model.clone(),
        };
        (info, needed, name)
    })
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
    let consent = match store.setting(feature.setting_key()) {
        Ok(v) => LlmConsent::from_setting(v.as_deref()).unwrap_or_else(|e| {
            log::error!("consent: {}: {e}", feature.name());
            errors.push("your consent");
            None
        }),
        Err(e) => {
            log::error!("consent: {}'s could not be read: {e}", feature.name());
            errors.push("your consent");
            None
        }
    };
    // One pick, so what is shown and whether it is allowed describe the same model.
    let now = current(shared);
    let allowed = match (&consent, &now) {
        (Some(consent), Some((info, _, _))) => consent.covers(info),
        _ => false,
    };
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
/// on, if the destination asked for is where the model it would use goes now. What is recorded is
/// the model's own destination and name. Answers `consent.state`, or an error for
/// `command.failed`: no model to agree to, the model changed while the user read (the shell asks
/// again), or the store refused (then nothing is saved).
pub fn allow(shared: &Shared, allow: &Allow, reference: Option<&str>) -> Result<Value, String> {
    let feature = allow.feature;
    let Some((_, needed, _)) = current(shared) else {
        return Err(format!(
            "no language model is set up for {}",
            feature.name()
        ));
    };
    if !allow.asked.same_destination(&needed) {
        // Nothing recorded: the user agreed to something that is no longer where it goes.
        shared.events.emit(state(shared, feature, None));
        return Err(format!(
            "{}'s model changed before you agreed; look again",
            feature.name()
        ));
    }
    let (switch_key, _) = switch_of(feature)?;
    let on = match (feature, allow.key.as_deref()) {
        (Feature::Edit, Some(key)) => key,
        _ => "on",
    };
    // The consent and the switch together, or neither: a consent saved without the switch (or
    // the reverse) is a state the user never chose.
    shared
        .store
        .set_settings(&[
            (feature.setting_key(), &needed.to_setting()),
            (switch_key, on),
        ])
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
