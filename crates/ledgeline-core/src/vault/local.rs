//! [`LocalVault`]: a vault that is a directory on the local filesystem.
//!
//! The desktop's storage, and a thin layer over [`crate::pinned`], which it
//! does not change: every read opens the final component `O_NOFOLLOW` and
//! insists on a regular file, every conditional write goes through
//! [`pinned::replace`] (directory descriptor, temp file created `O_EXCL`,
//! owner and mode carried forward from the replaced file's `fstat`, `fsync`,
//! re-hash through the descriptor, `renameat`, directory `fsync`), and every
//! create through [`pinned::create_new_pinned`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{Entry, EntryKind, FileIdentity, Revision, Vault, VaultError, VaultPath};
use crate::pinned;

/// A journal directory on the local filesystem.
#[derive(Debug, Clone)]
pub struct LocalVault {
    /// Canonicalized at construction — see [`LocalVault::new`].
    root: PathBuf,
}

impl LocalVault {
    /// A vault rooted at `root`, **canonicalized** when it exists.
    ///
    /// Canonical because every path this vault produces is relativized against
    /// it: on macOS a temp dir (`/var/…`) differs from its canonical spelling
    /// (`/private/var/…`), which would make every canonicalized child look like
    /// it is outside the root. Falls back to the path as given when it cannot
    /// be canonicalized (it does not exist); every operation then fails with
    /// the real error rather than here.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            root: fs::canonicalize(&root).unwrap_or(root),
        }
    }

    /// The vault for the journal at `main`: rooted at the directory of `main`'s
    /// canonical path, with `main`'s own name as the vault path. This is the
    /// "resolve the journal path once, at open" step — a symlinked journal is
    /// opened (and later written) through its real path.
    ///
    /// A `main` that cannot be canonicalized (it does not exist yet) roots at
    /// its parent as written, so a failure surfaces as the read's own error.
    ///
    /// # Errors
    /// [`VaultError::InvalidPath`] when `main` has no file name, or its name is
    /// not a valid vault path segment.
    pub fn for_journal(main: &Path) -> Result<(Self, VaultPath), VaultError> {
        let resolved = fs::canonicalize(main).unwrap_or_else(|_| main.to_path_buf());
        let name = resolved
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or_else(|| {
                VaultError::InvalidPath(super::VaultPathError::NotUtf8(resolved.clone()))
            })?;
        let main_path = VaultPath::root().child(name)?;
        let parent = resolved.parent().unwrap_or_else(|| Path::new(""));
        Ok((Self::new(parent), main_path))
    }

    /// The native path for `path`.
    #[must_use]
    pub fn native(&self, path: &VaultPath) -> PathBuf {
        path.to_native(&self.root)
    }

    fn entry(path: VaultPath, metadata: &fs::Metadata) -> Entry {
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            EntryKind::Symlink
        } else if file_type.is_dir() {
            EntryKind::Dir
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        Entry {
            path,
            kind,
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            availability: super::Availability::Local,
            identity: FileIdentity::of(metadata),
        }
    }

    fn revision(pin: &pinned::Pin, bytes: &[u8]) -> Revision {
        Revision {
            fingerprint: pin
                .fingerprint
                .clone()
                .unwrap_or_else(|| super::Fingerprint::of_bytes(bytes)),
            identity: pin.identity,
        }
    }
}

impl Vault for LocalVault {
    fn root(&self) -> &Path {
        &self.root
    }

    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        match fs::symlink_metadata(self.native(path)) {
            Ok(metadata) => Ok(Some(Self::entry(path.clone(), &metadata))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(VaultError::from_io(path, e)),
        }
    }

