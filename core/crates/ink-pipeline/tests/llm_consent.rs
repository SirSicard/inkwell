//! The consent gates for the features that send the user's words to a language model (owner
//! decision, 2026-09-28, and the PR #86 review): polish sends a dictation, and voice edit the
//! selection and the instruction, only where the user agreed that feature may send them. No path
//! turns polish on without that consent: the switch, a mode marked Polish, the default mode, a
//! voice command. A model that changed destination since the user agreed gets nothing, and the
//! take says why. A refused polish types the text as said; a refused edit changes nothing. Each
//! feature's consent is its own.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::{Rig, speech_48k};
use ink_core::mock::MockLlm;
use ink_core::{CancelToken, Endpoint, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse};
use ink_pipeline::consent::LlmConsent;
use ink_pipeline::events::{DictationEvent, EditFailure, Warning};
use ink_pipeline::modes::{Mode, ModeStore};

fn not_allowed(rig: &Rig) -> Vec<LlmConsent> {
    rig.events()
        .into_iter()
        .filter_map(|e| match e {
            DictationEvent::Warning(Warning::PolishNotAllowed(needs)) => Some(needs),
            _ => None,
        })
        .collect()
}

fn polishing_mode() -> ModeStore {
    ModeStore {
        default_id: "default".into(),
        modes: vec![Mode {
            polish_enabled: true,
            ..Mode::builtin_default()
        }],
    }
}

fn cloud(endpoint: &str) -> Endpoint {
    Endpoint::Remote(endpoint.into())
}

/// The switch on and a mode marked Polish (the default mode, as with no modes stored): without
/// consent nothing is sent, the text goes in as said, and the take names the consent it needs.
#[test]
fn without_consent_the_switch_and_a_polish_mode_send_nothing() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Polished."));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| {
            s.modes = polishing_mode();
            s.polish_wish = true;
            s.polish_consent = None;
        })
        .build();
    rig.dictate_fixture("as said", 1.5, -25.0);
    assert_eq!(rig.inserted(), ["As said. "]);
    assert_eq!(llm.calls(), 0, "nothing was sent");
    assert_eq!(not_allowed(&rig), [LlmConsent::OnDevice]);
}

/// Consent for this machine, and a model on it: polished.
#[test]
fn with_on_device_consent_an_on_device_model_polishes() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Polished."));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| {
            s.modes = polishing_mode();
            s.polish_consent = Some(LlmConsent::OnDevice);
        })
        .build();
    rig.dictate_fixture("as said", 1.5, -25.0);
    assert_eq!(rig.inserted(), ["Polished. "]);
    assert_eq!(llm.calls(), 1);
    assert!(not_allowed(&rig).is_empty());
}

/// A model that moves from this machine to a cloud provider (or between providers) while polish
/// is on.
struct Moving {
    to: std::sync::Mutex<Endpoint>,
    calls: AtomicUsize,
}

impl Moving {
    fn new(at: Endpoint) -> Self {
        Self {
            to: std::sync::Mutex::new(at),
            calls: AtomicUsize::new(0),
        }
    }

    fn move_to(&self, to: Endpoint) {
        *self.to.lock().unwrap() = to;
    }
}

impl Llm for Moving {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "shell".into(),
            model: "Some Cloud".into(),
            endpoint: self.to.lock().unwrap().clone(),
        }
    }

    fn complete(&self, _: &LlmRequest, _: &CancelToken) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            text: "Polished.".into(),
        })
    }
}

