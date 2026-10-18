//! [`CoordinatedVault`]: a folder the app was granted, read with `std::fs`
//! and written through the app's [`FileCoordinator`].

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ledgeline_core::vault::{
    Availability, Entry, EntryKind, Fingerprint, LocalVault, Revision, Vault, VaultError, VaultPath,
};

use crate::coordinator::{CoordinatorError, FileCoordinator, hash_hex};

/// How many times an unconditional write ([`Vault::write_atomic`]) re-reads
/// and retries when the file changes between its read and the coordinated
/// replace. Only app-owned files take that path, so contention means another
/// copy of the app is writing the same cache; three rounds is plenty.
const WRITE_ATTEMPTS: usize = 3;

/// A folder the app was granted, as a [`Vault`].
///
/// **Reads** are the desktop's: a [`LocalVault`] on the real path the picker
/// handed over, so a parse runs on plain `std::fs` (`O_NOFOLLOW`, regular
/// files only) with no call into Swift per file.
///
/// **Writes** — [`Vault::replace`], [`Vault::write_atomic`],
/// [`Vault::create_new`], [`Vault::create_dir_all`], [`Vault::remove_file`] —
/// and [`Vault::request_download`] go through the [`FileCoordinator`] when
/// there is one, because a file provider must see them coordinated. Without a
/// coordinator (tests, a folder inside the app's own container) they are the
/// [`LocalVault`]'s.
///
/// **Conditional writes survive the seam.** [`Vault::replace`] checks in Rust
/// first (refusing early, without calling Swift, when the file is already not
/// the one read) and then hands the coordinator the expected length and
/// FNV-1a hash, which it re-checks *inside* the coordinated write — see the
/// [coordinator contract](crate::coordinator). [`Vault::write_atomic`] has no
/// expectation of its own, so it reads the current revision and replaces
/// conditionally on that (or creates exclusively when there is nothing),
/// retrying a lost race a few times: the trait stays one conditional
/// primitive, never an unconditional overwrite.
///
/// **Revisions are content-only when coordinated.** A file provider replaces
/// a file's inode whenever it syncs one, identical bytes or not, and the Swift
/// side can only compare content — so with a coordinator, the revisions this
/// vault hands out carry no identity, and both checks agree on what "the same
/// file" means. A symlink is still never written through: the Rust read opens
/// `O_NOFOLLOW` and the contract has Swift refuse one too.
///
/// **Availability.** On Apple platforms a file provider's not-downloaded files
/// are [`Availability::NotDownloaded`]:
///
/// - a *dataless* file — real name, real size, `SF_DATALESS` in `st_flags` —
///   is classified from the metadata a stat or listing already has
///   ([`LocalVault::with_availability_probe`]), at no extra cost;
/// - a legacy iCloud placeholder `.Name.ext.icloud` is listed as `Name.ext`,
///   not downloaded, and never as a file of its own; when both exist, the
///   real file wins. `stat`, `availability` and reads of `Name.ext` know about
///   the placeholder too.
///
/// Reading a not-downloaded file fails with [`VaultError::NotDownloaded`] —
/// decided from the stat before the read where it can be, and otherwise from
/// `EDEADLK` under the runtime's fail-fast I/O policy
/// ([`crate::iopolicy`]). Nothing here waits for a download.
pub struct CoordinatedVault {
    local: LocalVault,
    coordinator: Option<Arc<dyn FileCoordinator>>,
    /// Map `.Name.ext.icloud` placeholders to `Name.ext`. On by default on
    /// Apple platforms only, where they exist.
    icloud_placeholders: bool,
}

impl fmt::Debug for CoordinatedVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoordinatedVault")
            .field("local", &self.local)
            .field("coordinated", &self.coordinator.is_some())
            .field("icloud_placeholders", &self.icloud_placeholders)
            .finish()
    }
}

