//! API keys in the OS keychain.
//!
//! Keys are only ever in the keychain: never in settings, never in a log, never in an error.
//! [`KeyStore`] is the seam, so tests can deny access and count what was asked. [`OsKeyStore`]
//! is the real one, over `keyring-core` and the per-OS stores.
//!
//! # Asking whether a key exists without reading it
//!
//! On macOS, reading a keychain item's secret can raise a system prompt ("Inkwell wants to use
//! your confidential information…") unless the app is already on the item's access list, so the
//! settings screen must be able to ask "is a key stored?" without reading one. Inkwell 0.2 did
//! that with an attributes-only query. Re-verified against keyring-core 1.0 and
//! apple-native-keyring-store 1.0.2:
//! - `Entry::get_credential` and `Entry::get_attributes` **read the secret** on the macOS store
//!   (the first calls `find_generic_password`; the second falls back to the default
//!   `CredentialApi::get_attributes`, which calls `get_secret`). Neither may be used for the
//!   question.
//! - `CredentialStoreApi::search` with a `service` and `user` spec is **attributes-only** on the
//!   macOS store: an `ItemSearchOptions` query with `load_attributes(true)` and no
//!   `load_data`, the same query 0.2 made. That is what [`OsKeyStore::has_key`] uses there.
//! - The Windows credential manager does not prompt on read, so there the question is answered
//!   with `get_attributes` (one `CredReadW`), which does load the secret into memory and drops it.
//!
//! Whether the macOS query is truly promptless can only be seen with a signed build and a real
//! keychain: it is on the manual checklist, with an `#[ignore]` test that exercises it.

use std::collections::HashMap;
use std::sync::Arc;

use keyring_core::{CredentialStore, Error as KeyringError};

/// The keychain service every key is stored under, as in Inkwell 0.2, so its keys are found. One
/// constant because the writer and the existence check must agree.
pub const KEYRING_SERVICE: &str = "inkwell";

/// An API key. Its `Debug` never shows it.
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key.
    pub fn new(key: String) -> Self {
        Self(key)
    }

    /// The key, for the one place it is needed: the request header.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// Why the key store could not answer. No variant carries a key, even a malformed one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStoreError {
    /// No key is stored for the provider.
    NotFound,
    /// The keychain refused: locked, the user denied the prompt, or no access.
    Denied,
    /// There is no key store on this platform, or it cannot do this.
    Unsupported,
    /// Anything else.
    Failed,
}

/// Where API keys live.
///
/// **Worker**, every method: reading a key can wait on a system prompt.
pub trait KeyStore: Send + Sync {
    /// Whether a key is stored for `provider`, **without reading it**, so no system prompt.
    fn has_key(&self, provider: &str) -> Result<bool, KeyStoreError>;

    /// Reads the key: the one call that may prompt. Call it only when the key is about to be
    /// sent.
    fn read_key(&self, provider: &str) -> Result<ApiKey, KeyStoreError>;

    /// Stores a key, replacing any other.
    fn save_key(&self, provider: &str, key: &str) -> Result<(), KeyStoreError>;

    /// Deletes the key. Deleting a key that is not there is not an error.
    fn delete_key(&self, provider: &str) -> Result<(), KeyStoreError>;
}

/// How [`OsKeyStore`] answers [`KeyStore::has_key`] (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExistenceCheck {
    /// A store search by service and user: attributes only on the macOS keychain.
    Search,
    /// `get_attributes` on the entry. Only for stores where that does not prompt (Windows).
    Attributes,
}

/// The OS keychain, through `keyring-core`.
pub struct OsKeyStore {
    store: Arc<CredentialStore>,
    existence: ExistenceCheck,
}

impl OsKeyStore {
    /// This OS's store: the macOS login keychain, or the Windows credential manager.
    pub fn native() -> Result<Self, KeyStoreError> {
        #[cfg(target_os = "macos")]
        {
            let store =
                apple_native_keyring_store::keychain::Store::new().map_err(|e| map_error(&e))?;
            Ok(Self::with_store(store, ExistenceCheck::Search))
        }
        #[cfg(target_os = "windows")]
        {
            let store = windows_native_keyring_store::Store::new().map_err(|e| map_error(&e))?;
            Ok(Self::with_store(store, ExistenceCheck::Attributes))
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(KeyStoreError::Unsupported)
        }
    }

    /// Any `keyring-core` store (the mock store in tests), answering existence with `existence`.
    /// Pass [`ExistenceCheck::Search`] for a store whose reads can prompt.
    pub fn with_store(store: Arc<CredentialStore>, existence: ExistenceCheck) -> Self {
        Self { store, existence }
    }

    fn entry(&self, provider: &str) -> Result<keyring_core::Entry, KeyStoreError> {
        self.store
            .build(KEYRING_SERVICE, provider, None)
            .map_err(|e| map_error(&e))
    }
}

impl KeyStore for OsKeyStore {
    fn has_key(&self, provider: &str) -> Result<bool, KeyStoreError> {
        match self.existence {
            ExistenceCheck::Search => {
                let spec = HashMap::from([("service", KEYRING_SERVICE), ("user", provider)]);
                match self.store.search(&spec) {
                    // Exact match on both: some stores search by substring.
                    Ok(found) => Ok(found.iter().any(|entry| {
                        entry.get_specifiers().is_some_and(|(service, user)| {
                            service == KEYRING_SERVICE && user == provider
                        })
                    })),
                    Err(KeyringError::NoEntry) => Ok(false),
                    Err(e) => Err(map_error(&e)),
                }
            }
            ExistenceCheck::Attributes => match self.entry(provider)?.get_attributes() {
                Ok(_) => Ok(true),
                Err(KeyringError::NoEntry) => Ok(false),
                Err(e) => Err(map_error(&e)),
            },
        }
    }

