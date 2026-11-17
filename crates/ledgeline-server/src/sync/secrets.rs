//! Where a bank connection's credential lives (plan 43 §3.6): the OS keychain,
//! one item per connection, keyed `ledgeline.sync.<connection id>`.
//!
//! The item holds the SimpleFIN **access URL**, credentials and host included.
//! Keeping the host in the keychain rather than in `ledgeline.toml` is the
//! point: that file travels with a cloned repository, and a hostile one must
//! never be able to send a stored credential to a server of its choosing.
//!
//! Three stores sit behind [`SecretStore`]:
//!
//! - [`KeyringStore`] — the real one, through the `keyring` crate (the
//!   `keychain` feature, on by default for the desktop builds);
//! - [`MemoryStore`] — a map in memory, for tests and the e2e fake bridge;
//! - [`UnavailableStore`] — what a build without a keychain gets (the iOS
//!   engine). Connecting is refused with a `setup` problem. **There is no
//!   plaintext fallback**, on any platform.
//!
//! The secret is handled as a plain `String` only at this boundary; callers
//! parse it straight into an `AccessUrl`, whose `Debug`/`Display` redact it.
//! Nothing here logs, formats or returns the value in an error.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// The keychain service name every item is stored under.
pub(crate) const SERVICE: &str = "ledgeline.sync";

/// Why the store could not answer. Never carries the secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// There is no secure store on this machine (no keychain, no Secret
    /// Service running, or a build without one).
    #[error("{0}")]
    Unavailable(String),
    /// The store is there but refused or failed this operation.
    #[error("{0}")]
    Failed(String),
}

/// The sentence a missing keychain is reported with, in the wizard and in
/// `setup` problems.
pub(crate) const NO_SECURE_STORAGE: &str = "No secure storage available on this computer, so \
     Ledgeline can't keep a bank connection. On Linux, start a Secret Service such as GNOME \
     Keyring or KWallet, then try again.";

/// A place to keep one secret per connection.
///
/// Blocking: the macOS Keychain and the Secret Service are synchronous calls
/// that may wait on the user (an unlock prompt). Callers run them on the
/// blocking pool, never on an async worker.
pub trait SecretStore: Send + Sync {
    /// Whether the store can be used at all. Probed before a claim, so a
    /// one-time setup token is never spent on a machine that cannot keep the
    /// access URL it is exchanged for.
    ///
    /// # Errors
    /// [`SecretError::Unavailable`] with a sentence for the user.
    fn check(&self) -> Result<(), SecretError>;

    /// The secret for `connection`, or `None` when there is none on this
    /// machine.
    ///
    /// # Errors
    /// [`SecretError`] when the store could not be read.
    fn get(&self, connection: &str) -> Result<Option<String>, SecretError>;

    /// Store (or replace) the secret for `connection`.
    ///
    /// # Errors
    /// [`SecretError`] when the store refused.
    fn set(&self, connection: &str, secret: &str) -> Result<(), SecretError>;

    /// Remove the secret for `connection`. Removing one that is not there is
    /// not an error.
    ///
    /// # Errors
    /// [`SecretError`] when the store refused.
    fn delete(&self, connection: &str) -> Result<(), SecretError>;
}

/// The account name an item is stored under: the connection id, which
/// `feeds::config` restricts to `[A-Za-z0-9_-]{1,64}`.
pub(crate) fn item_name(connection: &str) -> String {
    format!("{SERVICE}.{connection}")
}

/// Secrets in memory, for tests and the fake bridge. Lost when the process
/// exits, which is exactly what a test wants.
#[derive(Default)]
pub struct MemoryStore {
    items: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an item exists for `connection` (tests check removal).
    #[must_use]
    pub fn contains(&self, connection: &str) -> bool {
        self.items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&item_name(connection))
    }
}

impl SecretStore for MemoryStore {
    fn check(&self) -> Result<(), SecretError> {
        Ok(())
    }

    fn get(&self, connection: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&item_name(connection))
            .cloned())
    }

    fn set(&self, connection: &str, secret: &str) -> Result<(), SecretError> {
        self.items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(item_name(connection), secret.to_string());
        Ok(())
    }

    fn delete(&self, connection: &str) -> Result<(), SecretError> {
        self.items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&item_name(connection));
        Ok(())
    }
}

/// A store that never works: a build with no keychain.
#[derive(Default)]
pub struct UnavailableStore;

