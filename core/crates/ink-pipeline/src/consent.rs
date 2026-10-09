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
//! The store keeps each feature's consent as a setting ([`Feature::setting_key`]), read by
//! [`Feature::read`]. A value that does not read is no consent: the feature fails closed.
//!
//! **Polish holds one consent per destination** (owner decision, 2026-10-05): a dictation mode may
//! polish on a model of its own ([`crate::modes::ModelPin`]), so "the AI setting's model in the
//! cloud, the Notes mode on this machine" needs both. Its setting is a list
//! ([`consents_to_setting`]); a call passes when any of them covers the model reached, by the same
//! rule as one consent ([`LlmConsent::covers`]). The single consent an earlier build stored reads
//! as a list of one, and is written back as a list at the next change, so nothing is asked again.
//! (A build before this one reading the list finds no consent it knows and fails closed.) Voice
//! edit and a meeting's summary keep one consent each: each sends only to the AI setting's model,
//! so there is only ever one destination to agree to.

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

    /// Whether it holds one consent per destination (a list), rather than one consent.
    pub const fn per_destination(self) -> bool {
        matches!(self, Self::Polish)
    }

    /// Its consents as stored under [`setting_key`](Self::setting_key): none, one, or (for a
    /// feature [per destination](Self::per_destination)) several. `Err` for a value that does not
    /// read, which callers treat as no consent and report.
    pub fn read(self, value: Option<&str>) -> Result<Vec<LlmConsent>, UnreadableConsent> {
        if self.per_destination() {
            consents_from_setting(value)
        } else {
            LlmConsent::from_setting(value).map(|c| c.into_iter().collect())
        }
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

/// Where a model sends the words: this machine, or one endpoint elsewhere. What a consent is
/// given for, and what a dictation mode's own model is pinned to when the user picks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// A model on this machine (in this process, or a server on a loopback address).
    OnDevice,
    /// A model elsewhere, by its endpoint ([`Endpoint::Remote`]'s value).
    Cloud(String),
}

impl Destination {
    /// Where `info`'s model sends.
    pub fn of(info: &LlmInfo) -> Self {
        match &info.endpoint {
            Endpoint::InProcess | Endpoint::Loopback(_) => Self::OnDevice,
            Endpoint::Remote(endpoint) => Self::Cloud(endpoint.clone()),
        }
    }

    /// Whether `info`'s model sends here: on-device is any model on this machine (another local
    /// port is still this machine), a cloud destination only its own endpoint.
    pub fn covers(&self, info: &LlmInfo) -> bool {
        match (self, &info.endpoint) {
            (Self::OnDevice, endpoint) => endpoint.is_local(),
            (Self::Cloud(endpoint), Endpoint::Remote(to)) => endpoint == to,
            (Self::Cloud(_), _) => false,
        }
    }

    /// The stored form: `{"to":"on_device"}` or `{"to":"cloud","endpoint":…}`.
    pub fn to_value(&self) -> Value {
        match self {
            Self::OnDevice => json!({"to": "on_device"}),
            Self::Cloud(endpoint) => json!({"to": "cloud", "endpoint": endpoint}),
        }
    }

    /// Reads [`to_value`](Self::to_value)'s form, and nothing else.
    pub fn from_value(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        match obj.get("to").and_then(Value::as_str)? {
            "on_device" if obj.len() == 1 => Some(Self::OnDevice),
            "cloud" if obj.len() == 2 => obj
                .get("endpoint")
                .and_then(Value::as_str)
                .filter(|e| !e.trim().is_empty())
                .map(|e| Self::Cloud(e.to_owned())),
            _ => None,
        }
    }
}

/// How a cloud engine the shell registered is named as an endpoint: this, then its id. The shell
/// gives no address, so the id is all the endpoint says, and another cloud engine could register
/// under the same id later: a consent for one is kept to the model it was given for by name too
/// ([`LlmConsent::covers`]).
pub const SHELL_ENGINE_ENDPOINT: &str = "shell engine ";

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
    /// model on this machine; a cloud consent covers only models at the same endpoint, and for a
    /// cloud engine the shell registered ([`SHELL_ENGINE_ENDPOINT`]: an id, not an address) only
    /// the model of the name agreed to.
    pub fn covers(&self, info: &LlmInfo) -> bool {
        match self {
            Self::Cloud { endpoint, name } if endpoint.starts_with(SHELL_ENGINE_ENDPOINT) => {
                self.destination().covers(info) && *name == display_name(info)
            }
            _ => self.destination().covers(info),
        }
    }

    /// Where it lets the words go.
    pub fn destination(&self) -> Destination {
        match self {
            Self::OnDevice => Destination::OnDevice,
            Self::Cloud { endpoint, .. } => Destination::Cloud(endpoint.clone()),
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
        self.to_value().to_string()
    }

    fn to_value(&self) -> Value {
        match self {
            Self::OnDevice => json!({"to": "on_device"}),
            Self::Cloud { endpoint, name } => {
                json!({"to": "cloud", "endpoint": endpoint, "name": name})
            }
        }
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
        Self::from_value(&v).map(Some)
    }

    fn from_value(v: &Value) -> Result<Self, UnreadableConsent> {
        let obj = v.as_object().ok_or(UnreadableConsent)?;
        let text = |k: &str| obj.get(k).and_then(Value::as_str);
        match text("to") {
            Some("on_device") if obj.len() == 1 => Ok(Self::OnDevice),
            Some("cloud") if obj.len() == 3 => {
                let endpoint = text("endpoint")
                    .filter(|e| !e.trim().is_empty())
                    .ok_or(UnreadableConsent)?;
                Ok(Self::Cloud {
                    endpoint: endpoint.to_owned(),
                    name: text("name").ok_or(UnreadableConsent)?.to_owned(),
                })
            }
            _ => Err(UnreadableConsent),
        }
    }
}