    fn read_key(&self, provider: &str) -> Result<ApiKey, KeyStoreError> {
        match self.entry(provider)?.get_password() {
            Ok(key) if key.is_empty() => Err(KeyStoreError::NotFound),
            Ok(key) => Ok(ApiKey::new(key)),
            // Fail closed: anything but a clean "not there" is a refusal, and nothing is sent.
            Err(KeyringError::NoEntry) => Err(KeyStoreError::NotFound),
            Err(e) => Err(match map_error(&e) {
                KeyStoreError::Unsupported => KeyStoreError::Unsupported,
                _ => KeyStoreError::Denied,
            }),
        }
    }

    fn save_key(&self, provider: &str, key: &str) -> Result<(), KeyStoreError> {
        if key.is_empty() {
            return self.delete_key(provider);
        }
        self.entry(provider)?
            .set_password(key)
            .map_err(|e| map_error(&e))
    }

    fn delete_key(&self, provider: &str) -> Result<(), KeyStoreError> {
        match self.entry(provider)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(map_error(&e)),
        }
    }
}

/// Maps a keyring error to a variant with no payload. `BadEncoding` and `BadDataFormat` carry the
/// secret's bytes; they are matched by kind and dropped here, never formatted.
fn map_error(error: &KeyringError) -> KeyStoreError {
    match error {
        KeyringError::NoEntry => KeyStoreError::NotFound,
        // On macOS a denied or cancelled prompt arrives as a platform failure
        // (errSecAuthFailed, errSecUserCanceled), and a locked keychain as no storage access.
        KeyringError::NoStorageAccess(_) | KeyringError::PlatformFailure(_) => {
            KeyStoreError::Denied
        }
        KeyringError::NoDefaultStore | KeyringError::NotSupportedByStore(_) => {
            KeyStoreError::Unsupported
        }
        _ => KeyStoreError::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring_core::mock;

    fn mock_store() -> OsKeyStore {
        // The mock store's search matches substrings and lists entries with no secret, so it
        // is exercised through the attributes check; the search path is covered by the
        // `#[ignore]` keychain test below.
        OsKeyStore::with_store(mock::Store::new().unwrap(), ExistenceCheck::Attributes)
    }

    fn fail_next(store: &OsKeyStore, provider: &str, error: KeyringError) {
        let entry = store.store.build(KEYRING_SERVICE, provider, None).unwrap();
        let cred: &mock::Cred = entry.as_any().downcast_ref().unwrap();
        cred.set_error(error);
    }

    #[test]
    fn keys_round_trip_and_existence_agrees_with_what_was_stored() {
        let store = mock_store();
        assert_eq!(store.has_key("openai"), Ok(false));
        assert_eq!(
            store.read_key("openai").unwrap_err(),
            KeyStoreError::NotFound
        );

        store.save_key("openai", "sk-synthetic").unwrap();
        assert_eq!(store.has_key("openai"), Ok(true));
        assert_eq!(store.read_key("openai").unwrap().expose(), "sk-synthetic");

        // An empty key clears, as in 0.2's settings screen.
        store.save_key("openai", "").unwrap();
        assert_eq!(store.has_key("openai"), Ok(false));
        assert_eq!(store.delete_key("openai"), Ok(()));
    }

    #[test]
    fn anything_but_not_found_is_a_denial_when_reading() {
        let store = mock_store();
        store.save_key("anthropic", "sk-synthetic").unwrap();
        for error in [
            KeyringError::NoStorageAccess(Box::new(std::io::Error::other("locked"))),
            KeyringError::PlatformFailure(Box::new(std::io::Error::other("user denied"))),
            KeyringError::BadEncoding(b"sk-canary\xff".to_vec()),
            KeyringError::Ambiguous(Vec::new()),
        ] {
            fail_next(&store, "anthropic", error);
            assert_eq!(
                store.read_key("anthropic").unwrap_err(),
                KeyStoreError::Denied
            );
        }
        // The failures were one-shot; the key is still there.
        assert_eq!(
            store.read_key("anthropic").unwrap().expose(),
            "sk-synthetic"
        );
    }

    #[test]
    fn a_key_never_shows_in_debug_output() {
        let key = ApiKey::new("sk-synthetic-canary".into());
        assert_eq!(format!("{key:?}"), "ApiKey(<redacted>)");
    }

    /// Writes, finds and deletes an item in the real login keychain (macOS) or credential manager
    /// (Windows), under its own account name so it never touches a real provider's key. Run it by
    /// hand: on macOS, watch that `has_key` raises no prompt.
    #[test]
    #[ignore = "touches the real OS keychain; run by hand (manual checklist)"]
    fn the_os_existence_check_agrees_with_what_was_stored() {
        let store = OsKeyStore::native().expect("an OS key store");
        let account = "inkwell-selftest-provider";
        store.delete_key(account).unwrap();
        assert_eq!(store.has_key(account), Ok(false));
        store
            .save_key(account, "test-value-not-a-real-key")
            .unwrap();
        assert_eq!(store.has_key(account), Ok(true));
        store.delete_key(account).unwrap();
        assert_eq!(store.has_key(account), Ok(false));
    }
}
