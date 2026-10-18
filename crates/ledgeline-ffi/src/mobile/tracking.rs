//! [`TrackingVault`]: what the app's vault could not serve, remembered.
//!
//! The parser and the editor turn a storage failure into their own error
//! (a parse error naming the file, an `io::Error` carrying only its kind), so
//! by the time [`LedgelineApp::open`](super::LedgelineApp::open) or
//! [`refresh`](super::LedgelineApp::refresh) sees a failure, *which* file was
//! not downloaded is gone. This decorator sits between the server and the
//! real vault and keeps that answer:
//!
//! - every [`VaultError::NotDownloaded`] is recorded, and its download is
//!   requested at once ([`Vault::request_download`] is idempotent and returns
//!   immediately), so the `503` the client gets is truthful when it says the
//!   download was asked for;
//! - every read that found nothing is recorded too, which is how a file
//!   opened on its own tells "its `include`s are outside the grant" apart from
//!   any other failure.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use ledgeline_core::vault::{Availability, Entry, Revision, Vault, VaultError, VaultPath};

/// See the [module docs](self).
#[derive(Debug)]
pub(crate) struct TrackingVault {
    inner: Arc<dyn Vault>,
    seen: Mutex<Seen>,
}

#[derive(Debug, Default)]
struct Seen {
    not_downloaded: BTreeSet<VaultPath>,
    missing: BTreeSet<VaultPath>,
}

impl TrackingVault {
    pub(crate) fn new(inner: Arc<dyn Vault>) -> Self {
        Self {
            inner,
            seen: Mutex::default(),
        }
    }

    fn seen(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The not-downloaded paths recorded since the last call, cleared.
    pub(crate) fn take_not_downloaded(&self) -> Vec<VaultPath> {
        std::mem::take(&mut self.seen().not_downloaded)
            .into_iter()
            .collect()
    }

    /// The paths a read found missing since the last call, cleared.
    pub(crate) fn take_missing(&self) -> Vec<VaultPath> {
        std::mem::take(&mut self.seen().missing)
            .into_iter()
            .collect()
    }

    /// Clear both records.
    pub(crate) fn reset(&self) {
        *self.seen() = Seen::default();
    }

    /// Record what a read's result says about `path`, and pass it through.
    fn note<T>(&self, path: &VaultPath, result: Result<T, VaultError>) -> Result<T, VaultError> {
        match &result {
            Err(VaultError::NotDownloaded { .. }) => {
                let fresh = self.seen().not_downloaded.insert(path.clone());
                if fresh {
                    self.inner.request_download(std::slice::from_ref(path));
                }
            }
            Err(error) if error.is_not_found() => {
                self.seen().missing.insert(path.clone());
            }
            _ => {}
        }
        result
    }
}

impl Vault for TrackingVault {
    fn root(&self) -> &Path {
        self.inner.root()
    }

    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        self.inner.stat(path)
    }

    fn list_dir(&self, dir: &VaultPath, cap: Option<usize>) -> Result<Vec<Entry>, VaultError> {
        self.inner.list_dir(dir, cap)
    }

    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        self.inner.canonicalize(path)
    }

    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError> {
        self.note(path, self.inner.read(path))
    }

    fn read_prefix(&self, path: &VaultPath, max_len: u64) -> Result<Vec<u8>, VaultError> {
        self.note(path, self.inner.read_prefix(path, max_len))
    }

    fn read_pinned(
        &self,
        path: &VaultPath,
        cap: Option<u64>,
    ) -> Result<(Vec<u8>, Revision), VaultError> {
        self.note(path, self.inner.read_pinned(path, cap))
    }

    fn replace(
        &self,
        path: &VaultPath,
        bytes: &[u8],
        expected: &Revision,
    ) -> Result<Revision, VaultError> {
        self.note(path, self.inner.replace(path, bytes, expected))
    }

    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.note(path, self.inner.write_atomic(path, bytes))
    }

    fn create_new(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.inner.create_new(path, bytes)
    }

    fn create_new_shared(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.inner.create_new_shared(path, bytes)
    }

    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        self.inner.create_dir_all(path)
    }

    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        self.inner.remove_file(path)
    }

    fn availability(&self, path: &VaultPath) -> Availability {
        self.inner.availability(path)
    }

    fn request_download(&self, paths: &[VaultPath]) {
        self.inner.request_download(paths);
    }

    fn local_path(&self, path: &VaultPath) -> Option<PathBuf> {
        self.inner.local_path(path)
    }

    fn key(&self, path: &VaultPath) -> PathBuf {
        self.inner.key(path)
    }

    fn vault_path(&self, key: &Path) -> Result<VaultPath, VaultError> {
        self.inner.vault_path(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::vault::MemVault;

    #[test]
    fn not_downloaded_reads_are_recorded_once_and_downloaded() {
        let mem = Arc::new(
            MemVault::new()
                .with_file("a.journal", "a")
                .with_file("b.journal", "b"),
        );
        mem.set_availability("b.journal", Availability::NotDownloaded);
        let vault = TrackingVault::new(Arc::clone(&mem) as Arc<dyn Vault>);
        let p = |s: &str| VaultPath::new(s).unwrap();

        assert!(vault.read(&p("a.journal")).is_ok());
        assert!(vault.read(&p("b.journal")).is_err());
        // MemVault "downloads" on request; evicted again, the second refusal
        // is recorded but not re-requested until the record is taken.
        mem.set_availability("b.journal", Availability::NotDownloaded);
        assert!(vault.read_pinned(&p("b.journal"), None).is_err());
        assert!(vault.read(&p("gone.journal")).is_err());

        assert_eq!(mem.download_requests(), vec![p("b.journal")]);
        assert_eq!(vault.take_not_downloaded(), vec![p("b.journal")]);
        assert!(vault.take_not_downloaded().is_empty(), "taking clears");
        assert_eq!(vault.take_missing(), vec![p("gone.journal")]);
    }
}
