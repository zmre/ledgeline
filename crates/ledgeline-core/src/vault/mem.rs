//! [`MemVault`]: a vault held entirely in memory.
//!
//! For tests above all — a journal built from a handful of `with_file` calls
//! instead of a temp directory — and, because it has **no**
//! [`Vault::local_path`], the proof that a consumer works without a real
//! filesystem. It also models what a cloud-backed vault adds: files that are
//! listed but not downloaded ([`MemVault::set_availability`]), download
//! requests ([`MemVault::download_requests`]), and another program changing a
//! file behind the app's back ([`MemVault::modify_externally`],
//! [`MemVault::modify_in_place`]).
//!
//! Always compiled (not test-only) so other crates' tests can build one.
//!
//! Every write replaces a file's *identity* the way a rename-into-place does on
//! disk: each new content gets a fresh inode number, so a [`Revision`] pinned to
//! the old file stops matching even when the bytes are identical.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::SystemTime;

use super::{
    Availability, Entry, EntryKind, FileIdentity, Fingerprint, Revision, Vault, VaultError,
    VaultPath,
};

/// The `st_dev` every [`MemVault`] identity reports.
const MEM_DEV: u64 = 0x6d65_6d76;

/// How many links one resolution may follow before it is refused as a loop —
/// `MAXSYMLINKS` on Linux is 40.
const MAX_LINK_HOPS: usize = 40;

#[derive(Debug, Clone)]
struct MemFile {
    bytes: Arc<[u8]>,
    modified: SystemTime,
    availability: Availability,
    ino: u64,
}

#[derive(Debug, Default)]
struct MemState {
    files: BTreeMap<VaultPath, MemFile>,
    /// Every directory, including the root and every ancestor of every file.
    dirs: BTreeSet<VaultPath>,
    /// Symlinks, by location, to their target as written (relative to the
    /// link's own directory).
    links: BTreeMap<VaultPath, String>,
    downloads: Vec<VaultPath>,
}

impl MemState {
    fn add_ancestors(&mut self, path: &VaultPath) {
        let mut parent = path.parent();
        while let Some(dir) = parent {
            parent = dir.parent();
            if !self.dirs.insert(dir) {
                break;
            }
        }
    }

    fn occupied(&self, path: &VaultPath) -> bool {
        self.files.contains_key(path) || self.dirs.contains(path) || self.links.contains_key(path)
    }
}

/// An in-memory vault. Interior mutability, so it can be shared as
/// `Arc<dyn Vault>` and still written to.
#[derive(Debug)]
pub struct MemVault {
    root: PathBuf,
    state: RwLock<MemState>,
    next_ino: AtomicU64,
}

impl Default for MemVault {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses a test-supplied path, panicking on a bad one — these are fixture
/// literals, so a bad one is a bug in the test.
fn fixture_path(path: &str) -> VaultPath {
    VaultPath::new(path).unwrap_or_else(|e| panic!("invalid MemVault path {path:?}: {e}"))
}

fn io_error(path: &VaultPath, kind: io::ErrorKind, message: &'static str) -> VaultError {
    VaultError::Io {
        path: path.to_string(),
        source: io::Error::new(kind, message),
    }
}

impl MemVault {
    /// The default namespace root. Deliberately a path no real journal would
    /// sit at, so a key accidentally opened as a file fails loudly instead of
    /// reading something.
    pub const DEFAULT_ROOT: &'static str = "/ledgeline-mem-vault";

    /// An empty vault (just the root directory).
    #[must_use]
    pub fn new() -> Self {
        Self::with_root(Self::DEFAULT_ROOT)
    }

    /// An empty vault whose [`Vault::root`] is `root`.
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        let state = MemState {
            dirs: BTreeSet::from([VaultPath::root()]),
            ..MemState::default()
        };
        Self {
            root: root.into(),
            state: RwLock::new(state),
            next_ino: AtomicU64::new(1),
        }
    }

