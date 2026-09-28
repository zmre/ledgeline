//! `GET /api/holdings/profiles?symbols=AAPL,VTI` — how each held security
//! divides by asset class, sector, industry, security type, category and risk,
//! for the Holdings pie's "by category" views.
//!
//! # Sources, in precedence order
//!
//! 1. **Commodity tags** (`commodity VTI  ; sector: Technology`). The journal
//!    is the source of truth: a tag replaces that whole dimension. See
//!    [`crate::commodity_profile`] for every rule.
//! 2. **Yahoo Finance** `quoteSummary` ([`crate::yahoo_profile`]), looked up
//!    under the commodity's `yahoo:` tag exactly as the price update does
//!    ([`crate::prices_api::yahoo_ticker`]). Only for symbols whose tags leave
//!    some dimension open.
//!
//! # The cache: `commodity-profiles.json`
//!
//! What Yahoo said is kept in a JSON file BESIDE the main journal, so a
//! portfolio is fetched once a month rather than on every visit:
//!
//! - It holds Yahoo's facts only, never the tag-merged result, so editing a
//!   tag takes effect immediately with no refetch.
//! - A found profile is reused for [`FOUND_TTL`], a "no such ticker" (a
//!   private fund, a 401(k) trust) for [`NOT_FOUND_TTL`].
//! - It is never `include`d, never parsed as journal, never committed by the
//!   import's git step (which stages explicit paths only), and does not wake
//!   the live-reload watcher (which filters on the journal's own files).
//! - It is written with the same atomic rename as the journal, and only when
//!   this server may write at all (`editing_enabled`). A read-only session
//!   keeps its cache in memory.
//! - It is safe to delete. A missing, corrupt or future-versioned file is an
//!   empty cache, and the next request rewrites it.
//!
//! # Degrading
//!
//! Yahoo's crumb handshake is undocumented and can break or rate-limit at any
//! time. Nothing here turns that into an error response: a symbol Yahoo could
//! not be asked about is served from a stale cache entry if there is one, else
//! from its tags alone, and [`WireProfiles::yahoo`] says how much of that
//! happened so the SPA can show a quiet hint.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use futures::stream::{self, StreamExt};
use ledgeline_core::edit::atomic_write;
use ledgeline_core::model::Journal;
use ledgeline_core::reports::periods::iso_from_days;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::AppState;
use crate::commodity_profile::{Breakdown, Source, fully_tagged, resolve};
use crate::error::AppError;
use crate::prices_api::yahoo_ticker;
use crate::yahoo_profile::{ProfileError, ProfileFeed, YahooProfile};

/// The cache file's name, beside the main journal.
pub(crate) const CACHE_FILE: &str = "commodity-profiles.json";
/// The cache format. A file with any other version is ignored and replaced.
const CACHE_VERSION: u32 = 1;
/// How long a fetched profile is trusted. A fund's sector mix drifts slowly
/// and a stock's sector almost never changes.
const FOUND_TTL: Duration = Duration::from_secs(30 * 86_400);
/// How long "Yahoo has no such ticker" is trusted — shorter, so a `yahoo:` tag
/// added to fix a lookup is picked up within the week even if nobody deletes
/// the cache.
const NOT_FOUND_TTL: Duration = Duration::from_secs(7 * 86_400);
/// Simultaneous Yahoo requests. Lower than the price update's five: this
/// endpoint is rate-limited harder, and a cold cache is a one-off.
const FETCH_CONCURRENCY: usize = 4;
/// A ceiling on the whole fetch phase, well inside the SPA's 30 s request
/// timeout, so a slow Yahoo costs a partial answer rather than a failed page.
const FETCH_BUDGET: Duration = Duration::from_secs(15);
/// Most symbols one request may name, and the longest symbol.
const MAX_SYMBOLS: usize = 500;
const MAX_SYMBOL_BYTES: usize = 128;

// ===========================================================================
// Wire types
// ===========================================================================

#[derive(Deserialize)]
pub(crate) struct ProfilesQuery {
    /// Comma-separated commodity symbols.
    symbols: Option<String>,
}

/// `GET /api/holdings/profiles`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireProfiles {
    /// How the Yahoo side of this answer went. See [`YahooStatus`].
    yahoo: YahooStatus,
    /// One per requested symbol the journal knows, in request order. A symbol
    /// the journal has never mentioned is omitted, and never looked up.
    profiles: Vec<WireProfile>,
}