/// A per-destination feature's consents ([`Feature::per_destination`]) as stored: [`NO_CONSENT`]
/// for none, else a JSON list of consents. Also reads one consent alone (what a build before
/// consents per destination stored), as a list of one. Two consents for one destination read as
/// the first. Anything else does not read.
pub fn consents_from_setting(value: Option<&str>) -> Result<Vec<LlmConsent>, UnreadableConsent> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value == NO_CONSENT {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(value).map_err(|_| UnreadableConsent)?;
    let read = match &v {
        Value::Object(_) => vec![LlmConsent::from_value(&v)?],
        Value::Array(list) => list
            .iter()
            .map(LlmConsent::from_value)
            .collect::<Result<_, _>>()?,
        _ => return Err(UnreadableConsent),
    };
    let mut consents: Vec<LlmConsent> = Vec::with_capacity(read.len());
    for c in read {
        if !consents.iter().any(|have| have.same_destination(&c)) {
            consents.push(c);
        }
    }
    Ok(consents)
}

/// The stored form of a per-destination feature's consents: [`NO_CONSENT`] when there are none.
pub fn consents_to_setting(consents: &[LlmConsent]) -> String {
    if consents.is_empty() {
        return NO_CONSENT.to_owned();
    }
    Value::Array(consents.iter().map(LlmConsent::to_value).collect()).to_string()
}

/// **Worker.** `feature`'s consents as stored, read at the moment of use ([`Feature::read`]).
/// Ones that cannot be read are none (logged by name, never by value): the feature fails closed.
pub fn stored(store: &dyn Store, feature: Feature) -> Vec<LlmConsent> {
    let key = feature.setting_key();
    match store.setting(key) {
        Ok(value) => feature.read(value.as_deref()).unwrap_or_else(|e| {
            log::error!("consent: {key}: {e}; nothing is sent");
            Vec::new()
        }),
        Err(e) => {
            log::error!("consent: {key} could not be read ({e}); nothing is sent");
            Vec::new()
        }
    }
}