impl CoordinatedVault {
    /// The folder at `root` (canonicalized, as [`LocalVault::new`] does).
    pub fn new(root: impl Into<PathBuf>, coordinator: Option<Arc<dyn FileCoordinator>>) -> Self {
        let local = LocalVault::new(root);
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let local = local.with_availability_probe(dataless_probe);
        Self {
            local,
            coordinator,
            icloud_placeholders: cfg!(target_vendor = "apple"),
        }
    }

    /// Turn placeholder mapping on or off (tests exercise it everywhere).
    #[must_use]
    pub fn with_icloud_placeholders(mut self, enabled: bool) -> Self {
        self.icloud_placeholders = enabled;
        self
    }

    fn native_string(&self, path: &VaultPath) -> String {
        self.local.native(path).to_string_lossy().into_owned()
    }

    /// The placeholder entry standing in for `path`, when there is one and no
    /// real file.
    fn placeholder_for(&self, path: &VaultPath) -> Option<Entry> {
        if !self.icloud_placeholders {
            return None;
        }
        let name = path.file_name()?;
        let stub = path.parent()?.child(&placeholder_name(name)).ok()?;
        let entry = self.local.stat(&stub).ok()??;
        entry.is_file().then(|| Entry {
            path: path.clone(),
            // The stub is a small plist, not the file: its size says nothing.
            size: 0,
            availability: Availability::NotDownloaded,
            identity: None,
            ..entry
        })
    }

    /// A read that found nothing: not downloaded when a placeholder says so.
    fn missing(&self, path: &VaultPath, error: VaultError) -> VaultError {
        if error.is_not_found() && self.placeholder_for(path).is_some() {
            not_downloaded(path)
        } else {
            error
        }
    }

    /// Refuse a read before it starts when the file is known not to be here,
    /// so a read on a thread without the fail-fast policy cannot block on the
    /// download.
    fn require_local(&self, path: &VaultPath) -> Result<(), VaultError> {
        match self.availability(path) {
            Availability::NotDownloaded => Err(not_downloaded(path)),
            Availability::Local | Availability::Unknown => Ok(()),
        }
    }

    /// The coordinator, when writes go through one.
    fn coordinated(&self) -> Option<&dyn FileCoordinator> {
        self.coordinator.as_deref()
    }

    /// The revision a coordinated vault reports: content only (see the type
    /// docs).
    fn content_revision(bytes: &[u8]) -> Revision {
        Revision::of_bytes(bytes)
    }

    /// One coordinated conditional replace of `path`, expecting `expected`.
    fn coordinated_replace(
        &self,
        coordinator: &dyn FileCoordinator,
        path: &VaultPath,
        bytes: &[u8],
        expected: &Fingerprint,
    ) -> Result<Revision, CoordinatorError> {
        coordinator
            .coordinated_replace(
                self.native_string(path),
                bytes.to_vec(),
                expected.content_len(),
                hash_hex(expected),
            )
            .map(|()| Self::content_revision(bytes))
    }
}

fn not_downloaded(path: &VaultPath) -> VaultError {
    VaultError::NotDownloaded {
        path: path.to_string(),
    }
}

/// `.Name.ext.icloud`, iCloud's placeholder for `Name.ext`.
fn placeholder_name(name: &str) -> String {
    format!(".{name}.icloud")
}

/// `Name.ext` for a placeholder name `.Name.ext.icloud`.
fn placeholder_target(name: &str) -> Option<&str> {
    let inner = name.strip_prefix('.')?.strip_suffix(".icloud")?;
    (!inner.is_empty()).then_some(inner)
}

/// `SF_DATALESS` (`<sys/stat.h>`): the file's content is not on the device.
#[cfg(any(target_os = "macos", target_os = "ios", test))]
const SF_DATALESS: u32 = 0x4000_0000;

/// Availability from a file's BSD flags.
#[cfg(any(target_os = "macos", target_os = "ios", test))]
fn availability_from_flags(flags: u32) -> Availability {
    if flags & SF_DATALESS != 0 {
        Availability::NotDownloaded
    } else {
        Availability::Local
    }
}

