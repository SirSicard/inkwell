//! The clipboard: saving it, writing the text rendered on demand, and putting it back.
//!
//! **Delayed rendering.** The text goes on the clipboard as a promise: `SetClipboardData` with no
//! data. When an app pastes, Windows sends our window `WM_RENDERFORMAT`, and the text is handed
//! over then. That message is how the insertion knows the target read it (the Mac's promised
//! pasteboard, on Windows). It needs a window whose thread pumps messages while the worker waits,
//! so each insertion runs an **owner thread** with a message-only window, from the write to the
//! restore.
//!
//! **Kept out of history.** Three registered formats ride along: `CanIncludeInClipboardHistory`
//! and `CanUploadToCloudClipboard` set to 0 keep the dictated text out of Win+V history and cloud
//! clipboard sync, and `ExcludeClipboardContentFromMonitorProcessing` asks clipboard monitors to
//! ignore it. **Only the first two are enforced, and only by Windows' own history and sync.** A
//! third-party clipboard manager or a remote-desktop client (RDP, Parsec) may honour the request
//! or not; one that does not will read, and may keep, the dictated text. Its read also renders the
//! text before the paste, which `sequence` does not count as the target's.
//!
//! **Saving and restoring.** Every format backed by global memory is copied. Formats that are GDI
//! handles or private handles (bitmaps, metafiles, palettes, owner-display) cannot be copied this
//! way; Windows synthesises some of them from a saved bitmap or metafile, and the rest are counted
//! as lost, which makes the insertion report the clipboard as not restored. The restore happens
//! only if nobody wrote the clipboard since the text was rendered, checked with the clipboard's
//! sequence number while it is open, so a copy the user made meanwhile is kept.
//!
//! **The owner thread never stops serving.** While it waits for the clipboard (another app may
//! hold it open, and that app may be the reader waiting for our `WM_RENDERFORMAT`), it keeps
//! answering sent messages. A restore that fails is tried again; the text's state is dropped only
//! once a restore succeeded, and a thread that ends without one tries once more on its way out.
#![cfg(windows)]

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::PlatformError;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardOwner,
    GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE,
    MSG, MsgWaitForMultipleObjects, PM_NOREMOVE, PM_QS_SENDMESSAGE, PeekMessageW,
    PostThreadMessageW, QS_SENDMESSAGE, RegisterClassW, WINDOW_EX_STYLE, WM_APP, WM_QUIT,
    WM_RENDERALLFORMATS, WM_RENDERFORMAT, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use super::sequence::{Restore, WriteFailed};

/// Another app may hold the clipboard open for a moment; opening is retried this often...
const OPEN_ATTEMPTS: u32 = 20;
/// ...this far apart (200 ms in all).
const OPEN_RETRY: Duration = Duration::from_millis(10);

/// The owner thread's command message.
const WM_INK_RESTORE: u32 = WM_APP + 1;

const CF_TEXT_ID: u32 = CF_UNICODETEXT.0 as u32;

/// Standard clipboard format ids (`WinUser.h`) this module must tell apart.
mod cf {
    pub const BITMAP: u32 = 2;
    pub const METAFILEPICT: u32 = 3;
    pub const DIB: u32 = 8;
    pub const PALETTE: u32 = 9;
    pub const ENHMETAFILE: u32 = 14;
    pub const DIBV5: u32 = 17;
    pub const OWNERDISPLAY: u32 = 0x80;
    pub const DSPBITMAP: u32 = 0x82;
    pub const DSPMETAFILEPICT: u32 = 0x83;
    pub const DSPENHMETAFILE: u32 = 0x8E;
    pub const PRIVATE: std::ops::RangeInclusive<u32> = 0x200..=0x2FF;
    pub const GDIOBJ: std::ops::RangeInclusive<u32> = 0x300..=0x3FF;
}

/// What saving does with one format present on the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Global memory: copied byte for byte.
    Copy,
    /// A handle Windows synthesises again from another saved format: skipped, not lost.
    Synthesized,
    /// A handle that cannot be copied: lost.
    Lost,
}

