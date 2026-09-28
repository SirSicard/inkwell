//! Dictation polish's switch and consent, as the screens see them (`polish.get`, `polish.allow`,
//! and `setting.set` of `dictation.polish` to `off`), answered on `queries`' thread.
//!
//! Polish sends a dictation to a language model before it is typed, so it runs only with the
//! user's consent for where that model sends it ([`PolishConsent`]): this machine, or one named
//! cloud provider. The core keeps the consent ([`POLISH_CONSENT_SETTING`]) and is the only one
//! that writes it:
//!
//! - **On** only through `polish.allow`, which names the destination the user agreed to. It is
//!   recorded only if it is where the model polish would use goes now; otherwise the model changed
//!   while the user read, and the command fails so the shell can ask again. `setting.set` refuses
//!   `dictation.polish` = `on`.
//! - **Off** through `setting.set` (`off`): the consent is withdrawn in the same write, so turning
//!   polish on again always asks, and no stale consent can outlive the switch.
//! - Whatever turned polish on, the chain polishes only where the consent covers, checked by the
//!   model each call reaches (`ink_pipeline::chain`): a model that changed destination since gets
//!   nothing until the user agrees again, and the take says so (`polish_not_allowed`).

use ink_core::Llm as _;
use ink_pipeline::consent::{NO_CONSENT, PolishConsent};
use serde_json::Value;

use crate::events::event;
use crate::runtime::Shared;
use crate::voice::{POLISH_CONSENT_SETTING, POLISH_SETTING};

/// The model polish would use now, if one is registered: the consent it needs and its name.
fn current(shared: &Shared) -> Option<(ink_core::LlmInfo, PolishConsent, String)> {
    shared.llms.pick().map(|llm| {
        let info = llm.info();
        let needed = PolishConsent::for_model(&info);
        let name = match &needed {
            PolishConsent::Cloud { name, .. } => name.clone(),
            _ => info.model.clone(),
        };
        (info, needed, name)
    })
}

/// **Queries thread.** `polish.state`: the switch, where polish would send now, and the consent.
/// A part that cannot be read is reported ("couldn't …") and counts as off, or as no consent.
pub fn state(shared: &Shared, reference: Option<&str>) -> Value {
    let store = shared.store.as_ref();
    let mut errors = Vec::new();
    let on = match store.setting(POLISH_SETTING) {
        Ok(v) => v.as_deref() == Some("on"),
        Err(e) => {
            log::error!("polish: the switch could not be read: {e}");
            errors.push("the polish switch");
            false
        }
    };
    let consent = match store.setting(POLISH_CONSENT_SETTING) {
        Ok(v) => PolishConsent::from_setting(v.as_deref()).unwrap_or_else(|e| {
            log::error!("polish: {e}");
            errors.push("your polish consent");
            None
        }),
        Err(e) => {
            log::error!("polish: the consent could not be read: {e}");
            errors.push("your polish consent");
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
                PolishConsent::Cloud { endpoint, .. } => Some(endpoint.into()),
                PolishConsent::OnDevice => None,
            },
        ),
        None => (None, None, None),
    };
    event(
        "polish.state",
        &[
            ("on", Some(on.into())),
            ("to", to),
            ("name", name),
            ("endpoint", endpoint),
            ("allowed_to", consent.as_ref().map(|c| c.kind().into())),
            (
                "allowed_name",
                match &consent {
                    Some(PolishConsent::Cloud { name, .. }) => Some(name.clone().into()),
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

/// **Queries thread.** `polish.allow`: records the user's consent for `asked` (the destination
/// the shell showed; a cloud one's name is not needed) and turns polish on, if `asked` is where
/// the model polish would use goes now. What is recorded is the model's own destination and name. Answers `polish.state`, or an
/// error for `command.failed`: no model to agree to, the model changed while the user read (the
/// shell asks again), or the store refused.
pub fn allow(
    shared: &Shared,
    asked: &PolishConsent,
    reference: Option<&str>,
) -> Result<Value, String> {
    let Some((_, needed, _)) = current(shared) else {
        return Err("no language model is set up for polish".into());
    };
    if !asked.same_destination(&needed) {
        // Nothing recorded: the user agreed to something that is no longer where polish goes.
        shared.events.emit(state(shared, None));
        return Err("polish's model changed before you agreed; look again".into());
    }
    // The consent and the switch together, or neither: a consent saved without the switch (or
    // the reverse) is a state the user never chose.
    shared
        .store
        .set_settings(&[
            (POLISH_CONSENT_SETTING, &needed.to_setting()),
            (POLISH_SETTING, "on"),
        ])
        .map_err(|e| {
            log::error!("polish: the consent and the switch could not be saved: {e}");
            "couldn't save your consent, so polish stays off".to_owned()
        })?;
    shared.events.emit(crate::queries::setting_value(
        POLISH_SETTING,
        Some("on".into()),
    ));
    crate::voice::settings_changed(shared);
    Ok(state(shared, reference))
}

/// **Queries thread.** `setting.set` of `dictation.polish` to `off`: the switch off and the
/// consent withdrawn in one write, so turning polish on again always asks. A failure changes
/// neither, and is the command's to report.
pub fn turn_off(shared: &Shared) -> Result<(), String> {
    shared
        .store
        .set_settings(&[
            (POLISH_SETTING, "off"),
            (POLISH_CONSENT_SETTING, NO_CONSENT),
        ])
        .map_err(|e| {
            log::error!("polish: could not be turned off: {e}");
            format!("couldn't turn polish off: {e}")
        })
}