    fn read_state(&self) -> RwLockReadGuard<'_, MemState> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write_state(&self) -> RwLockWriteGuard<'_, MemState> {
        self.state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn fresh_file(&self, bytes: &[u8], availability: Availability) -> MemFile {
        MemFile {
            bytes: Arc::from(bytes),
            modified: SystemTime::now(),
            availability,
            ino: self.next_ino.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Builder form of [`Self::insert_file`].
    #[must_use]
    pub fn with_file(self, path: &str, contents: impl AsRef<[u8]>) -> Self {
        self.insert_file(path, contents);
        self
    }

    /// Builder form of [`Self::insert_dir`].
    #[must_use]
    pub fn with_dir(self, path: &str) -> Self {
        self.insert_dir(path);
        self
    }

    /// Creates or replaces a file (and its ancestor directories), as a new
    /// file: it gets a fresh identity.
    ///
    /// # Panics
    /// When `path` is not a valid [`VaultPath`].
    pub fn insert_file(&self, path: &str, contents: impl AsRef<[u8]>) {
        let path = fixture_path(path);
        let file = self.fresh_file(contents.as_ref(), Availability::Local);
        let mut state = self.write_state();
        state.add_ancestors(&path);
        state.links.remove(&path);
        state.files.insert(path, file);
    }

    /// Another program replaced the file — an editor's save-by-rename, a sync
    /// client landing a newer copy. New content **and** a new identity.
    ///
    /// # Panics
    /// When `path` is not a valid [`VaultPath`].
    pub fn modify_externally(&self, path: &str, contents: impl AsRef<[u8]>) {
        self.insert_file(path, contents);
    }

    /// Another program rewrote the file **in place** (`>`, a truncating
    /// write): new content, the same identity. Only the fingerprint can tell.
    ///
    /// # Panics
    /// When there is no such file.
    pub fn modify_in_place(&self, path: &str, contents: impl AsRef<[u8]>) {
        let path = fixture_path(path);
        let mut state = self.write_state();
        let file = state
            .files
            .get_mut(&path)
            .unwrap_or_else(|| panic!("no MemVault file {path}"));
        file.bytes = Arc::from(contents.as_ref());
        file.modified = SystemTime::now();
    }

    /// Creates a directory (and its ancestors).
    ///
    /// # Panics
    /// When `path` is not a valid [`VaultPath`].
    pub fn insert_dir(&self, path: &str) {
        let path = fixture_path(path);
        let mut state = self.write_state();
        state.add_ancestors(&path);
        state.dirs.insert(path);
    }

    /// Creates a symlink at `path` pointing at `target`, written as a link's
    /// target is: relative to the link's own directory.
    ///
    /// # Panics
    /// When `path` is not a valid [`VaultPath`].
    pub fn insert_symlink(&self, path: &str, target: &str) {
        let path = fixture_path(path);
        let mut state = self.write_state();
        state.add_ancestors(&path);
        state.files.remove(&path);
        state.links.insert(path, target.to_string());
    }

    /// Marks a file as listed-but-not-downloaded (or back).
    ///
    /// # Panics
    /// When there is no such file.
    pub fn set_availability(&self, path: &str, availability: Availability) {
        let path = fixture_path(path);
        let mut state = self.write_state();
        state
            .files
            .get_mut(&path)
            .unwrap_or_else(|| panic!("no MemVault file {path}"))
            .availability = availability;
    }

    /// Sets a file's modification time.
    ///
    /// # Panics
    /// When there is no such file.
    pub fn set_modified(&self, path: &str, modified: SystemTime) {
        let path = fixture_path(path);
        let mut state = self.write_state();
        state
            .files
            .get_mut(&path)
            .unwrap_or_else(|| panic!("no MemVault file {path}"))
            .modified = modified;
    }

    /// A file's current bytes, for assertions.
    #[must_use]
    pub fn contents(&self, path: &str) -> Option<Vec<u8>> {
        let path = VaultPath::new(path).ok()?;
        self.read_state().files.get(&path).map(|f| f.bytes.to_vec())
    }

    /// A file's current bytes as UTF-8, for assertions.
    #[must_use]
    pub fn text(&self, path: &str) -> Option<String> {
        self.contents(path)
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    /// Every path passed to [`Vault::request_download`] so far, in order.
    #[must_use]
    pub fn download_requests(&self) -> Vec<VaultPath> {
        self.read_state().downloads.clone()
    }

    fn identity(file: &MemFile) -> Option<FileIdentity> {
        Some(FileIdentity::from_raw((MEM_DEV, file.ino)))
    }

    fn revision(file: &MemFile) -> Revision {
        Revision {
            fingerprint: Fingerprint::of_bytes(&file.bytes),
            identity: Self::identity(file),
        }
    }

    fn file_entry(path: &VaultPath, file: &MemFile) -> Entry {
        Entry {
            path: path.clone(),
            kind: EntryKind::File,
            size: file.bytes.len() as u64,
            modified: file.modified,
            availability: file.availability,
            identity: Self::identity(file),
        }
    }

    fn plain_entry(path: &VaultPath, kind: EntryKind) -> Entry {
        Entry {
            path: path.clone(),
            kind,
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
            availability: Availability::Local,
            identity: None,
        }
    }

    fn entry_in(state: &MemState, path: &VaultPath) -> Option<Entry> {
        if let Some(file) = state.files.get(path) {
            return Some(Self::file_entry(path, file));
        }
        if state.links.contains_key(path) {
            return Some(Self::plain_entry(path, EntryKind::Symlink));
        }
        state
            .dirs
            .contains(path)
            .then(|| Self::plain_entry(path, EntryKind::Dir))
    }

    /// The readable file at `path`: never through a link, never a directory,
    /// never not-downloaded.
    fn readable<'s>(state: &'s MemState, path: &VaultPath) -> Result<&'s MemFile, VaultError> {
        match state.files.get(path) {
            Some(file) if file.availability == Availability::NotDownloaded => {
                Err(VaultError::NotDownloaded {
                    path: path.to_string(),
                })
            }
            Some(file) => Ok(file),
            None if state.links.contains_key(path) => Err(io_error(
                path,
                io::ErrorKind::InvalidInput,
                "the path is a symbolic link; refusing to follow it",
            )),
            None if state.dirs.contains(path) => Err(io_error(
                path,
                io::ErrorKind::InvalidInput,
                "the path is not a regular file",
            )),
            None => Err(VaultError::not_found(path)),
        }
    }

    /// The parent of `path` must be an existing, real directory.
    fn require_parent(state: &MemState, path: &VaultPath) -> Result<(), VaultError> {
        let parent = path.parent().ok_or_else(|| {
            io_error(path, io::ErrorKind::IsADirectory, "the root is a directory")
        })?;
        if state.dirs.contains(&parent) {
            Ok(())
        } else {
            Err(VaultError::not_found(&parent))
        }
    }

    /// Resolve every link in `path`, segment by segment.
    fn resolve(state: &MemState, path: &VaultPath) -> Result<VaultPath, VaultError> {
        let mut resolved = VaultPath::root();
        let mut pending: Vec<String> = path.segments().rev().map(str::to_string).collect();
        let mut hops = 0;
        while let Some(segment) = pending.pop() {
            let candidate = resolved.child(&segment)?;
            if let Some(target) = state.links.get(&candidate) {
                hops += 1;
                if hops > MAX_LINK_HOPS {
                    return Err(io_error(
                        path,
                        io::ErrorKind::InvalidInput,
                        "too many levels of symbolic links",
                    ));
                }
                // An absolute target, or one climbing above the root, leaves.
                let outside = || VaultError::OutsideRoot {
                    path: path.to_string(),
                };
                let joined = resolved.join(target).map_err(|_| outside())?;
                pending.extend(joined.segments().rev().map(str::to_string));
                resolved = VaultPath::root();
                continue;
            }
            if !state.occupied(&candidate) {
                return Err(VaultError::not_found(path));
            }
            resolved = candidate;
        }
        Ok(resolved)
    }
}

impl Vault for MemVault {
    fn root(&self) -> &Path {
        &self.root
    }

    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        Ok(Self::entry_in(&self.read_state(), path))
    }

    fn list_dir(&self, dir: &VaultPath, cap: Option<usize>) -> Result<Vec<Entry>, VaultError> {
        let state = self.read_state();
        if !state.dirs.contains(dir) {
            return Err(if state.occupied(dir) {
                io_error(dir, io::ErrorKind::NotADirectory, "not a directory")
            } else {
                VaultError::not_found(dir)
            });
        }
        let is_child = |p: &VaultPath| !p.is_root() && p.parent().as_ref() == Some(dir);
        let dirs = state
            .dirs
            .iter()
            .filter(|p| is_child(p))
            .map(|p| Self::plain_entry(p, EntryKind::Dir));
        let links = state
            .links
            .keys()
            .filter(|p| is_child(p))
            .map(|p| Self::plain_entry(p, EntryKind::Symlink));
        let files = state
            .files
            .iter()
            .filter(|(p, _)| is_child(p))
            .map(|(p, f)| Self::file_entry(p, f));
        Ok(dirs
            .chain(links)
            .chain(files)
            .take(cap.unwrap_or(usize::MAX))
            .collect())
    }

    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        Self::resolve(&self.read_state(), path)
    }

