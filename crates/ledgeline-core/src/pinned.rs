//! **Pinned** file writes: replace a file atomically without ever following a
//! symlink at write time, and only if it is still the file the caller read.
//!
//! # The bug this closes
//! Every write used to resolve symlinks at write time and then rename a temp
//! file into the *link target's* directory, copying the target's owner and mode
//! from a path `stat`. The routes guarded that with a `symlink_metadata` check
//! made earlier — sometimes several 120-second `hledger` runs earlier. A symlink
//! swapped in between redirected the write onto any file the user could write
//! (`~/.zshrc`), and the metadata carry-forward erased the trace.
//!
//! # Resolve once, pin
//! A symlinked journal still works, because the app resolves it **once**, when it
//! learns the file (the parser canonicalizes every source file; the editor keys
//! its buffers by that real path). From then on the caller holds a [`Pin`]: the
//! file's `(dev, ino)` and the fingerprint of the bytes it read. [`replace`]
//! writes to that real path and to nothing else:
//!
//! 1. the parent directory is opened **once**, and every later step is relative
//!    to that descriptor, so a directory swapped mid-write cannot redirect it;
//! 2. the target is opened `O_NOFOLLOW`, required to be a regular file, and its
//!    `(dev, ino)` compared with the pin — owner and mode come from *that*
//!    descriptor's `fstat`, never from a path;
//! 3. the temp file is created `O_CREAT | O_EXCL | O_NOFOLLOW`, `0600`;
//! 4. after the temp file is written and `fsync`ed, the pinned fingerprint is
//!    compared with the bytes read **through the target's descriptor**, and the
//!    name is re-`stat`ed without following links to prove it still names the
//!    same inode;
//! 5. only then is the temp file `renameat`ed over the name, within the same
//!    directory descriptor, and the directory `fsync`ed.
//!
//! A mismatch at any point is [`WriteError::Changed`] and writes nothing; the
//! caller reports it the way it reports any other stale revision (a `409`), and
//! the user reloads. Nothing here is a lock — see
//! [`crate::edit::JournalEditor::save`] for why there is none — so the residual
//! window is the microseconds between step 4 and step 5.
//!
//! # Platforms
//! The descriptor-relative implementation is Unix-only. Elsewhere the same API
//! falls back to path-based operations with the same checks made by path, which
//! is weaker and honestly so; Ledgeline ships for macOS and Linux.

use crate::edit::Fingerprint;
use std::io::Read;
use std::path::Path;
use thiserror::Error;

/// Mode for a temp file at creation, and for a target that does not yet exist:
/// owner-only. A financial record defaults closed rather than umask-derived.
#[cfg(unix)]
const NEW_FILE_MODE: u32 = 0o600;

/// Distinct temp names tried before giving up. `O_EXCL` refuses to open an
/// existing path, so a collision — accidental or hostile — costs a retry.
const TEMP_NAME_ATTEMPTS: usize = 8;

/// `(st_dev, st_ino)`: the pair that says "the same file", not "a file with the
/// same name".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    dev: u64,
    ino: u64,
}

impl FileIdentity {
    /// An identity from a raw `(dev, ino)` pair, as recorded by a directory scan.
    #[must_use]
    pub fn from_raw((dev, ino): (u64, u64)) -> Self {
        Self { dev, ino }
    }

