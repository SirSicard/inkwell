//! A mode's own language model (owner decision, 2026-10-05): a mode may name the model its
//! dictations are polished on, found at each take, and polish goes out only where the user's
//! polish consent covers the model it reaches, as for the AI setting's model. A mode whose model
//! the core does not hold now is not polished at all: never on another model in its place, which
//! could send the words somewhere the user did not pick for this mode.
//!
//! Also: a voice command's pin to a mode the user then deleted is dropped with the new settings.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use common::{Rig, speech_48k};
use ink_core::mock::MockLlm;
use ink_core::{Endpoint, Llm};
use ink_pipeline::chain::ModeModels;
use ink_pipeline::consent::LlmConsent;
use ink_pipeline::events::{DictationEvent, Warning};
use ink_pipeline::modes::{Mode, ModeStore};

/// The default mode, polishing, on `model` (`None`: the AI setting's).
fn polishing_on(model: Option<&str>) -> ModeStore {
    ModeStore {
        default_id: "default".into(),
        modes: vec![Mode {
            polish_enabled: true,
            polish_model: model.map(str::to_owned),
            ..Mode::builtin_default()
        }],
    }
}

/// A lookup holding `models` by id, as the core's registry of language models would.
fn lookup(models: &[(&str, Arc<MockLlm>)]) -> ModeModels {
    let by_id: HashMap<String, Arc<dyn Llm>> = models
        .iter()
        .map(|(id, m)| ((*id).to_owned(), m.clone() as Arc<dyn Llm>))
        .collect();
    Arc::new(move |id: &str| by_id.get(id).cloned())
}

fn warnings(rig: &Rig) -> Vec<Warning> {
    rig.events()
        .into_iter()
        .filter_map(|e| match e {
            DictationEvent::Warning(w) => Some(w),
            _ => None,
        })
        .collect()
}

fn cloud(endpoint: &str) -> Endpoint {
    Endpoint::Remote(endpoint.into())
}

/// The mode's model polishes, not the AI setting's.
#[test]
fn a_mode_s_own_model_polishes_its_dictations() {
    let setting = Arc::new(MockLlm::new(Endpoint::InProcess, "As said, setting."));
    let own = Arc::new(MockLlm::new(Endpoint::InProcess, "As said, mode."));
    let rig = Rig::builder()
        .llm(setting.clone())
        .settings(|s| {
            s.modes = polishing_on(Some("engine:own"));
            s.polish_consent = Some(LlmConsent::OnDevice);
        })
        .build();
    rig.chain
        .borrow_mut()
        .set_mode_models(Some(lookup(&[("engine:own", own.clone())])));
    rig.dictate_fixture("as said", 1.5, -25.0);
    assert_eq!(rig.inserted(), ["As said, mode. "]);
    assert_eq!((own.calls(), setting.calls()), (1, 0));
}

/// A mode without a model of its own polishes on the AI setting's, as before.
#[test]
fn a_mode_without_a_model_uses_the_ai_setting_s() {
    let setting = Arc::new(MockLlm::new(Endpoint::InProcess, "As said, setting."));
    let own = Arc::new(MockLlm::new(Endpoint::InProcess, "As said, mode."));
    let rig = Rig::builder()
        .llm(setting.clone())
        .settings(|s| {
            s.modes = polishing_on(None);
            s.polish_consent = Some(LlmConsent::OnDevice);
        })
        .build();
    rig.chain
        .borrow_mut()
        .set_mode_models(Some(lookup(&[("engine:own", own.clone())])));
    rig.dictate_fixture("as said", 1.5, -25.0);
    assert_eq!(rig.inserted(), ["As said, setting. "]);
    assert_eq!((own.calls(), setting.calls()), (0, 1));
}

/// Consent for this machine does not cover a mode's cloud model: nothing is sent anywhere, the
/// text goes in as said, and the take names the consent that model needs.
#[test]
fn a_mode_s_cloud_model_needs_consent_for_its_own_destination() {
    let setting = Arc::new(MockLlm::new(Endpoint::InProcess, "As said, setting."));
    let own = Arc::new(MockLlm::new(
        cloud("https://api.example.com/v1"),
        "As said, mode.",
    ));
    let rig = Rig::builder()
        .llm(setting.clone())
        .settings(|s| {
            s.modes = polishing_on(Some("provider:example"));
            s.polish_consent = Some(LlmConsent::OnDevice);
        })
        .build();
    rig.chain
        .borrow_mut()
        .set_mode_models(Some(lookup(&[("provider:example", own.clone())])));
    rig.dictate_fixture("as said", 1.5, -25.0);
    assert_eq!(rig.inserted(), ["As said. "]);
    assert_eq!(
        (own.calls(), setting.calls()),
        (0, 0),
        "nothing sent anywhere"
    );
    assert_eq!(
        warnings(&rig),
        [Warning::PolishNotAllowed(LlmConsent::Cloud {
            endpoint: "https://api.example.com/v1".into(),
            name: "mock (mock)".into(),
        })]
    );
}

