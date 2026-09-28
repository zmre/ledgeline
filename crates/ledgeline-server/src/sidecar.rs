//! Caches kept in a file BESIDE the journal — `benchmarks.prices.journal`
//! (`benchmarks_api`) and `commodity-profiles.json` (`profiles_api`) — plus the
//! request plumbing the two Yahoo-backed endpoints that own them share.
//!
//! # The rules, in one place
//!
//! - **Memory first.** The cache lives in [`AppState`] and is read from disk
//!   ONCE, on first use, then again only when the journal moves (a desktop
//!   File→Open), so a cache hit neither rereads nor reparses the file. A reader
//!   gets an `Arc` snapshot: nothing is cloned to answer it.
//! - **Write-through, copy-on-write.** [`SidecarCache::update`] changes the
//!   in-memory copy (cloning it only if a reader still holds the old snapshot)
//!   and, when this server may write at all ([`AppState::editing_enabled`]),
//!   writes the whole file atomically. A read-only session keeps its cache in
//!   memory for the life of the session and never writes beside the journal.
//! - **Short locks.** The lock is held only to load, read or merge — never
//!   across a network fetch. Callers fetch outside it and then merge, so a cache
//!   hit is never queued behind someone else's slow Yahoo request. Two requests
//!   that miss at once may both fetch; the second merge simply wins.
//! - **Disposable.** A missing, unreadable or unparseable file is an empty
//!   cache, a failed write is logged and not raised: the next request refetches.
//!
//! The file is never `include`d, never passed to the git safety net (which only
//! stages paths an import names), and sits outside the journal's include tree,
//! so the live-reload watcher ignores it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ledgeline_core::edit::atomic_write;
use ledgeline_core::model::Journal;

use crate::AppState;
use crate::error::AppError;
use crate::reports_api::compute;

/// Simultaneous Yahoo requests one sidecar endpoint makes: a cold cache is a
/// one-off, and Yahoo rate-limits a burst.
pub(crate) const FETCH_CONCURRENCY: usize = 4;

/// The longest symbol a sidecar endpoint accepts.
const MAX_SYMBOL_BYTES: usize = 128;

/// `symbols=` split on commas, trimmed, blanks dropped and de-duplicated in
/// order. More than `max` symbols, or one longer than any real symbol, is a
/// `400`; none at all is an empty list, which the caller judges.
pub(crate) fn parse_symbol_list(raw: Option<&str>, max: usize) -> Result<Vec<&str>, AppError> {
    let mut symbols: Vec<&str> = Vec::new();
    for symbol in raw.unwrap_or_default().split(',').map(str::trim) {
        if !symbol.is_empty() && !symbols.contains(&symbol) {
            symbols.push(symbol);
        }
    }
    if symbols.len() > max {
        return Err(AppError::BadRequest(format!(
            "at most {max} symbols may be requested at once"
        )));
    }
    if let Some(long) = symbols.iter().find(|s| s.len() > MAX_SYMBOL_BYTES) {
        return Err(AppError::BadRequest(format!(
            "commodity symbol is too long ({} bytes)",
            long.len()
        )));
    }
    Ok(symbols)
}

/// What a sidecar file holds, and how it reads and writes.
pub(crate) trait Sidecar: Default + Clone + Send + Sync + 'static {
    /// The file's name, beside the main journal.
    const FILE: &'static str;

    /// The file's contents as a cache. Anything unreadable is an empty cache
    /// (it is only a cache, and the next write replaces it), so this cannot
    /// fail; say why in the log if it discards something.
    fn decode(bytes: &[u8]) -> Self;

    /// The cache as the file's contents.
    ///
    /// # Errors
    /// Only if the value cannot be serialized; the write is then skipped.
    fn encode(&self) -> std::io::Result<Vec<u8>>;
}

/// The loaded cache and the file it mirrors.
#[derive(Default)]
struct Slot<T> {
    /// `None` until first loaded; then the path `value` was read from, itself
    /// `None` for a journal with no file behind it.
    loaded_from: Option<Option<PathBuf>>,
    value: Arc<T>,
}

/// One sidecar cache, shared by every clone of an [`AppState`].
pub(crate) struct SidecarCache<T> {
    /// A `tokio` mutex because the first load, and a write-through, run on the
    /// blocking pool under it.
    slot: tokio::sync::Mutex<Slot<T>>,
}

impl<T: Sidecar> Default for SidecarCache<T> {
    fn default() -> Self {
        Self {
            slot: tokio::sync::Mutex::new(Slot::default()),
        }
    }
}