    /// The identity of the file `meta` describes, or `None` on a platform with
    /// no portable inode identity.
    #[cfg(unix)]
    #[must_use]
    pub fn of(meta: &std::fs::Metadata) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;
        Some(Self {
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }

    /// No portable inode identity outside Unix.
    #[cfg(not(unix))]
    #[must_use]
    pub fn of(_meta: &std::fs::Metadata) -> Option<Self> {
        None
    }
}

/// What [`replace`] insists on finding before it writes. Each half is optional:
/// an absent identity is not compared, and an absent fingerprint is not
/// compared. [`Pin::default`] expects nothing beyond "a regular file or
/// nothing", which is right only for files the app owns outright.
#[derive(Debug, Clone, Default)]
pub struct Pin {
    /// The `(dev, ino)` the caller learned the file as.
    pub identity: Option<FileIdentity>,
    /// The content the caller read and based its edit on.
    pub fingerprint: Option<Fingerprint>,
}

impl Pin {
    /// A pin that expects nothing about an existing file. For app-owned files.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether this pin claims anything about the file that must already exist.
    fn expects_existing(&self) -> bool {
        self.identity.is_some() || self.fingerprint.is_some()
    }
}

/// Why [`replace`] wrote nothing.
#[derive(Debug, Error)]
pub enum WriteError {
    /// The file is no longer the one the caller pinned: a different inode, a
    /// symlink or other non-regular file, different content, or gone. Nothing
    /// was written; the caller should report a stale revision.
    #[error("the file changed on disk since it was read; refusing to overwrite")]
    Changed,
    /// An I/O failure. Nothing was renamed into place.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<WriteError> for std::io::Error {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::Io(error) => error,
            changed @ WriteError::Changed => std::io::Error::other(changed),
        }
    }
}

/// Read `path` **without following a symlink** at its final component, and
/// return its bytes with the [`Pin`] that describes them — identity and
/// fingerprint from the same open descriptor.
///
/// A symlink, a directory, a FIFO or a device is refused (`O_NONBLOCK`, so a
/// planted FIFO cannot hang the read).
///
/// # Errors
/// Any I/O error opening, `fstat`ing or reading the file; a non-regular file is
/// [`std::io::ErrorKind::InvalidInput`].
pub fn read_pinned(path: &Path) -> std::io::Result<(Vec<u8>, Pin)> {
    let (mut file, meta) = imp::open_existing(path)?.ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "the file does not exist")
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let pin = Pin {
        identity: FileIdentity::of(&meta),
        fingerprint: Some(Fingerprint::of_bytes(&bytes)),
    };
    Ok((bytes, pin))
}

/// [`read_pinned`], decoded as UTF-8. Non-UTF-8 content is
/// [`std::io::ErrorKind::InvalidData`], never a lossy decode.
///
/// # Errors
/// As [`read_pinned`], plus `InvalidData`.
pub fn read_pinned_string(path: &Path) -> std::io::Result<(String, Pin)> {
    let (bytes, pin) = read_pinned(path)?;
    let text = String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok((text, pin))
}

/// Atomically replace the file at `path` with `bytes`, if and only if it is
/// still the file `expected` describes. Returns the pin of what is now on disk,
/// so a caller writing the same file twice can pin the second write to the
/// first.
///
/// `path` is never followed at its final component: a symlink there is
/// [`WriteError::Changed`] when `expected` claims anything, and an I/O error
/// otherwise. A missing file is created `0600` when `expected` claims nothing,
/// and is [`WriteError::Changed`] when it does.
///
/// # Carried forward
/// Owner/group (best effort — an unprivileged process cannot give a file away)
/// and permission bits, from the `fstat` of the descriptor the identity check
/// used. Never from a path.
///
/// # Not carried forward
/// - **ACLs and extended attributes.** The new inode gets none.
/// - **Hard links.** `rename` rebinds only this name; a second hard link keeps
///   the OLD inode. Inherent to rename-based atomic writes; truncating in place
///   would preserve links but lets a crash leave a half-written journal.
/// - **Inode number, birth time, and open descriptors** on the old inode.
///
/// # Errors
/// [`WriteError::Changed`] or [`WriteError::Io`]; either way the original file
/// is untouched and no temp file is left behind.
pub fn replace(path: &Path, bytes: &[u8], expected: &Pin) -> Result<Pin, WriteError> {
    imp::replace(path, bytes, expected)
}

