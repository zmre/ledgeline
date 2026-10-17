//! The storage seam: every read and write of the journal's own directory tree
//! goes through a [`Vault`].
//!
//! Ledgeline used to assume `std::fs` everywhere. That is right for the desktop
//! and wrong for everything else it is meant to run on: an iOS file provider
//! hands back *dataless* files that block on `open()` while they download and
//! wants coordinated writes, and tests want a journal that lives in memory. The
//! trait is the one place those differences are allowed to live. The design
//! mirrors `mbr-core::vault`, with three deliberate differences, each of which
//! keeps a guarantee Ledgeline already made:
//!
//! - **Conditional writes.** mbr writes unconditionally. Ledgeline's editor
//!   refuses to overwrite a file that changed since it was read (DL-3), so the
//!   write primitive here is [`Vault::replace`], which takes the [`Revision`]
//!   the caller read and fails with [`VaultError::Changed`] when the file is no
//!   longer it. [`Vault::create_new`] is the exclusive create. Only files the
//!   app owns outright use the unconditional [`Vault::write_atomic`].
//! - **Symlinks are reported, never followed, by listings and writes.** mbr
//!   follows links everywhere. Ledgeline's scans refuse them and its writes
//!   never follow one (see [`crate::pinned`]), so [`Vault::stat`] and
//!   [`Vault::list_dir`] describe the link itself ([`EntryKind::Symlink`]).
//!   [`Vault::canonicalize`] is the one place links are resolved — exactly
//!   where an `include` target was always canonicalized — and the main journal
//!   path itself is canonicalized once, when the desktop opens it.
//! - **Revisions, not mtimes.** A [`Revision`] is the content
//!   [`Fingerprint`] plus an opaque identity (`(dev, ino)` on a
//!   [`LocalVault`]). A modification time never decides that a file changed.
//!
//! # Rules
//!
//! - **No platform `cfg` in core.** iCloud, file coordination and the dataless
//!   checks live in the FFI crate, which implements [`Vault`] (or wraps
//!   [`LocalVault`]). They are *injected*, never branched on here. The one
//!   platform-specific mapping, `EDEADLK` → [`VaultError::NotDownloaded`],
//!   needs no `cfg`: no other read returns it.
//! - **Containment at the boundary.** A [`VaultPath`] cannot name anything
//!   outside the vault lexically, so a value of that type is the proof that a
//!   path was checked.
//! - **`local_path` for subprocesses.** `hledger -f` and `git` need a real path.
//!   [`Vault::local_path`] returns one where it exists; a vault without one
//!   (iOS, [`MemVault`]) cannot run them, and callers say so.
//!
//! # Keys
//!
//! The parsed journal still records every source file as an absolute
//! [`PathBuf`] (`Transaction::source_file`, `Journal::source_files`). For a
//! vault those are [`Vault::key`] — the vault path under [`Vault::root`]. For a
//! [`LocalVault`] that is the canonical on-disk path, identical to the keys
//! before this seam existed; for any other vault it is an opaque identifier
//! that must only be turned back into a [`VaultPath`] ([`Vault::vault_path`]),
//! never opened.

mod local;
mod mem;
mod path;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use crate::edit::Fingerprint;
pub use crate::pinned::FileIdentity;
pub use local::LocalVault;
pub use mem::MemVault;
pub use path::{VaultPath, VaultPathError};

/// What an [`Entry`] is. Never the kind of a symlink's target: a link is a
/// [`EntryKind::Symlink`], which every scan in this crate refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    /// A FIFO, device or socket. Reading a FIFO with no writer blocks forever,
    /// so these are never opened.
    Other,
}

/// Whether a file's bytes are on this device.
///
/// A file provider can list a file whose content is still in the cloud: iOS
/// "dataless" files report their real size from `stat`, but reading them
/// blocks (or, with the reader's I/O policy, fails fast) until they download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Availability {
    /// The bytes are local; reading will not wait on the network.
    #[default]
    Local,
    /// Listed, but the content has to be downloaded first. Reads return
    /// [`VaultError::NotDownloaded`]; [`Vault::request_download`] asks for it.
    NotDownloaded,
    /// The vault cannot tell cheaply.
    Unknown,
}