/// Classifies `format`, given every format on the clipboard. Pure.
pub(crate) fn classify(format: u32, present: &[u32]) -> Class {
    let has = |f: u32| present.contains(&f);
    let bitmap_saved = has(cf::DIB) || has(cf::DIBV5);
    match format {
        cf::BITMAP | cf::PALETTE if bitmap_saved => Class::Synthesized,
        // Synthesised from the enhanced metafile, which is itself lost.
        cf::METAFILEPICT if has(cf::ENHMETAFILE) => Class::Synthesized,
        cf::BITMAP
        | cf::PALETTE
        | cf::METAFILEPICT
        | cf::ENHMETAFILE
        | cf::OWNERDISPLAY
        | cf::DSPBITMAP
        | cf::DSPMETAFILEPICT
        | cf::DSPENHMETAFILE => Class::Lost,
        f if cf::PRIVATE.contains(&f) || cf::GDIOBJ.contains(&f) => Class::Lost,
        _ => Class::Copy,
    }
}

/// A copy of the clipboard.
#[derive(Debug, Default)]
pub(crate) struct Saved {
    formats: Vec<(u32, Vec<u8>)>,
    /// Formats present that could not be copied.
    lost: usize,
}

/// Opens the clipboard for `owner`, retrying while another app holds it.
fn open(owner: Option<HWND>) -> Result<(), PlatformError> {
    for attempt in 0..OPEN_ATTEMPTS {
        // SAFETY: a window of this thread, or none.
        if unsafe { OpenClipboard(owner) }.is_ok() {
            return Ok(());
        }
        if attempt + 1 < OPEN_ATTEMPTS {
            thread::sleep(OPEN_RETRY);
        }
    }
    Err(PlatformError::Failed(
        "the clipboard stayed busy (another app holds it open)".into(),
    ))
}

/// How many times the owner thread tries a restore before giving up.
const RESTORE_ATTEMPTS: u32 = 3;

/// Waits up to `wait` while serving the messages other threads send this one (a reader's
/// `WM_RENDERFORMAT`). **Owner thread.**
fn serve_sent(wait: Duration) {
    let ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: no handles; wakes early when a sent message arrives.
    let _ = unsafe { MsgWaitForMultipleObjects(None, false, ms, QS_SENDMESSAGE) };
    let mut msg = MSG::default();
    // SAFETY: a live MSG; peeking dispatches pending sent messages and removes nothing else.
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE | PM_QS_SENDMESSAGE) };
}

/// Opens the clipboard for the owner `window`, serving sent messages between tries, so a reader
/// that holds the clipboard while it waits for our render is answered rather than waited out.
/// **Owner thread.**
fn open_serving(window: HWND) -> Result<(), PlatformError> {
    for attempt in 0..OPEN_ATTEMPTS {
        // SAFETY: this thread's window.
        if unsafe { OpenClipboard(Some(window)) }.is_ok() {
            return Ok(());
        }
        if attempt + 1 < OPEN_ATTEMPTS {
            serve_sent(OPEN_RETRY);
        }
    }
    Err(PlatformError::Failed(
        "the clipboard stayed busy (another app holds it open)".into(),
    ))
}

/// `attempt` up to `times` times, until one succeeds; the last error otherwise.
fn retry<T>(
    times: u32,
    mut attempt: impl FnMut() -> Result<T, PlatformError>,
) -> Result<T, PlatformError> {
    let mut last = Err(PlatformError::Failed("not tried".into()));
    for _ in 0..times.max(1) {
        last = attempt();
        if last.is_ok() {
            break;
        }
    }
    last
}

/// Closes the clipboard when dropped.
struct OpenGuard;

impl Drop for OpenGuard {
    fn drop(&mut self) {
        // SAFETY: opened by this thread.
        let _ = unsafe { CloseClipboard() };
    }
}