/// Create a **new** file at `path` holding `bytes`, owner-only, failing if
/// anything — a file, a directory, or a symlink, dangling or not — already
/// exists there. `O_CREAT | O_EXCL | O_NOFOLLOW`: the refusal is the kernel's,
/// decided at the open, rather than a check that expires before the write.
///
/// # Errors
/// [`std::io::ErrorKind::AlreadyExists`] if the name is taken, or any I/O error.
pub fn create_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    imp::create_new(path, bytes)
}

/// `path`'s parent directory (`.` for a bare name) and final component.
fn split(path: &Path) -> std::io::Result<(&Path, &std::ffi::OsStr)> {
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the path has no file name to write",
        )
    })?;
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok((dir, name))
}

/// A per-process-unique temp name for `name`.
fn temp_name(name: &std::ffi::OsStr) -> std::ffi::OsString {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut temp = std::ffi::OsString::from(".");
    temp.push(name);
    temp.push(format!(
        ".ledgeline-{}-{nanos}-{seq}.tmp",
        std::process::id()
    ));
    temp
}

/// The fingerprint of everything `file` holds, read from offset zero through the
/// descriptor — so it describes the inode the identity check opened, whatever
/// the name points at now.
fn fingerprint_of(file: &mut std::fs::File) -> std::io::Result<Fingerprint> {
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Fingerprint::of_bytes(&bytes))
}

#[cfg(unix)]
mod imp {
    use super::{
        FileIdentity, NEW_FILE_MODE, Pin, TEMP_NAME_ATTEMPTS, WriteError, fingerprint_of, split,
        temp_name,
    };
    use crate::edit::Fingerprint;
    use rustix::fs::{AtFlags, Mode, OFlags};
    use std::ffi::OsStr;
    use std::fs::{File, Metadata};
    use std::io::Write;
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;

    /// [`NEW_FILE_MODE`] as `rustix` spells it (`mode_t` is 16 bits on macOS).
    fn new_file_mode() -> Mode {
        Mode::from_bits_truncate(NEW_FILE_MODE as _)
    }