/// Whether any profile is poorer than it would have been with Yahoo working.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum YahooStatus {
    /// Every symbol that wanted Yahoo data has it (fresh or cached), or none
    /// wanted any.
    Ok,
    /// Some symbols that wanted Yahoo data could not get it and are shown from
    /// their tags alone.
    Partial,
    /// None could — Yahoo is unreachable, refusing, or rate-limiting.
    Unavailable,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireProfile {
    symbol: String,
    /// The ticker Yahoo was (or would be) asked for.
    yahoo_ticker: String,
    source: Source,
    /// When the Yahoo data used was fetched (`YYYY-MM-DD`), absent when none
    /// was used.
    #[serde(skip_serializing_if = "Option::is_none")]
    fetched_at: Option<String>,
    breakdown: Breakdown,
}

// ===========================================================================
// State
// ===========================================================================

/// One cached Yahoo answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CacheEntry {
    /// Unix seconds.
    fetched_at: u64,
    /// `None`: Yahoo answered that there is no such ticker.
    profile: Option<YahooProfile>,
}

impl CacheEntry {
    fn fresh(&self, now: u64) -> bool {
        let ttl = if self.profile.is_some() {
            FOUND_TTL
        } else {
            NOT_FOUND_TTL
        };
        now.saturating_sub(self.fetched_at) < ttl.as_secs()
    }
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, CacheEntry>,
}

/// The in-memory cache, keyed by Yahoo ticker, and the file it mirrors.
#[derive(Default)]
struct ProfileCache {
    /// The file `entries` was loaded from. Re-read when the journal (and so
    /// the directory) changes under a desktop File→Open.
    loaded_from: Option<Option<PathBuf>>,
    entries: BTreeMap<String, CacheEntry>,
}

/// The profile source plus its cache; one per [`AppState`].
pub(crate) struct Profiles {
    feed: Arc<dyn ProfileFeed>,
    /// A tokio mutex held for a whole request — including its fetches — so two
    /// tabs opening at once make one set of Yahoo requests, not two.
    cache: tokio::sync::Mutex<ProfileCache>,
}

impl Profiles {
    pub(crate) fn new(feed: Arc<dyn ProfileFeed>) -> Self {
        Self {
            feed,
            cache: tokio::sync::Mutex::new(ProfileCache::default()),
        }
    }
}

// ===========================================================================
// Handler
// ===========================================================================

/// One requested symbol, planned.
struct Planned {
    symbol: String,
    ticker: String,
    tags: Vec<(String, String)>,
    wants_yahoo: bool,
}

/// `GET /api/holdings/profiles?symbols=…`.
pub(crate) async fn profiles(
    State(state): State<AppState>,
    Query(query): Query<ProfilesQuery>,
) -> Result<Response, AppError> {
    let symbols = parse_symbols(query.symbols.as_deref())?;
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let planned = plan(journal, &symbols);
    let cache_path = cache_path(journal);
    let persist = state.editing_enabled();

    let profiles = state.profiles();
    let mut cache = profiles.cache.lock().await;
    if cache.loaded_from.as_ref() != Some(&cache_path) {
        cache.entries = match &cache_path {
            Some(path) => read_cache(path.clone()).await,
            None => BTreeMap::new(),
        };
        cache.loaded_from = Some(cache_path.clone());
    }

    let now = now_secs();
    let wanted: BTreeSet<String> = planned
        .iter()
        .filter(|plan| plan.wants_yahoo)
        .map(|plan| plan.ticker.clone())
        .collect();
    let stale: Vec<String> = wanted
        .iter()
        .filter(|ticker| {
            !cache
                .entries
                .get(*ticker)
                .is_some_and(|entry| entry.fresh(now))
        })
        .cloned()
        .collect();

    let fetched = fetch_all(Arc::clone(&profiles.feed), stale).await;
    let mut changed = false;
    for (ticker, outcome) in fetched {
        if let Ok(profile) = outcome {
            cache.entries.insert(
                ticker,
                CacheEntry {
                    fetched_at: now,
                    profile,
                },
            );
            changed = true;
        }
    }
    if changed
        && persist
        && let Some(path) = cache_path
    {
        write_cache(path, cache.entries.clone()).await;
    }

    let missing = wanted
        .iter()
        .filter(|ticker| !cache.entries.contains_key(*ticker))
        .count();
    let yahoo = match missing {
        0 => YahooStatus::Ok,
        n if n == wanted.len() => YahooStatus::Unavailable,
        _ => YahooStatus::Partial,
    };
    let body = WireProfiles {
        yahoo,
        profiles: planned
            .into_iter()
            .map(|plan| wire_profile(plan, &cache.entries))
            .collect(),
    };
    drop(cache);

    const NO_STORE: (HeaderName, HeaderValue) =
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(([NO_STORE], Json(body)).into_response())
}

