//! The general pasteboard: save, a promised (lazy) write, and a guarded restore.
//!
//! **The promise.** The text goes on the pasteboard as an `NSPasteboardItem` whose plain-text
//! type is backed by a data provider instead of bytes. The first time any app asks for the text,
//! AppKit calls the provider, which hands the text over and sends a read signal. That signal is
//! how the insertion knows the target took the paste, instead of guessing with a fixed delay.
//! The item also carries `org.nspasteboard.TransientType` and `org.nspasteboard.ConcealedType`
//! (the nspasteboard.org convention), so clipboard managers neither record nor show it.
//!
//! **Threads (measured, not assumed).** Every call here runs on the calling worker: `NSPasteboard`
//! is not main-thread-only, and writing, clearing and reading it from a worker thread works. What
//! does need the main thread is the promise: AppKit calls the provider on the main run loop
//! whichever thread wrote the item, and while that loop is not running a reading app blocks and
//! gets nothing. The shell keeps it running. Measured on macOS 27 with a scratch program on the
//! ruler pasteboard and `pbpaste -pboard ruler` as the reading app:
//! - written on the main thread or on a worker, the provider was called on the main thread;
//! - with no main run loop, the reader blocked until it was killed;
//! - the item keeps the provider alive after the writer drops its own reference;
//! - clearing and rewriting the pasteboard while a reader waits on the promise leaves that reader
//!   with nothing, so a paste that was not read in time cannot land later, and the fallbacks
//!   cannot insert the text twice.
//!
//! Keeping it off the main thread also keeps a slow save (another app providing its own data
//! lazily) from freezing the shell's UI.
#![cfg(target_os = "macos")]

use std::sync::mpsc::Sender;

use ink_core::PlatformError;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardItemDataProvider, NSPasteboardType,
    NSPasteboardTypeString,
};
use objc2_foundation::{NSArray, NSData, NSObject, NSObjectProtocol, NSString};

use super::sequence::{Restore, should_restore};

/// Marks an item as not worth keeping in a clipboard history (nspasteboard.org).
pub(crate) const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";
/// Marks an item as sensitive: clipboard managers must not show or store it (nspasteboard.org).
pub(crate) const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

/// Every item on a pasteboard, as (type, bytes) pairs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Saved {
    items: Vec<Vec<(String, Vec<u8>)>>,
}

/// The pasteboard the insertion uses: the general one, which Cmd+V reads.
pub(crate) fn general() -> Retained<NSPasteboard> {
    NSPasteboard::generalPasteboard()
}

/// `pasteboard`'s change count.
pub(crate) fn change_count(pasteboard: &NSPasteboard) -> i64 {
    pasteboard.changeCount() as i64
}

/// Copies every item, and every type of every item. Types whose data the owner will not provide
/// are skipped: there is nothing to put back for them.
pub(crate) fn save(pasteboard: &NSPasteboard) -> Result<Saved, PlatformError> {
    let Some(items) = pasteboard.pasteboardItems() else {
        return Err(PlatformError::Failed(
            "could not read the pasteboard's items".into(),
        ));
    };
    let items = items
        .to_vec()
        .into_iter()
        .map(|item| {
            item.types()
                .to_vec()
                .into_iter()
                .filter_map(|ty| item.dataForType(&ty).map(|d| (ty.to_string(), d.to_vec())))
                .collect()
        })
        .collect();
    Ok(Saved { items })
}

/// Clears `pasteboard` and writes `text` as a promise. Each read of the promise sends on `reads`.
/// Returns the change count the write produced, and the provider: the item retains it too, but
/// the caller holds it until the insertion is over rather than lean on that.
pub(crate) fn write_promise(
    pasteboard: &NSPasteboard,
    text: &str,
    reads: Sender<()>,
) -> Result<(i64, Retained<PasteProvider>), PlatformError> {
    let refused = || PlatformError::Failed("the pasteboard refused the paste".into());
    let provider = PasteProvider::new(text.to_owned(), reads);
    let item = NSPasteboardItem::new();
    let marker = NSData::new();
    for ty in [TRANSIENT_TYPE, CONCEALED_TYPE] {
        if !item.setData_forType(&marker, &NSString::from_str(ty)) {
            return Err(refused());
        }
    }
    // SAFETY: `NSPasteboardTypeString` is an immutable `NSString` constant that AppKit exports
    // for the life of the process.
    let string_type = unsafe { NSPasteboardTypeString };
    let promised = NSArray::from_slice(&[string_type]);
    if !item.setDataProvider_forTypes(ProtocolObject::from_ref(&*provider), &promised) {
        return Err(refused());
    }
    pasteboard.clearContents();
    let objects = NSArray::from_retained_slice(&[ProtocolObject::from_retained(item)]);
    if !pasteboard.writeObjects(&objects) {
        return Err(refused());
    }
    Ok((change_count(pasteboard), provider))
}