    /// Open `dir` as a directory descriptor every later step is relative to.
    pub(super) fn open_dir(dir: &Path) -> std::io::Result<OwnedFd> {
        Ok(rustix::fs::open(
            dir,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?)
    }

    /// Open `name` under `dir` read-only without following a symlink, and
    /// `fstat` it. `Ok(None)` when nothing is there.
    fn open_target(dir: &OwnedFd, name: &OsStr) -> std::io::Result<Option<(File, Metadata)>> {
        let fd = match rustix::fs::openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(rustix::io::Errno::LOOP) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "the path is a symbolic link; refusing to follow it",
                ));
            }
            Err(errno) => return Err(errno.into()),
        };
        let file = File::from(fd);
        let meta = file.metadata()?;
        if !meta.file_type().is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the path is not a regular file",
            ));
        }
        Ok(Some((file, meta)))
    }

    pub(super) fn open_existing(path: &Path) -> std::io::Result<Option<(File, Metadata)>> {
        let (dir, name) = split(path)?;
        open_target(&open_dir(dir)?, name)
    }

    pub(super) fn create_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let (dir, name) = split(path)?;
        let dir = open_dir(dir)?;
        let mut file = File::from(rustix::fs::openat(
            &dir,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            new_file_mode(),
        )?);
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        if let Err(error) = written {
            // We created it, and it holds a partial write: remove it so a retry
            // is not refused as "already exists".
            let _ = rustix::fs::unlinkat(&dir, name, AtFlags::empty());
            return Err(error);
        }
        let _ = rustix::fs::fsync(&dir);
        Ok(())
    }

    /// Create a fresh temp file in `dir`, retrying on a name collision.
    fn create_temp(dir: &OwnedFd, name: &OsStr) -> std::io::Result<(std::ffi::OsString, File)> {
        for _ in 0..TEMP_NAME_ATTEMPTS {
            let temp = temp_name(name);
            match rustix::fs::openat(
                dir,
                &temp,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                new_file_mode(),
            ) {
                Ok(fd) => return Ok((temp, File::from(fd))),
                Err(rustix::io::Errno::EXIST) => {}
                Err(errno) => return Err(errno.into()),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("could not create a temp file after {TEMP_NAME_ATTEMPTS} attempts"),
        ))
    }

    /// Whether `name` under `dir` still names the inode `existing` describes —
    /// or, when `existing` is `None`, still names nothing. Never follows a link.
    // `st_dev`/`st_ino` are `i32`/`u64` on macOS and `u64`/`u64` on 64-bit
    // Linux; widened the same way `std`'s `MetadataExt::{dev, ino}` widen them,
    // so the cast is a no-op on some targets and necessary on others.
    #[allow(clippy::unnecessary_cast)]
    pub(super) fn still_names(dir: &OwnedFd, name: &OsStr, existing: Option<&Metadata>) -> bool {
        match (
            rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW),
            existing,
        ) {
            (Err(rustix::io::Errno::NOENT), None) => true,
            (Ok(stat), Some(meta)) => {
                // `st_mode`'s type bits, compared directly: a regular file and
                // the same `(dev, ino)` the descriptor was opened on.
                FileIdentity::of(meta)
                    == Some(FileIdentity::from_raw((
                        stat.st_dev as u64,
                        stat.st_ino as u64,
                    )))
                    && rustix::fs::FileType::from_raw_mode(stat.st_mode as _)
                        == rustix::fs::FileType::RegularFile
            }
            _ => false,
        }
    }

    pub(super) fn replace(path: &Path, bytes: &[u8], expected: &Pin) -> Result<Pin, WriteError> {
        let (dir, name) = split(path)?;
        let dir = open_dir(dir)?;
        let target = match open_target(&dir, name) {
            Ok(target) => target,
            // A symlink or non-regular file where a pinned file was: changed.
            Err(error)
                if expected.expects_existing()
                    && error.kind() == std::io::ErrorKind::InvalidInput =>
            {
                return Err(WriteError::Changed);
            }
            Err(error) => return Err(error.into()),
        };
        match (&target, expected.expects_existing()) {
            (None, true) => return Err(WriteError::Changed),
            (Some((_, meta)), _)
                if expected
                    .identity
                    .is_some_and(|pinned| FileIdentity::of(meta) != Some(pinned)) =>
            {
                return Err(WriteError::Changed);
            }
            _ => {}
        }

        let (temp, mut file) = create_temp(&dir, name)?;
        let result = fill_and_install(&dir, name, &temp, &mut file, bytes, target, expected);
        if result.is_err() {
            let _ = rustix::fs::unlinkat(&dir, &temp, AtFlags::empty());
        }
        result
    }

    /// Steps 3–5 of the module docs, once the temp file exists. On any error the
    /// caller removes the temp file.
    fn fill_and_install(
        dir: &OwnedFd,
        name: &OsStr,
        temp: &OsStr,
        file: &mut File,
        bytes: &[u8],
        target: Option<(File, Metadata)>,
        expected: &Pin,
    ) -> Result<Pin, WriteError> {
        file.write_all(bytes)?;
        if let Some((_, meta)) = &target {
            carry_forward_metadata(file, meta);
        }
        file.sync_all()?;
        let written = Pin {
            identity: FileIdentity::of(&file.metadata()?),
            fingerprint: Some(Fingerprint::of_bytes(bytes)),
        };

        // Last look before the rename: the content through the descriptor the
        // identity check opened, then the name itself, unfollowed.
        let existing = match target {
            Some((mut handle, meta)) => {
                if let Some(pinned) = &expected.fingerprint
                    && !fingerprint_of(&mut handle)?.content_matches(pinned)
                {
                    return Err(WriteError::Changed);
                }
                Some(meta)
            }
            None => None,
        };
        if !still_names(dir, name, existing.as_ref()) {
            return Err(WriteError::Changed);
        }

        rustix::fs::renameat(dir, temp, dir, name).map_err(std::io::Error::from)?;
        // Best effort: durably record the rename in the directory.
        let _ = rustix::fs::fsync(dir);
        Ok(written)
    }

    /// Give the temp file the replaced file's ownership and permission bits so
    /// the rename does not relax them — from the `fstat` of the descriptor the
    /// identity check used, never from a path.
    ///
    /// Best effort by design: on failure the temp file keeps
    /// [`NEW_FILE_MODE`], which is *narrower* than any target it might replace.
    /// Ownership is set before the mode because `chown` clears the set-user-ID
    /// and set-group-ID bits.
    fn carry_forward_metadata(file: &File, existing: &Metadata) {
        let _ = std::os::unix::fs::fchown(file, Some(existing.uid()), Some(existing.gid()));
        let _ = file.set_permissions(std::fs::Permissions::from_mode(existing.mode() & 0o7777));
    }
}