/// A feature's model, bound to the user's consents for that feature: every call goes through
/// [`Llm::complete_if`], so the model that answers is checked against where the user agreed the
/// words may go: allowed when any of them covers it. Refused, nothing is sent
/// ([`LlmError::NotAllowed`]).
pub struct Consented<'a> {
    /// The model.
    pub inner: &'a dyn Llm,
    /// Empty: never agreed, so no model is allowed.
    pub consents: &'a [LlmConsent],
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
            self.consents.iter().any(|c| c.covers(info))
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

    /// Another cloud engine registered under an id a consent was given for is not the one agreed
    /// to: the shell's endpoint is only its id, so the name agreed to must match too. A provider's
    /// consent (an address) covers any model there: a mode may pick another model at it.
    #[test]
    fn a_cloud_shell_engine_consent_is_kept_to_the_model_agreed_to() {
        let c = LlmConsent::for_model(&cloud("shell engine cloud-a"));
        assert!(c.covers(&cloud("shell engine cloud-a")));
        assert!(!c.covers(&info(
            "shell",
            "Another model",
            Endpoint::Remote("shell engine cloud-a".into())
        )));
        let provider = LlmConsent::for_model(&info(
            "anthropic",
            "model-a",
            Endpoint::Remote("https://api.anthropic.com".into()),
        ));
        assert!(provider.covers(&info(
            "anthropic",
            "model-b",
            Endpoint::Remote("https://api.anthropic.com".into())
        )));
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

    /// Polish's consents, one per destination: the single consent an earlier build stored reads
    /// as a list of one; a list reads back; one destination is held once; anything else is
    /// refused. The others keep one consent, and refuse a list.
    #[test]
    fn polish_holds_a_consent_per_destination_and_reads_the_old_single_one() {
        let cloud = LlmConsent::Cloud {
            endpoint: "https://api.example.com/v1".into(),
            name: "Example".into(),
        };
        let both = vec![LlmConsent::OnDevice, cloud.clone()];
        let stored = consents_to_setting(&both);
        assert_eq!(consents_from_setting(Some(&stored)), Ok(both.clone()));
        assert_eq!(Feature::Polish.read(Some(&stored)), Ok(both.clone()));
        // Migration: what one consent per feature wrote.
        assert_eq!(
            Feature::Polish.read(Some(&cloud.to_setting())),
            Ok(vec![cloud.clone()])
        );
        assert_eq!(Feature::Polish.read(None), Ok(Vec::new()));
        assert_eq!(Feature::Polish.read(Some(NO_CONSENT)), Ok(Vec::new()));
        assert_eq!(consents_to_setting(&[]), NO_CONSENT);
        let twice = r#"[{"to":"on_device"},{"to":"on_device"},{"to":"cloud","endpoint":"https://api.example.com/v1","name":"Example"},{"to":"cloud","endpoint":"https://api.example.com/v1","name":"Newer"}]"#;
        assert_eq!(Feature::Polish.read(Some(twice)), Ok(both));
        for bad in [
            "[1]",
            r#"[{"to":"everywhere"}]"#,
            r#""on""#,
            "3",
            "[",
            r#"[{"to":"on_device"},{}]"#,
        ] {
            assert_eq!(
                Feature::Polish.read(Some(bad)),
                Err(UnreadableConsent),
                "{bad}"
            );
        }
        assert_eq!(
            Feature::Edit.read(Some(&stored)),
            Err(UnreadableConsent),
            "voice edit holds one consent"
        );
        assert_eq!(
            Feature::Meetings.read(Some(&cloud.to_setting())),
            Ok(vec![cloud])
        );
        assert!(Feature::Polish.per_destination());
        assert!(!Feature::Edit.per_destination() && !Feature::Meetings.per_destination());
    }

    /// Any consent in the list lets a call through, by the same rule as one consent: on-device
    /// for any model on this machine, a cloud consent for its own endpoint only.
    #[test]
    fn a_call_passes_when_any_consent_covers_the_model_it_reaches() {
        use ink_core::mock::MockLlm;
        let a = "https://api.a.example/v1";
        let consents = [LlmConsent::Cloud {
            endpoint: a.into(),
            name: "A".into(),
        }];
        let call = |endpoint: Endpoint, consents: &[LlmConsent]| {
            let model = MockLlm::new(endpoint, "ok");
            Consented {
                inner: &model,
                consents,
            }
            .complete(
                &LlmRequest {
                    system: String::new(),
                    user: "words".into(),
                    max_tokens: 8,
                    temperature: 0.0,
                    json_schema: None,
                },
                &CancelToken::new(),
            )
            .is_ok()
        };
        assert!(call(Endpoint::Remote(a.into()), &consents));
        assert!(!call(
            Endpoint::Remote("https://api.b.example/v1".into()),
            &consents
        ));
        assert!(!call(Endpoint::InProcess, &consents));
        let both = [consents[0].clone(), LlmConsent::OnDevice];
        assert!(call(Endpoint::InProcess, &both));
        assert!(call(
            Endpoint::Loopback("http://127.0.0.1:9/v1".into()),
            &both
        ));
        assert!(!call(
            Endpoint::Remote("https://api.b.example/v1".into()),
            &both
        ));
        assert!(!call(Endpoint::InProcess, &[]), "none: nothing is allowed");
    }

    #[test]
    fn a_destination_reads_back_and_covers_as_a_consent_does() {
        for d in [
            Destination::OnDevice,
            Destination::Cloud("https://api.example.com/v1".into()),
        ] {
            assert_eq!(Destination::from_value(&d.to_value()), Some(d));
        }
        for bad in [
            json!({"to": "cloud"}),
            json!({"to": "cloud", "endpoint": " "}),
            json!({"to": "on_device", "endpoint": "x"}),
            json!("on_device"),
        ] {
            assert_eq!(Destination::from_value(&bad), None, "{bad}");
        }
        let here = Destination::OnDevice;
        assert!(here.covers(&info(
            "x",
            "m",
            Endpoint::Loopback("http://127.0.0.1:1".into())
        )));
        assert!(!here.covers(&cloud("shell engine x")));
        let there = Destination::Cloud("shell engine x".into());
        assert!(there.covers(&cloud("shell engine x")));
        assert!(!there.covers(&cloud("shell engine y")));
        assert_eq!(Destination::of(&cloud("shell engine x")), there);
    }
}
