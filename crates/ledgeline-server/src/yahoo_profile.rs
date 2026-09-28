//! What Yahoo Finance knows about a security beyond its price: the sector and
//! industry of a stock, the category and asset mix of a fund, and what kind of
//! security it is. Feeds the Holdings pie's "by category" views
//! (`profiles_api`).
//!
//! # Why this is not in `yahoo.rs`
//!
//! The chart endpoint `yahoo.rs` reads is open. The `quoteSummary` endpoint
//! this module reads is not: it answers `401` unless the request carries a
//! **crumb** and the cookie that crumb was minted against. Getting one is a
//! two-step handshake that no documented API promises to keep working:
//!
//! 1. `GET https://fc.yahoo.com/` — answers `404`, and that is fine: the point
//!    is its `Set-Cookie` (an `A3` session cookie).
//! 2. `GET https://query2.finance.yahoo.com/v1/test/getcrumb` with that cookie
//!    — the crumb, as a bare text body.
//!
//! Then `quoteSummary/{ticker}?modules=…&crumb=…` with the same cookie. A crumb
//! lives for the whole session, so it is fetched once and reused; a `401` on a
//! later call means it went stale, and the client refreshes it once and
//! retries.
//!
//! All of that is brittle by nature, so everything above this module treats a
//! failure here as "no Yahoo data" rather than as an error: the pie falls back
//! to whatever the journal's own commodity tags say.
//!
//! # Cookies without a cookie jar
//!
//! reqwest's `cookies` feature would pull two crates into the lockfile for
//! exactly one cookie. [`cookie_header`] instead lifts the `name=value` pair off
//! each `Set-Cookie` of the handshake response and replays them as a `Cookie`
//! header — all a single-host, single-session exchange needs.
//!
//! # Seams
//!
//! - [`ProfileFeed`] is what `profiles_api` fetches through; the integration
//!   tests substitute a fake (`AppState::with_profile_source`).
//! - [`Transport`] is the HTTP layer under [`YahooProfileClient`], so the crumb
//!   handshake, the stale-crumb retry and the rate-limit backoff are unit-tested
//!   below against scripted responses, with no network.
//! - [`parse_quote_summary`] is pure and tested against real, canned responses.

use async_trait::async_trait;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;

/// Everything this app uses from one `quoteSummary` answer. Every field is
/// optional because every module is: a stock has no `fundProfile`, a fund has
/// no sector, and an obscure ticker may have almost nothing.
///
/// Serializable because this is exactly what `profiles_api` caches on disk:
/// the facts as fetched, never the tag-merged result, so editing a tag takes
/// effect without refetching anything.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct YahooProfile {
    /// `quoteType.quoteType`, verbatim: `EQUITY`, `ETF`, `MUTUALFUND`, …
    pub quote_type: Option<String>,
    /// `assetProfile.sector` — stocks only (`"Technology"`).
    pub sector: Option<String>,
    /// `assetProfile.industry` — stocks only (`"Consumer Electronics"`).
    pub industry: Option<String>,
    /// `fundProfile.categoryName` — funds only (`"Large Blend"`).
    pub category: Option<String>,
    /// `topHoldings`' position split — funds only.
    pub asset_mix: Option<AssetMix>,
    /// `topHoldings.sectorWeightings`, with Yahoo's snake-case keys turned into
    /// the same display names a stock's `sector` uses, so a fund's technology
    /// slice and a stock's land in one bucket. Fractions of the fund's EQUITY
    /// portion, in Yahoo's order. Empty for a stock or a bond fund.
    pub sector_weights: Vec<(String, f64)>,
    /// `defaultKeyStatistics.morningStarRiskRating` (1 = low … 5 = high),
    /// present for some funds only.
    pub risk_rating: Option<u8>,
}

/// A fund's position split, as fractions (Yahoo's `raw` values). Not
/// normalized here — that is a presentation decision `commodity_profile`
/// makes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetMix {
    pub equity: f64,
    pub bond: f64,
    pub cash: f64,
    /// `otherPosition` plus `preferredPosition` and `convertiblePosition`,
    /// which are too small and too hybrid to earn a class of their own.
    pub other: f64,
}