/// One directory entry, as listed or stat'ed — describing the entry itself,
/// never a symlink's target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: VaultPath,
    pub kind: EntryKind,
    /// Size in bytes (0 for directories on most backends).
    pub size: u64,
    /// Last modification time, [`SystemTime::UNIX_EPOCH`] when the backend
    /// cannot report one. **For ranking only, never for change detection** —
    /// see [`Revision`].
    pub modified: SystemTime,
    pub availability: Availability,
    /// The entry's identity (`(dev, ino)` on a [`LocalVault`]), when the
    /// backend has one. Lets a caller that scanned a file insist, at write
    /// time, that the name still holds the same file.
    pub identity: Option<FileIdentity>,
}

impl Entry {
    #[must_use]
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }

    #[must_use]
    pub fn is_file(&self) -> bool {
        self.kind == EntryKind::File
    }

    /// The entry's own name (last path segment); empty for the root.
    #[must_use]
    pub fn name(&self) -> &str {
        self.path.file_name().unwrap_or("")
    }

    /// The modification time, or `None` for the epoch placeholder a backend
    /// reports when it has none.
    #[must_use]
    pub fn modified_time(&self) -> Option<SystemTime> {
        (self.modified != SystemTime::UNIX_EPOCH).then_some(self.modified)
    }
}

/// What a caller read, and what a conditional write insists on finding.
///
/// The content [`Fingerprint`] is always present; the identity is whatever the
/// backend can vouch for (`(dev, ino)` on a [`LocalVault`]). Two revisions
/// describe the same file when the fingerprints match and, if the *expected*
/// revision carries an identity, the identities match too — a file replaced by
/// a byte-identical copy under the same name is still "the same" for a backend
/// with no identity, which is the honest best it can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub fingerprint: Fingerprint,
    pub identity: Option<FileIdentity>,
}

impl Revision {
    /// The revision of `bytes` with no identity: what a caller holds for text
    /// it was handed rather than read.
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self {
            fingerprint: Fingerprint::of_bytes(bytes),
            identity: None,
        }
    }

    /// Whether `current` is still the file `self` describes. See the type docs.
    #[must_use]
    pub fn matches(&self, current: &Revision) -> bool {
        self.fingerprint.content_matches(&current.fingerprint)
            && (self.identity.is_none() || self.identity == current.identity)
    }

    /// The opaque, comparable token a client echoes back as an `If-Match`
    /// ([`Fingerprint::token`]).
    #[must_use]
    pub fn token(&self) -> String {
        self.fingerprint.token()
    }

    /// This revision as a [`crate::pinned::Pin`].
    #[must_use]
    pub fn to_pin(&self) -> crate::pinned::Pin {
        crate::pinned::Pin {
            identity: self.identity,
            fingerprint: Some(self.fingerprint.clone()),
        }
    }
}

/// Storage failures, classified so callers can map them without inspecting
/// `io::ErrorKind`s themselves.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("not found: {path}")]
    NotFound { path: String },

    /// The file is listed but its content is not on this device (see
    /// [`Availability::NotDownloaded`]).
    #[error("not downloaded: {path}")]
    NotDownloaded { path: String },

    #[error("permission denied: {path}")]
    PermissionDenied { path: String },

    /// The path resolves (through a symlink) to somewhere outside the vault.
    /// `path` names what the caller asked for, never the resolved target.
    #[error("outside the vault root: {path}")]
    OutsideRoot { path: String },

    /// The vault (or this part of it) cannot be written.
    #[error("read-only: {path}")]
    ReadOnly { path: String },

    /// A conditional write found a file other than the one the caller read:
    /// different content, a different inode, a symlink, or nothing. Nothing
    /// was written.
    #[error("the file changed on disk since it was read; refusing to overwrite: {path}")]
    Changed { path: String },

    #[error(transparent)]
    InvalidPath(#[from] VaultPathError),

    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
}