/// On-device consent, then the model becomes a cloud one: the next take sends nothing and says
/// which provider it would need consent for. Never silently.
#[test]
fn a_model_moved_to_the_cloud_gets_nothing_until_the_user_agrees_again() {
    let llm = Arc::new(Moving::new(Endpoint::InProcess));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| {
            s.modes = polishing_mode();
            s.polish_consent = Some(LlmConsent::OnDevice);
        })
        .build();
    rig.answer_anything("as said");
    rig.dictate(&speech_48k(1.0, -25.0, 31));
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1, "polished on this Mac");

    llm.move_to(cloud("shell engine cloud-a"));
    rig.clear_events();
    rig.dictate(&speech_48k(1.0, -25.0, 32));
    assert_eq!(
        llm.calls.load(Ordering::SeqCst),
        1,
        "nothing sent to the cloud"
    );
    assert_eq!(rig.inserted().last().unwrap(), "As said. ");
    assert_eq!(
        not_allowed(&rig),
        [LlmConsent::Cloud {
            endpoint: "shell engine cloud-a".into(),
            name: "Some Cloud".into()
        }]
    );
}

/// Consent for one cloud provider is not consent for another.
#[test]
fn consent_for_one_provider_does_not_cover_another() {
    let llm = Arc::new(Moving::new(cloud("shell engine cloud-b")));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| {
            s.modes = polishing_mode();
            s.polish_consent = Some(LlmConsent::Cloud {
                endpoint: "shell engine cloud-a".into(),
                name: "Cloud A".into(),
            });
        })
        .build();
    rig.dictate_fixture("as said", 1.5, -25.0);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0);
    assert_eq!(rig.inserted(), ["As said. "]);
    assert_eq!(not_allowed(&rig).len(), 1);
}

/// A model that says it is on this machine when asked, but whose call reaches a cloud provider
/// (a pick that changed between the two): the consent is checked on the model the call reaches,
/// so nothing is sent.
struct PicksAgain {
    sent: AtomicBool,
}

impl Llm for PicksAgain {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "shell".into(),
            model: "on-device".into(),
            endpoint: Endpoint::InProcess,
        }
    }

    fn complete(&self, _: &LlmRequest, _: &CancelToken) -> Result<LlmResponse, LlmError> {
        self.sent.store(true, Ordering::SeqCst);
        Ok(LlmResponse {
            text: "Polished.".into(),
        })
    }

    fn complete_if(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
        allow: &dyn Fn(&LlmInfo) -> bool,
    ) -> Result<LlmResponse, LlmError> {
        // The model picked for this call is a cloud one.
        let picked = LlmInfo {
            provider: "shell".into(),
            model: "Cloud".into(),
            endpoint: cloud("shell engine cloud-a"),
        };
        if !allow(&picked) {
            return Err(LlmError::NotAllowed { refused: picked });
        }
        self.complete(request, cancel)
    }
}

#[test]
fn the_consent_is_checked_on_the_model_the_call_reaches() {
    let llm = Arc::new(PicksAgain {
        sent: AtomicBool::new(false),
    });
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| {
            s.modes = polishing_mode();
            s.polish_consent = Some(LlmConsent::OnDevice);
        })
        .build();
    rig.dictate_fixture("as said", 1.5, -25.0);
    assert!(
        !llm.sent.load(Ordering::SeqCst),
        "nothing reached the cloud"
    );
    assert_eq!(rig.inserted(), ["As said. "]);
    assert_eq!(not_allowed(&rig).len(), 1);
}

