//! Recently-opened journals: a tiny persistent list so the CLI can default to
//! the last journal you used and the desktop GUI can offer a File → "Open
//! Recent" submenu.
//!
//! # What an entry is: an opaque [`RecentLocation`]
//!
//! The store holds TOKENS, not paths. On the desktop a token is the journal's
//! canonical absolute path, and the path-shaped helpers here ([`record`],
//! [`list`], [`most_recent`]) read and write it that way. A sandboxed host
//! cannot reopen a file from a path at all — iOS hands back access only through
//! a security-scoped bookmark — so there a token is whatever id the host keeps
//! its bookmark under, written and read with [`record_location`] /
//! [`locations`] and resolved by the host itself.
//!
//! The file format did not change to make room for that: it is still a JSON
//! array of strings, most-recent-first, so a `recent.json` written before
//! tokens existed (an array of path strings) loads unchanged, and one written
//! now loads in an older build.
//!
//! The list lives in `recent.json` under the injected
//! [`AppDirs::config_dir`] (on the desktop `$LEDGELINE_CONFIG_DIR` or
//! `dirs::config_dir()/ledgeline`), deduplicated on the token (for a path, the
//! *canonicalized* path) and capped at [`MAX_RECENTS`]. The store is
//! intentionally forgiving: a missing file reads back as an empty list and a
//! write failure is logged but never fatal, so nothing here can break startup.
//!
//! # Why this file is security-relevant
//! [`most_recent`] chooses the journal `ledgeline` opens — and then *writes* —
//! when invoked with no arguments, so whatever can write `recent.json` chooses
//! that file. Three properties follow:
//!
//! - Entries are checked to be **regular files** ([`is_regular_file`]), not
//!   merely to exist. `exists()` is also true for a directory, a device node or a
//!   FIFO, and a FIFO planted here would hang the app at startup instead of
//!   failing cleanly.
//! - The store is written **atomically and owner-only** via
//!   [`ledgeline_core::edit::atomic_write`], so a crash mid-write cannot leave a
//!   truncated `recent.json`, and a fresh store is not group- or world-writable.
//! - A store that is present but **unreadable is moved aside, never silently
//!   replaced** ([`record_in`]), so a parse failure cannot erase the user's
//!   history beyond recovery.

use crate::app_dirs::AppDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Maximum number of entries retained on disk (the GUI shows a shorter slice).
pub const MAX_RECENTS: usize = 10;
/// File name of the recents store within the config directory.
const RECENT_FILE: &str = "recent.json";

/// One remembered journal, as an opaque token.
///
/// On the desktop the token is a canonical absolute path
/// ([`from_path`](Self::from_path) / [`to_path`](Self::to_path)); on iOS it is
/// the id of a security-scoped bookmark the host stores and resolves. The store
/// never interprets it beyond equality. Serialized as a bare JSON string, which
/// is what keeps the file format identical to the path-only one before it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecentLocation(String);

impl RecentLocation {
    /// Wrap a host-chosen token (a bookmark id) as-is.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The desktop token for `path`: canonicalized to an absolute path for a
    /// stable dedup key, falling back to a plain absolute path (then the input)
    /// if the file cannot be resolved.
    ///
    /// `None` for a path that is not valid UTF-8, which a JSON string cannot
    /// hold. (Before tokens such a path made the whole store fail to
    /// serialize; now it is simply not remembered.)
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        canonicalize(path)
            .into_os_string()
            .into_string()
            .ok()
            .map(Self)
    }

    /// The token as stored.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The desktop reading of the token: the path it names. Meaningless for a
    /// bookmark id, which a host resolves itself.
    #[must_use]
    pub fn to_path(&self) -> PathBuf {
        PathBuf::from(&self.0)
    }
}

/// Record `location` as the most-recently-opened journal: move it to the front
/// (deduplicating an earlier copy) and cap the list. Best-effort — any I/O
/// error is logged, never propagated.
pub fn record_location(dirs: &AppDirs, location: RecentLocation) {
    if let Some(file) = dirs.config_file(RECENT_FILE) {
        record_in(&file, location);
    }
}