/// The bytes of a global-memory handle.
fn copy_global(handle: HANDLE) -> Option<Vec<u8>> {
    let global = HGLOBAL(handle.0);
    // SAFETY: a clipboard data handle of a global-memory format, valid while the clipboard is open.
    unsafe {
        let size = GlobalSize(global);
        let data = GlobalLock(global);
        if data.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(data.cast::<u8>(), size).to_vec();
        let _ = GlobalUnlock(global);
        Some(bytes)
    }
}

/// Copies every format on the clipboard. **Worker.**
pub(crate) fn save() -> Result<Saved, PlatformError> {
    open(None)?;
    let _open = OpenGuard;
    let mut present = Vec::new();
    let mut format = 0;
    loop {
        // SAFETY: the clipboard is open on this thread.
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        present.push(format);
    }
    let mut saved = Saved::default();
    for &format in &present {
        match classify(format, &present) {
            Class::Synthesized => {}
            Class::Lost => saved.lost += 1,
            Class::Copy => {
                // SAFETY: the clipboard is open; a delayed format is rendered by its owner here.
                match unsafe { GetClipboardData(format) }
                    .ok()
                    .and_then(copy_global)
                {
                    Some(bytes) => saved.formats.push((format, bytes)),
                    None => saved.lost += 1,
                }
            }
        }
    }
    Ok(saved)
}

/// A global-memory block holding `bytes`, for `SetClipboardData`.
fn global_of(bytes: &[u8]) -> Option<HGLOBAL> {
    // SAFETY: a fresh moveable block of the right size, filled while locked.
    unsafe {
        let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).ok()?;
        let data = GlobalLock(global);
        if data.is_null() {
            let _ = GlobalFree(Some(global));
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(global);
        Some(global)
    }
}

/// Puts `bytes` on the (open) clipboard as `format`. On success the system owns the memory.
fn put(format: u32, bytes: &[u8]) -> bool {
    let Some(global) = global_of(bytes) else {
        return false;
    };
    // SAFETY: the clipboard is open and emptied by this thread's window; `global` is ours to give.
    if unsafe { SetClipboardData(format, Some(HANDLE(global.0))) }.is_ok() {
        true
    } else {
        // SAFETY: not taken by the clipboard, so still ours.
        let _ = unsafe { GlobalFree(Some(global)) };
        false
    }
}

/// Empties the open clipboard and puts `saved` on it. The number of formats lost.
fn put_back(saved: &Saved) -> Result<usize, PlatformError> {
    // SAFETY: the clipboard is open on this thread with a window.
    unsafe { EmptyClipboard() }
        .map_err(|e| PlatformError::Failed(format!("emptying the clipboard: {e}")))?;
    let failed = saved
        .formats
        .iter()
        .filter(|(format, bytes)| !put(*format, bytes))
        .count();
    Ok(saved.lost + failed)
}

/// The registered formats that keep the text out of history, cloud sync and monitors.
fn exclusion_formats() -> [(u32, [u8; 4]); 3] {
    // SAFETY: static wide strings; registering an existing name returns its id.
    let id = |name: PCWSTR| unsafe { RegisterClipboardFormatW(name) };
    [
        (
            id(w!("ExcludeClipboardContentFromMonitorProcessing")),
            [0; 4],
        ),
        (id(w!("CanIncludeInClipboardHistory")), 0u32.to_le_bytes()),
        (id(w!("CanUploadToCloudClipboard")), 0u32.to_le_bytes()),
    ]
}

/// The owner thread's state, read by its window procedure.
struct OwnerState {
    /// The text as UTF-16 with its terminating NUL.
    text: Vec<u16>,
    reads: Sender<()>,
    /// The clipboard's sequence number while it holds our text (updated after each render,
    /// which changes it).
    ours: u32,
}

thread_local! {
    static STATE: RefCell<Option<OwnerState>> = const { RefCell::new(None) };
}

/// Hands the text to the clipboard (inside `WM_RENDERFORMAT`, where the reader holds it open).
fn render(state: &mut OwnerState) {
    let bytes: Vec<u8> = state.text.iter().flat_map(|u| u.to_le_bytes()).collect();
    if put(CF_TEXT_ID, &bytes) {
        // SAFETY: no arguments.
        state.ours = unsafe { GetClipboardSequenceNumber() };
        let _ = state.reads.send(());
    }
}

