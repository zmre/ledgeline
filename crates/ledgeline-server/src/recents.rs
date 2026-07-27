//! Recently-opened journals: a tiny persistent list of absolute journal paths so
//! the CLI can default to the last journal you used and the desktop GUI can offer
//! a File → "Open Recent" submenu.
//!
//! The list is stored as a JSON array of absolute path strings under the OS config
//! directory (`dirs::config_dir()/ledgeline/recent.json`), most-recent-first,
//! deduplicated on the *canonicalized* path and capped at [`MAX_RECENTS`]. The
//! store is intentionally forgiving: a missing file reads back as an empty list
//! and a write failure is logged but never fatal, so nothing here can break
//! startup. Setting `$LEDGELINE_CONFIG_DIR` overrides the directory (used by
//! tests and the server smoke test).
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

use std::path::{Path, PathBuf};

/// Maximum number of entries retained on disk (the GUI shows a shorter slice).
const MAX_RECENTS: usize = 10;
/// Env override for the directory that holds `recent.json` (tests + smoke tests).
const CONFIG_DIR_ENV: &str = "LEDGELINE_CONFIG_DIR";
/// Application subdirectory under the OS config dir.
const APP_DIR: &str = "ledgeline";
/// File name of the recents store within the config directory.
const RECENT_FILE: &str = "recent.json";

/// Record `path` as the most-recently-opened journal: canonicalize it, move it to
/// the front (deduplicating any prior spelling of the same file), and cap the
/// list. Best-effort — any I/O error is logged, never propagated.
pub(crate) fn record(path: impl AsRef<Path>) {
    if let Some(file) = recent_file() {
        record_in(&file, path.as_ref());
    }
}

/// The recently-opened journals that still exist on disk, most-recent-first.
pub(crate) fn list() -> Vec<PathBuf> {
    recent_file().map(|file| list_in(&file)).unwrap_or_default()
}

/// The most-recently-opened journal that is still a usable file, if any (the
/// CLI's default when no journal is given).
///
/// Filtering is lazy, so this stats only as far as the first usable entry rather
/// than all [`MAX_RECENTS`] of them — one `stat` on the startup path instead of
/// ten, which matters when a stale entry points at an unresponsive network mount.
pub(crate) fn most_recent() -> Option<PathBuf> {
    recent_file().and_then(|file| usable_entries(&file).next())
}

/// A concise, human-readable label for a recent-journal menu entry: the path with
/// the user's home directory collapsed to `~` when applicable.
#[cfg(feature = "gui")]
pub(crate) fn display_label(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

/// Directory holding `recent.json`: `$LEDGELINE_CONFIG_DIR` when set, else
/// `dirs::config_dir()/ledgeline`. `None` only if the platform exposes no config
/// dir and no override is set (recents are then silently disabled).
fn config_dir() -> Option<PathBuf> {
    match std::env::var_os(CONFIG_DIR_ENV) {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => dirs::config_dir().map(|base| base.join(APP_DIR)),
    }
}

/// Full path to the recents store file, if a config directory is available.
fn recent_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(RECENT_FILE))
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
    Entries(Vec<PathBuf>),
    /// Present, but unreadable or not a JSON array of paths. Distinguished from
    /// [`Missing`](Self::Missing) precisely so it is never silently overwritten.
    Corrupt,
}

/// Move `path` to the front of the store at `file`, deduped and capped.
///
/// If the existing store is unreadable it is moved aside rather than overwritten:
/// reading a corrupt file as "no history" and then writing a fresh one-entry list
/// over it would destroy a history we had merely failed to *parse*.
fn record_in(file: &Path, path: &Path) {
    let entry = canonicalize(path);
    let existing = match read_stored(file) {
        Stored::Entries(entries) => entries,
        Stored::Missing => Vec::new(),
        Stored::Corrupt => {
            set_corrupt_aside(file);
            Vec::new()
        }
    };
    let entries: Vec<PathBuf> = std::iter::once(entry.clone())
        .chain(existing.into_iter().filter(|old| old != &entry))
        .take(MAX_RECENTS)
        .collect();
    write_raw(file, &entries);
}

/// Read the store at `file`, keeping only entries that are usable journals.
fn list_in(file: &Path) -> Vec<PathBuf> {
    usable_entries(file).collect()
}

/// The stored entries that are usable journals, filtered **lazily** so a caller
/// that needs only the first ([`most_recent`]) does not stat the whole list.
fn usable_entries(file: &Path) -> impl Iterator<Item = PathBuf> {
    read_raw(file)
        .into_iter()
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
fn read_raw(file: &Path) -> Vec<PathBuf> {
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
    serde_json::from_str::<Vec<PathBuf>>(&text).map_or(Stored::Corrupt, Stored::Entries)
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
fn write_raw(file: &Path, entries: &[PathBuf]) {
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

        record_in(&store, &a);
        record_in(&store, &b);
        record_in(&store, &c);
        // Most-recent-first.
        assert_eq!(
            read_raw(&store),
            vec![canonicalize(&c), canonicalize(&b), canonicalize(&a)]
        );

        // Re-recording an older entry moves it to the front without duplicating.
        record_in(&store, &a);
        assert_eq!(
            read_raw(&store),
            vec![canonicalize(&a), canonicalize(&c), canonicalize(&b)]
        );

        // A different spelling of the same file dedupes to one canonical entry.
        record_in(&store, &dir.path().join("./b.journal"));
        let entries = read_raw(&store);
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
            record_in(&store, path);
        }

        let entries = read_raw(&store);
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

        write_raw(
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
        assert!(read_raw(&store).is_empty());
        // Corrupt / non-array content.
        fs::write(&store, "{ not valid json ]").expect("write garbage");
        assert!(read_raw(&store).is_empty());
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

        write_raw(&store, &[canonicalize(&subdir), canonicalize(&journal)]);

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

        record_in(&store, &journal);

        let aside = store.with_extension("json.corrupt");
        assert_eq!(
            fs::read_to_string(&aside).expect("the unreadable bytes are kept"),
            "{ not valid json ]"
        );
        assert_eq!(
            read_raw(&store),
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

        record_in(&store, &journal);

        assert!(!store.with_extension("json.corrupt").exists());
        assert_eq!(read_raw(&store), vec![canonicalize(&journal)]);
    }

    /// `write_raw` goes through the hardened atomic write, so the store lands
    /// complete and owner-only, and no temp file is left in the config dir.
    #[test]
    fn the_store_is_written_atomically_and_owner_only() {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("recent.json");
        let journal = touch(dir.path(), "j.journal");

        record_in(&store, &journal);

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
        record_in(&store, &journal);
        assert_eq!(read_raw(&store), vec![canonicalize(&journal)]);
    }
}