/// "Polish on" by voice command, with the switch off and no mode marked Polish: without consent it
/// sends nothing; with it, it polishes (the command still works for a user who agreed).
#[test]
fn a_voice_command_turns_polish_on_only_where_the_user_agreed() {
    for (consent, polished) in [(None, false), (Some(LlmConsent::OnDevice), true)] {
        let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Polished."));
        let rig = Rig::builder()
            .llm(llm.clone())
            .settings(|s| {
                s.commands.enabled = true;
                s.polish_wish = false;
                s.polish_consent = consent.clone();
            })
            .build();
        let command = speech_48k(1.0, -30.0, 41);
        rig.teach(&command, "inkwell polish on");
        rig.dictate(&command);
        assert!(rig.inserted().is_empty(), "a command is not typed");
        rig.answer_anything("as said");
        rig.dictate(&speech_48k(1.0, -25.0, 42));
        if polished {
            assert_eq!(rig.inserted(), ["Polished. "]);
            assert_eq!(llm.calls(), 1);
        } else {
            assert_eq!(rig.inserted(), ["As said. "]);
            assert_eq!(llm.calls(), 0, "the command sent nothing without consent");
            assert_eq!(not_allowed(&rig), [LlmConsent::OnDevice]);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Voice edit
// ---------------------------------------------------------------------------------------------

fn edit_refusals(rig: &Rig) -> Vec<LlmConsent> {
    rig.events()
        .into_iter()
        .filter_map(|e| match e {
            DictationEvent::EditFailed(EditFailure::NotAllowed(needs)) => Some(needs),
            _ => None,
        })
        .collect()
}

/// Select, hold the edit key, say what to change.
fn edit(rig: &Rig, seed: u64) {
    rig.answer_anything("make it formal");
    rig.platform.set_selection(Some("hey all"));
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(1.2, -25.0, seed));
    rig.edit_release();
    rig.silence(0.6);
}

/// No edit consent: the selection and the instruction are never sent, the selection is left
/// alone, and the Drop hears why.
#[test]
fn without_consent_a_voice_edit_sends_nothing_and_changes_nothing() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Dear team,"));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| s.edit_consent = None)
        .build();
    edit(&rig, 61);
    assert_eq!(llm.calls(), 0, "nothing was sent");
    assert!(rig.inserted().is_empty(), "the selection is left alone");
    assert_eq!(edit_refusals(&rig), [LlmConsent::OnDevice]);
}

/// Each feature's consent is its own: polish's consent does not let voice edit send, and edit's
/// does not let polish send.
#[test]
fn polish_consent_is_not_edit_consent_nor_the_reverse() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Rewritten."));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| {
            s.modes = polishing_mode();
            s.polish_consent = Some(LlmConsent::OnDevice);
            s.edit_consent = None;
        })
        .build();
    edit(&rig, 62);
    assert_eq!(llm.calls(), 0);
    assert_eq!(edit_refusals(&rig).len(), 1);

    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Polished."));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| {
            s.modes = polishing_mode();
            s.polish_consent = None;
            s.edit_consent = Some(LlmConsent::OnDevice);
        })
        .build();
    rig.dictate_fixture("as said", 1.5, -25.0);
    assert_eq!(llm.calls(), 0);
    assert_eq!(rig.inserted(), ["As said. "]);
    assert_eq!(not_allowed(&rig).len(), 1);
}

/// Edit consent for this machine, then the model becomes a cloud one: the next edit sends
/// nothing, changes nothing, and names the provider.
#[test]
fn an_edit_model_moved_to_the_cloud_gets_nothing_until_the_user_agrees_again() {
    let llm = Arc::new(Moving::new(Endpoint::InProcess));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| s.edit_consent = Some(LlmConsent::OnDevice))
        .build();
    edit(&rig, 63);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1, "edited on this Mac");
    assert_eq!(rig.inserted(), ["Polished."]);

    llm.move_to(cloud("shell engine cloud-a"));
    rig.clear_events();
    edit(&rig, 64);
    assert_eq!(
        llm.calls.load(Ordering::SeqCst),
        1,
        "nothing sent to the cloud"
    );
    assert_eq!(rig.inserted(), ["Polished."], "the selection is left alone");
    assert_eq!(
        edit_refusals(&rig),
        [LlmConsent::Cloud {
            endpoint: "shell engine cloud-a".into(),
            name: "Some Cloud".into()
        }]
    );
}

/// The edit's consent is checked on the model the call reaches, too.
#[test]
fn the_edit_consent_is_checked_on_the_model_the_call_reaches() {
    let llm = Arc::new(PicksAgain {
        sent: AtomicBool::new(false),
    });
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| s.edit_consent = Some(LlmConsent::OnDevice))
        .build();
    edit(&rig, 65);
    assert!(
        !llm.sent.load(Ordering::SeqCst),
        "nothing reached the cloud"
    );
    assert!(rig.inserted().is_empty());
    assert_eq!(edit_refusals(&rig).len(), 1);
}