impl VaultError {
    /// Classifies an `io::Error` raised for `path`.
    ///
    /// `ErrorKind::Deadlock` is `EDEADLK`, which is what a read of a dataless
    /// file returns on Apple platforms when the reading thread has opted out of
    /// materializing it — so it means "not downloaded" wherever it occurs. The
    /// mapping needs no platform `cfg`: no other read returns it.
    pub fn from_io(path: impl fmt::Display, source: io::Error) -> Self {
        let path = path.to_string();
        match source.kind() {
            io::ErrorKind::NotFound => Self::NotFound { path },
            io::ErrorKind::PermissionDenied => Self::PermissionDenied { path },
            io::ErrorKind::Deadlock => Self::NotDownloaded { path },
            _ => Self::Io { path, source },
        }
    }

    /// Maps a [`crate::pinned::WriteError`] for `path`.
    pub fn from_write(path: impl fmt::Display, error: crate::pinned::WriteError) -> Self {
        match error {
            crate::pinned::WriteError::Changed => Self::Changed {
                path: path.to_string(),
            },
            crate::pinned::WriteError::Io(source) => Self::from_io(path, source),
        }
    }

    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }

    pub fn not_found(path: impl fmt::Display) -> Self {
        Self::NotFound {
            path: path.to_string(),
        }
    }

    /// The `io::ErrorKind` this error converts to (see the `From` impl), for a
    /// caller that already branches on kinds.
    #[must_use]
    pub fn kind(&self) -> io::ErrorKind {
        match self {
            Self::NotFound { .. } => io::ErrorKind::NotFound,
            Self::PermissionDenied { .. } | Self::OutsideRoot { .. } | Self::ReadOnly { .. } => {
                io::ErrorKind::PermissionDenied
            }
            Self::NotDownloaded { .. } => io::ErrorKind::WouldBlock,
            Self::InvalidPath(_) => io::ErrorKind::InvalidInput,
            Self::Changed { .. } => io::ErrorKind::Other,
            Self::Io { source, .. } => source.kind(),
        }
    }

    /// A read refused because the name is a symlink or not a regular file —
    /// the shape [`crate::pinned::read_pinned`] reports as `InvalidInput`.
    #[must_use]
    pub fn is_not_regular_file(&self) -> bool {
        matches!(self, Self::Io { source, .. } if source.kind() == io::ErrorKind::InvalidInput)
    }
}

/// Kind-preserving, so handlers that answer with a status derived from
/// `io::ErrorKind` keep answering the same status.
impl From<VaultError> for io::Error {
    fn from(error: VaultError) -> Self {
        match error {
            VaultError::Io { source, .. } => source,
            other => io::Error::new(other.kind(), other),
        }
    }
}

/// A journal's storage.
///
/// Methods are synchronous: the desktop implementation is blocking `std::fs`,
/// and async callers already move journal I/O to `spawn_blocking`, which is
/// where a foreign implementation's blocking calls belong too.
pub trait Vault: Send + Sync + fmt::Debug {
    /// The namespace [`Vault::key`]s live under. For [`LocalVault`], the
    /// canonical directory; for any other vault, an identifier never opened.
    fn root(&self) -> &Path;

    /// The entry at `path`, **not** following a symlink there, or `None` when
    /// nothing is there.
    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError>;

    /// The immediate children of directory `dir`, in no particular order, each
    /// describing the entry itself (a symlink is [`EntryKind::Symlink`]).
    ///
    /// At most `cap` entries are returned when `cap` is `Some`, and the listing
    /// stops reading there — so one enormous directory cannot defeat a scan's
    /// entry budget by allocating. Which entries survive a cap is unspecified.
    /// Children that cannot be described (a name that is not UTF-8 or not a
    /// valid [`VaultPath`] segment) are skipped.
    fn list_dir(&self, dir: &VaultPath, cap: Option<usize>) -> Result<Vec<Entry>, VaultError>;

    /// Resolves symlinks in `path` and returns where it really is inside the
    /// vault. [`VaultError::NotFound`] when it does not exist;
    /// [`VaultError::OutsideRoot`] when it resolves outside the vault.
    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError>;