/// The message-only window's procedure. **Owner thread.**
unsafe extern "system" fn owner_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_RENDERFORMAT if wparam.0 as u32 == CF_TEXT_ID => {
            STATE.with(|s| {
                if let Some(state) = s.borrow_mut().as_mut() {
                    render(state);
                }
            });
            LRESULT(0)
        }
        WM_RENDERALLFORMATS => {
            // The window is going away while it still owns the promise: render it for good.
            // SAFETY: this thread's window.
            if unsafe { OpenClipboard(Some(window)) }.is_ok() {
                let _open = OpenGuard;
                // SAFETY: the clipboard is open.
                if unsafe { GetClipboardOwner() }.is_ok_and(|owner| owner == window) {
                    STATE.with(|s| {
                        if let Some(state) = s.borrow_mut().as_mut() {
                            render(state);
                        }
                    });
                }
            }
            LRESULT(0)
        }
        // SAFETY: the default handling for everything else.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// The owner window's class name.
const CLASS_NAME: PCWSTR = w!("InkwellClipboardOwner");

/// The window class, registered once per process.
fn window_class() -> Result<PCWSTR, PlatformError> {
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    let name = CLASS_NAME;
    let ok = *REGISTERED.get_or_init(|| {
        // SAFETY: a class with a valid procedure and this module's instance.
        unsafe {
            let Ok(module) = GetModuleHandleW(None) else {
                return false;
            };
            let class = WNDCLASSW {
                lpfnWndProc: Some(owner_proc),
                hInstance: module.into(),
                lpszClassName: name,
                ..Default::default()
            };
            RegisterClassW(&class) != 0
        }
    });
    if ok {
        Ok(name)
    } else {
        Err(PlatformError::Failed(
            "could not register the clipboard window class".into(),
        ))
    }
}

/// A restore request to the owner thread.
struct RestoreRequest(SyncSender<Result<Restore, PlatformError>>);

/// The running owner thread of one insertion.
pub(crate) struct Owner {
    thread: Option<JoinHandle<()>>,
    thread_id: u32,
    requests: Sender<RestoreRequest>,
}

impl Owner {
    /// Puts the restore to the owner thread and waits for its answer.
    pub(crate) fn restore(&self) -> Result<Restore, PlatformError> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.requests
            .send(RestoreRequest(reply))
            .map_err(|_| PlatformError::Failed("the clipboard thread has ended".into()))?;
        // SAFETY: the owner thread's queue exists (made before it reported ready).
        unsafe { PostThreadMessageW(self.thread_id, WM_INK_RESTORE, WPARAM(0), LPARAM(0)) }
            .map_err(|e| PlatformError::Failed(format!("reaching the clipboard thread: {e}")))?;
        answer
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| PlatformError::Failed("the clipboard restore did not answer".into()))?
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: as in `restore`. A failed post means the thread already ended.
        let posted = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let (Ok(()), Some(thread)) = (posted, self.thread.take()) {
            let _ = thread.join();
        }
    }
}

/// Starts an owner thread, which writes `text` as a promise and keeps `saved` for the restore.
/// The receiver gets a message each time a target reads the text.
pub(crate) fn write_delayed(
    text: &str,
    saved: Saved,
) -> Result<(Owner, Receiver<()>), WriteFailed> {
    window_class().map_err(|_| WriteFailed {
        clipboard_back: true,
    })?;
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    let (reads_tx, reads) = mpsc::channel();
    let (requests_tx, requests) = mpsc::channel::<RestoreRequest>();
    let (ready_tx, ready) = mpsc::sync_channel::<Result<u32, WriteFailed>>(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let spawned = thread::Builder::new().name("ink-clipboard".into()).spawn({
        let cancelled = Arc::clone(&cancelled);
        move || owner_thread(utf16, reads_tx, saved, &requests, &ready_tx, &cancelled)
    });
    let Ok(thread) = spawned else {
        return Err(WriteFailed {
            clipboard_back: true,
        });
    };
    match ready.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(thread_id)) => Ok((
            Owner {
                thread: Some(thread),
                thread_id,
                requests: requests_tx,
            },
            reads,
        )),
        Ok(Err(failed)) => {
            let _ = thread.join();
            Err(failed)
        }
        // The thread is stuck opening the clipboard. When it gets there it sees this (or the
        // dropped receiver), puts the saved clipboard back and ends, so no promise of the text
        // outlives this answer. Whether that restore will succeed is unknown here.
        Err(_) => {
            cancelled.store(true, Ordering::Release);
            Err(WriteFailed {
                clipboard_back: false,
            })
        }
    }
}

