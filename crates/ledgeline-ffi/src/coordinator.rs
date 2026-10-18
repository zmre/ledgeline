//! The callback the app implements for every WRITE that must go through the
//! platform's file coordination rather than plain `std::fs`.
//!
//! # The contract, precisely
//!
//! Paths are absolute: the vault's canonical root plus the file's place in it.
//! Every method is called from one of Ledgeline's background threads, never
//! the main thread, and may block until the operation is done. Reads never
//! cross this boundary — they stay in Rust on `std::fs`.
//!
//! ## `coordinated_replace` is CONDITIONAL
//!
//! Ledgeline never overwrites a file it did not read: an edit made on another
//! device, synced in while the app had the journal open, must not be silently
//! clobbered. Rust checks first, but only the coordinated block can check
//! *atomically* with the write, so the implementation must:
//!
//! 1. open an `NSFileCoordinator` write (`.forReplacing`) on `path`;
//! 2. **inside** the accessor block, refuse with [`CoordinatorError::Changed`]
//!    if `path` is missing, is a symbolic link, or is not a regular file;
//! 3. read its current bytes and compute the [fingerprint](#the-fingerprint);
//!    if the length is not `expected_len` or the hash is not `expected_hash`,
//!    refuse with [`CoordinatorError::Changed`] and write nothing;
//! 4. otherwise replace the file with `contents` atomically (write to the
//!    item-replacement directory, then `replaceItemAt`), still inside the
//!    block.
//!
//! ## The fingerprint
//!
//! `expected_len` is the byte length. `expected_hash` is the **FNV-1a 64-bit**
//! hash of the bytes as exactly **16 lowercase hexadecimal digits**,
//! zero-padded (`format!("{:016x}")`; Swift `String(format: "%016llx", h)`).
//! FNV-1a 64: start with `h = 0xcbf29ce484222325`; for each byte `b`,
//! `h ^= b; h = h &* 0x100000001b3` (wrapping multiply, mod 2^64). Test
//! vectors: `""` → `cbf29ce484222325`, `"a"` → `af63dc4c8601ec8c`,
//! `"foobar"` → `85944171f73967e8`. It is
//! [`Fingerprint::content_hash`](ledgeline_core::vault::Fingerprint::content_hash),
//! which Ledgeline uses for the same check on the desktop.
//!
//! ## The rest
//!
//! - `coordinated_create` is EXCLUSIVE: refuse with
//!   [`CoordinatorError::AlreadyExists`] if anything is at `path`. The parent
//!   directory exists.
//! - `coordinated_create_directory` creates `path` and missing parents.
//! - `coordinated_remove` deletes the file at `path` (a symlink is removed,
//!   not followed).
//! - `request_download` starts downloads and returns at once
//!   (`startDownloadingUbiquitousItem(at:)` for iCloud; a coordinated read on
//!   a background queue for other providers). No answer is needed: the files
//!   simply become readable, and the next `refresh()` picks them up.

use ledgeline_core::vault::{Fingerprint, VaultError};

/// File operations the app performs on Ledgeline's behalf. See the
/// [module docs](self) for the exact contract, the conditional replace above
/// all.
#[cfg_attr(target_vendor = "apple", uniffi::export(with_foreign))]
pub trait FileCoordinator: Send + Sync {
    /// Replace the file at `path` with `contents` **only if** its current
    /// bytes are `expected_len` long and hash to `expected_hash` — checked
    /// inside the coordinated write. [`CoordinatorError::Changed`] otherwise,
    /// with nothing written.
    fn coordinated_replace(
        &self,
        path: String,
        contents: Vec<u8>,
        expected_len: u64,
        expected_hash: String,
    ) -> Result<(), CoordinatorError>;

    /// Create a NEW file at `path`; [`CoordinatorError::AlreadyExists`] if
    /// anything is there.
    fn coordinated_create(&self, path: String, contents: Vec<u8>) -> Result<(), CoordinatorError>;

    /// Create `path` and any missing parents as directories.
    fn coordinated_create_directory(&self, path: String) -> Result<(), CoordinatorError>;

    /// Delete the file at `path`.
    fn coordinated_remove(&self, path: String) -> Result<(), CoordinatorError>;