/// Every stored token, most-recent-first, unfiltered: a host whose tokens are
/// not paths decides for itself which ones still resolve.
#[must_use]
pub fn locations(dirs: &AppDirs) -> Vec<RecentLocation> {
    dirs.config_file(RECENT_FILE)
        .map(|file| read_raw(&file))
        .unwrap_or_default()
}

/// Record the journal at `path` (the desktop form of [`record_location`]):
/// canonicalize it so another spelling of the same file dedupes to one entry.
pub fn record(dirs: &AppDirs, path: impl AsRef<Path>) {
    match RecentLocation::from_path(path.as_ref()) {
        Some(location) => record_location(dirs, location),
        None => eprintln!(
            "ledgeline: not remembering {} in recents: the path is not valid UTF-8",
            path.as_ref().display()
        ),
    }
}

/// The recently-opened journals that still exist on disk, most-recent-first.
#[must_use]
pub fn list(dirs: &AppDirs) -> Vec<PathBuf> {
    dirs.config_file(RECENT_FILE)
        .map(|file| list_in(&file))
        .unwrap_or_default()
}

/// The most-recently-opened journal that is still a usable file, if any (the
/// CLI's default when no journal is given).
///
/// Filtering is lazy, so this stats only as far as the first usable entry rather
/// than all [`MAX_RECENTS`] of them — one `stat` on the startup path instead of
/// ten, which matters when a stale entry points at an unresponsive network mount.
#[must_use]
pub fn most_recent(dirs: &AppDirs) -> Option<PathBuf> {
    dirs.config_file(RECENT_FILE)
        .and_then(|file| usable_entries(&file).next())
}

