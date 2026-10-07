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
//! - It is loaded once and kept in memory, written through atomically only when
//!   this server may write at all, and safe to delete: [`crate::sidecar`] has
//!   the rules both beside-the-journal caches follow. A future-versioned file
//!   is an empty cache too.
//!
//! # Degrading
//!
//! Yahoo's crumb handshake is undocumented and can break or rate-limit at any
//! time. Nothing here turns that into an error response: a symbol Yahoo could
//! not be asked about is served from a stale cache entry if there is one, else
//! from its tags alone, and [`WireProfiles::yahoo`] says how much of that
//! happened so the SPA can show a quiet hint.

use axum::extract::{Query, State};
use axum::response::Response;
use futures::stream::{self, StreamExt};
use ledgeline_core::model::Journal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use crate::AppState;
use crate::commodity_profile::{Breakdown, fully_tagged, resolve};
use crate::error::AppError;
use crate::prices_api::yahoo_ticker;
use crate::reports_api::unix_now;
use crate::security::no_store_json;
use crate::sidecar::{FETCH_CONCURRENCY, Sidecar, SidecarCache, parse_symbol_list};
use crate::yahoo_profile::{ProfileError, ProfileFeed, YahooProfile};

/// The cache format. A file with any other version is ignored and replaced.
const CACHE_VERSION: u32 = 1;
/// How long a fetched profile is trusted. A fund's sector mix drifts slowly
/// and a stock's sector almost never changes.
const FOUND_TTL: Duration = Duration::from_secs(30 * 86_400);
/// How long "Yahoo has no such ticker" is trusted — shorter, so a `yahoo:` tag
/// added to fix a lookup is picked up within the week even if nobody deletes
/// the cache.
const NOT_FOUND_TTL: Duration = Duration::from_secs(7 * 86_400);
/// A ceiling on the whole fetch phase, well inside the SPA's 30 s request
/// timeout, so a slow Yahoo costs a partial answer rather than a failed page.
const FETCH_BUDGET: Duration = Duration::from_secs(15);
/// Most symbols one request may name.
const MAX_SYMBOLS: usize = 500;

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
pub(crate) struct WireProfile {
    symbol: String,
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

/// The file's shape: a version and the entries.
#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, CacheEntry>,
}

/// What Yahoo said, keyed by Yahoo ticker: `commodity-profiles.json`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ProfileCache(BTreeMap<String, CacheEntry>);

impl Sidecar for ProfileCache {
    const FILE: &'static str = "commodity-profiles.json";

    /// Every entry in the file; empty when it is not JSON or another version.
    fn decode(bytes: &[u8]) -> Self {
        Self(
            serde_json::from_slice::<CacheFile>(bytes)
                .ok()
                .filter(|file| file.version == CACHE_VERSION)
                .map(|file| file.entries)
                .unwrap_or_default(),
        )
    }

    /// Pretty-printed, so a curious user can read (or hand-edit) it.
    fn encode(&self) -> std::io::Result<Vec<u8>> {
        let file = CacheFile {
            version: CACHE_VERSION,
            entries: self.0.clone(),
        };
        let mut bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

/// The profile source plus its cache; one per [`AppState`].
pub(crate) struct Profiles {
    feed: Arc<dyn ProfileFeed>,
    cache: SidecarCache<ProfileCache>,
}

impl Profiles {
    pub(crate) fn new(feed: Arc<dyn ProfileFeed>) -> Self {
        Self {
            feed,
            cache: SidecarCache::default(),
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
    let symbols = parse_symbol_list(query.symbols.as_deref(), MAX_SYMBOLS)?;
    let planned = plan(&state.snapshot().journal, &symbols);

    let profiles = state.profiles();
    let cached = profiles.cache.snapshot(&state).await?;
    let now = unix_now();
    let wanted: BTreeSet<String> = planned
        .iter()
        .filter(|plan| plan.wants_yahoo)
        .map(|plan| plan.ticker.clone())
        .collect();
    let stale: Vec<String> = wanted
        .iter()
        .filter(|ticker| !cached.0.get(*ticker).is_some_and(|entry| entry.fresh(now)))
        .cloned()
        .collect();

    // Fetched with no lock held; only the merge below takes it.
    let found: Vec<(String, CacheEntry)> = fetch_all(Arc::clone(&profiles.feed), stale)
        .await
        .into_iter()
        .filter_map(|(ticker, outcome)| {
            let profile = outcome.ok()?;
            Some((
                ticker,
                CacheEntry {
                    fetched_at: now,
                    profile,
                },
            ))
        })
        .collect();
    let cache = if found.is_empty() {
        cached
    } else {
        drop(cached); // so the merge need not copy a snapshot nobody reads
        profiles
            .cache
            .update(&state, |entries| entries.0.extend(found))
            .await?
    };

    let missing = wanted
        .iter()
        .filter(|ticker| !cache.0.contains_key(*ticker))
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
            .map(|plan| wire_profile(plan, &cache))
            .collect(),
    };
    Ok(no_store_json(body))
}

fn wire_profile(plan: Planned, cache: &ProfileCache) -> WireProfile {
    let yahoo = plan
        .wants_yahoo
        .then(|| cache.0.get(&plan.ticker))
        .flatten()
        .and_then(|entry| entry.profile.as_ref());
    WireProfile {
        breakdown: resolve(&plan.tags, yahoo),
        symbol: plan.symbol,
    }
}

/// Each symbol the journal knows, with its tags and Yahoo ticker. A symbol it
/// does not know is dropped here, so this endpoint can only ever ask Yahoo
/// about the journal's own commodities.
fn plan(journal: &Journal, symbols: &[&str]) -> Vec<Planned> {
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
        .filter(|symbol| known.contains(*symbol))
        .map(|&symbol| {
            let tags: Vec<(String, String)> = journal
                .commodity_tags
                .iter()
                .filter(|(commodity, _)| commodity.0 == symbol)
                .flat_map(|(_, tags)| tags.iter().cloned())
                .collect();
            Planned {
                ticker: yahoo_ticker(&journal.commodity_tags, symbol),
                wants_yahoo: !fully_tagged(&tags),
                symbol: symbol.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let cache = ProfileCache(
            [
                ("VTI".to_string(), entry(1_700_000_000, true)),
                ("PRIVATE".to_string(), entry(1_700_000_000, false)),
            ]
            .into(),
        );
        let bytes = cache.encode().unwrap();
        assert_eq!(ProfileCache::decode(&bytes), cache);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"version\": 1"), "human-readable: {text}");
    }

    #[test]
    fn a_corrupt_or_foreign_cache_is_empty() {
        assert!(ProfileCache::decode(b"{not json").0.is_empty());
        let foreign = br#"{"version":99,"entries":{"VTI":{"fetchedAt":1,"profile":null}}}"#;
        assert!(ProfileCache::decode(foreign).0.is_empty());
    }
}