/// The owner thread: write, report ready, pump until a restore or quit.
fn owner_thread(
    text: Vec<u16>,
    reads: Sender<()>,
    saved: Saved,
    requests: &Receiver<RestoreRequest>,
    ready: &SyncSender<Result<u32, WriteFailed>>,
    cancelled: &AtomicBool,
) {
    let mut msg = MSG::default();
    // SAFETY: a live MSG; makes this thread's queue before anyone posts to it.
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    // SAFETY: a registered class; a message-only window with no other resources.
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS_NAME,
            w!(""),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
    };
    let Ok(window) = window else {
        let _ = ready.send(Err(WriteFailed {
            clipboard_back: true,
        }));
        return;
    };
    STATE.with(|s| {
        *s.borrow_mut() = Some(OwnerState {
            text,
            reads,
            ours: 0,
        });
    });
    match write(window, &saved) {
        Ok(()) => {
            // SAFETY: no arguments.
            let thread_id = unsafe { GetCurrentThreadId() };
            if cancelled.load(Ordering::Acquire) || ready.send(Ok(thread_id)).is_err() {
                // The writer gave up waiting: nobody will paste or restore, so the promise of
                // the text must not stay on the clipboard.
                let _ = restore(window, &saved);
                STATE.with(|s| s.borrow_mut().take());
                // SAFETY: this thread's window, destroyed once.
                let _ = unsafe { DestroyWindow(window) };
                return;
            }
        }
        Err(failed) => {
            let _ = ready.send(Err(failed));
            STATE.with(|s| s.borrow_mut().take());
            // SAFETY: this thread's window, destroyed once.
            let _ = unsafe { DestroyWindow(window) };
            return;
        }
    }
    loop {
        // SAFETY: a live MSG; any window of this thread.
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 <= 0 {
            break; // WM_QUIT, or an error
        }
        if msg.hwnd.is_invalid() && msg.message == WM_INK_RESTORE {
            if let Ok(RestoreRequest(reply)) = requests.try_recv() {
                let restored = retry(RESTORE_ATTEMPTS, || restore(window, &saved));
                if restored.is_ok() {
                    // The text is done with: a late render (on destroy) must never put it back
                    // over the restored clipboard.
                    STATE.with(|s| s.borrow_mut().take());
                }
                let _ = reply.send(restored);
            }
            continue;
        }
        // SAFETY: a message this thread received.
        unsafe { DispatchMessageW(&msg) };
    }
    if STATE.with(|s| s.borrow().is_some()) {
        // No restore succeeded (or none was asked for): one more try before the promise goes.
        let _ = retry(RESTORE_ATTEMPTS, || restore(window, &saved));
    }
    // With the state gone, WM_RENDERALLFORMATS (sent on destroy while we still own a promise)
    // renders nothing: the dictated text never outlives this thread on the clipboard.
    STATE.with(|s| s.borrow_mut().take());
    // SAFETY: this thread's window, destroyed once.
    let _ = unsafe { DestroyWindow(window) };
}

