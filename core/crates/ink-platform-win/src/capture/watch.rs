//! The device watch: an `IMMNotificationClient` that tells the core when endpoints come, go,
//! change state or are renamed, and when a console default changes.
//!
//! **Its own thread.** The client is registered on an enumerator that must outlive it, inside a
//! COM apartment that must outlive both, so a thread of its own (`ink-device-watch`) joins the MTA,
//! registers, then blocks until it is told to stop, and unregisters on that same thread. It holds
//! no lock and does no work while it waits (architecture rule 9).
//!
//! **The callbacks** run on the audio service's notification threads. Windows documents that they
//! must not block, nor register or unregister clients. Each maps what changed to a
//! [`DeviceChange`] and hands it to the core's callback, which only enqueues; the devices are read
//! later on the core's own thread. The callback sits in a slot behind a mutex that nothing else
//! holds for longer than a swap: after [`DeviceWatch::stop`] empties it, a notification still in
//! flight finds nothing to call, so the core never hears from a watch it stopped.
#![cfg(windows)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{DeviceChange, EventSink, PlatformError};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE, EDataFlow, ERole, IMMNotificationClient, IMMNotificationClient_Impl, eCapture,
    eConsole, eRender,
};
use windows::core::{PCWSTR, implement};

use super::devices;
use crate::com::{ComScope, device_error};

/// How long [`DeviceWatch::start`] waits for the thread to register.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// The core's callback, while the watch runs; `None` once stopped.
type Slot = Mutex<Option<EventSink<DeviceChange>>>;

/// What a default-device notification means for the core: only the console role, which is what
/// capture opens and the user sets as the default; the multimedia and communications roles move
/// with it, or are not Inkwell's. Pure.
pub(crate) fn default_change(flow: EDataFlow, role: ERole) -> Option<DeviceChange> {
    if role != eConsole {
        return None;
    }
    if flow == eCapture {
        Some(DeviceChange::DefaultInput)
    } else if flow == eRender {
        Some(DeviceChange::DefaultOutput)
    } else {
        None
    }
}

/// What a property notification means for the core: a new name is a change to the list; every
/// other property (volume curves, formats, jack details) is not. Pure.
pub(crate) fn property_change(key: &PROPERTYKEY) -> Option<DeviceChange> {
    (*key == PKEY_Device_FriendlyName).then_some(DeviceChange::Devices)
}

/// The notification client: tells the slot's callback, if there is one.
#[implement(IMMNotificationClient)]
struct Notifier(Arc<Slot>);

impl Notifier {
    /// Hands `change` to the core. A panic in its callback stops here; it never unwinds into the
    /// audio service.
    fn tell(&self, change: Option<DeviceChange>) {
        let Some(change) = change else {
            return;
        };
        let slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sink) = slot.as_ref() {
            let _ = catch_unwind(AssertUnwindSafe(|| sink(change)));
        }
    }
}

impl IMMNotificationClient_Impl for Notifier_Impl {
    fn OnDeviceStateChanged(
        &self,
        _id: &PCWSTR,
        _state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        self.tell(Some(DeviceChange::Devices));
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.tell(Some(DeviceChange::Devices));
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.tell(Some(DeviceChange::Devices));
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _id: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.tell(default_change(flow, role));
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, key: &PROPERTYKEY) -> windows::core::Result<()> {
        self.tell(property_change(key));
        Ok(())
    }
}

/// A running device watch: its thread and the core's callback.
pub(crate) struct DeviceWatch {
    stop: SyncSender<()>,
    thread: JoinHandle<()>,
    slot: Arc<Slot>,
}