/// Path-based fallback for platforms without `openat`. The same checks, made by
/// path rather than through descriptors, so they narrow the race rather than
/// close it.
#[cfg(not(unix))]
mod imp {
    use super::{
        FileIdentity, Pin, TEMP_NAME_ATTEMPTS, WriteError, fingerprint_of, split, temp_name,
    };
    use crate::edit::Fingerprint;
    use std::fs::{File, Metadata};
    use std::io::Write;
    use std::path::Path;

    pub(super) fn open_existing(path: &Path) -> std::io::Result<Option<(File, Metadata)>> {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
            Ok(meta) if !meta.file_type().is_file() => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the path is not a regular file",
            )),
            Ok(meta) => Ok(Some((File::open(path)?, meta))),
        }
    }

    pub(super) fn create_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    pub(super) fn replace(path: &Path, bytes: &[u8], expected: &Pin) -> Result<Pin, WriteError> {
        let target = match open_existing(path) {
            Ok(target) => target,
            Err(error)
                if expected.expects_existing()
                    && error.kind() == std::io::ErrorKind::InvalidInput =>
            {
                return Err(WriteError::Changed);
            }
            Err(error) => return Err(error.into()),
        };
        match target {
            None if expected.expects_existing() => return Err(WriteError::Changed),
            Some((mut handle, _)) => {
                if let Some(pinned) = &expected.fingerprint
                    && !fingerprint_of(&mut handle)?.content_matches(pinned)
                {
                    return Err(WriteError::Changed);
                }
            }
            None => {}
        }
        let (dir, name) = split(path)?;
        let (temp_path, mut file) = (0..TEMP_NAME_ATTEMPTS)
            .find_map(|_| {
                let temp_path = dir.join(temp_name(name));
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp_path)
                {
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    result => Some(result.map(|file| (temp_path, file))),
                }
            })
            .unwrap_or_else(|| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "could not create a temp file",
                ))
            })?;
        let result = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| {
                drop(file);
                std::fs::rename(&temp_path, path)
            });
        if let Err(error) = result {
            let _ = std::fs::remove_file(&temp_path);
            return Err(error.into());
        }
        Ok(Pin {
            identity: std::fs::symlink_metadata(path)
                .ok()
                .and_then(|meta| FileIdentity::of(&meta)),
            fingerprint: Some(Fingerprint::of_bytes(bytes)),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    /// A scratch directory removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "ledgeline-pinned-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
            std::fs::create_dir_all(&dir).expect("create scratch dir");
            Self(dir)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .expect("read scratch dir")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, text: &str, mode: u32) {
        std::fs::write(path, text).expect("write fixture");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).expect("lstat").mode() & 0o7777
    }

    #[test]
    fn a_pinned_replace_writes_and_returns_the_new_pin() {
        let scratch = Scratch::new("ok");
        let path = scratch.join("a.journal");
        write(&path, "old\n", 0o640);
        let (_, pin) = read_pinned(&path).expect("read");

        let written = replace(&path, b"new\n", &pin).expect("replace");

        assert_eq!(std::fs::read_to_string(&path).expect("read back"), "new\n");
        assert_eq!(mode_of(&path), 0o640, "mode carried from the fd's fstat");
        let (_, now) = read_pinned(&path).expect("re-read");
        assert_eq!(
            written.identity, now.identity,
            "the returned pin is the new inode"
        );
        assert!(
            written
                .fingerprint
                .as_ref()
                .is_some_and(|fp| fp.content_matches(now.fingerprint.as_ref().expect("fp")))
        );
        // And it pins a second write.
        replace(&path, b"newer\n", &written).expect("second replace pinned to the first");
        assert_eq!(
            scratch.entries(),
            vec!["a.journal".to_string()],
            "no temp left"
        );
    }

    #[test]
    fn owner_and_mode_come_from_the_descriptor_not_a_path() {
        let scratch = Scratch::new("owner");
        let path = scratch.join("a.journal");
        write(&path, "x\n", 0o604);
        let before = std::fs::metadata(&path).expect("stat");

        replace(&path, b"y\n", &Pin::none()).expect("replace");

        let after = std::fs::metadata(&path).expect("stat");
        assert_eq!(after.mode() & 0o7777, 0o604);
        assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
        assert_ne!(after.ino(), before.ino(), "a rename installs a new inode");
    }

    #[test]
    fn a_symlink_at_the_final_component_is_refused_and_not_followed() {
        let scratch = Scratch::new("link");
        let victim = scratch.join("victim.rc");
        let link = scratch.join("a.journal");
        write(&victim, "precious\n", 0o600);
        std::os::unix::fs::symlink(&victim, &link).expect("symlink");

        let unpinned = replace(&link, b"payload\n", &Pin::none());
        assert!(matches!(unpinned, Err(WriteError::Io(_))), "{unpinned:?}");
        let pinned = replace(
            &link,
            b"payload\n",
            &Pin {
                identity: None,
                fingerprint: Some(Fingerprint::of_bytes(b"precious\n")),
            },
        );
        assert!(matches!(pinned, Err(WriteError::Changed)), "{pinned:?}");
        assert!(read_pinned(&link).is_err(), "reads refuse the link too");

        assert_eq!(
            std::fs::read_to_string(&victim).expect("victim"),
            "precious\n"
        );
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("lstat")
                .file_type()
                .is_symlink(),
            "the link itself is not replaced either"
        );
        assert_eq!(
            scratch.entries(),
            vec!["a.journal".to_string(), "victim.rc".to_string()],
            "no temp left behind"
        );
    }

    #[test]
    fn a_link_swapped_in_after_the_read_is_refused() {
        let scratch = Scratch::new("swap");
        let path = scratch.join("a.journal");
        let victim = scratch.join("victim.rc");
        write(&path, "books\n", 0o600);
        write(&victim, "books\n", 0o600); // same bytes: only identity can tell
        let (_, pin) = read_pinned(&path).expect("read");

        std::fs::remove_file(&path).expect("rm");
        std::os::unix::fs::symlink(&victim, &path).expect("swap in a link");

        let result = replace(&path, b"payload\n", &pin);
        assert!(matches!(result, Err(WriteError::Changed)), "{result:?}");
        assert_eq!(std::fs::read_to_string(&victim).expect("victim"), "books\n");
    }

    #[test]
    fn a_different_inode_under_the_same_name_is_refused() {
        let scratch = Scratch::new("inode");
        let path = scratch.join("a.journal");
        write(&path, "books\n", 0o600);
        let (_, pin) = read_pinned(&path).expect("read");
        // Keep the old inode alive so its number cannot be reused.
        let hold = scratch.join("held");
        std::fs::rename(&path, &hold).expect("move aside");
        write(&path, "books\n", 0o600);

        let result = replace(&path, b"payload\n", &pin);
        assert!(matches!(result, Err(WriteError::Changed)), "{result:?}");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "books\n");
    }

    #[test]
    fn an_identity_change_between_open_and_rename_is_refused() {
        // Drive the internal last-look directly: the target was opened (its
        // metadata captured), then the name was re-pointed before the rename.
        let scratch = Scratch::new("late");
        let path = scratch.join("a.journal");
        write(&path, "books\n", 0o600);
        let dir = imp::open_dir(&scratch.0).expect("open dir");
        let meta = std::fs::symlink_metadata(&path).expect("lstat");
        assert!(imp::still_names(&dir, "a.journal".as_ref(), Some(&meta)));

        std::fs::rename(&path, scratch.join("held")).expect("move aside");
        std::os::unix::fs::symlink(scratch.join("held"), &path).expect("link");
        assert!(
            !imp::still_names(&dir, "a.journal".as_ref(), Some(&meta)),
            "a link to the very same inode is still not the pinned name"
        );
        std::fs::remove_file(&path).expect("rm link");
        assert!(!imp::still_names(&dir, "a.journal".as_ref(), Some(&meta)));
        assert!(imp::still_names(&dir, "a.journal".as_ref(), None));
    }

    #[test]
    fn content_changed_since_the_read_is_refused() {
        let scratch = Scratch::new("content");
        let path = scratch.join("a.journal");
        write(&path, "books\n", 0o600);
        let (_, pin) = read_pinned(&path).expect("read");
        // An in-place edit keeps the inode, so only the fingerprint can tell.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, b"user edit\n"))
            .expect("append in place");

        let result = replace(&path, b"payload\n", &pin);
        assert!(matches!(result, Err(WriteError::Changed)), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "books\nuser edit\n"
        );
        assert_eq!(scratch.entries(), vec!["a.journal".to_string()]);
    }

    #[test]
    fn a_pinned_file_that_vanished_is_changed_and_not_recreated() {
        let scratch = Scratch::new("gone");
        let path = scratch.join("a.journal");
        write(&path, "books\n", 0o600);
        let (_, pin) = read_pinned(&path).expect("read");
        std::fs::remove_file(&path).expect("rm");

        let result = replace(&path, b"payload\n", &pin);
        assert!(matches!(result, Err(WriteError::Changed)), "{result:?}");
        assert!(!path.exists());
    }

    #[test]
    fn an_unpinned_replace_creates_a_missing_file_owner_only() {
        let scratch = Scratch::new("fresh");
        let path = scratch.join("prefs.json");
        replace(&path, b"{}\n", &Pin::none()).expect("create");
        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "{}\n");
    }

    #[test]
    fn a_fifo_is_refused_without_hanging() {
        let scratch = Scratch::new("fifo");
        let path = scratch.join("a.journal");
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("run mkfifo");
        assert!(status.success(), "mkfifo failed");
        assert!(read_pinned(&path).is_err());
        assert!(replace(&path, b"x", &Pin::none()).is_err());
    }

    #[test]
    fn create_new_refuses_an_existing_file_and_any_symlink() {
        let scratch = Scratch::new("create");
        let victim = scratch.join("victim.rc");
        write(&victim, "precious\n", 0o600);
        let live = scratch.join("live.journal");
        std::os::unix::fs::symlink(&victim, &live).expect("live link");
        let dangling = scratch.join("dangling.journal");
        std::os::unix::fs::symlink(scratch.join("nowhere"), &dangling).expect("dangling link");

        for path in [&victim, &live, &dangling] {
            let error = create_new(path, b"payload\n").expect_err("must refuse");
            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{path:?}");
        }
        assert_eq!(
            std::fs::read_to_string(&victim).expect("victim"),
            "precious\n"
        );
        assert!(
            !scratch.join("nowhere").exists(),
            "a dangling link is not followed"
        );

        let fresh = scratch.join("fresh.journal");
        create_new(&fresh, b"hello\n").expect("create");
        assert_eq!(std::fs::read_to_string(&fresh).expect("read"), "hello\n");
        assert_eq!(mode_of(&fresh), 0o600);
    }
}