impl SecretStore for UnavailableStore {
    fn check(&self) -> Result<(), SecretError> {
        Err(SecretError::Unavailable(NO_SECURE_STORAGE.to_string()))
    }

    fn get(&self, _connection: &str) -> Result<Option<String>, SecretError> {
        Ok(None)
    }

    fn set(&self, _connection: &str, _secret: &str) -> Result<(), SecretError> {
        self.check()
    }

    fn delete(&self, _connection: &str) -> Result<(), SecretError> {
        Ok(())
    }
}

/// The OS keychain, through `keyring`.
///
/// On macOS the item is a generic password in the login keychain, which does
/// not sync through iCloud: a second Mac opening the same ledger reads "Not
/// connected on this computer" and needs its own setup token. That is the
/// safer default, and the plan accepts it.
#[cfg(feature = "keychain")]
#[derive(Default)]
pub struct KeyringStore;

#[cfg(feature = "keychain")]
impl KeyringStore {
    fn entry(connection: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, &item_name(connection)).map_err(keyring_error)
    }
}

/// `keyring`'s error as ours. Its `Display` never includes the secret; the
/// variants that mean "no store here" become [`SecretError::Unavailable`].
#[cfg(feature = "keychain")]
fn keyring_error(error: keyring::Error) -> SecretError {
    match error {
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            SecretError::Unavailable(NO_SECURE_STORAGE.to_string())
        }
        other => SecretError::Failed(format!("The keychain refused: {other}")),
    }
}

#[cfg(feature = "keychain")]
impl SecretStore for KeyringStore {
    fn check(&self) -> Result<(), SecretError> {
        // A read of an item that is not there is the cheapest question that
        // reaches the store: `NoEntry` means it answered.
        match Self::entry("probe")?.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(error)),
        }
    }

    fn get(&self, connection: &str) -> Result<Option<String>, SecretError> {
        match Self::entry(connection)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(error)),
        }
    }

    fn set(&self, connection: &str, secret: &str) -> Result<(), SecretError> {
        Self::entry(connection)?
            .set_password(secret)
            .map_err(keyring_error)
    }

    fn delete(&self, connection: &str) -> Result<(), SecretError> {
        match Self::entry(connection)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(error)),
        }
    }
}

/// The store a fresh `AppState` uses: the keychain when this build has one.
pub(crate) fn default_store() -> std::sync::Arc<dyn SecretStore> {
    #[cfg(feature = "keychain")]
    {
        std::sync::Arc::new(KeyringStore)
    }
    #[cfg(not(feature = "keychain"))]
    {
        std::sync::Arc::new(UnavailableStore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_store_round_trips_and_deletes() {
        let store = MemoryStore::new();
        assert_eq!(store.get("c1"), Ok(None));
        store.set("c1", "https://u:p@h/x").expect("set");
        assert_eq!(store.get("c1"), Ok(Some("https://u:p@h/x".to_string())));
        assert!(store.contains("c1"));
        store.delete("c1").expect("delete");
        assert!(!store.contains("c1"));
        store.delete("c1").expect("deleting nothing is fine");
    }

    #[test]
    fn the_unavailable_store_refuses_to_keep_anything() {
        let store = UnavailableStore;
        assert!(matches!(store.check(), Err(SecretError::Unavailable(_))));
        assert!(store.set("c1", "secret").is_err());
        assert_eq!(store.get("c1"), Ok(None));
    }

    #[test]
    fn items_are_named_after_the_connection() {
        assert_eq!(item_name("4f6c-1"), "ledgeline.sync.4f6c-1");
    }

    /// The real keychain, by hand on a Mac: `cargo test -p ledgeline
    /// keychain_round_trip -- --ignored`. It writes and removes one item.
    #[cfg(feature = "keychain")]
    #[test]
    #[ignore = "touches the real OS keychain; run by hand"]
    fn keychain_round_trip() {
        let store = KeyringStore;
        store.check().expect("a keychain is available");
        let id = "ledgeline-test-roundtrip";
        store
            .set(id, "https://demo:demo@example.invalid/simplefin")
            .expect("set");
        assert_eq!(
            store.get(id).expect("get").as_deref(),
            Some("https://demo:demo@example.invalid/simplefin")
        );
        store.delete(id).expect("delete");
        assert_eq!(store.get(id).expect("get after delete"), None);
    }
}