/// `<journal dir>/<name>`, or `None` for a journal with no file behind it.
fn sidecar_path(journal: &Journal, name: &str) -> Option<PathBuf> {
    journal
        .source_files
        .first()
        .and_then(|main| main.parent())
        .map(|dir| dir.join(name))
}

impl<T: Sidecar> SidecarCache<T> {
    /// The cache as it stands for `state`'s current journal.
    ///
    /// # Errors
    /// Only if the blocking pool is shutting down.
    pub(crate) async fn snapshot(&self, state: &AppState) -> Result<Arc<T>, AppError> {
        let path = sidecar_path(&state.snapshot().journal, T::FILE);
        let mut slot = self.slot.lock().await;
        Self::current(&mut slot, path).await?;
        Ok(Arc::clone(&slot.value))
    }

    /// Change the cache with `merge` and, when this session may write, store
    /// it; returns the cache as merged.
    ///
    /// `merge` runs under the lock on the CURRENT cache — which a concurrent
    /// request may have extended since this one took its snapshot — so it
    /// should fold its news in rather than replace the whole.
    ///
    /// # Errors
    /// Only if the blocking pool is shutting down.
    pub(crate) async fn update(
        &self,
        state: &AppState,
        merge: impl FnOnce(&mut T),
    ) -> Result<Arc<T>, AppError> {
        let path = sidecar_path(&state.snapshot().journal, T::FILE);
        let mut slot = self.slot.lock().await;
        Self::current(&mut slot, path.clone()).await?;
        merge(Arc::make_mut(&mut slot.value));
        let merged = Arc::clone(&slot.value);
        if state.editing_enabled()
            && let Some(path) = path
        {
            let value = Arc::clone(&merged);
            let axum::Json(()) = compute(move || {
                store(&path, value.as_ref());
                Ok(())
            })
            .await?;
        }
        Ok(merged)
    }

    /// Make `slot` mirror `path`, loading it if it does not already.
    async fn current(slot: &mut Slot<T>, path: Option<PathBuf>) -> Result<(), AppError> {
        if slot.loaded_from.as_ref() == Some(&path) {
            return Ok(());
        }
        let value = match path.clone() {
            Some(file) => compute(move || Ok(load::<T>(&file))).await?.0,
            None => T::default(),
        };
        slot.value = Arc::new(value);
        slot.loaded_from = Some(path);
        Ok(())
    }
}

/// The file at `path` as a cache; empty when there is none.
fn load<T: Sidecar>(path: &Path) -> T {
    std::fs::read(path).map_or_else(|_| T::default(), |bytes| T::decode(&bytes))
}

/// Write `value` to `path` atomically. A failure is logged, not raised: the
/// answer is served from memory all the same, and a later request rewrites it.
/// The file name is fixed and ours, so the log names no path.
fn store<T: Sidecar>(path: &Path, value: &T) {
    if let Err(error) = value.encode().and_then(|bytes| atomic_write(path, &bytes)) {
        eprintln!("ledgeline: could not write {}: {}", T::FILE, error.kind());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_are_split_trimmed_and_deduplicated_in_order() {
        assert_eq!(
            parse_symbol_list(Some(" AAPL, VTI,,AAPL ,"), 10).unwrap(),
            ["AAPL", "VTI"]
        );
        assert!(parse_symbol_list(None, 10).unwrap().is_empty());
        assert!(parse_symbol_list(Some(" , "), 10).unwrap().is_empty());
    }

    #[test]
    fn too_many_or_too_long_symbols_are_refused() {
        assert!(matches!(
            parse_symbol_list(Some("A,B,C"), 2),
            Err(AppError::BadRequest(_))
        ));
        let long = "X".repeat(MAX_SYMBOL_BYTES + 1);
        assert!(matches!(
            parse_symbol_list(Some(&long), 10),
            Err(AppError::BadRequest(_))
        ));
    }

    /// A byte-counting sidecar for the load/store round trip.
    #[derive(Debug, Default, Clone, PartialEq)]
    struct Text(String);

    impl Sidecar for Text {
        const FILE: &'static str = "sidecar-test.txt";
        fn decode(bytes: &[u8]) -> Self {
            Self(String::from_utf8_lossy(bytes).into_owned())
        }
        fn encode(&self) -> std::io::Result<Vec<u8>> {
            Ok(self.0.clone().into_bytes())
        }
    }

    #[test]
    fn a_missing_file_loads_empty_and_a_stored_one_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(Text::FILE);
        assert_eq!(load::<Text>(&path), Text::default());
        store(&path, &Text("hello".to_string()));
        assert_eq!(load::<Text>(&path), Text("hello".to_string()));
    }
}