/// A failure fetching or decoding a profile. "Yahoo has no such ticker" is not
/// one of these — that is `Ok(None)`.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ProfileError {
    #[error("request to Yahoo Finance failed: {0}")]
    Http(String),
    #[error("Yahoo Finance refused the session handshake: {0}")]
    Handshake(String),
    #[error("Yahoo Finance is rate-limiting requests")]
    RateLimited,
    #[error("Yahoo Finance response could not be parsed: {0}")]
    Shape(String),
}

/// Where `profiles_api` gets a ticker's profile. `ticker` is Yahoo's own symbol
/// (already resolved through the journal's `yahoo:` tag).
///
/// `pub` and re-exported from the crate root so the integration tests can hand
/// a fake to `AppState::with_profile_source`.
#[async_trait]
pub trait ProfileFeed: Send + Sync {
    async fn profile(&self, ticker: &str) -> Result<Option<YahooProfile>, ProfileError>;
}

// ===========================================================================
// Transport
// ===========================================================================

/// One HTTP answer, reduced to what the handshake needs.
#[derive(Debug, Clone, Default)]
pub(crate) struct HttpReply {
    pub(crate) status: u16,
    /// Every `Set-Cookie` header value, verbatim.
    pub(crate) set_cookies: Vec<String>,
    pub(crate) body: Vec<u8>,
}

/// A GET with an optional `Cookie` header. A non-2xx status is NOT an error
/// here: the handshake's first step is a `404` by design, and a `401`/`404` on
/// `quoteSummary` carries meaning the client acts on. Only a transport failure
/// (DNS, TLS, timeout) is an `Err`.
#[async_trait]
pub(crate) trait Transport: Send + Sync {
    async fn get(&self, url: &Url, cookie: Option<&str>) -> Result<HttpReply, ProfileError>;
}

/// A browser-like User-Agent. The crumb endpoint answers a bare or
/// library-looking agent with `429`s, which is the whole reason the handshake
/// looks like a page visit.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 \
     (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// A per-request ceiling, so one hung connection costs seconds rather than the
/// SPA's whole 30 s request budget.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

/// The real transport.
pub(crate) struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub(crate) fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Transport for ReqwestTransport {
    async fn get(&self, url: &Url, cookie: Option<&str>) -> Result<HttpReply, ProfileError> {
        let request = self
            .client
            .get(url.clone())
            .timeout(REQUEST_TIMEOUT)
            .header(reqwest::header::USER_AGENT, BROWSER_USER_AGENT);
        let request = match cookie {
            Some(cookie) => request.header(reqwest::header::COOKIE, cookie),
            None => request,
        };
        let response = request
            .send()
            .await
            .map_err(|error| ProfileError::Http(error.to_string()))?;
        let status = response.status().as_u16();
        let set_cookies = response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok().map(str::to_string))
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(|error| ProfileError::Http(error.to_string()))?
            .to_vec();
        Ok(HttpReply {
            status,
            set_cookies,
            body,
        })
    }
}

// ===========================================================================
// The client
// ===========================================================================

const COOKIE_URL: &str = "https://fc.yahoo.com/";
const CRUMB_URL: &str = "https://query2.finance.yahoo.com/v1/test/getcrumb";
const SUMMARY_BASE: &str = "https://query2.finance.yahoo.com/v10/finance/quoteSummary";
/// Every module [`parse_quote_summary`] reads. Asking for a module a ticker
/// lacks is harmless — it is simply absent from the answer.
const MODULES: &str = "assetProfile,quoteType,fundProfile,topHoldings,defaultKeyStatistics";

/// How long a failed handshake is remembered before another is attempted.
/// Without it, a rate-limited handshake would be retried once per symbol in
/// the same batch — thirty more requests into the limiter that just refused
/// the first one.
const HANDSHAKE_BACKOFF: Duration = Duration::from_secs(60);

/// A cookie + crumb pair. Compared by value so a stale-crumb refresh only
/// discards the session it actually saw fail, never a fresher one another task
/// already obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Session {
    cookie: String,
    crumb: String,
}

