//! Consent for sending the user's words to a language model: which feature, and where to.
//!
//! Each feature that sends text to a language model runs only with the user's consent for that
//! feature ([`Feature`]), given for a destination: **this machine** (a model in this process, such
//! as Apple's on-device model, or a server on a loopback address), or **one named cloud provider**
//! (a model whose endpoint is elsewhere). That is dictation polish (the dictation), voice edit
//! (the selection and the spoken instruction), and a meeting's summary and Ask (the meeting's
//! transcript). The call is checked against the model it reaches ([`ink_core::Llm::complete_if`]
//! with [`LlmConsent::covers`], through [`Consented`]), so a model that changed from on-device to
//! a cloud provider, or from one provider to another, never receives a word until the user agrees
//! again. Without a covering consent nothing is sent and the chain says why
//! ([`Warning::PolishNotAllowed`](crate::events::Warning::PolishNotAllowed),
//! [`EditFailure::NotAllowed`](crate::events::EditFailure::NotAllowed),
//! [`MeetingWarning::SummaryNotAllowed`](crate::meeting::events::MeetingWarning::SummaryNotAllowed)).
//!
//! The store keeps each feature's consent as a setting ([`Feature::setting_key`]), written by
//! [`LlmConsent::to_setting`] and read by [`LlmConsent::from_setting`]. A value that does not read
//! is no consent: the feature fails closed.

use ink_core::{CancelToken, Endpoint, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse, Store};
use serde_json::{Value, json};

/// A feature that sends the user's words to a language model, each with a consent of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Feature {
    /// Dictation polish: the dictated text, before it is typed.
    Polish,
    /// Voice edit: the selected text and the spoken instruction.
    Edit,
    /// A meeting's summary (with its commitments) and Ask: the meeting's transcript. One consent
    /// for both: they send the same transcript to the same model.
    Meetings,
}

impl Feature {
    /// Every feature.
    pub const ALL: [Self; 3] = [Self::Polish, Self::Edit, Self::Meetings];

    /// Its name in commands and events.
    pub fn name(self) -> &'static str {
        match self {
            Self::Polish => "polish",
            Self::Edit => "edit",
            Self::Meetings => "meetings",
        }
    }

    /// The feature `name` names.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == name)
    }

    /// The store setting holding its consent.
    pub const fn setting_key(self) -> &'static str {
        match self {
            Self::Polish => "llm.consent.polish",
            Self::Edit => "llm.consent.edit",
            Self::Meetings => "llm.consent.meetings",
        }
    }
}

/// What the user agreed to: a feature may send their words to this destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LlmConsent {
    /// A model on this machine: the words stay on it.
    OnDevice,
    /// One cloud provider: the words leave this machine for it.
    Cloud {
        /// Where the model sends the words ([`Endpoint::Remote`]'s value): the consent's key. A
        /// model with another endpoint is another provider and needs consent of its own.
        endpoint: String,
        /// The provider's name as the user saw it when agreeing.
        name: String,
    },
}

/// The setting's value when no consent is given (or it was withdrawn).
pub const NO_CONSENT: &str = "none";

/// A stored consent that does not read. Treated as no consent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnreadableConsent;

impl std::fmt::Display for UnreadableConsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a stored consent cannot be read")
    }
}

impl std::error::Error for UnreadableConsent {}

impl LlmConsent {
    /// The consent `info`'s model needs: what the user must agree to before a feature sends to it.
    pub fn for_model(info: &LlmInfo) -> Self {
        match &info.endpoint {
            Endpoint::InProcess | Endpoint::Loopback(_) => Self::OnDevice,
            Endpoint::Remote(endpoint) => Self::Cloud {
                endpoint: endpoint.clone(),
                name: display_name(info),
            },
        }
    }

    /// Whether this consent lets the feature send to `info`'s model. On-device consent covers any
    /// model on this machine; a cloud consent covers only models at the same endpoint.
    pub fn covers(&self, info: &LlmInfo) -> bool {
        match (self, &info.endpoint) {
            (Self::OnDevice, endpoint) => endpoint.is_local(),
            (Self::Cloud { endpoint, .. }, Endpoint::Remote(to)) => endpoint == to,
            (Self::Cloud { .. }, _) => false,
        }
    }

    /// Whether this is the same destination as `other` (the names may differ).
    pub fn same_destination(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::OnDevice, Self::OnDevice) => true,
            (Self::Cloud { endpoint: a, .. }, Self::Cloud { endpoint: b, .. }) => a == b,
            _ => false,
        }
    }

    /// `on_device` or `cloud`, as the events name it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::OnDevice => "on_device",
            Self::Cloud { .. } => "cloud",
        }
    }

    /// The stored form: a JSON object.
    pub fn to_setting(&self) -> String {
        match self {
            Self::OnDevice => json!({"to": "on_device"}),
            Self::Cloud { endpoint, name } => {
                json!({"to": "cloud", "endpoint": endpoint, "name": name})
            }
        }
        .to_string()
    }

    /// Reads the stored form. `Ok(None)` for no consent (nothing stored, or [`NO_CONSENT`]);
    /// `Err` for a value that does not read, which callers treat as no consent and report.
    pub fn from_setting(value: Option<&str>) -> Result<Option<Self>, UnreadableConsent> {
        let Some(value) = value else {
            return Ok(None);
        };
        if value == NO_CONSENT {
            return Ok(None);
        }
        let v: Value = serde_json::from_str(value).map_err(|_| UnreadableConsent)?;
        let obj = v.as_object().ok_or(UnreadableConsent)?;
        let text = |k: &str| obj.get(k).and_then(Value::as_str);
        match text("to") {
            Some("on_device") if obj.len() == 1 => Ok(Some(Self::OnDevice)),
            Some("cloud") if obj.len() == 3 => {
                let endpoint = text("endpoint")
                    .filter(|e| !e.trim().is_empty())
                    .ok_or(UnreadableConsent)?;
                Ok(Some(Self::Cloud {
                    endpoint: endpoint.to_owned(),
                    name: text("name").ok_or(UnreadableConsent)?.to_owned(),
                }))
            }
            _ => Err(UnreadableConsent),
        }
    }
}