impl DeviceWatch {
    /// **Worker.** Starts the watch thread and waits until its client is registered.
    pub(crate) fn start(on_change: EventSink<DeviceChange>) -> Result<Self, PlatformError> {
        let slot: Arc<Slot> = Arc::new(Mutex::new(Some(on_change)));
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("ink-device-watch".into())
            .spawn({
                let slot = Arc::clone(&slot);
                move || run(&slot, &ready_tx, &stopped)
            })
            .map_err(|e| {
                PlatformError::Failed(format!("could not start the device watch thread: {e}"))
            })?;
        let watch = Self { stop, thread, slot };
        match ready.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(watch),
            Ok(Err(e)) => {
                watch.stop();
                Err(e)
            }
            Err(_) => {
                // Told to stop, it unregisters the client if it registered after all; the slot is
                // emptied now, so nothing is heard from a watch that failed to start. Not joined:
                // a thread stuck in the audio service must not hang the caller too.
                let _ = watch.stop.try_send(());
                *watch.slot.lock().unwrap_or_else(PoisonError::into_inner) = None;
                Err(PlatformError::Failed(
                    "the device watch did not start in time".into(),
                ))
            }
        }
    }

    /// **Worker.** Unregisters the client, ends the thread and empties the slot: when it returns,
    /// the core's callback will not run again.
    pub(crate) fn stop(self) {
        // A full or closed channel means the thread is already on its way out.
        let _ = self.stop.try_send(());
        let _ = self.thread.join();
        // A notification Windows was already delivering finishes first (it holds the lock).
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The watch thread: join the MTA, register, say so, wait to be stopped, unregister.
fn run(slot: &Arc<Slot>, ready: &SyncSender<Result<(), PlatformError>>, stopped: &Receiver<()>) {
    let registered = (|| {
        let com = ComScope::enter()?;
        let enumerator = devices::enumerator()?;
        let client: IMMNotificationClient = Notifier(Arc::clone(slot)).into();
        // SAFETY: a live client, unregistered below on this thread before it is released.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }
            .map_err(|e| device_error("watching the audio devices", &e))?;
        Ok((com, enumerator, client))
    })();
    let (com, enumerator, client) = match registered {
        Ok(held) => held,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    // Blocks until told to stop, or until the owner is gone (a dropped sender).
    let _ = stopped.recv();
    // SAFETY: registered above on this enumerator; never called from inside a callback.
    let _ = unsafe { enumerator.UnregisterEndpointNotificationCallback(&client) };
    drop(client);
    drop(enumerator);
    drop(com);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use windows::Win32::Media::Audio::{eAll, eCommunications, eMultimedia};

    use super::*;

    #[test]
    fn only_the_console_defaults_count() {
        assert_eq!(
            default_change(eCapture, eConsole),
            Some(DeviceChange::DefaultInput)
        );
        assert_eq!(
            default_change(eRender, eConsole),
            Some(DeviceChange::DefaultOutput)
        );
        assert_eq!(default_change(eRender, eMultimedia), None);
        assert_eq!(default_change(eCapture, eCommunications), None);
        assert_eq!(default_change(eAll, eConsole), None);
    }

    #[test]
    fn only_a_new_name_among_the_properties_counts() {
        assert_eq!(
            property_change(&PKEY_Device_FriendlyName),
            Some(DeviceChange::Devices)
        );
        assert_eq!(
            property_change(&windows::Win32::Media::Audio::PKEY_AudioEngine_DeviceFormat),
            None
        );
    }

    /// The client tells the core while the slot holds its callback, and nothing once emptied; a
    /// panicking callback is stopped at the client.
    #[test]
    fn the_client_tells_the_slot_and_nothing_once_it_is_empty() {
        let told = Arc::new(AtomicUsize::new(0));
        let t = Arc::clone(&told);
        let slot: Arc<Slot> = Arc::new(Mutex::new(Some(Arc::new(move |_| {
            t.fetch_add(1, Ordering::SeqCst);
        }))));
        let client = Notifier(Arc::clone(&slot));
        client.tell(Some(DeviceChange::Devices));
        client.tell(None);
        assert_eq!(told.load(Ordering::SeqCst), 1);
        *slot.lock().unwrap() = None;
        client.tell(Some(DeviceChange::Devices));
        assert_eq!(told.load(Ordering::SeqCst), 1, "nothing after stop");
        *slot.lock().unwrap() = Some(Arc::new(|_| panic!("the core's bug")));
        client.tell(Some(DeviceChange::Devices));
    }

    /// The real audio service registers and unregisters the client; after `stop` the slot is
    /// empty. Needs no device and no permission (enumeration works over SSH).
    #[test]
    #[ignore = "talks to the Windows audio service"]
    fn the_real_watch_starts_and_stops() {
        let told = Arc::new(AtomicUsize::new(0));
        let t = Arc::clone(&told);
        let watch = DeviceWatch::start(Arc::new(move |_| {
            t.fetch_add(1, Ordering::SeqCst);
        }))
        .expect("watching");
        let slot = Arc::clone(&watch.slot);
        watch.stop();
        assert!(slot.lock().unwrap().is_none(), "emptied");
        // And again: a second watch on the same process works.
        DeviceWatch::start(Arc::new(|_| {})).expect("again").stop();
    }
}
