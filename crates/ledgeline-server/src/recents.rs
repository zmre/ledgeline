//! Recently-opened journals: a tiny persistent list of absolute journal paths so
//! the CLI can default to the last journal you used and the desktop GUI can offer
//! a File → "Open Recent" submenu.
//!
//! The list is stored as a JSON array of absolute path strings under the OS config
//! directory (`dirs::config_dir()/ledgeline/recent.json`), most-recent-first,
//! deduplicated on the *canonicalized* path and capped at [`MAX_RECENTS`]. The
//! store is intentionally forgiving: a missing or corrupt file reads back as an
//! empty list and a write failure is logged but never fatal, so nothing here can
//! break startup. Setting `$LEDGELINE_CONFIG_DIR` overrides the directory (used by
//! tests and the server smoke test).

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

/// The most-recently-opened journal that still exists, if any (the CLI's default
/// when no journal is given).
pub(crate) fn most_recent() -> Option<PathBuf> {
    list().into_iter().next()
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

/// Move `path` to the front of the store at `file`, deduped and capped.
fn record_in(file: &Path, path: &Path) {
    let entry = canonicalize(path);
    let mut entries = read_raw(file);
    entries.retain(|existing| existing != &entry);
    entries.insert(0, entry);
    entries.truncate(MAX_RECENTS);
    write_raw(file, &entries);
}

/// Read the store at `file`, keeping only entries that still exist on disk.
fn list_in(file: &Path) -> Vec<PathBuf> {
    read_raw(file)
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

/// Read the raw stored list. A missing or corrupt file (or one that is not a JSON
/// array of strings) reads back as an empty list — never an error.
fn read_raw(file: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<PathBuf>>(&text).ok())
        .unwrap_or_default()
}

/// Persist `entries` as pretty JSON, creating the config directory if needed. Any
/// failure is logged and swallowed so a write error can never abort the app.
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
            if let Err(error) = std::fs::write(file, json) {
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