/// **Worker.** `feature`'s consent as stored, read at the moment of use. One that cannot be read
/// is none (logged by name, never by value): the feature fails closed.
pub fn stored(store: &dyn Store, feature: Feature) -> Option<LlmConsent> {
    let key = feature.setting_key();
    match store.setting(key) {
        Ok(value) => LlmConsent::from_setting(value.as_deref()).unwrap_or_else(|e| {
            log::error!("consent: {key}: {e}; nothing is sent");
            None
        }),
        Err(e) => {
            log::error!("consent: {key} could not be read ({e}); nothing is sent");
            None
        }
    }
}

/// A feature's model, bound to the user's consent for that feature: every call goes through
/// [`Llm::complete_if`], so the model that answers is checked against where the user agreed the
/// words may go. Refused, nothing is sent ([`LlmError::NotAllowed`]).
pub struct Consented<'a> {
    /// The model.
    pub inner: &'a dyn Llm,
    /// `None`: never agreed, so no model is allowed.
    pub consent: Option<&'a LlmConsent>,
}

impl Llm for Consented<'_> {
    fn info(&self) -> LlmInfo {
        self.inner.info()
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        self.inner.complete_if(request, cancel, &|info| {
            self.consent.is_some_and(|c| c.covers(info))
        })
    }
}

/// A model's name for the user: its provider and model, or the model alone for a shell engine
/// (whose provider is the shell, which names the model itself).
fn display_name(info: &LlmInfo) -> String {
    if info.provider == "shell" || info.provider.trim().is_empty() {
        info.model.clone()
    } else {
        format!("{} ({})", info.model, info.provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(provider: &str, model: &str, endpoint: Endpoint) -> LlmInfo {
        LlmInfo {
            provider: provider.into(),
            model: model.into(),
            endpoint,
        }
    }

    fn cloud(endpoint: &str) -> LlmInfo {
        info("shell", "Cloud model", Endpoint::Remote(endpoint.into()))
    }

    #[test]
    fn on_device_consent_covers_every_model_on_this_machine_and_no_other() {
        let c = LlmConsent::OnDevice;
        assert!(c.covers(&info("shell", "on-device", Endpoint::InProcess)));
        assert!(c.covers(&info(
            "local",
            "m",
            Endpoint::Loopback("http://127.0.0.1:8080/v1".into())
        )));
        assert!(!c.covers(&cloud("shell engine cloud-a")));
    }

    #[test]
    fn cloud_consent_covers_only_its_own_provider() {
        let c = LlmConsent::for_model(&cloud("shell engine cloud-a"));
        assert!(c.covers(&cloud("shell engine cloud-a")));
        assert!(
            !c.covers(&cloud("shell engine cloud-b")),
            "another provider needs its own consent"
        );
        assert!(
            !c.covers(&info("shell", "on-device", Endpoint::InProcess)),
            "consent for a provider is not consent for this machine either"
        );
    }

    #[test]
    fn the_destination_is_named_for_the_user() {
        assert_eq!(
            LlmConsent::for_model(&info(
                "anthropic",
                "claude",
                Endpoint::Remote("https://api.anthropic.com/v1".into())
            )),
            LlmConsent::Cloud {
                endpoint: "https://api.anthropic.com/v1".into(),
                name: "claude (anthropic)".into()
            }
        );
        assert_eq!(
            LlmConsent::for_model(&cloud("shell engine x")),
            LlmConsent::Cloud {
                endpoint: "shell engine x".into(),
                name: "Cloud model".into()
            }
        );
    }

    #[test]
    fn features_have_names_and_settings_of_their_own() {
        for f in Feature::ALL {
            assert_eq!(Feature::parse(f.name()), Some(f));
        }
        let keys: std::collections::HashSet<_> = Feature::ALL.map(Feature::setting_key).into();
        assert_eq!(keys.len(), Feature::ALL.len(), "each its own setting");
        assert_eq!(
            Feature::parse("summary"),
            None,
            "summary and Ask are `meetings`"
        );
    }

    #[test]
    fn the_stored_form_reads_back_and_anything_else_is_refused() {
        for c in [
            LlmConsent::OnDevice,
            LlmConsent::Cloud {
                endpoint: "https://api.example.com/v1".into(),
                name: "Example".into(),
            },
        ] {
            assert_eq!(LlmConsent::from_setting(Some(&c.to_setting())), Ok(Some(c)));
        }
        assert_eq!(LlmConsent::from_setting(None), Ok(None));
        assert_eq!(LlmConsent::from_setting(Some(NO_CONSENT)), Ok(None));
        for bad in [
            "",
            "on",
            "{}",
            r#"{"to":"everywhere"}"#,
            r#"{"to":"cloud","name":"x"}"#,
            r#"{"to":"cloud","endpoint":" ","name":"x"}"#,
            r#"{"to":"on_device","endpoint":"x"}"#,
        ] {
            assert_eq!(
                LlmConsent::from_setting(Some(bad)),
                Err(UnreadableConsent),
                "{bad}"
            );
        }
    }
}
