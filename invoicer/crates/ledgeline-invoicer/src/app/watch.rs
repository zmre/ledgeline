//! The watcher (ARCHITECTURE §5.4): the books folder, recursively (its
//! markdown and its journal files), plus the folder of any journal file
//! outside it; every settled burst of changes goes to
//! [`OpenBooks::apply_changes`], which re-reads what changed and runs the
//! reconciler over the invoices it touched.
//!
//! Debounced as Ledgeline's is: one editor save is several events, so the
//! loop waits for the filesystem to be quiet for [`DEBOUNCE`] before acting.
//! The reconciler's own `paid:` writes come back as events too; the index
//! sees their content unchanged, so they end there.
//!
//! MBR's server has its own watcher for its pages and index; this one is the
//! invoicer's books only.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::APP;
use super::books::OpenBooks;

/// How long the filesystem must be quiet before a burst is acted on.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Start watching `books`. Dropping the watcher stops it (and its thread).
///
/// # Errors
/// When the books folder can't be watched; the app then runs without live
/// reconciling.
pub(crate) fn spawn(books: Arc<OpenBooks>) -> notify::Result<RecommendedWatcher> {
    let root = books.vault().root().to_path_buf();
    let (sender, receiver) = mpsc::channel::<Vec<PathBuf>>();
    let filter_root = root.clone();
    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            Ok(event) if !changes_files(&event.kind) => {}
            Ok(event) => {
                let paths: Vec<PathBuf> = event
                    .paths
                    .into_iter()
                    .filter(|p| !hidden_under(&filter_root, p))
                    .collect();
                if !paths.is_empty() {
                    // A closed receiver means the loop is gone (shutdown).
                    let _ = sender.send(paths);
                }
            }
            Err(error) => eprintln!("{}: watch error: {error}", APP.log_prefix),
        })?;
    watcher.watch(&root, RecursiveMode::Recursive)?;
    let outside: BTreeSet<PathBuf> = books
        .session()
        .journal_files()
        .iter()
        .filter(|f| !f.starts_with(&root))
        .filter_map(|f| f.parent().map(Path::to_path_buf))
        .collect();
    for dir in outside {
        if let Err(error) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
            eprintln!(
                "{}: not watching {}: {error}",
                APP.log_prefix,
                dir.display()
            );
        }
    }
    std::thread::Builder::new()
        .name("invoicer-watch".to_string())
        .spawn(move || watch_loop(&receiver, &books))
        .map_err(notify::Error::io)?;
    Ok(watcher)
}

/// Coalesce events and act once per settled burst, until the watcher goes.
fn watch_loop(events: &mpsc::Receiver<Vec<PathBuf>>, books: &OpenBooks) {
    while let Ok(first) = events.recv() {
        let mut changed: BTreeSet<PathBuf> = first.into_iter().collect();
        while let Ok(more) = events.recv_timeout(DEBOUNCE) {
            changed.extend(more);
        }
        let changed: Vec<PathBuf> = changed.into_iter().collect();
        books.apply_changes(&changed);
    }
}

/// Whether an event can mean a file changed. Linux's inotify also reports
/// reads (open, close-after-read); every page MBR serves and every check
/// reads the books, and a reader polling faster than [`DEBOUNCE`] would keep
/// a burst from ever settling. A close after writing still counts: it is
/// the end of a save.
fn changes_files(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

/// Whether `path` is inside a hidden folder (or is a hidden file) under
/// `root`: `.git`, `.mbr`, the invoicer's own `.ledgeline-invoicer/`.
/// Discovery skips them too.
fn hidden_under(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|rel| {
        rel.components().any(|c| match c {
            Component::Normal(name) => name.to_string_lossy().starts_with('.'),
            _ => false,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{changes_files, hidden_under};
    use notify::EventKind;
    use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind};
    use std::path::Path;

    /// Reads don't count (inotify reports them; a poller would starve the
    /// debounce); writes, creates, renames, removals and a close after a
    /// write do.
    #[test]
    fn only_events_that_can_change_a_file_count() {
        assert!(!changes_files(&EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!changes_files(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(!changes_files(&EventKind::Access(AccessKind::Read)));
        assert!(changes_files(&EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));
        assert!(changes_files(&EventKind::Modify(ModifyKind::Data(
            DataChange::Any
        ))));
        assert!(changes_files(&EventKind::Create(CreateKind::File)));
        assert!(changes_files(&EventKind::Any));
    }

    #[test]
    fn hidden_folders_under_the_root_are_skipped() {
        let root = Path::new("/books");
        assert!(hidden_under(root, Path::new("/books/.git/index")));
        assert!(hidden_under(
            root,
            Path::new("/books/.ledgeline-invoicer/undo/x.json")
        ));
        assert!(hidden_under(root, Path::new("/books/invoices/.x.md.swp")));
        assert!(!hidden_under(root, Path::new("/books/invoices/105.md")));
        assert!(!hidden_under(root, Path::new("/books/books.journal")));
        // Outside the root (an included journal elsewhere): never hidden.
        assert!(!hidden_under(root, Path::new("/other/.hidden/x.journal")));
    }
}