#[derive(Debug, Default)]
enum SessionState {
    #[default]
    None,
    Ready(Session),
    Failed {
        at: Instant,
        error: ProfileError,
    },
}

/// The real `quoteSummary` client: one session per process, shared by every
/// request, obtained lazily on first use.
pub(crate) struct YahooProfileClient {
    transport: Arc<dyn Transport>,
    /// A tokio mutex because the handshake's `.await`s happen under it — which
    /// is the point: concurrent first requests wait for ONE handshake rather
    /// than each starting their own.
    session: tokio::sync::Mutex<SessionState>,
}

impl YahooProfileClient {
    pub(crate) fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            session: tokio::sync::Mutex::new(SessionState::None),
        }
    }

    /// The current session, performing the handshake if there is none. A
    /// recent failure is returned as-is without touching the network.
    async fn session(&self) -> Result<Session, ProfileError> {
        let mut state = self.session.lock().await;
        match &*state {
            SessionState::Ready(session) => return Ok(session.clone()),
            SessionState::Failed { at, error } if at.elapsed() < HANDSHAKE_BACKOFF => {
                return Err(error.clone());
            }
            SessionState::Failed { .. } | SessionState::None => {}
        }
        match self.handshake().await {
            Ok(session) => {
                *state = SessionState::Ready(session.clone());
                Ok(session)
            }
            Err(error) => {
                *state = SessionState::Failed {
                    at: Instant::now(),
                    error: error.clone(),
                };
                Err(error)
            }
        }
    }

    /// Forget `stale`, unless another task has already replaced it.
    async fn invalidate(&self, stale: &Session) {
        let mut state = self.session.lock().await;
        if matches!(&*state, SessionState::Ready(current) if current == stale) {
            *state = SessionState::None;
        }
    }

    async fn handshake(&self) -> Result<Session, ProfileError> {
        let cookie_url = parse_url(COOKIE_URL)?;
        let landing = self.transport.get(&cookie_url, None).await?;
        let cookie = cookie_header(&landing.set_cookies).ok_or_else(|| {
            ProfileError::Handshake(format!(
                "no session cookie was set (HTTP {})",
                landing.status
            ))
        })?;
        let crumb_url = parse_url(CRUMB_URL)?;
        let reply = self.transport.get(&crumb_url, Some(&cookie)).await?;
        if reply.status == 429 {
            return Err(ProfileError::RateLimited);
        }
        if !(200..300).contains(&reply.status) {
            return Err(ProfileError::Handshake(format!(
                "crumb request answered HTTP {}",
                reply.status
            )));
        }
        let crumb = String::from_utf8_lossy(&reply.body).trim().to_string();
        // A real crumb is a short opaque token (`kZ3xR5c.3Yq`, sometimes with
        // a `/`). An HTML page or an error sentence served with a 200 is not
        // one, and using it would turn every later request into a 401.
        let plausible =
            |c: char| c.is_ascii_graphic() && !matches!(c, '<' | '>' | '"' | '\'' | '{' | '}');
        if crumb.is_empty() || crumb.len() > 64 || !crumb.chars().all(plausible) {
            return Err(ProfileError::Handshake(
                "crumb response was not a crumb".to_string(),
            ));
        }
        Ok(Session { cookie, crumb })
    }

    async fn summary(&self, ticker: &str, session: &Session) -> Result<HttpReply, ProfileError> {
        let url = summary_url(ticker, &session.crumb)?;
        self.transport.get(&url, Some(&session.cookie)).await
    }
}

#[async_trait]
impl ProfileFeed for YahooProfileClient {
    async fn profile(&self, ticker: &str) -> Result<Option<YahooProfile>, ProfileError> {
        let session = self.session().await?;
        let mut reply = self.summary(ticker, &session).await?;
        // A stale crumb: refresh once and retry once. A second 401 is reported
        // rather than looped on.
        if reply.status == 401 || reply.status == 403 {
            self.invalidate(&session).await;
            let fresh = self.session().await?;
            reply = self.summary(ticker, &fresh).await?;
        }
        match reply.status {
            // 404 is Yahoo's "no such symbol", and it carries a JSON body
            // saying so; the parser reads that as `None`.
            200..=299 | 404 => parse_quote_summary(&reply.body),
            429 => Err(ProfileError::RateLimited),
            status => Err(ProfileError::Http(format!(
                "quoteSummary answered HTTP {status}"
            ))),
        }
    }
}