    /// The whole file, never through a symlink at `path` (refused as an
    /// [`VaultError::Io`] of kind `InvalidInput`) and never a non-regular file.
    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError> {
        self.read_pinned(path, None).map(|(bytes, _)| bytes)
    }

    /// At most the first `max_len` bytes, opened as [`Vault::read`] opens.
    fn read_prefix(&self, path: &VaultPath, max_len: u64) -> Result<Vec<u8>, VaultError>;

    /// The whole file with the [`Revision`] describing exactly those bytes. A
    /// file longer than `cap` is refused (kind `FileTooLarge`) without reading
    /// more than `cap + 1` bytes of it.
    fn read_pinned(
        &self,
        path: &VaultPath,
        cap: Option<u64>,
    ) -> Result<(Vec<u8>, Revision), VaultError>;

    /// Atomically replaces `path` with `bytes` **if and only if** it is still
    /// the file `expected` describes; [`VaultError::Changed`] otherwise, with
    /// nothing written. Returns the revision of what is now there.
    fn replace(
        &self,
        path: &VaultPath,
        bytes: &[u8],
        expected: &Revision,
    ) -> Result<Revision, VaultError>;

    /// Atomically replaces (or creates) `path` with `bytes`, expecting nothing
    /// beyond "a regular file, or nothing" — a symlink there is refused, never
    /// followed. **For files the app owns outright**; a user's books go through
    /// [`Vault::replace`].
    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError>;

    /// Creates a **new** file at `path`, failing (kind `AlreadyExists`) if
    /// anything — a file, a directory, a symlink — is already there. The
    /// refusal is decided at the create, not by a check made before it.
    fn create_new(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError>;

    /// [`Vault::create_new`] for a file the user authors through Ledgeline (a
    /// rules file, a projection) rather than one the app owns: same exclusive
    /// create, but with the permissions an ordinary new file gets on this
    /// backend (on disk, `0666` less the umask) instead of owner-only. The
    /// default is [`Vault::create_new`], for backends with no permission bits.
    fn create_new_shared(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.create_new(path, bytes)
    }

    /// Creates `path` and any missing ancestors as directories.
    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError>;

    /// Removes the file at `path` (a symlink is removed, not followed).
    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError>;

    /// Whether the file at `path` is on this device, decided without reading
    /// it. The default answers [`Availability::Local`] and does no I/O.
    fn availability(&self, _path: &VaultPath) -> Availability {
        Availability::Local
    }

    /// Asks for not-downloaded files to be fetched. Returns immediately; a
    /// no-op for vaults whose files are always local.
    fn request_download(&self, _paths: &[VaultPath]) {}

    /// A real filesystem path for `path`, when one exists — what `hledger -f`
    /// and `git` are handed. `None` means "only reachable through the vault".
    fn local_path(&self, _path: &VaultPath) -> Option<PathBuf> {
        None
    }

    /// The whole file as UTF-8. Invalid UTF-8 is an [`VaultError::Io`] of kind
    /// `InvalidData`, never a lossy decode.
    fn read_to_string(&self, path: &VaultPath) -> Result<String, VaultError> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|e| VaultError::Io {
            path: path.to_string(),
            source: io::Error::new(io::ErrorKind::InvalidData, e),
        })
    }

    /// The key a parsed journal records for `path`: `path` under
    /// [`Vault::root`].
    fn key(&self, path: &VaultPath) -> PathBuf {
        path.to_native(self.root())
    }

    /// The vault path for a key — the inverse of [`Vault::key`]. Lexical: it
    /// does not resolve symlinks.
    fn vault_path(&self, key: &Path) -> Result<VaultPath, VaultError> {
        VaultPath::from_absolute(self.root(), key).map_err(|e| match e {
            VaultPathError::NotUnderRoot { path, .. } => VaultError::OutsideRoot {
                path: path.display().to_string(),
            },
            other => VaultError::InvalidPath(other),
        })
    }

    /// Whether `path` is an existing regular file (not a symlink). Errors read
    /// as `false`.
    fn is_file(&self, path: &VaultPath) -> bool {
        matches!(self.stat(path), Ok(Some(entry)) if entry.is_file())
    }

    /// Whether `path` is an existing directory (not a symlink to one). Errors
    /// read as `false`.
    fn is_dir(&self, path: &VaultPath) -> bool {
        matches!(self.stat(path), Ok(Some(entry)) if entry.is_dir())
    }
}

