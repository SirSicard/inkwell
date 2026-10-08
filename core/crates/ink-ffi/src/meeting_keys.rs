//! A meeting's global start/stop key, owned by the meetings thread independently of dictation.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ink_core::{EventSink, HotkeyBinding, HotkeyEvent, HotkeySource, Store};
use serde_json::Value;

use crate::events::event;

/// The stored meeting shortcut; `off` leaves all keys free.
pub const KEY_SETTING: &str = "meetings.key";
/// The existing Windows Record Now chord, now a global start/stop toggle.
#[cfg(windows)]
pub const DEFAULT_KEY: &str = "ctrl+shift+r";
/// The other shells retain their native shortcut behavior; do not reserve a new chord there.
#[cfg(not(windows))]
pub const DEFAULT_KEY: &str = "off";

/// A callback's identity, checked again on the meetings thread after it has been queued.
pub enum KeyEvent {
    /// One press, never the held key's repeats.
    Pressed(u64),
    /// The OS removed this binding; no retries or recording on its behalf.
    Lost(u64),
}

/// The meetings thread's binding and its state reported to the shell.
pub struct MeetingKeys {
    source: Option<Arc<dyn HotkeySource>>,
    key: String,
    active: bool,
    suspended: bool,
    generation: u64,
    error: Option<String>,
}

impl MeetingKeys {
    /// No hook is held until the core is ready and asks for its initial binding.
    pub fn new(source: Option<Arc<dyn HotkeySource>>) -> Self {
        Self {
            source,
            key: DEFAULT_KEY.into(),
            active: false,
            suspended: false,
            generation: 0,
            error: None,
        }
    }

    /// Replaces the binding after a confirmed setting change, or a suspend/resume request.
    pub fn bind(&mut self, store: &dyn Store, sink: EventSink<KeyEvent>) {
        let result = (|| {
            let key = store
                .setting(KEY_SETTING)
                .map_err(|e| format!("couldn't read the meeting shortcut: {e}"))?
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_KEY.into());
            let key = crate::hotkey::spelling(&key);
            let unchanged = self.key == key;
            self.key = key;
            if !self.suspended && self.key != "off" {
                crate::hotkey::unique_setting(store, KEY_SETTING, &self.key)?;
                // An unrelated dictation/edit setting must not reset the meeting chord mid-hold:
                // its next repeat would otherwise belong to a fresh hook and toggle again.
                if self.active && unchanged {
                    return Ok(());
                }
            }
            self.stop();
            self.error = None;
            if self.suspended || self.key == "off" {
                return Ok(());
            }
            let source = self
                .source
                .as_ref()
                .ok_or_else(|| "this platform has no global meeting shortcut".to_owned())?;
            let generation = self.generation;
            let held = AtomicBool::new(false);
            // The platform callback does no recording or store work: only one bounded amount of
            // state and an enqueue. Repeats and a cancelled hold cannot toggle a meeting twice.
            let on_key = Arc::new(move |event| match event {
                HotkeyEvent::Pressed { .. } if !held.swap(true, Ordering::AcqRel) => {
                    sink(KeyEvent::Pressed(generation))
                }
                HotkeyEvent::Released { .. } | HotkeyEvent::Cancelled => {
                    held.store(false, Ordering::Release);
                }
                HotkeyEvent::Lost => {
                    held.store(false, Ordering::Release);
                    sink(KeyEvent::Lost(generation));
                }
                HotkeyEvent::Pressed { .. } => {}
            });
            source
                .start(&HotkeyBinding(self.key.clone()), on_key)
                .map_err(|e| format!("couldn't hold the meeting shortcut: {e}"))?;
            self.active = true;
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.stop();
            self.error = Some(error);
        }
    }

    /// Suspension stops the hook before its acknowledgement is emitted.
    pub fn suspend(&mut self, suspended: bool) {
        if self.suspended == suspended {
            return;
        }
        self.suspended = suspended;
        self.stop();
    }