    fn read_prefix(&self, path: &VaultPath, max_len: u64) -> Result<Vec<u8>, VaultError> {
        let state = self.read_state();
        let bytes = &Self::readable(&state, path)?.bytes;
        let end = usize::try_from(max_len)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        Ok(bytes[..end].to_vec())
    }

    fn read_pinned(
        &self,
        path: &VaultPath,
        cap: Option<u64>,
    ) -> Result<(Vec<u8>, Revision), VaultError> {
        let state = self.read_state();
        let file = Self::readable(&state, path)?;
        if cap.is_some_and(|cap| file.bytes.len() as u64 > cap) {
            return Err(io_error(
                path,
                io::ErrorKind::FileTooLarge,
                "the file is larger than this read allows",
            ));
        }
        Ok((file.bytes.to_vec(), Self::revision(file)))
    }

    fn replace(
        &self,
        path: &VaultPath,
        bytes: &[u8],
        expected: &Revision,
    ) -> Result<Revision, VaultError> {
        let mut state = self.write_state();
        let changed = || VaultError::Changed {
            path: path.to_string(),
        };
        let current = match Self::readable(&state, path) {
            Ok(file) => Self::revision(file),
            // Not downloaded: nothing to compare against, and nothing written.
            Err(error @ VaultError::NotDownloaded { .. }) => return Err(error),
            // Gone, a link, a directory: not the file the caller read.
            Err(_) => return Err(changed()),
        };
        if !expected.matches(&current) {
            return Err(changed());
        }
        let file = self.fresh_file(bytes, Availability::Local);
        let revision = Self::revision(&file);
        state.files.insert(path.clone(), file);
        Ok(revision)
    }

    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        let mut state = self.write_state();
        Self::require_parent(&state, path)?;
        if state.links.contains_key(path) || state.dirs.contains(path) {
            return Err(io_error(
                path,
                io::ErrorKind::InvalidInput,
                "the path is not a regular file",
            ));
        }
        let file = self.fresh_file(bytes, Availability::Local);
        let revision = Self::revision(&file);
        state.files.insert(path.clone(), file);
        Ok(revision)
    }

    fn create_new(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        let mut state = self.write_state();
        Self::require_parent(&state, path)?;
        if state.occupied(path) {
            return Err(io_error(
                path,
                io::ErrorKind::AlreadyExists,
                "something already exists at this name",
            ));
        }
        let file = self.fresh_file(bytes, Availability::Local);
        let revision = Self::revision(&file);
        state.files.insert(path.clone(), file);
        Ok(revision)
    }

    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        let mut state = self.write_state();
        if let Some(blocker) = path
            .ancestors()
            .find(|a| state.files.contains_key(a) || state.links.contains_key(a))
        {
            return Err(io_error(
                &blocker,
                io::ErrorKind::NotADirectory,
                "not a directory",
            ));
        }
        state.add_ancestors(path);
        state.dirs.insert(path.clone());
        Ok(())
    }

    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        let mut state = self.write_state();
        if state.files.remove(path).is_some() || state.links.remove(path).is_some() {
            Ok(())
        } else {
            Err(VaultError::not_found(path))
        }
    }

    fn availability(&self, path: &VaultPath) -> Availability {
        let state = self.read_state();
        match state.files.get(path) {
            Some(file) => file.availability,
            None if state.dirs.contains(path) => Availability::Local,
            None => Availability::Unknown,
        }
    }

    fn request_download(&self, paths: &[VaultPath]) {
        let mut state = self.write_state();
        for path in paths {
            if let Some(file) = state.files.get_mut(path) {
                file.availability = Availability::Local;
            }
            state.downloads.push(path.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    #[test]
    fn files_imply_their_directories() {
        let vault = MemVault::new().with_file("a/b/c.journal", "c");
        assert!(vault.is_dir(&vp("a")));
        assert!(vault.is_dir(&vp("a/b")));
        assert!(vault.is_file(&vp("a/b/c.journal")));
        assert!(vault.is_dir(&VaultPath::root()));
        assert_eq!(vault.stat(&vp("a/x")).unwrap(), None);
        assert_eq!(vault.local_path(&vp("a/b/c.journal")), None);
    }

    #[test]
    fn list_dir_lists_immediate_children_with_links_as_links() {
        let vault = MemVault::new()
            .with_file("a/b/c.journal", "c")
            .with_file("a/d.journal", "d")
            .with_dir("a/empty");
        vault.insert_symlink("a/link.journal", "d.journal");
        let mut names: Vec<(String, EntryKind)> = vault
            .list_dir(&vp("a"), None)
            .unwrap()
            .into_iter()
            .map(|e| (e.path.to_string(), e.kind))
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                ("a/b".to_string(), EntryKind::Dir),
                ("a/d.journal".to_string(), EntryKind::File),
                ("a/empty".to_string(), EntryKind::Dir),
                ("a/link.journal".to_string(), EntryKind::Symlink),
            ]
        );
        assert_eq!(vault.list_dir(&vp("a"), Some(2)).unwrap().len(), 2);
        assert!(
            vault
                .list_dir(&vp("missing"), None)
                .unwrap_err()
                .is_not_found()
        );
        assert!(vault.list_dir(&vp("a/d.journal"), None).is_err());
    }

    #[test]
    fn reads_refuse_links_and_honour_caps() {
        let vault = MemVault::new().with_file("x.journal", "hello");
        vault.insert_symlink("y.journal", "x.journal");
        assert_eq!(vault.read(&vp("x.journal")).unwrap(), b"hello");
        assert_eq!(vault.read_prefix(&vp("x.journal"), 2).unwrap(), b"he");
        assert!(
            vault
                .read(&vp("y.journal"))
                .unwrap_err()
                .is_not_regular_file()
        );
        let err = vault.read_pinned(&vp("x.journal"), Some(4)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::FileTooLarge);
        assert_eq!(
            vault.canonicalize(&vp("y.journal")).unwrap(),
            vp("x.journal")
        );
    }

    #[test]
    fn canonicalize_follows_links_inside_and_refuses_ones_that_leave() {
        let vault = MemVault::new().with_file("real/a.journal", "a");
        vault.insert_symlink("alias", "real");
        vault.insert_symlink("up", "../outside");
        vault.insert_symlink("abs", "/etc");
        vault.insert_symlink("loop", "loop");
        assert_eq!(
            vault.canonicalize(&vp("alias/a.journal")).unwrap(),
            vp("real/a.journal")
        );
        assert!(matches!(
            vault.canonicalize(&vp("up/x")),
            Err(VaultError::OutsideRoot { .. })
        ));
        assert!(matches!(
            vault.canonicalize(&vp("abs/passwd")),
            Err(VaultError::OutsideRoot { .. })
        ));
        assert!(vault.canonicalize(&vp("loop")).is_err(), "a link loop ends");
        assert!(
            vault
                .canonicalize(&vp("real/missing"))
                .unwrap_err()
                .is_not_found()
        );
    }

    #[test]
    fn not_downloaded_files_are_listed_but_unreadable_until_requested() {
        let vault = MemVault::new().with_file("cloud.journal", "; cloud");
        vault.set_availability("cloud.journal", Availability::NotDownloaded);
        let entry = vault.stat(&vp("cloud.journal")).unwrap().unwrap();
        assert_eq!(entry.availability, Availability::NotDownloaded);
        assert_eq!(entry.size, 7);
        assert!(matches!(
            vault.read_pinned(&vp("cloud.journal"), None),
            Err(VaultError::NotDownloaded { .. })
        ));
        assert!(matches!(
            vault.replace(&vp("cloud.journal"), b"x", &Revision::of_bytes(b"; cloud")),
            Err(VaultError::NotDownloaded { .. })
        ));
        vault.request_download(&[vp("cloud.journal")]);
        assert_eq!(vault.download_requests(), vec![vp("cloud.journal")]);
        assert_eq!(vault.read(&vp("cloud.journal")).unwrap(), b"; cloud");
    }

    #[test]
    fn replace_detects_both_kinds_of_external_change() {
        let vault = MemVault::new().with_file("a.journal", "one");
        let (_, read) = vault.read_pinned(&vp("a.journal"), None).unwrap();

        // Replaced behind our back with IDENTICAL bytes: a new identity.
        vault.modify_externally("a.journal", "one");
        assert!(matches!(
            vault.replace(&vp("a.journal"), b"two", &read),
            Err(VaultError::Changed { .. })
        ));

        // Rewritten in place: same identity, different bytes.
        let (_, read) = vault.read_pinned(&vp("a.journal"), None).unwrap();
        vault.modify_in_place("a.journal", "uno");
        assert!(matches!(
            vault.replace(&vp("a.journal"), b"two", &read),
            Err(VaultError::Changed { .. })
        ));
        assert_eq!(vault.text("a.journal").unwrap(), "uno");

        // Unchanged: the write lands and hands back the new revision.
        let (_, read) = vault.read_pinned(&vp("a.journal"), None).unwrap();
        let written = vault.replace(&vp("a.journal"), b"two", &read).unwrap();
        assert_eq!(vault.text("a.journal").unwrap(), "two");
        assert!(!read.matches(&written));
        vault.replace(&vp("a.journal"), b"three", &written).unwrap();

        // Gone, or a link now: changed.
        vault.insert_symlink("a.journal", "elsewhere");
        assert!(matches!(
            vault.replace(&vp("a.journal"), b"x", &written),
            Err(VaultError::Changed { .. })
        ));
    }

    #[test]
    fn create_new_is_exclusive_and_needs_a_parent() {
        let vault = MemVault::new().with_file("docs/a.journal", "old");
        vault.insert_symlink("docs/link", "nowhere");
        let exists = |e: VaultError| e.kind() == io::ErrorKind::AlreadyExists;
        assert!(exists(
            vault.create_new(&vp("docs/a.journal"), b"x").unwrap_err()
        ));
        assert!(exists(
            vault.create_new(&vp("docs/link"), b"x").unwrap_err()
        ));
        assert!(exists(vault.create_new(&vp("docs"), b"x").unwrap_err()));
        assert!(
            vault
                .create_new(&vp("nope/x.journal"), b"x")
                .unwrap_err()
                .is_not_found()
        );
        vault.create_new(&vp("docs/b.journal"), b"b").unwrap();
        assert_eq!(vault.text("docs/b.journal").unwrap(), "b");
    }

    #[test]
    fn write_atomic_refuses_links_and_directories() {
        let vault = MemVault::new().with_file("docs/a.journal", "old");
        vault.insert_symlink("docs/link", "a.journal");
        vault.write_atomic(&vp("docs/a.journal"), b"new").unwrap();
        assert_eq!(vault.text("docs/a.journal").unwrap(), "new");
        assert!(vault.write_atomic(&vp("docs/link"), b"x").is_err());
        assert!(vault.write_atomic(&vp("docs"), b"x").is_err());
        assert!(vault.create_dir_all(&vp("docs/a.journal/sub")).is_err());
        vault.create_dir_all(&vp("new/deeper")).unwrap();
        vault.write_atomic(&vp("new/deeper/x"), b"x").unwrap();
        vault.remove_file(&vp("new/deeper/x")).unwrap();
        assert!(vault.remove_file(&vp("new/deeper/x")).is_err());
    }

    #[test]
    fn keys_live_under_the_virtual_root() {
        let vault = MemVault::new();
        let key = vault.key(&vp("a/b.journal"));
        assert!(key.starts_with(MemVault::DEFAULT_ROOT));
        assert_eq!(vault.vault_path(&key).unwrap(), vp("a/b.journal"));
    }
}