/// Places a native, absolute path inside `vault`, or `None` when it lands
/// nowhere in it.
///
/// Lexically first ([`Vault::vault_path`]). When that fails and the vault has a
/// real root ([`Vault::local_path`]), the path is resolved the way the desktop
/// always resolved one — the deepest existing ancestor canonicalized, the rest
/// re-appended — so `/tmp/books/a.journal` still lands in a vault rooted at
/// `/private/tmp/books`. A vault with no local path answers lexically only.
#[must_use]
pub fn place_native(vault: &dyn Vault, native: &Path) -> Option<VaultPath> {
    if !native.is_absolute() {
        return None;
    }
    let normalized = lexically_normalize(native);
    vault.vault_path(&normalized).ok().or_else(|| {
        vault.local_path(&VaultPath::root())?;
        vault.vault_path(&canonicalize_deepest(&normalized)).ok()
    })
}

/// Collapse `.` and `..` components textually, without touching storage. A
/// `..` at a root is kept, so the result still fails any containment test.
pub(crate) fn lexically_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    path.components()
        .fold(PathBuf::new(), |mut out, component| {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !out.pop() {
                        out.push(Component::ParentDir);
                    }
                }
                other => out.push(other),
            }
            out
        })
}

/// The symlink-free form of an absolute native `path` whose tail may not exist
/// yet: the deepest existing ancestor canonicalized, the remaining components
/// re-appended. Never consults the process's working directory.
pub(crate) fn canonicalize_deepest(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    let mut trailing: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    while let (Some(name), Some(parent)) = (
        cursor.file_name().map(std::ffi::OsStr::to_os_string),
        cursor.parent().map(Path::to_path_buf),
    ) {
        trailing.push(name);
        if let Ok(canonical) = std::fs::canonicalize(&parent) {
            return trailing
                .iter()
                .rev()
                .fold(canonical, |resolved, part| resolved.join(part));
        }
        cursor = parent;
    }
    path.to_path_buf()
}

