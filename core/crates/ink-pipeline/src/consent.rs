//! Polish's consent: where the user agreed their dictated words may go before they are typed.
//!
//! Polish sends a dictation to a language model. It runs only with the user's consent, given for
//! a destination: **this machine** (a model in this process, such as Apple's on-device model, or
//! a server on a loopback address), or **one named cloud provider** (a model whose endpoint is
//! elsewhere). The chain checks the consent against the model each polish call reaches
//! ([`ink_core::Llm::complete_if`]), so a model that changed from on-device to a cloud provider,
//! or from one provider to another, never receives a word until the user agrees again. Without a
//! consent that covers the model, the take goes out as written and the chain says why
//! ([`Warning::PolishNotAllowed`](crate::events::Warning::PolishNotAllowed)).
//!
//! The store keeps the consent as a setting (`dictation.polish_consent` in ink-ffi), written by
//! [`PolishConsent::to_setting`] and read by [`PolishConsent::from_setting`]. A value that does
//! not read is no consent: polish fails closed.

use ink_core::{Endpoint, LlmInfo};
use serde_json::{Value, json};

/// What the user agreed to: polish may send dictations to this destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolishConsent {
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
        f.write_str("the stored polish consent cannot be read")
    }
}

impl std::error::Error for UnreadableConsent {}

impl PolishConsent {
    /// The consent `info`'s model needs: what the user must agree to before polish sends to it.
    pub fn for_model(info: &LlmInfo) -> Self {
        match &info.endpoint {
            Endpoint::InProcess | Endpoint::Loopback(_) => Self::OnDevice,
            Endpoint::Remote(endpoint) => Self::Cloud {
                endpoint: endpoint.clone(),
                name: display_name(info),
            },
        }
    }

    /// Whether this consent lets polish send to `info`'s model. On-device consent covers any
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
        let c = PolishConsent::OnDevice;
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
        let c = PolishConsent::for_model(&cloud("shell engine cloud-a"));
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
            PolishConsent::for_model(&info(
                "anthropic",
                "claude",
                Endpoint::Remote("https://api.anthropic.com/v1".into())
            )),
            PolishConsent::Cloud {
                endpoint: "https://api.anthropic.com/v1".into(),
                name: "claude (anthropic)".into()
            }
        );
        assert_eq!(
            PolishConsent::for_model(&cloud("shell engine x")),
            PolishConsent::Cloud {
                endpoint: "shell engine x".into(),
                name: "Cloud model".into()
            }
        );
    }

    #[test]
    fn the_stored_form_reads_back_and_anything_else_is_refused() {
        for c in [
            PolishConsent::OnDevice,
            PolishConsent::Cloud {
                endpoint: "https://api.example.com/v1".into(),
                name: "Example".into(),
            },
        ] {
            assert_eq!(
                PolishConsent::from_setting(Some(&c.to_setting())),
                Ok(Some(c))
            );
        }
        assert_eq!(PolishConsent::from_setting(None), Ok(None));
        assert_eq!(PolishConsent::from_setting(Some(NO_CONSENT)), Ok(None));
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
                PolishConsent::from_setting(Some(bad)),
                Err(UnreadableConsent),
                "{bad}"
            );
        }
    }
}