/// A concise, human-readable label for a recent-journal menu entry: the path with
/// the user's home directory collapsed to `~` when applicable.
///
/// The home directory is read here for DISPLAY only — it decides no location,
/// so it is not part of [`AppDirs`].
#[cfg(feature = "gui")]
#[must_use]
pub fn display_label(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

/// Canonicalize to an absolute path for stable dedup keys, falling back to a
/// plain absolute path (then the input) if the file cannot be resolved.
fn canonicalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// What the store at a given path currently holds.
enum Stored {
    /// No file there — an empty list, and nothing to preserve.
    Missing,
    /// A well-formed list (possibly empty).
    Entries(Vec<RecentLocation>),
    /// Present, but unreadable or not a JSON array of strings. Distinguished
    /// from [`Missing`](Self::Missing) precisely so it is never silently
    /// overwritten.
    Corrupt,
}

/// Move `entry` to the front of the store at `file`, deduped and capped.
///
/// If the existing store is unreadable it is moved aside rather than overwritten:
/// reading a corrupt file as "no history" and then writing a fresh one-entry list
/// over it would destroy a history we had merely failed to *parse*.
fn record_in(file: &Path, entry: RecentLocation) {
    let existing = match read_stored(file) {
        Stored::Entries(entries) => entries,
        Stored::Missing => Vec::new(),
        Stored::Corrupt => {
            set_corrupt_aside(file);
            Vec::new()
        }
    };
    let entries: Vec<RecentLocation> = std::iter::once(entry.clone())
        .chain(existing.into_iter().filter(|old| old != &entry))
        .take(MAX_RECENTS)
        .collect();
    write_raw(file, &entries);
}

/// Read the store at `file`, keeping only entries that are usable journals.
fn list_in(file: &Path) -> Vec<PathBuf> {
    usable_entries(file).collect()
}

/// The stored entries, read as paths, that are usable journals — filtered
/// **lazily** so a caller that needs only the first ([`most_recent`]) does not
/// stat the whole list.
fn usable_entries(file: &Path) -> impl Iterator<Item = PathBuf> {
    read_raw(file)
        .into_iter()
        .map(|location| location.to_path())
        .filter(|path| is_regular_file(path))
}

/// Whether `path` is something that can actually be opened as a journal.
///
/// `Path::exists` was not enough: it is equally true for a directory, a device
/// node, a unix socket, or a FIFO — and since [`most_recent`] picks the file the
/// app opens with no arguments, a FIFO here would block startup forever rather
/// than fail. Symlinks are followed, so a symlinked journal still qualifies.
///
/// Known residual cost, unchanged by this fix: this is a blocking `stat`, so an
/// entry on an unresponsive network mount can still stall startup. Bounding that
/// needs a watchdog thread, which is more machinery than a recents list warrants;
/// [`most_recent`]'s laziness reduces the exposure to the first entry only.
fn is_regular_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

/// Read the raw stored list, treating anything unreadable as empty. Callers that
/// need to tell "missing" from "corrupt" use [`read_stored`].
fn read_raw(file: &Path) -> Vec<RecentLocation> {
    match read_stored(file) {
        Stored::Entries(entries) => entries,
        Stored::Missing | Stored::Corrupt => Vec::new(),
    }
}

/// Classify the store at `file`.
fn read_stored(file: &Path) -> Stored {
    let Ok(text) = std::fs::read_to_string(file) else {
        // Absent, unreadable, or not UTF-8. Only a genuine absence is safe to
        // treat as "no history"; anything else is content we failed to read.
        return if file.exists() {
            Stored::Corrupt
        } else {
            Stored::Missing
        };
    };
    serde_json::from_str::<Vec<RecentLocation>>(&text).map_or(Stored::Corrupt, Stored::Entries)
}

/// Rename an unreadable store to `recent.json.corrupt` so its bytes survive for
/// recovery and the next write starts from a clean file.
fn set_corrupt_aside(file: &Path) {
    let aside = file.with_extension("json.corrupt");
    match std::fs::rename(file, &aside) {
        Ok(()) => eprintln!(
            "ledgeline: recents file {} was unreadable; kept a copy at {} and started a new list",
            file.display(),
            aside.display()
        ),
        Err(error) => eprintln!(
            "ledgeline: could not set aside unreadable recents {}: {error}",
            file.display()
        ),
    }
}

/// Persist `entries` as pretty JSON, creating the config directory if needed.
///
/// Written through [`ledgeline_core::edit::atomic_write`] (temp file, `fsync`,
/// `rename`) rather than `fs::write`, so a crash or a full disk mid-write leaves
/// the previous store intact instead of a truncated file that would read back as
/// corrupt. A store created here is owner-only. Any failure is logged and
/// swallowed so a write error can never abort the app.
fn write_raw(file: &Path, entries: &[RecentLocation]) {
    if let Some(parent) = file.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        eprintln!(
            "ledgeline: could not create config dir {}: {error}",
            parent.display()
        );
        return;
    }
    match serde_json::to_string_pretty(entries) {
        Ok(json) => {
            if let Err(error) = ledgeline_core::edit::atomic_write(file, json.as_bytes()) {
                eprintln!(
                    "ledgeline: could not write recents {}: {error}",
                    file.display()
                );
            }
        }
        Err(error) => eprintln!("ledgeline: could not serialize recents: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// The desktop token for `path`.
    fn loc(path: &Path) -> RecentLocation {
        RecentLocation::from_path(path).expect("UTF-8 test path")
    }

    /// The stored tokens, read back as paths.
    fn raw_paths(store: &Path) -> Vec<PathBuf> {
        read_raw(store)
            .iter()
            .map(RecentLocation::to_path)
            .collect()
    }

    /// Write raw path entries, as an older build (or a hand edit) would.
    fn write_paths(store: &Path, paths: &[PathBuf]) {
        let entries: Vec<RecentLocation> = paths
            .iter()
            .map(|p| RecentLocation::new(p.to_str().expect("UTF-8")))
            .collect();
        write_raw(store, &entries);
    }

    /// Create an empty file at `dir/name` and return its path (so `canonicalize`
    /// during `record_in` resolves an existing file).
    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, "").expect("write temp journal");
        path
    }

    #[test]
    fn record_moves_to_front_and_dedupes() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let a = touch(dir.path(), "a.journal");
        let b = touch(dir.path(), "b.journal");
        let c = touch(dir.path(), "c.journal");

        record_in(&store, loc(&a));
        record_in(&store, loc(&b));
        record_in(&store, loc(&c));
        // Most-recent-first.
        assert_eq!(
            raw_paths(&store),
            vec![canonicalize(&c), canonicalize(&b), canonicalize(&a)]
        );

        // Re-recording an older entry moves it to the front without duplicating.
        record_in(&store, loc(&a));
        assert_eq!(
            raw_paths(&store),
            vec![canonicalize(&a), canonicalize(&c), canonicalize(&b)]
        );

        // A different spelling of the same file dedupes to one canonical entry.
        record_in(&store, loc(&dir.path().join("./b.journal")));
        let entries = raw_paths(&store);
        assert_eq!(entries.first(), Some(&canonicalize(&b)));
        assert_eq!(
            entries.iter().filter(|p| **p == canonicalize(&b)).count(),
            1,
            "the same file must appear only once"
        );
    }

    #[test]
    fn record_caps_at_max() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let total = MAX_RECENTS + 3;
        let paths: Vec<PathBuf> = (0..total)
            .map(|i| touch(dir.path(), &format!("j{i}.journal")))
            .collect();
        for path in &paths {
            record_in(&store, loc(path));
        }

        let entries = raw_paths(&store);
        assert_eq!(entries.len(), MAX_RECENTS, "capped at MAX_RECENTS");
        // The last-recorded is at the front; the oldest few were dropped.
        assert_eq!(entries.first(), Some(&canonicalize(paths.last().unwrap())));
        assert!(
            !entries.contains(&canonicalize(&paths[0])),
            "the oldest entry is evicted once the cap is exceeded"
        );
    }

    #[test]
    fn list_skips_missing_paths() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let present = touch(dir.path(), "present.journal");
        let missing = dir.path().join("gone.journal"); // never created

        write_paths(
            &store,
            &[canonicalize(&present), missing, canonicalize(&present)],
        );
        // `list_in` keeps only entries that still exist on disk.
        assert_eq!(list_in(&store), vec![canonicalize(&present); 2]);
    }

    #[test]
    fn missing_or_corrupt_store_reads_empty() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        // Missing file.
        assert!(raw_paths(&store).is_empty());
        // Corrupt / non-array content.
        fs::write(&store, "{ not valid json ]").expect("write garbage");
        assert!(raw_paths(&store).is_empty());
    }

    /// SEC-10 / DL-6: entries must be regular FILES. `exists()` is also true for
    /// a directory, and for a FIFO — which `most_recent()` would hand to the
    /// journal reader, hanging startup instead of failing.
    #[test]
    fn list_skips_entries_that_are_not_regular_files() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let journal = touch(dir.path(), "real.journal");
        let subdir = dir.path().join("a-directory");
        fs::create_dir(&subdir).expect("create dir");

        write_paths(&store, &[canonicalize(&subdir), canonicalize(&journal)]);

        assert!(subdir.exists(), "the decoy really is present on disk");
        assert_eq!(
            list_in(&store),
            vec![canonicalize(&journal)],
            "a directory must not be offered as a recent journal"
        );
        assert_eq!(
            usable_entries(&store).next(),
            Some(canonicalize(&journal)),
            "most_recent's lazy filter must skip past it too"
        );
    }

    /// The corrupt-store path must not erase history beyond recovery: the bytes
    /// are moved aside so they can be recovered by hand.
    #[test]
    fn a_corrupt_store_is_preserved_rather_than_erased() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let journal = touch(dir.path(), "j.journal");
        fs::write(&store, "{ not valid json ]").expect("write garbage");

        record_in(&store, loc(&journal));

        let aside = store.with_extension("json.corrupt");
        assert_eq!(
            fs::read_to_string(&aside).expect("the unreadable bytes are kept"),
            "{ not valid json ]"
        );
        assert_eq!(
            raw_paths(&store),
            vec![canonicalize(&journal)],
            "and the store recovers rather than staying broken"
        );
    }

    /// An absent store is NOT corrupt — recording into a fresh config dir must
    /// not leave a stray `.corrupt` file behind.
    #[test]
    fn a_missing_store_is_not_treated_as_corrupt() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let journal = touch(dir.path(), "j.journal");

        record_in(&store, loc(&journal));

        assert!(!store.with_extension("json.corrupt").exists());
        assert_eq!(raw_paths(&store), vec![canonicalize(&journal)]);
    }

    /// `write_raw` goes through the hardened atomic write, so the store lands
    /// complete and owner-only, and no temp file is left in the config dir.
    #[test]
    fn the_store_is_written_atomically_and_owner_only() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let journal = touch(dir.path(), "j.journal");

        record_in(&store, loc(&journal));

        let leftovers: Vec<PathBuf> = fs::read_dir(dir.path())
            .expect("read config dir")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.to_string_lossy().contains(".ledgeline-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&store).expect("stat").permissions().mode() & 0o777;
            assert_eq!(
                mode & !0o600,
                0,
                "a freshly created recents store must not be wider than 0600, got {mode:o}"
            );
        }
    }

    #[test]
    fn record_creates_missing_config_dir() {
        let dir = TempDir::new().expect("temp dir");
        // Nested, not-yet-existing directory: `write_raw` must create it.
        let store = dir.path().join("nested").join("deeper").join("recent.json");
        let journal = touch(dir.path(), "j.journal");
        record_in(&store, loc(&journal));
        assert_eq!(raw_paths(&store), vec![canonicalize(&journal)]);
    }

    /// A `recent.json` written before tokens existed — an array of absolute
    /// path strings — must load unchanged, and a store written now must still
    /// be that shape so an older build can read it.
    #[test]
    fn the_file_format_is_the_path_array_it_always_was() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let journal = touch(dir.path(), "old.journal");
        let canonical = canonicalize(&journal);
        let legacy = serde_json::to_string_pretty(&vec![canonical.clone()]).expect("json");
        fs::write(&store, &legacy).expect("write legacy store");

        assert_eq!(list_in(&store), vec![canonical.clone()]);

        let other = touch(dir.path(), "new.journal");
        record_in(&store, loc(&other));
        let reread: Vec<PathBuf> =
            serde_json::from_str(&fs::read_to_string(&store).expect("read")).expect("old shape");
        assert_eq!(reread, vec![canonicalize(&other), canonical]);
    }

    /// A host token that is not a path (an iOS bookmark id) round-trips and
    /// dedupes like any other, and is never offered as a desktop path.
    #[test]
    fn an_opaque_token_round_trips_and_is_not_a_usable_path() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let bookmark = RecentLocation::new("bookmark:6F1C-77A2");
        record_in(&store, bookmark.clone());
        record_in(&store, RecentLocation::new("bookmark:other"));
        record_in(&store, bookmark.clone());

        assert_eq!(
            read_raw(&store),
            vec![bookmark, RecentLocation::new("bookmark:other")]
        );
        assert!(list_in(&store).is_empty(), "a bookmark id is not a file");
    }

    /// The public entry points read the injected config dir, not the
    /// environment.
    #[test]
    fn the_store_lives_in_the_injected_config_dir() {
        let dir = TempDir::new().expect("temp dir");
        let dirs = AppDirs::under(dir.path());
        let journal = touch(dir.path(), "j.journal");
        record(&dirs, &journal);
        assert!(dir.path().join("config").join("recent.json").is_file());
        assert_eq!(most_recent(&dirs), Some(canonicalize(&journal)));
        assert_eq!(locations(&dirs), vec![loc(&journal)]);

        let no_config = AppDirs {
            config_dir: None,
            ..AppDirs::under(dir.path())
        };
        record(&no_config, &journal);
        assert!(list(&no_config).is_empty());
    }
}