/// And the reverse: consent for a cloud provider does not cover a mode's on-device model, nor
/// another provider's.
#[test]
fn cloud_consent_covers_a_mode_s_model_only_at_that_provider() {
    let setting = Arc::new(MockLlm::new(
        cloud("https://api.a.example/v1"),
        "As said, setting.",
    ));
    let local = Arc::new(MockLlm::new(Endpoint::InProcess, "As said, locally."));
    let same = Arc::new(MockLlm::new(
        cloud("https://api.a.example/v1"),
        "As said, same.",
    ));
    let other = Arc::new(MockLlm::new(
        cloud("https://api.b.example/v1"),
        "As said, other.",
    ));
    let consent = LlmConsent::Cloud {
        endpoint: "https://api.a.example/v1".into(),
        name: "A".into(),
    };
    let models = lookup(&[
        ("engine:local", local.clone()),
        ("provider:a", same.clone()),
        ("provider:b", other.clone()),
    ]);
    for (id, polished) in [
        ("engine:local", false),
        ("provider:b", false),
        ("provider:a", true),
    ] {
        let rig = Rig::builder()
            .llm(setting.clone())
            .settings(|s| {
                s.modes = polishing_on(Some(id));
                s.polish_consent = Some(consent.clone());
            })
            .build();
        rig.chain.borrow_mut().set_mode_models(Some(models.clone()));
        rig.dictate_fixture("as said", 1.5, -25.0);
        let want = if polished {
            "As said, same. "
        } else {
            "As said. "
        };
        assert_eq!(rig.inserted(), [want], "{id}");
        assert_eq!(
            warnings(&rig)
                .iter()
                .filter(|w| matches!(w, Warning::PolishNotAllowed(_)))
                .count(),
            usize::from(!polished),
            "{id}"
        );
    }
    assert_eq!((local.calls(), other.calls(), same.calls()), (0, 0, 1));
    assert_eq!(
        setting.calls(),
        0,
        "never the setting's model in a mode's place"
    );
}

/// A mode whose model is gone (let go of, or another provider chosen) is not polished, and never
/// on the AI setting's model instead, even where that one's consent is given.
#[test]
fn a_mode_whose_model_is_gone_is_not_polished_and_says_so() {
    let setting = Arc::new(MockLlm::new(Endpoint::InProcess, "As said, setting."));
    for models in [Some(lookup(&[])), None] {
        let rig = Rig::builder()
            .llm(setting.clone())
            .settings(|s| {
                s.modes = polishing_on(Some("engine:gone"));
                s.polish_consent = Some(LlmConsent::OnDevice);
            })
            .build();
        rig.chain.borrow_mut().set_mode_models(models);
        rig.dictate_fixture("as said", 1.5, -25.0);
        assert_eq!(rig.inserted(), ["As said. "]);
        assert_eq!(warnings(&rig), [Warning::PolishModelMissing]);
    }
    assert_eq!(setting.calls(), 0);
}

/// A mode with its own model and polish off sends nothing, and says nothing.
#[test]
fn a_mode_s_model_is_not_looked_up_when_the_mode_does_not_polish() {
    let own = Arc::new(MockLlm::new(Endpoint::InProcess, "As said, mode."));
    let rig = Rig::builder()
        .settings(|s| {
            s.modes = polishing_on(Some("engine:own"));
            s.modes.modes[0].polish_enabled = false;
            s.polish_consent = Some(LlmConsent::OnDevice);
        })
        .build();
    rig.chain
        .borrow_mut()
        .set_mode_models(Some(lookup(&[("engine:own", own.clone())])));
    rig.dictate_fixture("as said", 1.5, -25.0);
    assert_eq!(rig.inserted(), ["As said. "]);
    assert_eq!(own.calls(), 0);
    assert!(warnings(&rig).is_empty());
}

/// A voice command pinned a mode; the user deleted it: the new settings drop the pin, so a mode
/// added later under the same id is never picked by it. A pin to a mode still there stays.
#[test]
fn a_pin_to_a_deleted_mode_is_dropped_with_the_new_settings() {
    let chat = Mode {
        id: "chat".into(),
        name: "Chat".into(),
        style: ink_pipeline::style::Style::Casual,
        ..Mode::builtin_default()
    };
    let with_chat = ModeStore {
        default_id: "default".into(),
        modes: vec![Mode::builtin_default(), chat.clone()],
    };
    let rig = Rig::builder()
        .settings(|s| {
            s.commands.enabled = true;
            s.modes = with_chat.clone();
        })
        .build();
    let command = speech_48k(1.0, -30.0, 21);
    rig.teach(&command, "inkwell casual mode");
    rig.dictate(&command);
    assert_eq!(rig.chain.borrow().pinned_mode(), Some("chat"));

    let mut settings = ink_pipeline::chain::DictationSettings {
        modes: with_chat.clone(),
        ..Default::default()
    };
    rig.chain.borrow_mut().set_settings(settings.clone());
    assert_eq!(
        rig.chain.borrow().pinned_mode(),
        Some("chat"),
        "still there"
    );

    settings.modes = ModeStore::default();
    rig.chain.borrow_mut().set_settings(settings.clone());
    assert_eq!(rig.chain.borrow().pinned_mode(), None, "dropped");
    settings.modes = with_chat;
    rig.chain.borrow_mut().set_settings(settings);
    assert_eq!(
        rig.chain.borrow().pinned_mode(),
        None,
        "not picked up again"
    );
}