    /// Start downloading files that are listed but not on the device, and
    /// return at once.
    fn request_download(&self, paths: Vec<String>);
}

/// Why a coordinated operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Error))]
pub enum CoordinatorError {
    /// The conditional replace found a file other than the expected one (or
    /// none, or a symlink). Nothing was written.
    #[error("changed: {message}")]
    Changed { message: String },
    /// An exclusive create found something already there.
    #[error("already exists: {message}")]
    AlreadyExists { message: String },
    #[error("not found: {message}")]
    NotFound { message: String },
    #[error("permission denied: {message}")]
    PermissionDenied { message: String },
    /// The provider has the file but not its content, and could not get it.
    #[error("not downloaded: {message}")]
    NotDownloaded { message: String },
    #[error("{message}")]
    Failed { message: String },
}

/// A Swift error that is not a `CoordinatorError` (or a panic across the
/// boundary) still has to come back as one.
#[cfg(target_vendor = "apple")]
impl From<uniffi::UnexpectedUniFFICallbackError> for CoordinatorError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            message: error.reason,
        }
    }
}

impl CoordinatorError {
    /// The vault error for a failed operation on `path`, keeping the class a
    /// caller branches on — [`VaultError::Changed`] above all, which the
    /// editor turns into its external-change refusal (`409`).
    pub(crate) fn into_vault_error(self, path: impl std::fmt::Display) -> VaultError {
        let path = path.to_string();
        match self {
            Self::Changed { .. } => VaultError::Changed { path },
            Self::NotFound { .. } => VaultError::NotFound { path },
            Self::PermissionDenied { .. } => VaultError::PermissionDenied { path },
            Self::NotDownloaded { .. } => VaultError::NotDownloaded { path },
            Self::AlreadyExists { message } => VaultError::Io {
                path,
                source: std::io::Error::new(std::io::ErrorKind::AlreadyExists, message),
            },
            Self::Failed { message } => VaultError::Io {
                path,
                source: std::io::Error::other(message),
            },
        }
    }
}

/// `expected_hash` for `fingerprint`: 16 lowercase hex digits. See the module
/// docs.
#[must_use]
pub fn hash_hex(fingerprint: &Fingerprint) -> String {
    format!("{:016x}", fingerprint.content_hash())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_keep_their_class_across_the_boundary() {
        let msg = || "x".to_string();
        assert!(matches!(
            CoordinatorError::Changed { message: msg() }.into_vault_error("a.journal"),
            VaultError::Changed { .. }
        ));
        assert!(
            CoordinatorError::NotFound { message: msg() }
                .into_vault_error("a.journal")
                .is_not_found()
        );
        assert!(matches!(
            CoordinatorError::PermissionDenied { message: msg() }.into_vault_error("a"),
            VaultError::PermissionDenied { .. }
        ));
        assert!(matches!(
            CoordinatorError::NotDownloaded { message: msg() }.into_vault_error("a"),
            VaultError::NotDownloaded { .. }
        ));
        assert_eq!(
            CoordinatorError::AlreadyExists { message: msg() }
                .into_vault_error("a")
                .kind(),
            std::io::ErrorKind::AlreadyExists
        );
        let VaultError::Io { source, .. } = CoordinatorError::Failed {
            message: "disk full".into(),
        }
        .into_vault_error("a") else {
            panic!("an I/O error");
        };
        assert_eq!(source.to_string(), "disk full");
    }

    /// The documented encoding, against the published FNV-1a 64 vectors the
    /// Swift side is told to test with.
    #[test]
    fn the_hash_encoding_is_sixteen_lowercase_hex_digits() {
        assert_eq!(hash_hex(&Fingerprint::of_bytes(b"")), "cbf29ce484222325");
        assert_eq!(hash_hex(&Fingerprint::of_bytes(b"a")), "af63dc4c8601ec8c");
        assert_eq!(
            hash_hex(&Fingerprint::of_bytes(b"foobar")),
            "85944171f73967e8"
        );
        // Zero-padded even when the hash is small.
        assert_eq!(format!("{:016x}", 0xabu64), "00000000000000ab");
    }
}