fn wire_profile(plan: Planned, entries: &BTreeMap<String, CacheEntry>) -> WireProfile {
    let entry = plan
        .wants_yahoo
        .then(|| entries.get(&plan.ticker))
        .flatten();
    let yahoo = entry.and_then(|entry| entry.profile.as_ref());
    let resolved = resolve(&plan.tags, yahoo);
    let fetched_at = match resolved.source {
        Source::Yahoo | Source::Mixed => entry.map(|entry| iso_date(entry.fetched_at)),
        Source::Tags | Source::None => None,
    };
    WireProfile {
        symbol: plan.symbol,
        yahoo_ticker: plan.ticker,
        source: resolved.source,
        fetched_at,
        breakdown: resolved.breakdown,
    }
}

/// `symbols=` split on commas, trimmed, de-duplicated in order. Absent or
/// empty is no symbols (an empty answer), not an error.
fn parse_symbols(raw: Option<&str>) -> Result<Vec<String>, AppError> {
    let mut seen = BTreeSet::new();
    let symbols: Vec<String> = raw
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .filter(|symbol| seen.insert(symbol.to_string()))
        .map(str::to_string)
        .collect();
    if symbols.len() > MAX_SYMBOLS {
        return Err(AppError::BadRequest(format!(
            "at most {MAX_SYMBOLS} symbols may be classified at once"
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

/// Each symbol the journal knows, with its tags and Yahoo ticker. A symbol it
/// does not know is dropped here, so this endpoint can only ever ask Yahoo
/// about the journal's own commodities.
fn plan(journal: &Journal, symbols: &[String]) -> Vec<Planned> {
    let known: BTreeSet<&str> = journal
        .commodity_styles
        .iter()
        .map(|(commodity, _)| commodity.0.as_str())
        .chain(
            journal
                .commodity_tags
                .iter()
                .map(|(commodity, _)| commodity.0.as_str()),
        )
        .collect();
    symbols
        .iter()
        .filter(|symbol| known.contains(symbol.as_str()))
        .map(|symbol| {
            let tags: Vec<(String, String)> = journal
                .commodity_tags
                .iter()
                .filter(|(commodity, _)| &commodity.0 == symbol)
                .flat_map(|(_, tags)| tags.iter().cloned())
                .collect();
            Planned {
                ticker: yahoo_ticker(&journal.commodity_tags, symbol),
                wants_yahoo: !fully_tagged(&tags),
                symbol: symbol.clone(),
                tags,
            }
        })
        .collect()
}

/// Fetch every ticker with bounded concurrency, each abandoned at one shared
/// deadline. A ticker whose fetch failed or ran out of time is returned as an
/// `Err` and left out of the cache.
async fn fetch_all(
    feed: Arc<dyn ProfileFeed>,
    tickers: Vec<String>,
) -> Vec<(String, Result<Option<YahooProfile>, ProfileError>)> {
    let deadline = tokio::time::Instant::now() + FETCH_BUDGET;
    stream::iter(tickers)
        .map(|ticker| {
            let feed = Arc::clone(&feed);
            async move {
                let outcome = tokio::time::timeout_at(deadline, feed.profile(&ticker))
                    .await
                    .unwrap_or_else(|_| Err(ProfileError::Http("timed out".to_string())));
                (ticker, outcome)
            }
        })
        .buffer_unordered(FETCH_CONCURRENCY)
        .collect()
        .await
}

// ===========================================================================
// The cache file
// ===========================================================================

/// `commodity-profiles.json` beside the main journal, or `None` for a journal
/// with no file behind it.
fn cache_path(journal: &Journal) -> Option<PathBuf> {
    journal
        .source_files
        .first()
        .and_then(|main| main.parent())
        .map(|dir| dir.join(CACHE_FILE))
}

async fn read_cache(path: PathBuf) -> BTreeMap<String, CacheEntry> {
    tokio::task::spawn_blocking(move || load_cache_file(&path))
        .await
        .unwrap_or_default()
}

async fn write_cache(path: PathBuf, entries: BTreeMap<String, CacheEntry>) {
    let written = tokio::task::spawn_blocking(move || save_cache_file(&path, entries)).await;
    // Best effort: a cache that cannot be written only means the next visit
    // fetches again. The file name is fixed and ours, so this names no path.
    if let Ok(Err(error)) = written {
        eprintln!("ledgeline: could not write {CACHE_FILE}: {error}");
    }
}

/// Every entry in the file at `path`; empty when it is missing, unreadable,
/// not JSON, or another version.
pub(crate) fn load_cache_file(path: &Path) -> BTreeMap<String, CacheEntry> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
        .filter(|file| file.version == CACHE_VERSION)
        .map(|file| file.entries)
        .unwrap_or_default()
}

/// Write `entries` to `path` atomically, pretty-printed so a curious user can
/// read (or hand-edit) it.
pub(crate) fn save_cache_file(
    path: &Path,
    entries: BTreeMap<String, CacheEntry>,
) -> std::io::Result<()> {
    let file = CacheFile {
        version: CACHE_VERSION,
        entries,
    };
    let mut bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn iso_date(unix_secs: u64) -> String {
    iso_from_days(i64::try_from(unix_secs / 86_400).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(fetched_at: u64, found: bool) -> CacheEntry {
        CacheEntry {
            fetched_at,
            profile: found.then(|| YahooProfile {
                quote_type: Some("ETF".to_string()),
                category: Some("Large Blend".to_string()),
                sector_weights: vec![("Technology".to_string(), 0.4)],
                ..YahooProfile::default()
            }),
        }
    }

    const DAY: u64 = 86_400;

    #[test]
    fn found_entries_live_thirty_days_not_found_seven() {
        let now = 100 * DAY;
        assert!(entry(now - 29 * DAY, true).fresh(now));
        assert!(!entry(now - 30 * DAY, true).fresh(now));
        assert!(entry(now - 6 * DAY, false).fresh(now));
        assert!(!entry(now - 7 * DAY, false).fresh(now));
        // A clock that went backwards is not "expired".
        assert!(entry(now + DAY, true).fresh(now));
    }

    #[test]
    fn the_cache_file_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CACHE_FILE);
        let entries: BTreeMap<String, CacheEntry> = [
            ("VTI".to_string(), entry(1_700_000_000, true)),
            ("PRIVATE".to_string(), entry(1_700_000_000, false)),
        ]
        .into();
        save_cache_file(&path, entries.clone()).unwrap();
        assert_eq!(load_cache_file(&path), entries);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"), "human-readable: {text}");
    }

    #[test]
    fn a_missing_corrupt_or_foreign_cache_is_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CACHE_FILE);
        assert!(load_cache_file(&path).is_empty());
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_cache_file(&path).is_empty());
        std::fs::write(
            &path,
            r#"{"version":99,"entries":{"VTI":{"fetchedAt":1,"profile":null}}}"#,
        )
        .unwrap();
        assert!(load_cache_file(&path).is_empty());
    }

    #[test]
    fn symbols_are_split_trimmed_and_deduplicated() {
        assert_eq!(
            parse_symbols(Some(" AAPL, VTI,,AAPL ,")).unwrap(),
            vec!["AAPL".to_string(), "VTI".to_string()]
        );
        assert!(parse_symbols(None).unwrap().is_empty());
    }

    #[test]
    fn too_many_or_too_long_symbols_are_refused() {
        let many = (0..=MAX_SYMBOLS)
            .map(|i| format!("S{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(matches!(
            parse_symbols(Some(&many)),
            Err(AppError::BadRequest(_))
        ));
        let long = "X".repeat(MAX_SYMBOL_BYTES + 1);
        assert!(matches!(
            parse_symbols(Some(&long)),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn iso_date_is_the_utc_calendar_day() {
        assert_eq!(iso_date(946_684_800), "2000-01-01");
        assert_eq!(iso_date(946_684_800 + DAY - 1), "2000-01-01");
    }
}