    fn list_dir(&self, dir: &VaultPath, cap: Option<usize>) -> Result<Vec<Entry>, VaultError> {
        let native = self.native(dir);
        let entries = fs::read_dir(&native).map_err(|e| VaultError::from_io(dir, e))?;
        Ok(entries
            .filter_map(Result::ok)
            .take(cap.unwrap_or(usize::MAX))
            .filter_map(|entry| {
                let name = entry.file_name();
                let path = dir.child(name.to_str()?).ok()?;
                // `DirEntry::metadata` does not traverse a symlink, so a link is
                // described as itself. An entry that vanished mid-listing is
                // dropped rather than reported.
                let metadata = entry.metadata().ok()?;
                Some(Self::entry(path, &metadata))
            })
            .collect())
    }

    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        let canonical =
            fs::canonicalize(self.native(path)).map_err(|e| VaultError::from_io(path, e))?;
        VaultPath::from_absolute(&self.root, &canonical).map_err(|_| VaultError::OutsideRoot {
            path: path.to_string(),
        })
    }

    fn read_prefix(&self, path: &VaultPath, max_len: u64) -> Result<Vec<u8>, VaultError> {
        pinned::read_prefix(&self.native(path), max_len).map_err(|e| VaultError::from_io(path, e))
    }

    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError> {
        pinned::read_capped(&self.native(path), u64::MAX)
            .map(|(bytes, _)| bytes)
            .map_err(|e| VaultError::from_io(path, e))
    }

    fn read_pinned(
        &self,
        path: &VaultPath,
        cap: Option<u64>,
    ) -> Result<(Vec<u8>, Revision), VaultError> {
        let (bytes, pin) = pinned::read_pinned_capped(&self.native(path), cap.unwrap_or(u64::MAX))
            .map_err(|e| VaultError::from_io(path, e))?;
        let revision = Self::revision(&pin, &bytes);
        Ok((bytes, revision))
    }

    fn replace(
        &self,
        path: &VaultPath,
        bytes: &[u8],
        expected: &Revision,
    ) -> Result<Revision, VaultError> {
        let written = pinned::replace(&self.native(path), bytes, &expected.to_pin())
            .map_err(|e| VaultError::from_write(path, e))?;
        Ok(Self::revision(&written, bytes))
    }

    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        let written = pinned::replace(&self.native(path), bytes, &pinned::Pin::none())
            .map_err(|e| VaultError::from_write(path, e))?;
        Ok(Self::revision(&written, bytes))
    }

    fn create_new(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        let created = pinned::create_new_pinned(&self.native(path), bytes)
            .map_err(|e| VaultError::from_io(path, e))?;
        Ok(Self::revision(&created, bytes))
    }

    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        fs::create_dir_all(self.native(path)).map_err(|e| VaultError::from_io(path, e))
    }

    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        fs::remove_file(self.native(path)).map_err(|e| VaultError::from_io(path, e))
    }

    fn local_path(&self, path: &VaultPath) -> Option<PathBuf> {
        Some(self.native(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{Availability, canonicalize_existing_prefix, place_native};

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    /// A scratch directory removed on drop. Core has no `tempfile`; this is the
    /// same pattern `pinned.rs`' tests use.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "ledgeline-vault-{tag}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
            fs::create_dir_all(&dir).expect("create scratch dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(tag: &str) -> (Scratch, LocalVault) {
        let dir = Scratch::new(tag);
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("main.journal"), "include sub/a.journal\n").unwrap();
        fs::write(dir.path().join("sub/a.journal"), "; a\n").unwrap();
        let vault = LocalVault::new(dir.path());
        (dir, vault)
    }

    #[test]
    fn root_is_canonical() {
        let (dir, vault) = fixture("root");
        assert_eq!(vault.root(), fs::canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn for_journal_resolves_the_main_file_once() {
        let (dir, _vault) = fixture("for-journal");
        let (vault, main) = LocalVault::for_journal(&dir.path().join("main.journal")).unwrap();
        assert_eq!(main, vp("main.journal"));
        assert_eq!(vault.root(), fs::canonicalize(dir.path()).unwrap());
        assert_eq!(
            vault.key(&main),
            fs::canonicalize(dir.path().join("main.journal")).unwrap()
        );
    }

    #[test]
    fn stat_reports_kind_size_identity_and_absence() {
        let (_dir, vault) = fixture("stat");
        let a = vault.stat(&vp("sub/a.journal")).unwrap().unwrap();
        assert!(a.is_file());
        assert_eq!(a.size, 4);
        assert_eq!(a.availability, Availability::Local);
        assert!(a.identity.is_some());
        assert!(vault.stat(&vp("sub")).unwrap().unwrap().is_dir());
        assert!(vault.is_dir(&VaultPath::root()));
        assert_eq!(vault.stat(&vp("missing")).unwrap(), None);
        assert!(vault.is_file(&vp("main.journal")));
    }

    #[test]
    fn list_dir_lists_children_and_honours_a_cap() {
        let (_dir, vault) = fixture("list");
        let mut names: Vec<(String, EntryKind)> = vault
            .list_dir(&VaultPath::root(), None)
            .unwrap()
            .into_iter()
            .map(|e| (e.path.to_string(), e.kind))
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                ("main.journal".to_string(), EntryKind::File),
                ("sub".to_string(), EntryKind::Dir),
            ]
        );
        assert_eq!(
            vault.list_dir(&VaultPath::root(), Some(1)).unwrap().len(),
            1
        );
        assert!(
            vault
                .list_dir(&vp("missing"), None)
                .unwrap_err()
                .is_not_found()
        );
    }

    #[test]
    fn reads_whole_files_prefixes_and_revisions() {
        let (_dir, vault) = fixture("read");
        assert_eq!(vault.read(&vp("sub/a.journal")).unwrap(), b"; a\n");
        assert_eq!(vault.read_prefix(&vp("sub/a.journal"), 2).unwrap(), b"; ");
        let (bytes, revision) = vault.read_pinned(&vp("sub/a.journal"), None).unwrap();
        assert_eq!(bytes, b"; a\n");
        assert!(Revision::of_bytes(b"; a\n").matches(&revision));
        assert!(revision.identity.is_some());
        let too_big = vault
            .read_pinned(&vp("sub/a.journal"), Some(3))
            .unwrap_err();
        assert_eq!(too_big.kind(), io::ErrorKind::FileTooLarge);
        assert!(vault.read(&vp("missing")).unwrap_err().is_not_found());
    }

    #[test]
    fn replace_is_conditional_on_the_revision_read() {
        let (dir, vault) = fixture("replace");
        let path = vp("sub/a.journal");
        let (_, revision) = vault.read_pinned(&path, None).unwrap();
        let written = vault.replace(&path, b"; b\n", &revision).unwrap();
        assert_eq!(
            fs::read(dir.path().join("sub/a.journal")).unwrap(),
            b"; b\n"
        );
        // The OLD revision no longer describes the file.
        assert!(matches!(
            vault.replace(&path, b"; c\n", &revision),
            Err(VaultError::Changed { .. })
        ));
        // The one `replace` handed back does.
        vault.replace(&path, b"; c\n", &written).unwrap();
        // A change behind our back.
        fs::write(dir.path().join("sub/a.journal"), "; external\n").unwrap();
        assert!(matches!(
            vault.replace(&path, b"; d\n", &written),
            Err(VaultError::Changed { .. })
        ));
        assert_eq!(
            fs::read(dir.path().join("sub/a.journal")).unwrap(),
            b"; external\n"
        );
    }

    #[test]
    fn create_new_is_exclusive_and_write_atomic_is_not() {
        let (dir, vault) = fixture("create");
        let created = vault.create_new(&vp("sub/new.journal"), b"x").unwrap();
        assert!(created.identity.is_some());
        let again = vault.create_new(&vp("sub/new.journal"), b"y").unwrap_err();
        assert_eq!(again.kind(), io::ErrorKind::AlreadyExists);
        vault.write_atomic(&vp("sub/new.journal"), b"z").unwrap();
        assert_eq!(fs::read(dir.path().join("sub/new.journal")).unwrap(), b"z");
        vault.create_dir_all(&vp("deep/er")).unwrap();
        vault.write_atomic(&vp("deep/er/f"), b"f").unwrap();
        vault.remove_file(&vp("deep/er/f")).unwrap();
        assert_eq!(vault.stat(&vp("deep/er/f")).unwrap(), None);
    }

    #[test]
    fn local_paths_and_keys_are_the_native_paths() {
        let (_dir, vault) = fixture("keys");
        let native = vault.root().join("sub").join("a.journal");
        assert_eq!(vault.local_path(&vp("sub/a.journal")), Some(native.clone()));
        assert_eq!(vault.key(&vp("sub/a.journal")), native);
        assert_eq!(vault.vault_path(&native).unwrap(), vp("sub/a.journal"));
        assert_eq!(place_native(&vault, &native), Some(vp("sub/a.journal")));
        let outside = std::env::temp_dir().join("elsewhere.journal");
        assert!(matches!(
            vault.vault_path(&outside),
            Err(VaultError::OutsideRoot { .. })
        ));
    }

    #[cfg(unix)]
    mod symlinks {
        use super::*;
        use std::os::unix::fs::symlink;

        fn with_links(tag: &str) -> (Scratch, Scratch, LocalVault) {
            let (dir, vault) = fixture(tag);
            let outside = Scratch::new(&format!("{tag}-outside"));
            fs::write(outside.path().join("secret.journal"), "secret").unwrap();
            symlink(
                outside.path().join("secret.journal"),
                dir.path().join("leak.journal"),
            )
            .unwrap();
            symlink(outside.path(), dir.path().join("leakdir")).unwrap();
            symlink(
                dir.path().join("sub/a.journal"),
                dir.path().join("alias.journal"),
            )
            .unwrap();
            (dir, outside, vault)
        }

        #[test]
        fn stat_and_list_describe_the_link_itself() {
            let (_dir, _outside, vault) = with_links("links-stat");
            let entry = vault.stat(&vp("alias.journal")).unwrap().unwrap();
            assert_eq!(entry.kind, EntryKind::Symlink);
            assert!(!vault.is_file(&vp("alias.journal")));
            let listed = vault.list_dir(&VaultPath::root(), None).unwrap();
            let kind = |name: &str| listed.iter().find(|e| e.name() == name).map(|e| e.kind);
            assert_eq!(kind("leak.journal"), Some(EntryKind::Symlink));
            assert_eq!(kind("leakdir"), Some(EntryKind::Symlink));
        }

        #[test]
        fn reads_and_writes_refuse_a_link_at_the_final_component() {
            let (dir, outside, vault) = with_links("links-rw");
            let err = vault.read(&vp("alias.journal")).unwrap_err();
            assert!(err.is_not_regular_file(), "{err:?}");
            let err = vault.read_pinned(&vp("leak.journal"), None).unwrap_err();
            assert!(err.is_not_regular_file(), "{err:?}");
            assert!(vault.write_atomic(&vp("leak.journal"), b"x").is_err());
            let expected = Revision::of_bytes(b"secret");
            assert!(matches!(
                vault.replace(&vp("leak.journal"), b"x", &expected),
                Err(VaultError::Changed { .. })
            ));
            assert_eq!(
                fs::read(outside.path().join("secret.journal")).unwrap(),
                b"secret"
            );
            assert!(vault.create_new(&vp("alias.journal"), b"x").is_err());
            drop(dir);
        }

        #[test]
        fn canonicalize_resolves_inside_and_refuses_outside() {
            let (_dir, _outside, vault) = with_links("links-canon");
            assert_eq!(
                vault.canonicalize(&vp("alias.journal")).unwrap(),
                vp("sub/a.journal")
            );
            assert!(matches!(
                vault.canonicalize(&vp("leak.journal")),
                Err(VaultError::OutsideRoot { .. })
            ));
            assert!(matches!(
                canonicalize_existing_prefix(&vault, &vp("leakdir/new.journal")),
                Err(VaultError::OutsideRoot { .. })
            ));
            // The outside path is never named in the error.
            let err = vault.canonicalize(&vp("leak.journal")).unwrap_err();
            assert_eq!(err.to_string(), "outside the vault root: leak.journal");
        }

        #[test]
        fn a_symlinked_journal_is_opened_through_its_real_path() {
            let (dir, _vault) = fixture("links-main");
            let holder = Scratch::new("links-main-holder");
            let link = holder.path().join("books.journal");
            symlink(dir.path().join("main.journal"), &link).unwrap();
            let (vault, main) = LocalVault::for_journal(&link).unwrap();
            assert_eq!(main, vp("main.journal"));
            assert_eq!(vault.root(), fs::canonicalize(dir.path()).unwrap());
        }
    }
}
