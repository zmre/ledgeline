//! [`SingleFileVault`]: one journal file opened in place, with nothing around
//! it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ledgeline_core::vault::{
    Availability, Entry, EntryKind, Revision, Vault, VaultError, VaultPath,
};

/// A vault rooted at a file's folder that contains exactly that file.
///
/// iOS grants access to a file opened in place (from Files, the share sheet,
/// or a provider that cannot grant folders) and to **nothing beside it**, so
/// the vault does not pretend otherwise: the root lists the one file, and
/// every other path does not exist. The folder itself is never touched — not
/// listed, not stat'ed — since asking would only be refused.
///
/// For a journal that means **its `include`s are unavailable**: they read as
/// missing, so a journal that includes other files does not open this way
/// (the app is told to ask for the folder instead). Everything the server
/// would create *beside* the journal — an `accounts.journal`, a rules file, the
/// benchmark price cache — is refused as read-only.
///
/// The file itself behaves as in the wrapped vault (a
/// [`CoordinatedVault`](crate::CoordinatedVault) in the app): read with
/// `std::fs`, written through the coordinator, reported not downloaded when it
/// is dataless.
///
/// "Grant access to the folder" is not an upgrade of this vault: the app opens
/// the parent as an ordinary folder vault, which replaces this one.
#[derive(Debug)]
pub struct SingleFileVault<V> {
    inner: V,
    file: VaultPath,
}

impl<V: Vault> SingleFileVault<V> {
    /// `inner` is rooted at the file's folder; `file_name` is the file.
    ///
    /// # Errors
    /// [`VaultError::InvalidPath`] when `file_name` is not one path segment.
    pub fn new(inner: V, file_name: &str) -> Result<Self, VaultError> {
        let file = VaultPath::root().child(file_name)?;
        Ok(Self { inner, file })
    }

    /// The one file, as a vault path.
    pub fn file(&self) -> &VaultPath {
        &self.file
    }

    fn only_the_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        if *path == self.file {
            Ok(())
        } else {
            Err(VaultError::not_found(path))
        }
    }

    fn writable(&self, path: &VaultPath) -> Result<(), VaultError> {
        if *path == self.file {
            Ok(())
        } else {
            Err(VaultError::ReadOnly {
                path: path.to_string(),
            })
        }
    }

    fn root_entry() -> Entry {
        Entry {
            path: VaultPath::root(),
            kind: EntryKind::Dir,
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
            availability: Availability::Local,
            identity: None,
        }
    }
}

impl<V: Vault> Vault for SingleFileVault<V> {
    fn root(&self) -> &Path {
        self.inner.root()
    }

    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        if path.is_root() {
            Ok(Some(Self::root_entry()))
        } else if *path == self.file {
            self.inner.stat(path)
        } else {
            Ok(None)
        }
    }

    fn list_dir(&self, dir: &VaultPath, _cap: Option<usize>) -> Result<Vec<Entry>, VaultError> {
        if !dir.is_root() {
            return Err(VaultError::not_found(dir));
        }
        Ok(self.inner.stat(&self.file)?.into_iter().collect())
    }

    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        // No `realpath`: resolving would walk the folder. The file is taken as
        // named, which is the name the grant was for.
        if path.is_root() || *path == self.file {
            Ok(path.clone())
        } else {
            Err(VaultError::not_found(path))
        }
    }

    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError> {
        self.only_the_file(path)?;
        self.inner.read(path)
    }

    fn read_prefix(&self, path: &VaultPath, max_len: u64) -> Result<Vec<u8>, VaultError> {
        self.only_the_file(path)?;
        self.inner.read_prefix(path, max_len)
    }

    fn read_pinned(
        &self,
        path: &VaultPath,
        cap: Option<u64>,
    ) -> Result<(Vec<u8>, Revision), VaultError> {
        self.only_the_file(path)?;
        self.inner.read_pinned(path, cap)
    }

    fn replace(
        &self,
        path: &VaultPath,
        bytes: &[u8],
        expected: &Revision,
    ) -> Result<Revision, VaultError> {
        self.writable(path)?;
        self.inner.replace(path, bytes, expected)
    }

    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.writable(path)?;
        self.inner.write_atomic(path, bytes)
    }

    /// Always refused: the one file exists, and nothing else may be made.
    fn create_new(&self, path: &VaultPath, _bytes: &[u8]) -> Result<Revision, VaultError> {
        Err(VaultError::ReadOnly {
            path: path.to_string(),
        })
    }

    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        Err(VaultError::ReadOnly {
            path: path.to_string(),
        })
    }

    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        Err(VaultError::ReadOnly {
            path: path.to_string(),
        })
    }

    fn availability(&self, path: &VaultPath) -> Availability {
        if *path == self.file {
            self.inner.availability(path)
        } else if path.is_root() {
            Availability::Local
        } else {
            Availability::Unknown
        }
    }

    fn request_download(&self, paths: &[VaultPath]) {
        if paths.contains(&self.file) {
            self.inner
                .request_download(std::slice::from_ref(&self.file));
        }
    }

    /// The file's real path, and nothing else's — the folder is outside the
    /// grant.
    fn local_path(&self, path: &VaultPath) -> Option<PathBuf> {
        (*path == self.file)
            .then(|| self.inner.local_path(path))
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::vault::MemVault;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    fn single() -> SingleFileVault<MemVault> {
        let inner = MemVault::new()
            .with_file("books.journal", "; books\n")
            .with_file("neighbour.journal", "; not granted\n");
        SingleFileVault::new(inner, "books.journal").unwrap()
    }

    #[test]
    fn only_the_one_file_exists() {
        let vault = single();
        let names: Vec<String> = vault
            .list_dir(&VaultPath::root(), None)
            .unwrap()
            .iter()
            .map(|e| e.name().to_string())
            .collect();
        assert_eq!(names, vec!["books.journal"]);
        assert!(vault.read(&vp("books.journal")).is_ok());
        assert!(
            vault
                .read(&vp("neighbour.journal"))
                .unwrap_err()
                .is_not_found()
        );
        assert_eq!(vault.stat(&vp("neighbour.journal")).unwrap(), None);
        assert!(vault.canonicalize(&vp("neighbour.journal")).is_err());
        assert!(vault.list_dir(&vp("sub"), None).is_err());
    }

    #[test]
    fn only_the_one_file_is_writable() {
        let vault = single();
        let (_, read) = vault.read_pinned(&vp("books.journal"), None).unwrap();
        vault
            .replace(&vp("books.journal"), b"; edited\n", &read)
            .unwrap();
        for refused in [
            vault.create_new(&vp("accounts.journal"), b"x"),
            vault.write_atomic(&vp("neighbour.journal"), b"x"),
            vault.create_new(&vp("books.journal"), b"x"),
        ] {
            assert!(matches!(refused, Err(VaultError::ReadOnly { .. })));
        }
        assert!(vault.create_dir_all(&vp("sub")).is_err());
        assert!(vault.remove_file(&vp("books.journal")).is_err());
    }
}