/// Canonicalizes `path` through `vault` when it exists; when it does not, its
/// deepest existing ancestor is canonicalized and the missing tail re-appended.
///
/// The vault form of the old `canonical_include`: a target that is not there
/// yet (a typo, or a file only the editor's overrides hold) still has its
/// directory's symlinks resolved, so a missing file under a directory link out
/// of the vault is [`VaultError::OutsideRoot`], not a lexical pass.
///
/// # Errors
/// [`VaultError::OutsideRoot`] when the path or an ancestor resolves outside
/// the vault; any other error canonicalizing an ancestor that exists.
pub fn canonicalize_existing_prefix(
    vault: &dyn Vault,
    path: &VaultPath,
) -> Result<VaultPath, VaultError> {
    let mut missing: Vec<String> = Vec::new();
    let mut cursor = path.clone();
    loop {
        match vault.canonicalize(&cursor) {
            Ok(canonical) => {
                return missing
                    .iter()
                    .rev()
                    .try_fold(canonical, |resolved, name| resolved.child(name))
                    .map_err(VaultError::from);
            }
            Err(VaultError::NotFound { .. }) => {
                let (Some(name), Some(parent)) = (cursor.file_name(), cursor.parent()) else {
                    // The root itself is missing: nothing to resolve against.
                    return Ok(path.clone());
                };
                missing.push(name.to_string());
                cursor = parent;
            }
            Err(other) => return Err(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_are_classified() {
        let nf = VaultError::from_io("a.journal", io::Error::from(io::ErrorKind::NotFound));
        assert!(nf.is_not_found());
        assert!(matches!(
            VaultError::from_io("a", io::Error::from(io::ErrorKind::PermissionDenied)),
            VaultError::PermissionDenied { .. }
        ));
        // EDEADLK: a dataless file read with materialization turned off.
        assert!(matches!(
            VaultError::from_io("a", io::Error::from(io::ErrorKind::Deadlock)),
            VaultError::NotDownloaded { .. }
        ));
        assert!(matches!(
            VaultError::from_io("a", io::Error::other("boom")),
            VaultError::Io { .. }
        ));
        let refused = VaultError::from_io("a", io::Error::from(io::ErrorKind::InvalidInput));
        assert!(refused.is_not_regular_file());
    }

    #[test]
    fn vault_errors_keep_their_io_kind() {
        let kind = |e: VaultError| io::Error::from(e).kind();
        assert_eq!(kind(VaultError::not_found("x")), io::ErrorKind::NotFound);
        assert_eq!(
            kind(VaultError::OutsideRoot { path: "x".into() }),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            kind(VaultError::NotDownloaded { path: "x".into() }),
            io::ErrorKind::WouldBlock
        );
        let original = io::Error::new(io::ErrorKind::StorageFull, "disk full");
        assert_eq!(
            kind(VaultError::from_io("x", original)),
            io::ErrorKind::StorageFull
        );
    }

    #[test]
    fn write_errors_map_changed_and_io() {
        assert!(matches!(
            VaultError::from_write("a", crate::pinned::WriteError::Changed),
            VaultError::Changed { .. }
        ));
        assert!(
            VaultError::from_write(
                "a",
                crate::pinned::WriteError::Io(io::Error::from(io::ErrorKind::NotFound))
            )
            .is_not_found()
        );
    }

    #[test]
    fn revisions_match_on_content_and_on_identity_when_expected() {
        let a = Revision::of_bytes(b"one");
        assert!(a.matches(&Revision::of_bytes(b"one")));
        assert!(!a.matches(&Revision::of_bytes(b"two")));
        let pinned = Revision {
            identity: Some(FileIdentity::from_raw((1, 7))),
            ..a.clone()
        };
        let same_inode = pinned.clone();
        let other_inode = Revision {
            identity: Some(FileIdentity::from_raw((1, 8))),
            ..a.clone()
        };
        assert!(pinned.matches(&same_inode));
        assert!(!pinned.matches(&other_inode), "a different inode changed");
        assert!(!pinned.matches(&a), "an identity was expected and is gone");
        assert!(
            a.matches(&other_inode),
            "no identity expected, none compared"
        );
        assert_eq!(a.token(), Fingerprint::of_bytes(b"one").token());
    }

    #[test]
    fn lexical_normalization_never_climbs_above_a_root() {
        assert_eq!(
            lexically_normalize(Path::new("/a/./b/../c")),
            PathBuf::from("/a/c")
        );
        assert_eq!(lexically_normalize(Path::new("/..")), PathBuf::from("/.."));
    }

    #[test]
    fn place_native_is_lexical_for_a_vault_without_a_local_path() {
        let vault = MemVault::new().with_file("a/b.journal", "");
        let key = vault.key(&VaultPath::new("a/b.journal").unwrap());
        assert_eq!(
            place_native(&vault, &key),
            Some(VaultPath::new("a/b.journal").unwrap())
        );
        assert_eq!(
            place_native(&vault, &key.join("../../..").join("etc/passwd")),
            None
        );
        assert_eq!(place_native(&vault, Path::new("relative.journal")), None);
    }

    #[test]
    fn existing_prefix_canonicalization_keeps_the_missing_tail() {
        let vault = MemVault::new().with_file("a/b.journal", "");
        let p = |s: &str| VaultPath::new(s).unwrap();
        assert_eq!(
            canonicalize_existing_prefix(&vault, &p("a/missing/x.journal")).unwrap(),
            p("a/missing/x.journal")
        );
        assert_eq!(
            canonicalize_existing_prefix(&vault, &p("a/b.journal")).unwrap(),
            p("a/b.journal")
        );
    }
}