/// Writes the promise and the exclusion formats. On a failure after the clipboard was emptied,
/// puts `saved` back at once.
fn write(window: HWND, saved: &Saved) -> Result<(), WriteFailed> {
    open_serving(window).map_err(|_| WriteFailed {
        clipboard_back: true,
    })?;
    let _open = OpenGuard;
    // SAFETY: open with our window, which becomes the owner.
    if unsafe { EmptyClipboard() }.is_err() {
        return Err(WriteFailed {
            clipboard_back: true,
        });
    }
    // SAFETY: open and ours; no data means rendered on demand.
    let promised = unsafe { SetClipboardData(CF_TEXT_ID, None) }.is_ok();
    let excluded = promised
        && exclusion_formats()
            .iter()
            .all(|(format, bytes)| *format != 0 && put(*format, bytes));
    if promised && excluded {
        // The clipboard closes when `_open` drops; the sequence number after that is ours.
        drop(_open);
        // SAFETY: no arguments.
        let ours = unsafe { GetClipboardSequenceNumber() };
        STATE.with(|s| {
            if let Some(state) = s.borrow_mut().as_mut() {
                state.ours = ours;
            }
        });
        return Ok(());
    }
    // Never leave a promise on the clipboard that might end up in history: put theirs back.
    let back = put_back(saved).is_ok_and(|lost| lost == 0);
    Err(WriteFailed {
        clipboard_back: back,
    })
}