/// The [`LocalVault`] probe: `st_flags` of the metadata already fetched.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn dataless_probe(metadata: &std::fs::Metadata) -> Availability {
    #[cfg(target_os = "ios")]
    use std::os::ios::fs::MetadataExt;
    #[cfg(target_os = "macos")]
    use std::os::macos::fs::MetadataExt;
    availability_from_flags(metadata.st_flags())
}

impl Vault for CoordinatedVault {
    fn root(&self) -> &Path {
        self.local.root()
    }

    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        match self.local.stat(path)? {
            Some(entry) => Ok(Some(entry)),
            None => Ok(self.placeholder_for(path)),
        }
    }

    fn list_dir(&self, dir: &VaultPath, cap: Option<usize>) -> Result<Vec<Entry>, VaultError> {
        let entries = self.local.list_dir(dir, cap)?;
        if !self.icloud_placeholders {
            return Ok(entries);
        }
        let real: std::collections::HashSet<String> = entries
            .iter()
            .filter(|e| placeholder_target(e.name()).is_none())
            .map(|e| e.name().to_owned())
            .collect();
        Ok(entries
            .into_iter()
            .filter_map(|entry| match placeholder_target(entry.name()) {
                None => Some(entry),
                Some(_) if entry.kind != EntryKind::File => Some(entry),
                Some(target) if real.contains(target) => None,
                Some(target) => Some(Entry {
                    path: dir.child(target).ok()?,
                    size: 0,
                    availability: Availability::NotDownloaded,
                    identity: None,
                    ..entry
                }),
            })
            .collect())
    }

    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        self.local.canonicalize(path).or_else(|e| {
            // A placeholder's file does not exist on disk under its own name;
            // it resolves to itself under its (real, canonical) parent.
            match (e.is_not_found(), self.placeholder_for(path), path.parent()) {
                (true, Some(_), Some(parent)) => {
                    let parent = self.local.canonicalize(&parent)?;
                    Ok(parent.child(path.file_name().unwrap_or_default())?)
                }
                _ => Err(e),
            }
        })
    }

    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError> {
        self.require_local(path)?;
        self.local.read(path).map_err(|e| self.missing(path, e))
    }

    fn read_prefix(&self, path: &VaultPath, max_len: u64) -> Result<Vec<u8>, VaultError> {
        self.require_local(path)?;
        self.local
            .read_prefix(path, max_len)
            .map_err(|e| self.missing(path, e))
    }

    fn read_pinned(
        &self,
        path: &VaultPath,
        cap: Option<u64>,
    ) -> Result<(Vec<u8>, Revision), VaultError> {
        self.require_local(path)?;
        let (bytes, revision) = self
            .local
            .read_pinned(path, cap)
            .map_err(|e| self.missing(path, e))?;
        Ok(match self.coordinated() {
            Some(_) => {
                let revision = Revision {
                    identity: None,
                    ..revision
                };
                (bytes, revision)
            }
            None => (bytes, revision),
        })
    }

    fn replace(
        &self,
        path: &VaultPath,
        bytes: &[u8],
        expected: &Revision,
    ) -> Result<Revision, VaultError> {
        let Some(coordinator) = self.coordinated() else {
            return self.local.replace(path, bytes, expected);
        };
        // The cheap, early half of the check: refuse without calling Swift
        // when the file is already not the one read. Gone, a symlink, a
        // directory — all "not the file the caller read".
        let current = match self.read_pinned(path, None) {
            Ok((_, current)) => current,
            Err(error @ VaultError::NotDownloaded { .. }) => return Err(error),
            Err(_) => {
                return Err(VaultError::Changed {
                    path: path.to_string(),
                });
            }
        };
        if !expected.fingerprint.content_matches(&current.fingerprint) {
            return Err(VaultError::Changed {
                path: path.to_string(),
            });
        }
        // The authoritative half, atomic with the write.
        self.coordinated_replace(coordinator, path, bytes, &expected.fingerprint)
            .map_err(|e| e.into_vault_error(path))
    }

    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        let Some(coordinator) = self.coordinated() else {
            return self.local.write_atomic(path, bytes);
        };
        for _ in 0..WRITE_ATTEMPTS {
            let outcome = match self.read_pinned(path, None) {
                Ok((_, current)) => {
                    self.coordinated_replace(coordinator, path, bytes, &current.fingerprint)
                }
                Err(error) if error.is_not_found() => coordinator
                    .coordinated_create(self.native_string(path), bytes.to_vec())
                    .map(|()| Self::content_revision(bytes)),
                // A symlink, a directory, a not-downloaded file: never
                // written over.
                Err(error) => return Err(error),
            };
            match outcome {
                Ok(revision) => return Ok(revision),
                // Lost a race with another writer: read again and retry.
                Err(CoordinatorError::Changed { .. } | CoordinatorError::AlreadyExists { .. }) => {}
                Err(other) => return Err(other.into_vault_error(path)),
            }
        }
        Err(VaultError::Changed {
            path: path.to_string(),
        })
    }

    fn create_new(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        match self.coordinated() {
            Some(coordinator) => coordinator
                .coordinated_create(self.native_string(path), bytes.to_vec())
                .map(|()| Self::content_revision(bytes))
                .map_err(|e| e.into_vault_error(path)),
            None => self.local.create_new(path, bytes),
        }
    }

    /// The same coordinated create: a file provider decides a new file's
    /// permissions, not the app.
    fn create_new_shared(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        match self.coordinated() {
            Some(_) => self.create_new(path, bytes),
            None => self.local.create_new_shared(path, bytes),
        }
    }

    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        match self.coordinated() {
            Some(coordinator) => coordinator
                .coordinated_create_directory(self.native_string(path))
                .map_err(|e| e.into_vault_error(path)),
            None => self.local.create_dir_all(path),
        }
    }

    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        match self.coordinated() {
            Some(coordinator) => coordinator
                .coordinated_remove(self.native_string(path))
                .map_err(|e| e.into_vault_error(path)),
            None => self.local.remove_file(path),
        }
    }

    fn availability(&self, path: &VaultPath) -> Availability {
        match self.local.availability(path) {
            // A missing file answers `Unknown` with the dataless probe but
            // `Local` without one (no I/O), so consult the placeholder in both
            // cases — `stat` and `list_dir` already do, and the three agree.
            Availability::Local | Availability::Unknown
                if self.icloud_placeholders
                    && !matches!(self.local.stat(path), Ok(Some(_)))
                    && self.placeholder_for(path).is_some() =>
            {
                Availability::NotDownloaded
            }
            availability => availability,
        }
    }

    fn request_download(&self, paths: &[VaultPath]) {
        if let Some(coordinator) = self.coordinated()
            && !paths.is_empty()
        {
            coordinator.request_download(paths.iter().map(|p| self.native_string(p)).collect());
        }
    }

    fn local_path(&self, path: &VaultPath) -> Option<PathBuf> {
        self.local.local_path(path)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A coordinator that records every call and then does the operation
    /// with `std::fs` — honouring the contract, conditional replace included —
    /// so the folder stays consistent for the test.
    #[derive(Default)]
    pub(crate) struct FakeCoordinator {
        pub(crate) calls: Mutex<Vec<String>>,
        pub(crate) downloads: Mutex<Vec<String>>,
        /// Bytes to write over the target INSIDE the coordinated block,
        /// before the check — another device's edit landing between Rust's
        /// pre-check and the coordinated write.
        pub(crate) sneak_in: Mutex<Option<Vec<u8>>>,
    }

    impl FakeCoordinator {
        pub(crate) fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        fn log(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
    }

    fn failed(e: std::io::Error) -> CoordinatorError {
        let message = e.to_string();
        match e.kind() {
            std::io::ErrorKind::AlreadyExists => CoordinatorError::AlreadyExists { message },
            std::io::ErrorKind::NotFound => CoordinatorError::NotFound { message },
            _ => CoordinatorError::Failed { message },
        }
    }

    impl FileCoordinator for FakeCoordinator {
        fn coordinated_replace(
            &self,
            path: String,
            contents: Vec<u8>,
            expected_len: u64,
            expected_hash: String,
        ) -> Result<(), CoordinatorError> {
            self.log(format!("replace {path}"));
            if let Some(intruder) = self.sneak_in.lock().unwrap().take() {
                std::fs::write(&path, intruder).map_err(failed)?;
            }
            let meta = std::fs::symlink_metadata(&path).map_err(|e| CoordinatorError::Changed {
                message: e.to_string(),
            })?;
            if !meta.is_file() {
                return Err(CoordinatorError::Changed {
                    message: "not a regular file".into(),
                });
            }
            let current = Fingerprint::of_bytes(&std::fs::read(&path).map_err(failed)?);
            if current.content_len() != expected_len || hash_hex(&current) != expected_hash {
                return Err(CoordinatorError::Changed {
                    message: "content differs".into(),
                });
            }
            ledgeline_core::edit::atomic_write(Path::new(&path), &contents).map_err(failed)
        }
        fn coordinated_create(
            &self,
            path: String,
            contents: Vec<u8>,
        ) -> Result<(), CoordinatorError> {
            self.log(format!("create {path}"));
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .and_then(|mut f| f.write_all(&contents))
                .map_err(failed)
        }
        fn coordinated_create_directory(&self, path: String) -> Result<(), CoordinatorError> {
            self.log(format!("mkdir {path}"));
            std::fs::create_dir_all(path).map_err(failed)
        }
        fn coordinated_remove(&self, path: String) -> Result<(), CoordinatorError> {
            self.log(format!("remove {path}"));
            std::fs::remove_file(&path).map_err(failed)
        }
        fn request_download(&self, paths: Vec<String>) {
            self.downloads.lock().unwrap().extend(paths);
        }
    }

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("books")).unwrap();
        std::fs::write(dir.path().join("books/main.journal"), "; main\n").unwrap();
        // A placeholder alone, and a placeholder beside its real file.
        std::fs::write(dir.path().join("books/.2025.journal.icloud"), "plist").unwrap();
        std::fs::write(dir.path().join("books/.main.journal.icloud"), "plist").unwrap();
        dir
    }

    fn coordinated(dir: &Path) -> (Arc<FakeCoordinator>, CoordinatedVault) {
        let coordinator = Arc::new(FakeCoordinator::default());
        let vault = CoordinatedVault::new(
            dir,
            Some(Arc::clone(&coordinator) as Arc<dyn FileCoordinator>),
        );
        (coordinator, vault)
    }

    #[test]
    fn writes_and_downloads_go_through_the_coordinator_and_reads_never_do() {
        let dir = folder();
        let (coordinator, vault) = coordinated(dir.path());
        let root = vault.root().to_path_buf();
        let main = vp("books/main.journal");

        let (_, read) = vault.read_pinned(&main, None).unwrap();
        assert_eq!(
            read.identity, None,
            "coordinated revisions are content-only"
        );
        let written = vault.replace(&main, b"; edited\n", &read).unwrap();
        assert_eq!(written, Revision::of_bytes(b"; edited\n"));
        vault.create_dir_all(&vp("new/deep")).unwrap();
        vault.create_new(&vp("new/deep/a.journal"), b"a").unwrap();
        vault.write_atomic(&vp("new/deep/a.journal"), b"b").unwrap();
        vault.write_atomic(&vp("new/cache.journal"), b"c").unwrap();
        vault.remove_file(&vp("new/deep/a.journal")).unwrap();

        let p = |s: &str| root.join(s).display().to_string();
        assert_eq!(
            coordinator.calls(),
            vec![
                format!("replace {}", p("books/main.journal")),
                format!("mkdir {}", p("new/deep")),
                format!("create {}", p("new/deep/a.journal")),
                format!("replace {}", p("new/deep/a.journal")),
                format!("create {}", p("new/cache.journal")),
                format!("remove {}", p("new/deep/a.journal")),
            ]
        );
        assert_eq!(
            std::fs::read(root.join("books/main.journal")).unwrap(),
            b"; edited\n"
        );
        assert_eq!(std::fs::read(root.join("new/cache.journal")).unwrap(), b"c");

        vault.request_download(&[vp("books/2025.journal")]);
        assert_eq!(
            *coordinator.downloads.lock().unwrap(),
            vec![p("books/2025.journal")]
        );
        assert_eq!(coordinator.calls().len(), 6, "reads never call back");
    }

    /// The external-change guarantee across the seam: a stale revision is
    /// refused in Rust before Swift is asked, and a change landing INSIDE the
    /// coordinated block is refused there — both as `Changed`, nothing written.
    #[test]
    fn a_conditional_replace_refuses_a_changed_file_on_both_sides() {
        let dir = folder();
        let (coordinator, vault) = coordinated(dir.path());
        let main = vp("books/main.journal");
        let (_, read) = vault.read_pinned(&main, None).unwrap();

        // Changed before the call: Rust's pre-check refuses, Swift never asked.
        std::fs::write(dir.path().join("books/main.journal"), "; other device\n").unwrap();
        assert!(matches!(
            vault.replace(&main, b"mine", &read),
            Err(VaultError::Changed { .. })
        ));
        assert!(coordinator.calls().is_empty());

        // Changed between the pre-check and the write: the coordinator's own
        // check refuses, and the intruder's bytes survive.
        let (_, read) = vault.read_pinned(&main, None).unwrap();
        *coordinator.sneak_in.lock().unwrap() = Some(b"; synced in\n".to_vec());
        assert!(matches!(
            vault.replace(&main, b"mine", &read),
            Err(VaultError::Changed { .. })
        ));
        assert_eq!(coordinator.calls().len(), 1);
        assert_eq!(
            std::fs::read(dir.path().join("books/main.journal")).unwrap(),
            b"; synced in\n"
        );

        // Gone entirely: still `Changed`, never a create.
        std::fs::remove_file(dir.path().join("books/main.journal")).unwrap();
        std::fs::remove_file(dir.path().join("books/.main.journal.icloud")).unwrap();
        assert!(matches!(
            vault.replace(&main, b"mine", &read),
            Err(VaultError::Changed { .. })
        ));
    }

    #[test]
    fn an_exclusive_create_refuses_an_existing_file() {
        let dir = folder();
        let (_, vault) = coordinated(dir.path());
        let error = vault
            .create_new(&vp("books/main.journal"), b"x")
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_never_written_through() {
        let dir = folder();
        let (coordinator, vault) = coordinated(dir.path());
        std::fs::write(dir.path().join("target.journal"), "t").unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("target.journal"),
            dir.path().join("link.journal"),
        )
        .unwrap();
        let link = vp("link.journal");
        assert!(vault.write_atomic(&link, b"x").is_err());
        assert!(matches!(
            vault.replace(&link, b"x", &Revision::of_bytes(b"t")),
            Err(VaultError::Changed { .. })
        ));
        assert!(coordinator.calls().is_empty());
        assert_eq!(
            std::fs::read(dir.path().join("target.journal")).unwrap(),
            b"t"
        );
    }

    #[test]
    fn without_a_coordinator_writes_are_local_and_keep_their_identity() {
        let dir = folder();
        let vault = CoordinatedVault::new(dir.path(), None);
        let main = vp("books/main.journal");
        let (_, read) = vault.read_pinned(&main, None).unwrap();
        assert!(read.identity.is_some(), "the desktop's revision, unchanged");
        vault.replace(&main, b"; local\n", &read).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("books/main.journal")).unwrap(),
            b"; local\n"
        );
        vault.request_download(&[main]);
    }

    #[test]
    fn icloud_placeholders_stand_for_their_files() {
        let dir = folder();
        let vault = CoordinatedVault::new(dir.path(), None).with_icloud_placeholders(true);
        let mut names: Vec<(String, Availability)> = vault
            .list_dir(&vp("books"), None)
            .unwrap()
            .into_iter()
            .map(|e| (e.name().to_owned(), e.availability))
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            names,
            vec![
                ("2025.journal".to_string(), Availability::NotDownloaded),
                ("main.journal".to_string(), Availability::Local),
            ],
            "the real file wins; a placeholder is never listed as itself"
        );
        let stub = vp("books/2025.journal");
        let entry = vault.stat(&stub).unwrap().unwrap();
        assert!(entry.is_file());
        assert_eq!(entry.availability, Availability::NotDownloaded);
        assert_eq!(vault.availability(&stub), Availability::NotDownloaded);
        assert!(matches!(
            vault.read(&stub),
            Err(VaultError::NotDownloaded { .. })
        ));
        assert!(matches!(
            vault.read_pinned(&stub, None),
            Err(VaultError::NotDownloaded { .. })
        ));
        assert!(matches!(
            vault.read_prefix(&stub, 10),
            Err(VaultError::NotDownloaded { .. })
        ));
        assert_eq!(vault.canonicalize(&stub).unwrap(), stub);
        assert!(
            vault
                .read(&vp("books/missing.journal"))
                .unwrap_err()
                .is_not_found()
        );
        assert_eq!(vault.stat(&vp("books/missing.journal")).unwrap(), None);

        let off = CoordinatedVault::new(dir.path(), None).with_icloud_placeholders(false);
        assert_eq!(off.list_dir(&vp("books"), None).unwrap().len(), 3);
        assert_eq!(off.stat(&stub).unwrap(), None);
    }

    /// A coordinated write never lands a real file over a placeholder: the
    /// content is in the cloud, and writing would fork it.
    #[test]
    fn a_placeholder_is_not_written_over() {
        let dir = folder();
        let (coordinator, vault) = coordinated(dir.path());
        let vault = vault.with_icloud_placeholders(true);
        assert!(matches!(
            vault.write_atomic(&vp("books/2025.journal"), b"x"),
            Err(VaultError::NotDownloaded { .. })
        ));
        assert!(coordinator.calls().is_empty());
    }

    #[test]
    fn placeholder_names_round_trip() {
        assert_eq!(placeholder_name("a.journal"), ".a.journal.icloud");
        assert_eq!(placeholder_target(".a.journal.icloud"), Some("a.journal"));
        assert_eq!(placeholder_target(".icloud"), None);
        assert_eq!(placeholder_target("..icloud"), None);
        assert_eq!(placeholder_target("a.journal.icloud"), None);
        assert_eq!(placeholder_target(".a.journal"), None);
    }

    #[test]
    fn the_dataless_flag_means_not_downloaded() {
        assert_eq!(availability_from_flags(0), Availability::Local);
        assert_eq!(
            availability_from_flags(SF_DATALESS | 0x20),
            Availability::NotDownloaded
        );
        assert_eq!(availability_from_flags(0x20), Availability::Local);
    }

    /// The probe is wired in on Apple: an ordinary file is local.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn ordinary_files_are_local_through_the_probe() {
        let dir = folder();
        let vault = CoordinatedVault::new(dir.path(), None);
        let main = vp("books/main.journal");
        assert_eq!(vault.availability(&main), Availability::Local);
        assert_eq!(
            vault.stat(&main).unwrap().unwrap().availability,
            Availability::Local
        );
    }
}