/// Puts `saved` back, if the change count is still `ours`. The check runs immediately before the
/// write, so the window in which a copy made by the user could be overwritten is as small as the
/// pasteboard allows; it cannot be closed, since the pasteboard has no compare-and-swap.
pub(crate) fn restore_if_unchanged(
    pasteboard: &NSPasteboard,
    saved: Saved,
    ours: i64,
) -> Result<Restore, PlatformError> {
    if !should_restore(ours, change_count(pasteboard)) {
        return Ok(Restore::KeptNewerCopy);
    }
    pasteboard.clearContents();
    let items: Vec<_> = saved
        .items
        .iter()
        .filter(|types| !types.is_empty())
        .map(|types| {
            let item = NSPasteboardItem::new();
            for (ty, bytes) in types {
                item.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(ty));
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    if items.is_empty() || pasteboard.writeObjects(&NSArray::from_retained_slice(&items)) {
        Ok(Restore::Restored)
    } else {
        Err(PlatformError::Failed(
            "the pasteboard refused the saved items".into(),
        ))
    }
}

/// The promise's data and its read signal.
pub(crate) struct ProviderIvars {
    text: String,
    reads: Sender<()>,
}

define_class!(
    // SAFETY:
    // - `NSObject` has no subclassing requirements.
    // - `PasteProvider` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "InkwellPasteProvider"]
    #[ivars = ProviderIvars]
    pub(crate) struct PasteProvider;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for PasteProvider {}

    // SAFETY: the method below has the protocol's signature.
    unsafe impl NSPasteboardItemDataProvider for PasteProvider {
        /// Called by AppKit when an app reads a promised type. Only the plain-text type is
        /// promised; anything else is left unanswered.
        #[unsafe(method(pasteboard:item:provideDataForType:))]
        fn provide(
            &self,
            _pasteboard: Option<&NSPasteboard>,
            item: &NSPasteboardItem,
            ty: &NSPasteboardType,
        ) {
            // SAFETY: an immutable constant exported by AppKit, as in `write_promise`.
            let string_type = unsafe { NSPasteboardTypeString };
            if ty.isEqualToString(string_type) {
                item.setString_forType(&NSString::from_str(&self.ivars().text), ty);
                // The sequence may have stopped listening (it timed out); nothing to tell then.
                let _ = self.ivars().reads.send(());
            }
        }
    }
);

// SAFETY: the ivars are `Send + Sync` (an immutable `String` and an `mpsc::Sender`), and the only
// method reads them; `NSObject`'s own state is thread-safe for retain and release.
unsafe impl Send for PasteProvider {}
// SAFETY: as above.
unsafe impl Sync for PasteProvider {}

impl PasteProvider {
    fn new(text: String, reads: Sender<()>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ProviderIvars { text, reads });
        // SAFETY: `NSObject`'s designated initialiser, called once on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use objc2_foundation::NSString;

    use super::*;

    /// A private pasteboard with a unique name, released when dropped. The tests below exercise
    /// the real pasteboard code on it, so they run on CI and never touch the user's clipboard.
    struct Private(Retained<NSPasteboard>);

    impl Private {
        fn new() -> Self {
            Self(NSPasteboard::pasteboardWithUniqueName())
        }

        fn put(&self, items: &[&[(&str, &[u8])]]) {
            self.0.clearContents();
            let items: Vec<_> =
                items
                    .iter()
                    .map(|types| {
                        let item = NSPasteboardItem::new();
                        for (ty, bytes) in *types {
                            assert!(item.setData_forType(
                                &NSData::with_bytes(bytes),
                                &NSString::from_str(ty)
                            ));
                        }
                        ProtocolObject::from_retained(item)
                    })
                    .collect();
            assert!(self.0.writeObjects(&NSArray::from_retained_slice(&items)));
        }

        fn types(&self) -> Vec<String> {
            self.0
                .types()
                .map(|t| t.to_vec().iter().map(ToString::to_string).collect())
                .unwrap_or_default()
        }
    }

    impl Drop for Private {
        fn drop(&mut self) {
            // SAFETY: `releaseGlobally` takes no arguments and returns nothing; it frees the
            // unique pasteboard in the pasteboard server. objc2 skips it (its return type is
            // `oneway void`), hence the raw send.
            unsafe {
                let _: () = msg_send![&*self.0, releaseGlobally];
            }
        }
    }

    const PLAIN: &str = "public.utf8-plain-text";
    const CUSTOM: &str = "com.example.synthetic-flavour";

    #[test]
    fn the_markers_are_the_nspasteboard_org_types() {
        assert_eq!(TRANSIENT_TYPE, "org.nspasteboard.TransientType");
        assert_eq!(CONCEALED_TYPE, "org.nspasteboard.ConcealedType");
    }

    /// Runs on CI: a provider is plain Objective-C, no pasteboard or permission involved.
    #[test]
    fn a_provider_answers_only_the_promised_type_and_signals_the_read() {
        let (tx, rx) = std::sync::mpsc::channel();
        let provider = PasteProvider::new("synthetic text".into(), tx);
        let item = NSPasteboardItem::new();
        // SAFETY: an immutable constant exported by AppKit.
        let string_type = unsafe { NSPasteboardTypeString };
        // Through the Objective-C method, as AppKit calls it.
        let provide = |ty: &NSPasteboardType| {
            provider.pasteboard_item_provideDataForType(None, &item, ty);
        };
        provide(&NSString::from_str("public.rtf"));
        assert!(rx.try_recv().is_err(), "an unpromised type is not a read");
        provide(string_type);
        assert!(rx.try_recv().is_ok());
        assert_eq!(
            item.stringForType(string_type).map(|s| s.to_string()),
            Some("synthetic text".into())
        );
    }

    #[test]
    fn a_promise_is_marked_transient_and_concealed() {
        let board = Private::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let (count, _provider) = write_promise(&board.0, "synthetic text", tx).expect("write");
        assert_eq!(count, change_count(&board.0));
        let types = board.types();
        for ty in [TRANSIENT_TYPE, CONCEALED_TYPE, PLAIN] {
            assert!(types.iter().any(|t| t == ty), "{ty} missing from {types:?}");
        }
        // Declaring the types reads nothing: the promise is still unread.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn saved_items_come_back_byte_for_byte() {
        let board = Private::new();
        board.put(&[
            &[(PLAIN, b"first item"), (CUSTOM, &[0, 1, 2, 255])],
            &[(PLAIN, b"second item")],
        ]);
        let saved = save(&board.0).expect("save");
        let (tx, _rx) = std::sync::mpsc::channel();
        let (ours, _provider) = write_promise(&board.0, "synthetic text", tx).expect("write");
        let expected = saved.clone();
        assert_eq!(
            restore_if_unchanged(&board.0, saved, ours).expect("restore"),
            Restore::Restored
        );
        assert_eq!(save(&board.0).expect("save again"), expected);
    }

    #[test]
    fn a_copy_made_after_the_promise_is_never_overwritten() {
        let board = Private::new();
        board.put(&[&[(PLAIN, b"before")]]);
        let saved = save(&board.0).expect("save");
        let (tx, _rx) = std::sync::mpsc::channel();
        let (ours, _provider) = write_promise(&board.0, "synthetic text", tx).expect("write");
        board.put(&[&[(PLAIN, b"the user's newer copy")]]);
        assert_eq!(
            restore_if_unchanged(&board.0, saved, ours).expect("restore"),
            Restore::KeptNewerCopy
        );
        let now = save(&board.0).expect("save");
        assert_eq!(
            now,
            Saved {
                items: vec![vec![(PLAIN.into(), b"the user's newer copy".to_vec())]]
            }
        );
    }

    #[test]
    fn an_empty_pasteboard_is_restored_empty() {
        let board = Private::new();
        board.0.clearContents();
        let saved = save(&board.0).expect("save");
        assert_eq!(saved, Saved::default());
        let (tx, _rx) = std::sync::mpsc::channel();
        let (ours, _provider) = write_promise(&board.0, "synthetic text", tx).expect("write");
        assert_eq!(
            restore_if_unchanged(&board.0, saved, ours).expect("restore"),
            Restore::Restored
        );
        assert!(board.types().is_empty(), "{:?}", board.types());
    }
}