/// Puts `saved` back if the clipboard still holds our text.
fn restore(window: HWND, saved: &Saved) -> Result<Restore, PlatformError> {
    open_serving(window)?;
    let _open = OpenGuard;
    let ours = STATE.with(|s| s.borrow().as_ref().map_or(0, |state| state.ours));
    // SAFETY: no arguments; read while the clipboard is open, so nobody can write in between.
    if unsafe { GetClipboardSequenceNumber() } != ours {
        return Ok(Restore::KeptNewerCopy);
    }
    Ok(match put_back(saved)? {
        0 => Restore::Restored,
        lost => Restore::RestoredPartly { lost },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_is_retried_until_one_succeeds() {
        let mut calls = 0;
        let result = retry(3, || {
            calls += 1;
            if calls < 3 {
                Err(PlatformError::Failed("busy".into()))
            } else {
                Ok(Restore::Restored)
            }
        });
        assert_eq!(result.unwrap(), Restore::Restored);
        assert_eq!(calls, 3);
        let mut calls = 0;
        let failed: Result<Restore, _> = retry(3, || {
            calls += 1;
            Err(PlatformError::Failed(format!("busy {calls}")))
        });
        assert!(matches!(failed, Err(PlatformError::Failed(m)) if m == "busy 3"));
        let mut calls = 0;
        let _ = retry(3, || {
            calls += 1;
            Ok::<_, PlatformError>(())
        });
        assert_eq!(calls, 1, "a success is not repeated");
    }

    #[test]
    fn global_memory_formats_are_copied() {
        for format in [1, 7, CF_TEXT_ID, cf::DIB, cf::DIBV5, 15, 16, 0xC000, 0xC123] {
            assert_eq!(classify(format, &[format]), Class::Copy, "{format:#x}");
        }
    }

    #[test]
    fn a_bitmap_beside_a_dib_is_synthesised_and_alone_is_lost() {
        assert_eq!(
            classify(cf::BITMAP, &[cf::BITMAP, cf::DIB]),
            Class::Synthesized
        );
        assert_eq!(
            classify(cf::PALETTE, &[cf::BITMAP, cf::DIBV5, cf::PALETTE]),
            Class::Synthesized
        );
        assert_eq!(classify(cf::BITMAP, &[cf::BITMAP]), Class::Lost);
    }

    #[test]
    fn metafiles_and_private_handles_are_lost() {
        assert_eq!(classify(cf::ENHMETAFILE, &[cf::ENHMETAFILE]), Class::Lost);
        assert_eq!(
            classify(cf::METAFILEPICT, &[cf::ENHMETAFILE, cf::METAFILEPICT]),
            Class::Synthesized
        );
        assert_eq!(classify(cf::METAFILEPICT, &[cf::METAFILEPICT]), Class::Lost);
        assert_eq!(classify(cf::OWNERDISPLAY, &[]), Class::Lost);
        assert_eq!(classify(0x200, &[]), Class::Lost);
        assert_eq!(classify(0x3FF, &[]), Class::Lost);
    }

    /// The writer gave up before the owner thread was ready: the thread must put the clipboard
    /// back and end, leaving no promise of the text. Uses the real clipboard (of the session it
    /// runs in), so by hand: `cargo test -p ink-platform-win -- --ignored abandoned`.
    #[test]
    #[ignore = "uses the real clipboard"]
    fn an_abandoned_write_restores_and_ends() {
        let original = "clipboard before an abandoned insertion";
        let mut bytes: Vec<u8> = original.encode_utf16().flat_map(u16::to_le_bytes).collect();
        bytes.extend_from_slice(&[0, 0]);
        let (owner, _) = write_delayed(
            "placeholder",
            Saved {
                formats: vec![(CF_TEXT_ID, bytes)],
                lost: 0,
            },
        )
        .expect("setup write");
        assert_eq!(owner.restore().unwrap(), Restore::Restored);
        drop(owner);

        window_class().unwrap();
        let (reads_tx, _reads) = mpsc::channel();
        let (_requests_tx, requests) = mpsc::channel();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        drop(ready); // the writer stopped waiting
        let saved = save().unwrap();
        let text: Vec<u16> = "Synthetic abandoned text.\0".encode_utf16().collect();
        let thread = thread::spawn(move || {
            owner_thread(
                text,
                reads_tx,
                saved,
                &requests,
                &ready_tx,
                &AtomicBool::new(false),
            )
        });
        thread.join().expect("the thread ended by itself");
        let back = save().unwrap();
        let (_, bytes) = back
            .formats
            .iter()
            .find(|(f, _)| *f == CF_TEXT_ID)
            .expect("text back");
        let text: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        assert_eq!(
            String::from_utf16_lossy(&text).trim_end_matches('\0'),
            original
        );
    }

    /// Writes and restores the real clipboard, so it runs by hand: it would clobber a desktop
    /// user's clipboard for a moment, and a CI runner may have no window station clipboard.
    /// On the PC: `cargo test -p ink-platform-win -- --ignored clipboard`.
    #[test]
    #[ignore = "uses the real clipboard"]
    fn a_promise_is_rendered_on_read_and_the_clipboard_comes_back() {
        // Put something known on the clipboard first.
        let original = "clipboard before the insertion";
        let (owner, _reads) = {
            let mut bytes: Vec<u8> = original.encode_utf16().flat_map(u16::to_le_bytes).collect();
            bytes.extend_from_slice(&[0, 0]);
            let saved = Saved {
                formats: vec![(CF_TEXT_ID, bytes)],
                lost: 0,
            };
            write_delayed("placeholder", saved).expect("setup write")
        };
        assert_eq!(owner.restore().unwrap(), Restore::Restored);
        drop(owner);

        let saved = save().expect("save");
        let (owner, reads) = write_delayed("Synthetic insertion text.", saved).expect("write");
        // Read it as a target would: this renders the promise.
        let read = {
            open(None).unwrap();
            let _open = OpenGuard;
            // SAFETY: the clipboard is open.
            let handle = unsafe { GetClipboardData(CF_TEXT_ID) }.expect("rendered");
            copy_global(handle).unwrap()
        };
        let text: Vec<u16> = read
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        assert_eq!(
            String::from_utf16_lossy(&text).trim_end_matches('\0'),
            "Synthetic insertion text."
        );
        reads
            .recv_timeout(Duration::from_secs(1))
            .expect("the read was seen");
        assert_eq!(owner.restore().unwrap(), Restore::Restored);
        drop(owner);
        let back = save().unwrap();
        let (_, bytes) = back
            .formats
            .iter()
            .find(|(f, _)| *f == CF_TEXT_ID)
            .expect("text back");
        let text: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        assert_eq!(
            String::from_utf16_lossy(&text).trim_end_matches('\0'),
            original
        );
    }
}