fn parse_url(raw: &str) -> Result<Url, ProfileError> {
    Url::parse(raw).map_err(|error| ProfileError::Shape(error.to_string()))
}

/// `quoteSummary/{ticker}?modules=…&crumb=…`, with the ticker as one
/// percent-encoded path segment (`BRK-B` and `^GSPC` both occur).
fn summary_url(ticker: &str, crumb: &str) -> Result<Url, ProfileError> {
    let mut url = parse_url(SUMMARY_BASE)?;
    url.path_segments_mut()
        .map_err(|()| ProfileError::Shape("the summary URL cannot take a path".to_string()))?
        .push(ticker);
    url.query_pairs_mut()
        .append_pair("modules", MODULES)
        .append_pair("crumb", crumb);
    Ok(url)
}

/// A `Cookie` header replaying every `Set-Cookie`'s `name=value` pair, or
/// `None` when there were none (or none well-formed).
pub(crate) fn cookie_header(set_cookies: &[String]) -> Option<String> {
    let pairs: Vec<&str> = set_cookies
        .iter()
        .filter_map(|header| header.split(';').next())
        .map(str::trim)
        .filter(|pair| {
            pair.split_once('=')
                .is_some_and(|(name, _)| !name.is_empty())
        })
        .collect();
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

// ===========================================================================
// Parsing
// ===========================================================================

#[derive(Deserialize)]
struct SummaryResponse {
    #[serde(rename = "quoteSummary")]
    quote_summary: QuoteSummary,
}

#[derive(Deserialize)]
struct QuoteSummary {
    #[serde(default)]
    result: Option<Vec<SummaryResult>>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SummaryResult {
    asset_profile: Option<AssetProfile>,
    quote_type: Option<QuoteTypeModule>,
    fund_profile: Option<FundProfile>,
    top_holdings: Option<TopHoldings>,
    default_key_statistics: Option<KeyStatistics>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct AssetProfile {
    sector: Option<String>,
    industry: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct QuoteTypeModule {
    quote_type: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FundProfile {
    category_name: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TopHoldings {
    stock_position: Raw,
    bond_position: Raw,
    cash_position: Raw,
    other_position: Raw,
    preferred_position: Raw,
    convertible_position: Raw,
    /// A list of single-key objects: `[{"technology": {"raw": 0.36}}, …]`.
    sector_weightings: Vec<std::collections::BTreeMap<String, Raw>>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct KeyStatistics {
    morning_star_risk_rating: Raw,
}

/// Yahoo's `{"raw": 0.1, "fmt": "10%"}` — or `{}` when it has no value.
#[derive(Default, Clone, Copy, Deserialize)]
#[serde(default)]
struct Raw {
    raw: Option<f64>,
}

/// The pure half of a fetch: a `quoteSummary` body → the profile it
/// describes, or `None` when Yahoo answered "no such symbol" (a `null`
/// result). Anything that is not a `quoteSummary` document at all is a
/// [`ProfileError::Shape`].
pub(crate) fn parse_quote_summary(bytes: &[u8]) -> Result<Option<YahooProfile>, ProfileError> {
    let parsed: SummaryResponse =
        serde_json::from_slice(bytes).map_err(|error| ProfileError::Shape(error.to_string()))?;
    let Some(result) = parsed.quote_summary.result.into_iter().flatten().next() else {
        return Ok(None);
    };
    let (sector, industry) = result
        .asset_profile
        .map(|profile| (non_blank(profile.sector), non_blank(profile.industry)))
        .unwrap_or_default();
    let holdings = result.top_holdings;
    let asset_mix = holdings.as_ref().and_then(asset_mix);
    let sector_weights = holdings
        .map(|holdings| {
            holdings
                .sector_weightings
                .into_iter()
                .flatten()
                .filter_map(|(key, raw)| {
                    raw.raw
                        .filter(|weight| weight.is_finite() && *weight > 0.0)
                        .map(|weight| (sector_name(&key), weight))
                })
                .collect()
        })
        .unwrap_or_default();
    let profile = YahooProfile {
        // Yahoo answers an unknown or delisted symbol with a result whose
        // quoteType is the literal `"NONE"` rather than with "not found".
        quote_type: result
            .quote_type
            .and_then(|module| non_blank(module.quote_type))
            .filter(|quote_type| !quote_type.eq_ignore_ascii_case("NONE")),
        sector,
        industry,
        category: result
            .fund_profile
            .and_then(|fund| non_blank(fund.category_name)),
        asset_mix,
        sector_weights,
        risk_rating: result
            .default_key_statistics
            .and_then(|stats| stats.morning_star_risk_rating.raw)
            .filter(|rating| (1.0..=5.0).contains(rating))
            // In range 1..=5 by the filter, so the cast cannot truncate.
            .map(|rating| rating.round() as u8),
    };
    // A result that says nothing at all is a "not found" in all but name, and
    // is cached with the shorter not-found lifetime.
    Ok((profile != YahooProfile::default()).then_some(profile))
}

/// The position split, or `None` when every position is absent or zero — an
/// ETF whose `topHoldings` came back empty says nothing about its asset class,
/// and reporting it as 0% of everything would be a claim.
fn asset_mix(holdings: &TopHoldings) -> Option<AssetMix> {
    let value = |raw: Raw| raw.raw.filter(|v| v.is_finite()).unwrap_or(0.0);
    let mix = AssetMix {
        equity: value(holdings.stock_position),
        bond: value(holdings.bond_position),
        cash: value(holdings.cash_position),
        other: value(holdings.other_position)
            + value(holdings.preferred_position)
            + value(holdings.convertible_position),
    };
    let any = [mix.equity, mix.bond, mix.cash, mix.other]
        .iter()
        .any(|v| *v != 0.0);
    any.then_some(mix)
}

fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// A fund's `sectorWeightings` key as the display name a stock's `sector`
/// field uses for the same sector. Yahoo's fund keys are snake-case, and
/// title-casing them (`financial_services` → `Financial Services`) gives the
/// stock names — except `realestate`, the one key that is not a mechanical
/// transform, which is mapped explicitly.
fn sector_name(key: &str) -> String {
    match key {
        "realestate" => "Real Estate".to_string(),
        other => other
            .split('_')
            .filter(|word| !word.is_empty())
            .map(|word| {
                let mut chars = word.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    // Real responses, captured 2026-09-28 from the live endpoint with the
    // handshake above, trimmed of nothing.
    const AAPL: &str = include_str!("../../../fixtures/yahoo/qs_AAPL.json");
    const VTI: &str = include_str!("../../../fixtures/yahoo/qs_VTI.json");
    const BND: &str = include_str!("../../../fixtures/yahoo/qs_BND.json");
    const VFIAX: &str = include_str!("../../../fixtures/yahoo/qs_VFIAX.json");

    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 1e-9
    }

    #[test]
    fn a_stock_has_a_sector_and_industry_and_no_fund_data() {
        let profile = parse_quote_summary(AAPL.as_bytes()).unwrap().unwrap();
        assert_eq!(profile.quote_type.as_deref(), Some("EQUITY"));
        assert_eq!(profile.sector.as_deref(), Some("Technology"));
        assert_eq!(profile.industry.as_deref(), Some("Consumer Electronics"));
        assert_eq!(profile.category, None);
        assert_eq!(profile.asset_mix, None);
        assert!(profile.sector_weights.is_empty());
    }

    #[test]
    fn an_equity_etf_has_a_category_a_mix_and_named_sector_weights() {
        let profile = parse_quote_summary(VTI.as_bytes()).unwrap().unwrap();
        assert_eq!(profile.quote_type.as_deref(), Some("ETF"));
        assert_eq!(profile.sector, None, "a fund's assetProfile sector is null");
        assert_eq!(profile.category.as_deref(), Some("Large Blend"));
        let mix = profile.asset_mix.unwrap();
        assert!(close(mix.equity, 0.9978));
        assert!(close(mix.cash, 0.0019));
        let names: Vec<&str> = profile
            .sector_weights
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert!(names.contains(&"Technology"));
        assert!(names.contains(&"Real Estate"), "realestate is mapped");
        assert!(names.contains(&"Financial Services"));
        assert_eq!(names.len(), 11);
        let sum: f64 = profile.sector_weights.iter().map(|(_, w)| w).sum();
        assert!((sum - 1.0).abs() < 0.01, "weights are of the equity part");
    }

    #[test]
    fn a_bond_etf_is_mostly_bonds_with_no_sector_weights() {
        let profile = parse_quote_summary(BND.as_bytes()).unwrap().unwrap();
        assert_eq!(profile.category.as_deref(), Some("Intermediate Core Bond"));
        let mix = profile.asset_mix.unwrap();
        assert!(close(mix.bond, 0.9864));
        assert!(close(mix.equity, 0.0));
        // convertiblePosition 0.0001 folds into other.
        assert!(close(mix.other, 0.0001));
        assert!(profile.sector_weights.is_empty());
    }

    #[test]
    fn a_mutual_fund_parses_like_an_etf() {
        let profile = parse_quote_summary(VFIAX.as_bytes()).unwrap().unwrap();
        assert_eq!(profile.quote_type.as_deref(), Some("MUTUALFUND"));
        assert_eq!(profile.category.as_deref(), Some("Large Blend"));
        assert!(profile.asset_mix.unwrap().equity > 0.99);
        assert!(!profile.sector_weights.is_empty());
    }

    #[test]
    fn a_null_result_is_not_found() {
        let body = br#"{"quoteSummary":{"result":null,"error":{"code":"Not Found","description":"Quote not found for symbol: ZZZZ"}}}"#;
        assert_eq!(parse_quote_summary(body).unwrap(), None);
    }

    /// Seen live (2026-09-28) for a symbol Yahoo does not list: a result, with
    /// `quoteType: "NONE"` and nothing else of use.
    #[test]
    fn a_quote_type_of_none_with_nothing_else_is_not_found() {
        let body = br#"{"quoteSummary":{"result":[{"quoteType":{"quoteType":"NONE","symbol":"BAL"},"assetProfile":{}}],"error":null}}"#;
        assert_eq!(parse_quote_summary(body).unwrap(), None);
    }

    #[test]
    fn missing_modules_leave_their_fields_empty() {
        let body = br#"{"quoteSummary":{"result":[{"quoteType":{"quoteType":"CRYPTOCURRENCY"}}],"error":null}}"#;
        let profile = parse_quote_summary(body).unwrap().unwrap();
        assert_eq!(profile.quote_type.as_deref(), Some("CRYPTOCURRENCY"));
        assert_eq!(
            profile,
            YahooProfile {
                quote_type: Some("CRYPTOCURRENCY".to_string()),
                ..YahooProfile::default()
            }
        );
    }

    #[test]
    fn empty_raw_objects_and_blank_strings_read_as_absent() {
        let body = br#"{"quoteSummary":{"result":[{
            "assetProfile":{"sector":"  ","industry":""},
            "topHoldings":{"stockPosition":{},"bondPosition":{"raw":0.0},"sectorWeightings":[{"technology":{}}]},
            "defaultKeyStatistics":{"morningStarRiskRating":{"raw":3,"fmt":"3"}}
        }],"error":null}}"#;
        let profile = parse_quote_summary(body).unwrap().unwrap();
        assert_eq!(profile.sector, None);
        assert_eq!(profile.industry, None);
        assert_eq!(profile.asset_mix, None, "all-zero mix is no mix");
        assert!(profile.sector_weights.is_empty());
        assert_eq!(profile.risk_rating, Some(3));
    }

    #[test]
    fn an_out_of_range_risk_rating_is_dropped() {
        let body = br#"{"quoteSummary":{"result":[{"quoteType":{"quoteType":"MUTUALFUND"},"defaultKeyStatistics":{"morningStarRiskRating":{"raw":0}}}],"error":null}}"#;
        assert_eq!(
            parse_quote_summary(body).unwrap().unwrap().risk_rating,
            None
        );
    }

    #[test]
    fn garbage_is_a_shape_error() {
        assert!(matches!(
            parse_quote_summary(b"<html>nope</html>"),
            Err(ProfileError::Shape(_))
        ));
    }

    #[test]
    fn unknown_sector_keys_are_title_cased() {
        assert_eq!(sector_name("space_mining"), "Space Mining");
        assert_eq!(sector_name("realestate"), "Real Estate");
    }

    #[test]
    fn cookie_header_replays_name_value_pairs_only() {
        let headers = vec![
            "A3=d=AQAB&S=xyz; Expires=Tue, 28 Sep 2027 22:30:17 GMT; Domain=.yahoo.com; Secure"
                .to_string(),
            "B=1; Path=/".to_string(),
            "; broken".to_string(),
        ];
        assert_eq!(
            cookie_header(&headers).as_deref(),
            Some("A3=d=AQAB&S=xyz; B=1")
        );
        assert_eq!(cookie_header(&[]), None);
    }

    #[test]
    fn summary_url_encodes_the_ticker_and_carries_the_crumb() {
        // One path segment, whatever the ticker holds.
        let url = summary_url("A/B", "ab/c").unwrap();
        assert!(url.path().ends_with("/quoteSummary/A%2FB"), "{url}");
        let crumb = url
            .query_pairs()
            .find(|(key, _)| key == "crumb")
            .map(|(_, value)| value.into_owned());
        assert_eq!(crumb.as_deref(), Some("ab/c"));
    }

    // ---- the handshake, against a scripted transport ----

    /// Answers each request from a per-URL-prefix script, in order, and
    /// records what was asked.
    struct Scripted {
        replies: Mutex<VecDeque<(&'static str, HttpReply)>>,
        seen: Mutex<Vec<(String, Option<String>)>>,
    }

    impl Scripted {
        fn new(replies: Vec<(&'static str, HttpReply)>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.into()),
                seen: Mutex::new(Vec::new()),
            })
        }

        fn seen(&self) -> Vec<(String, Option<String>)> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Transport for Scripted {
        async fn get(&self, url: &Url, cookie: Option<&str>) -> Result<HttpReply, ProfileError> {
            self.seen
                .lock()
                .unwrap()
                .push((url.to_string(), cookie.map(str::to_string)));
            let (prefix, reply) = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("unscripted request to {url}"));
            assert!(
                url.as_str().starts_with(prefix),
                "expected a request to {prefix}, got {url}"
            );
            Ok(reply)
        }
    }

    fn reply(status: u16, body: &str) -> HttpReply {
        HttpReply {
            status,
            set_cookies: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn landing(cookie: &str) -> HttpReply {
        HttpReply {
            status: 404,
            set_cookies: vec![format!("{cookie}; Domain=.yahoo.com; Secure")],
            body: Vec::new(),
        }
    }

    #[tokio::test]
    async fn handshake_then_summary_with_cookie_and_crumb() {
        let transport = Scripted::new(vec![
            (COOKIE_URL, landing("A3=one")),
            (CRUMB_URL, reply(200, "crumb1")),
            (SUMMARY_BASE, reply(200, AAPL)),
        ]);
        let client = YahooProfileClient::new(transport.clone());
        let profile = client.profile("AAPL").await.unwrap().unwrap();
        assert_eq!(profile.sector.as_deref(), Some("Technology"));

        let seen = transport.seen();
        assert_eq!(
            seen[1].1.as_deref(),
            Some("A3=one"),
            "crumb asked with the cookie"
        );
        assert!(seen[2].0.contains("crumb=crumb1"));
        assert_eq!(seen[2].1.as_deref(), Some("A3=one"));
    }

    #[tokio::test]
    async fn the_session_is_reused_across_tickers() {
        let transport = Scripted::new(vec![
            (COOKIE_URL, landing("A3=one")),
            (CRUMB_URL, reply(200, "crumb1")),
            (SUMMARY_BASE, reply(200, AAPL)),
            (SUMMARY_BASE, reply(200, VTI)),
        ]);
        let client = YahooProfileClient::new(transport.clone());
        client.profile("AAPL").await.unwrap();
        client.profile("VTI").await.unwrap();
        assert_eq!(transport.seen().len(), 4, "one handshake, two summaries");
    }

    #[tokio::test]
    async fn a_401_refreshes_the_crumb_once_and_retries() {
        let transport = Scripted::new(vec![
            (COOKIE_URL, landing("A3=one")),
            (CRUMB_URL, reply(200, "old")),
            (SUMMARY_BASE, reply(401, "{}")),
            (COOKIE_URL, landing("A3=two")),
            (CRUMB_URL, reply(200, "new")),
            (SUMMARY_BASE, reply(200, VTI)),
        ]);
        let client = YahooProfileClient::new(transport.clone());
        let profile = client.profile("VTI").await.unwrap().unwrap();
        assert_eq!(profile.quote_type.as_deref(), Some("ETF"));
        let seen = transport.seen();
        assert!(seen[5].0.contains("crumb=new"));
        assert_eq!(seen[5].1.as_deref(), Some("A3=two"));
    }

    #[tokio::test]
    async fn a_second_401_is_reported_not_looped_on() {
        let transport = Scripted::new(vec![
            (COOKIE_URL, landing("A3=one")),
            (CRUMB_URL, reply(200, "old")),
            (SUMMARY_BASE, reply(401, "{}")),
            (COOKIE_URL, landing("A3=two")),
            (CRUMB_URL, reply(200, "new")),
            (SUMMARY_BASE, reply(401, "{}")),
        ]);
        let client = YahooProfileClient::new(transport);
        assert!(matches!(
            client.profile("VTI").await,
            Err(ProfileError::Http(_))
        ));
    }

    #[tokio::test]
    async fn a_404_summary_is_not_found() {
        let transport = Scripted::new(vec![
            (COOKIE_URL, landing("A3=one")),
            (CRUMB_URL, reply(200, "crumb1")),
            (
                SUMMARY_BASE,
                reply(
                    404,
                    r#"{"quoteSummary":{"result":null,"error":{"code":"Not Found"}}}"#,
                ),
            ),
        ]);
        let client = YahooProfileClient::new(transport);
        assert_eq!(client.profile("ZZZZ").await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_rate_limited_handshake_fails_fast_for_the_rest_of_the_batch() {
        // Only ONE handshake is scripted: a second attempt inside the backoff
        // window would panic on an unscripted request.
        let transport = Scripted::new(vec![
            (COOKIE_URL, landing("A3=one")),
            (CRUMB_URL, reply(429, "Too Many Requests")),
        ]);
        let client = YahooProfileClient::new(transport.clone());
        assert_eq!(client.profile("AAPL").await, Err(ProfileError::RateLimited));
        assert_eq!(client.profile("VTI").await, Err(ProfileError::RateLimited));
        assert_eq!(transport.seen().len(), 2);
    }

    #[tokio::test]
    async fn a_landing_page_with_no_cookie_is_a_handshake_error() {
        let transport = Scripted::new(vec![(COOKIE_URL, reply(404, ""))]);
        let client = YahooProfileClient::new(transport);
        assert!(matches!(
            client.profile("AAPL").await,
            Err(ProfileError::Handshake(_))
        ));
    }

    #[tokio::test]
    async fn an_html_crumb_is_refused() {
        let transport = Scripted::new(vec![
            (COOKIE_URL, landing("A3=one")),
            (CRUMB_URL, reply(200, "<html><body>consent</body></html>")),
        ]);
        let client = YahooProfileClient::new(transport);
        assert!(matches!(
            client.profile("AAPL").await,
            Err(ProfileError::Handshake(_))
        ));
    }

    #[test]
    fn a_fund_sector_key_reads_as_the_stock_sector_name() {
        assert_eq!(sector_name("financial_services"), "Financial Services");
        assert_eq!(sector_name("technology"), "Technology");
        assert_eq!(sector_name("real_estate"), "Real Estate");
        assert_eq!(sector_name("realestate"), "Real Estate");
    }
}