    /// Stops the callback and invalidates anything it previously queued.
    pub fn stop(&mut self) {
        if let Some(source) = &self.source {
            source.stop();
        }
        self.active = false;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Identity attached to queued key events, invalidated whenever the binding changes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether an event still belongs to the current, active binding.
    pub fn accepts(&self, generation: u64) -> bool {
        self.active && !self.suspended && self.generation == generation
    }

    /// Reports a removed current hook once, leaving it inactive until an explicit rebind.
    pub fn lost(&mut self, generation: u64) -> bool {
        if !self.accepts(generation) {
            return false;
        }
        self.stop();
        self.error = Some("the meeting shortcut was lost; choose it again to retry".into());
        true
    }

    /// The shell's confirmed configuration, hook availability, and capture acknowledgement.
    pub fn state(&self, reference: Option<&str>) -> Value {
        event(
            "meetings.shortcut.state",
            &[
                ("key", Some(self.key.as_str().into())),
                ("active", Some(self.active.into())),
                ("suspended", Some(self.suspended.into())),
                ("error", self.error.as_deref().map(Into::into)),
                ("ref", reference.map(Into::into)),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::PlatformError;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Keys {
        held: Mutex<Option<(String, EventSink<HotkeyEvent>)>>,
    }
    impl Keys {
        fn emit(&self, event: HotkeyEvent) {
            let sink = self.held.lock().unwrap().as_ref().unwrap().1.clone();
            sink(event);
        }
    }
    impl HotkeySource for Keys {
        fn start(
            &self,
            binding: &HotkeyBinding,
            sink: EventSink<HotkeyEvent>,
        ) -> Result<(), PlatformError> {
            *self.held.lock().unwrap() = Some((binding.0.clone(), sink));
            Ok(())
        }
        fn stop(&self) {
            *self.held.lock().unwrap() = None;
        }
    }

    #[test]
    fn meeting_shortcut_binds_without_dictation_and_suspends_before_ack() {
        let store = ink_store::SqliteStore::open_in_memory().unwrap();
        store.set_setting(KEY_SETTING, "ctrl+shift+r").unwrap();
        let source = Arc::new(Keys::default());
        let mut keys = MeetingKeys::new(Some(source.clone()));
        keys.bind(&store, Arc::new(|_| {}));
        assert!(keys.state(None)["active"].as_bool().unwrap());
        assert_eq!(
            source.held.lock().unwrap().as_ref().unwrap().0,
            "ctrl+shift+r"
        );
        let old = keys.generation();
        keys.suspend(true);
        assert!(source.held.lock().unwrap().is_none());
        assert!(!keys.accepts(old));
        assert_eq!(keys.state(Some("capture"))["ref"], "capture");
        assert_eq!(keys.state(None)["key"], "ctrl+shift+r");
        assert_eq!(keys.state(None)["suspended"], true);
        keys.suspend(false);
        keys.bind(&store, Arc::new(|_| {}));
        assert!(keys.accepts(keys.generation()));
        let resumed = keys.generation();
        keys.suspend(false);
        keys.bind(&store, Arc::new(|_| {}));
        assert_eq!(
            keys.generation(),
            resumed,
            "duplicate resume must not reset a held hook"
        );
    }

    #[test]
    fn meeting_shortcut_stored_collision_and_off_leave_no_hook() {
        let store = ink_store::SqliteStore::open_in_memory().unwrap();
        store.set_setting(KEY_SETTING, "ctrl+shift+r").unwrap();
        store
            .set_setting(crate::voice::KEY_SETTING, "Ctrl+Shift+R")
            .unwrap();
        let source = Arc::new(Keys::default());
        let mut keys = MeetingKeys::new(Some(source.clone()));
        keys.bind(&store, Arc::new(|_| {}));
        assert!(!keys.state(None)["active"].as_bool().unwrap());
        assert!(keys.state(None)["error"].as_str().is_some());
        assert!(source.held.lock().unwrap().is_none());
        store.set_setting(KEY_SETTING, "off").unwrap();
        keys.bind(&store, Arc::new(|_| {}));
        assert_eq!(keys.state(None)["key"], "off");
        assert!(keys.state(None).get("error").is_none());
        assert!(source.held.lock().unwrap().is_none());
    }

    #[test]
    fn meeting_shortcut_held_repeats_and_lost_hooks_never_repeat_the_action() {
        use std::sync::atomic::AtomicUsize;
        let store = ink_store::SqliteStore::open_in_memory().unwrap();
        store.set_setting(KEY_SETTING, "ctrl+shift+r").unwrap();
        let source = Arc::new(Keys::default());
        let mut keys = MeetingKeys::new(Some(source.clone()));
        let presses = Arc::new(AtomicUsize::new(0));
        let count = presses.clone();
        keys.bind(
            &store,
            Arc::new(move |event| {
                if matches!(event, KeyEvent::Pressed(_)) {
                    count.fetch_add(1, Ordering::Relaxed);
                }
            }),
        );
        source.emit(HotkeyEvent::Pressed { at_ns: 1 });
        source.emit(HotkeyEvent::Pressed { at_ns: 2 });
        assert_eq!(presses.load(Ordering::Relaxed), 1);
        let same_generation = keys.generation();
        // A settings screen loading or changing a different action preserves this held hook.
        keys.bind(
            &store,
            Arc::new(|_| panic!("unchanged binding must retain its callback")),
        );
        assert_eq!(keys.generation(), same_generation);
        source.emit(HotkeyEvent::Pressed { at_ns: 2 });
        assert_eq!(presses.load(Ordering::Relaxed), 1);
        source.emit(HotkeyEvent::Released { at_ns: 3 });
        source.emit(HotkeyEvent::Pressed { at_ns: 4 });
        assert_eq!(presses.load(Ordering::Relaxed), 2);
        let old = keys.generation();
        assert!(keys.lost(old));
        assert!(!keys.accepts(old));
        assert!(!keys.lost(old));
        assert!(source.held.lock().unwrap().is_none());
    }
}
