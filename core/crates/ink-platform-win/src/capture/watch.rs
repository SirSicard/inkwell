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
//! holds for longer than a swap: after [`DeviceWatch::stop`] (or a drop) empties it, a
//! notification still in flight finds nothing to call, so the core never hears from a watch it
//! stopped. Stopping waits up to [`STOP_TIMEOUT`] for the thread to unregister, then lets it go.
#![cfg(windows)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{DeviceChange, EventSink, PlatformError};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, eCapture, eConsole, eRender,
};
use windows::core::{PCWSTR, implement};

use super::devices;
use crate::com::{ComScope, device_error};

/// How long [`DeviceWatch::start`] waits for the thread to register.
const START_TIMEOUT: Duration = Duration::from_secs(5);
/// How long stopping waits for the thread to unregister before it lets the thread go: a thread
/// stuck in the audio service must not hang the core's sound thread, nor the app's quit, too.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

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

/// A running device watch: its thread and the core's callback. Stopped when dropped.
pub(crate) struct DeviceWatch {
    stop: SyncSender<()>,
    /// Says the thread has unregistered and is ending (a dropped sender).
    done: Receiver<()>,
    thread: Option<JoinHandle<()>>,
    slot: Arc<Slot>,
}

impl DeviceWatch {
    /// **Worker.** Starts the watch thread and waits until its client is registered.
    pub(crate) fn start(on_change: EventSink<DeviceChange>) -> Result<Self, PlatformError> {
        let slot: Arc<Slot> = Arc::new(Mutex::new(Some(on_change)));
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::sync_channel(1);
        let (done_tx, done) = mpsc::sync_channel::<()>(0);
        let thread = thread::Builder::new()
            .name("ink-device-watch".into())
            .spawn({
                let slot = Arc::clone(&slot);
                move || {
                    run(&slot, &ready_tx, &stopped);
                    // Dropped once unregistered (or never registered): `done` sees it go.
                    drop(done_tx);
                }
            })
            .map_err(|e| {
                PlatformError::Failed(format!("could not start the device watch thread: {e}"))
            })?;
        let watch = Self {
            stop,
            done,
            thread: Some(thread),
            slot,
        };
        match ready.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(watch),
            // Dropped: stopped, and the thread (which has ended or will) joined or let go.
            Ok(Err(e)) => Err(e),
            Err(_) => Err(PlatformError::Failed(
                "the device watch did not start in time".into(),
            )),
        }
    }

    /// **Worker.** Unregisters the client, ends the thread and empties the slot: when it returns,
    /// the core's callback will not run again. The same as dropping it.
    pub(crate) fn stop(self) {
        drop(self);
    }
}

impl Drop for DeviceWatch {
    fn drop(&mut self) {
        // A full or closed channel means the thread is already on its way out. One that has not
        // registered yet (a start that timed out) unregisters as soon as it has.
        let _ = self.stop.try_send(());
        // Joined only once it says it has unregistered; a thread stuck in the audio service is
        // let go of (it unregisters when it can, and the empty slot below keeps it from the core).
        let unregistered = matches!(
            self.done.recv_timeout(STOP_TIMEOUT),
            Err(RecvTimeoutError::Disconnected)
        );
        if let Some(thread) = self.thread.take()
            && unregistered
        {
            let _ = thread.join();
        }
        // A notification Windows was already delivering finishes first (it holds the lock).
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// What the watch thread holds while it watches, released in this order: the client, the
/// enumerator it is registered on, then the apartment.
struct Registered {
    client: IMMNotificationClient,
    enumerator: IMMDeviceEnumerator,
    _com: ComScope,
}

/// Joins the MTA and registers a notification client telling `slot`.
fn register(slot: &Arc<Slot>) -> Result<Registered, PlatformError> {
    let com = ComScope::enter()?;
    let enumerator = devices::enumerator()?;
    let client: IMMNotificationClient = Notifier(Arc::clone(slot)).into();
    // SAFETY: a live client, unregistered on this thread (`run`) before it is released.
    unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }
        .map_err(|e| device_error("watching the audio devices", &e))?;
    Ok(Registered {
        client,
        enumerator,
        _com: com,
    })
}

/// The watch thread: register, say so, wait to be stopped, unregister.
fn run(slot: &Arc<Slot>, ready: &SyncSender<Result<(), PlatformError>>, stopped: &Receiver<()>) {
    let registered = match register(slot) {
        Ok(registered) => registered,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    // Blocks until told to stop, or until the owner is gone (a dropped sender).
    let _ = stopped.recv();
    // SAFETY: registered on this enumerator by this thread; never called from inside a callback.
    let _ = unsafe {
        registered
            .enumerator
            .UnregisterEndpointNotificationCallback(&registered.client)
    };
    drop(registered);
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
