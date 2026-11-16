//! SimpleFIN: the setup token, the access URL and the `/accounts` body.
//!
//! The protocol (<https://www.simplefin.org/protocol.html>) has three steps,
//! and this module covers the parts of each that need no network:
//!
//! 1. The user pastes a **setup token**: base64 of a one-time claim URL.
//!    [`decode_setup_token`] checks it is one, and refuses anything that isn't
//!    `https://`.
//! 2. The server `POST`s the claim URL and gets back an **access URL**,
//!    `https://user:pass@host/path`. [`AccessUrl`] splits it so the
//!    credentials travel only in an `Authorization` header, and so no `Debug`
//!    or `Display` of it can ever print them.
//! 3. `GET {base}/accounts` returns JSON, v1 or v2. [`parse_accounts`] reads
//!    either into a provider-neutral [`FeedBatch`].
//!
//! # The loopback escape
//!
//! The spec requires HTTPS. The server's tests run a fake SimpleFIN on
//! `http://127.0.0.1`, so every entry point takes `allow_loopback_http`. With
//! it `false` — the only value production code passes — plain `http://` is
//! refused even for loopback.

use super::{
    FeedAccount, FeedBatch, FeedConnection, FeedCurrency, FeedProblem, FeedTxn, ProblemKind,
    is_decimal_text, sanitize,
};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD_PAD_INDIFFERENT, URL_SAFE_PAD_INDIFFERENT};
use serde_json::{Map, Value};
use std::fmt;
use url::{Host, Url};

/// What a pasted setup token decodes to.
///
/// `Debug` shows only the host: the claim URL is a one-time secret until it is
/// claimed, and whoever claims it first gets the access.
#[derive(Clone, PartialEq, Eq)]
pub struct SetupToken {
    /// The URL to `POST` to, exactly once.
    pub claim_url: String,
    /// Its host, to name who the user is about to trust.
    pub host: String,
}

impl fmt::Debug for SetupToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetupToken")
            .field("claim_url", &"<redacted>")
            .field("host", &self.host)
            .finish()
    }
}

/// Why a setup token or access URL was refused. The messages are written to be
/// shown as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// Not base64 at all, or empty.
    #[error(
        "That doesn't look like a SimpleFIN setup token. Copy the whole token and paste it again."
    )]
    NotBase64,
    /// Decodes, but not to a web address.
    #[error("That token doesn't contain a web address.")]
    NotUrl,
    /// A web address, but not `https://`.
    #[error("That token's address isn't secure (https), so Ledgeline won't use it.")]
    NotHttps,
    /// An `https://` address with no `/claim/` in its path.
    #[error("That token's address isn't a SimpleFIN claim address.")]
    NotAClaimUrl,
    /// An access URL with no username or password in it.
    #[error("SimpleFIN's access address has no credentials in it.")]
    NoCredentials,
}

/// [`decode_setup_token_with`] with plain `http://` always refused.
///
/// # Errors
/// See [`TokenError`].
pub fn decode_setup_token(input: &str) -> Result<SetupToken, TokenError> {
    decode_setup_token_with(input, false)
}

/// Decode a pasted setup token.
///
/// Whitespace anywhere is ignored (a token pasted from an email may be
/// wrapped). Standard and URL-safe base64 are both accepted, padded or not.
/// The decoded text must be an `https://` URL whose path contains `/claim/`
/// followed by something; `http://` is accepted only for a loopback host and
/// only when `allow_loopback_http` is true (tests).
///
/// # Errors
/// See [`TokenError`].
pub fn decode_setup_token_with(
    input: &str,
    allow_loopback_http: bool,
) -> Result<SetupToken, TokenError> {
    let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Err(TokenError::NotBase64);
    }
    let bytes = STANDARD_PAD_INDIFFERENT
        .decode(&compact)
        .or_else(|_| URL_SAFE_PAD_INDIFFERENT.decode(&compact))
        .map_err(|_| TokenError::NotBase64)?;
    let text = String::from_utf8(bytes).map_err(|_| TokenError::NotUrl)?;
    let url = parse_secure_url(text.trim(), allow_loopback_http)?;
    let claims_something = url
        .path()
        .split_once("/claim/")
        .is_some_and(|(_, rest)| !rest.is_empty());
    if !claims_something {
        return Err(TokenError::NotAClaimUrl);
    }
    Ok(SetupToken {
        host: url.host_str().unwrap_or_default().to_string(),
        claim_url: url.into(),
    })
}

/// `text` as a URL that is `https://` (or loopback `http://` when allowed) and
/// names a host.
fn parse_secure_url(text: &str, allow_loopback_http: bool) -> Result<Url, TokenError> {
    let url = Url::parse(text).map_err(|_| TokenError::NotUrl)?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(TokenError::NotUrl);
    }
    let secure = match url.scheme() {
        "https" => true,
        "http" => allow_loopback_http && is_loopback(&url),
        _ => false,
    };
    if secure {
        Ok(url)
    } else {
        Err(TokenError::NotHttps)
    }
}

/// `localhost`, `127.0.0.0/8` or `::1`.
fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// A claimed SimpleFIN access URL, split so its credentials can travel only in
/// an `Authorization` header.
///
/// **`Debug` and `Display` redact the username and the password.** There is no
/// serde derive: the only way to get the secret back out as text is
/// [`to_secret_string`](Self::to_secret_string), which exists for the keychain.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessUrl {
    /// `scheme://host[:port]/path`, with no userinfo, query or trailing slash.
    pub base: String,
    /// The Basic-auth username, percent-decoded.
    pub username: String,
    /// The Basic-auth password, percent-decoded.
    pub password: String,
    /// The host, for display and for "fetch custom currencies only from here".
    pub host: String,
}

impl AccessUrl {
    /// Parse a full access URL, `https://user:pass@host/path`.
    ///
    /// # Errors
    /// [`TokenError::NotUrl`] / [`TokenError::NotHttps`] as for a setup token,
    /// and [`TokenError::NoCredentials`] when the username or password is
    /// missing.
    pub fn parse(text: &str, allow_loopback_http: bool) -> Result<Self, TokenError> {
        let url = parse_secure_url(text.trim(), allow_loopback_http)?;
        let username = percent_decode(url.username())?;
        let password = percent_decode(url.password().unwrap_or_default())?;
        if username.is_empty() || password.is_empty() {
            return Err(TokenError::NoCredentials);
        }
        let host = url.host_str().unwrap_or_default().to_string();
        let port = url
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        let base = format!(
            "{}://{host}{port}{}",
            url.scheme(),
            url.path().trim_end_matches('/')
        );
        Ok(Self {
            base,
            username,
            password,
            host,
        })
    }

    /// The full URL, credentials included, for storing in the keychain and
    /// nowhere else. [`AccessUrl::parse`] of it gives back an equal value.
    #[must_use]
    pub fn to_secret_string(&self) -> String {
        match self.base.split_once("://") {
            Some((scheme, rest)) => format!(
                "{scheme}://{}:{}@{rest}",
                percent_encode(&self.username),
                percent_encode(&self.password)
            ),
            None => self.base.clone(),
        }
    }

    /// `{base}/{path}`, e.g. `endpoint("accounts")`. No credentials.
    #[must_use]
    pub fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base, path.trim_start_matches('/'))
    }
}

impl fmt::Debug for AccessUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessUrl")
            .field("base", &self.base)
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .field("host", &self.host)
            .finish()
    }
}

impl fmt::Display for AccessUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.base.split_once("://") {
            Some((scheme, rest)) => write!(f, "{scheme}://<redacted>@{rest}"),
            None => f.write_str(&self.base),
        }
    }
}

/// URL userinfo is percent-encoded; Basic auth wants the raw text.
fn percent_decode(text: &str) -> Result<String, TokenError> {
    let bytes = text.as_bytes();
    let hex = |at: usize| {
        bytes
            .get(at)
            .and_then(|b| char::from(*b).to_digit(16))
            .and_then(|d| u8::try_from(d).ok())
    };
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match (bytes[at], hex(at + 1), hex(at + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push(high * 16 + low);
                at += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| TokenError::NotUrl)
}

/// Everything but RFC 3986's unreserved characters, percent-encoded.
fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Why an `/accounts` body couldn't be read at all. (A body that reads but has
/// odd parts is not an error: the odd parts become [`FeedProblem`]s.)
///
/// Neither variant carries any of the body's text: it is bank data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Not JSON.
    #[error("SimpleFIN's response isn't valid JSON (line {line}, column {column}).")]
    NotJson {
        /// 1-based line of the first error.
        line: usize,
        /// 1-based column of the first error.
        column: usize,
    },
    /// JSON, but not an object.
    #[error("SimpleFIN's response isn't a JSON object.")]
    NotAnObject,
}

/// Read a SimpleFIN `/accounts` response, v1 or v2.
///
/// - **v2** has `errlist` (`{code, msg, conn_id?, account_id?}`) and
///   `connections`; an account names its connection with `conn_id`.
/// - **v1** has `errors` (plain strings) and an `org` object on each account.
///
/// When `errlist` is present, `errors` is ignored: a v2 server may send both
/// for old clients, and reading both would report every problem twice.
///
/// Unknown fields are ignored and optional ones tolerated. An account with no
/// id, or a transaction with no id or an amount that isn't a plain decimal
/// (see [`is_decimal_text`]), is dropped and reported as a
/// [`ProblemKind::Other`] problem. Nothing here panics on any input.
///
/// # Errors
/// [`ParseError`] when the body isn't a JSON object.
pub fn parse_accounts(body: &[u8]) -> Result<FeedBatch, ParseError> {
    let value: Value = serde_json::from_slice(body).map_err(|error| ParseError::NotJson {
        line: error.line(),
        column: error.column(),
    })?;
    let Value::Object(root) = value else {
        return Err(ParseError::NotAnObject);
    };

    let connections = parse_connections(&root);
    let provider_problems = parse_errors(&root);
    let (accounts, read_problems) = root
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| parse_account(entry, &connections))
        .fold(
            (Vec::new(), Vec::new()),
            |(mut accounts, mut problems), (account, found)| {
                accounts.extend(account);
                problems.extend(found);
                (accounts, problems)
            },
        );

    Ok(FeedBatch {
        connections,
        accounts,
        problems: provider_problems.into_iter().chain(read_problems).collect(),
        notices: parse_notices(&root),
        field_names: field_names(&root),
    })
}

/// Most field names [`field_names`] lists, so a hostile response can't grow
/// the state file without bound.
const MAX_FIELD_NAMES: usize = 200;

/// Every key the response's accounts and transactions used, and their
/// `extra` objects' keys, as `account.KEY`, `account.extra.KEY`,
/// `transaction.KEY` and `transaction.extra.KEY`. A key keeps only
/// `[A-Za-z0-9_.-]`, at most 40 of them.
fn field_names(root: &Map<String, Value>) -> std::collections::BTreeSet<String> {
    fn keys<'a>(
        prefix: &'a str,
        object: &'a Map<String, Value>,
    ) -> impl Iterator<Item = String> + 'a {
        let extra = object
            .get("extra")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|extra| extra.keys())
            .map(move |key| (format!("{prefix}.extra"), key));
        object
            .keys()
            .map(move |key| (prefix.to_string(), key))
            .chain(extra)
            .filter_map(|(prefix, key)| {
                let key: String = key
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
                    .take(40)
                    .collect();
                (!key.is_empty()).then(|| format!("{prefix}.{key}"))
            })
    }
    let accounts = root
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object);
    accounts
        .flat_map(|account| {
            let transactions = account
                .get("transactions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .flat_map(|txn| keys("transaction", txn));
            keys("account", account).chain(transactions)
        })
        .scan(std::collections::BTreeSet::new(), |seen, name| {
            (seen.len() < MAX_FIELD_NAMES).then(|| {
                seen.insert(name.clone());
                name
            })
        })
        .collect()
}

/// The first of `keys` that `object` has with a non-null value. SimpleFIN
/// spells some keys with hyphens (`balance-date`); tolerate underscores too.
fn field<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| object.get(*key).filter(|value| !value.is_null()))
}

/// A string, or a number as its text.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// [`text`] of the first present key, trimmed, when not empty.
fn nonempty(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    field(object, keys)
        .and_then(text)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Epoch seconds from an integer, a finite float (truncated) or a numeric
/// string.
fn epoch(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|f| f.is_finite() && f.abs() < 9.0e15)
                // Checked by the filter above: well inside i64.
                .map(|f| f.trunc() as i64)
        }),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// An `http(s)` URL as text, or nothing. Server-sent URLs end up behind an
/// "open" button, so a `javascript:` or `file:` one must never get through.
fn web_url(text: &str) -> Option<String> {
    Url::parse(text.trim())
        .ok()
        .filter(|url| matches!(url.scheme(), "https" | "http") && url.host_str().is_some())
        .map(String::from)
}

/// A bare domain (v1 `org.domain`) as an `https://` URL.
fn domain_url(domain: &str) -> Option<String> {
    let domain = domain.trim();
    let plain = !domain.is_empty()
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    if plain {
        web_url(&format!("https://{domain}"))
    } else {
        web_url(domain)
    }
}

fn parse_connections(root: &Map<String, Value>) -> Vec<FeedConnection> {
    root.get("connections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|object| {
            let id = nonempty(object, &["conn_id", "id"])?;
            Some(FeedConnection {
                name: nonempty(object, &["name"])
                    .map(|name| sanitize(&name))
                    .unwrap_or_default(),
                org_url: nonempty(object, &["org_url", "org-url"]).and_then(|u| web_url(&u)),
                id,
            })
        })
        .collect()
}

/// `errlist` when present, else `errors`.
fn parse_errors(root: &Map<String, Value>) -> Vec<FeedProblem> {
    if let Some(list) = root.get("errlist").and_then(Value::as_array) {
        return list.iter().filter_map(errlist_entry).collect();
    }
    root.get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(v1_problem)
        .collect()
}

fn errlist_entry(entry: &Value) -> Option<FeedProblem> {
    let object = match entry {
        Value::String(text) => return v1_problem(text),
        Value::Object(object) => object,
        _ => return None,
    };
    let code = sanitize(&nonempty(object, &["code"]).unwrap_or_default());
    let message = sanitize(&nonempty(object, &["msg", "message"]).unwrap_or_default());
    if code.is_empty() && message.is_empty() {
        return None;
    }
    Some(FeedProblem {
        kind: kind_for_code(&code, &message),
        code,
        message,
        conn_id: nonempty(object, &["conn_id"]),
        account_id: nonempty(object, &["account_id", "acct_id"]),
    })
}

/// A v1 `errors` string. v1 has no codes, so the text decides.
fn v1_problem(text: &str) -> Option<FeedProblem> {
    let message = sanitize(text);
    if message.is_empty() {
        return None;
    }
    let kind = if message.to_lowercase().contains("may need attention") {
        ProblemKind::NeedsAttention
    } else if mentions_quota(&message) {
        ProblemKind::Quota
    } else {
        ProblemKind::Gen
    };
    Some(FeedProblem {
        kind,
        code: String::new(),
        message,
        conn_id: None,
        account_id: None,
    })
}

/// The kind for a v2 `errlist` code.
///
/// Connection and account codes mean what they say. A general or unknown code
/// whose message talks about a quota or limit is a [`ProblemKind::Quota`]
/// warning, because the Bridge sends those as ordinary errors and auto-sync
/// must notice them; see [`mentions_quota`] for why the 90-day cap isn't one.
fn kind_for_code(code: &str, message: &str) -> ProblemKind {
    let code = code.to_ascii_lowercase();
    match code.as_str() {
        "con.auth" => ProblemKind::ConAuth,
        "act.failed" => ProblemKind::ActFailed,
        "act.missingdata" => ProblemKind::ActMissingData,
        "act" => ProblemKind::Act,
        other if other.starts_with("act.") => ProblemKind::Act,
        _ if mentions_quota(message) => ProblemKind::Quota,
        "gen.api" => ProblemKind::GenApi,
        "gen.auth" => ProblemKind::GenAuth,
        "gen" => ProblemKind::Gen,
        other if other.starts_with("gen.") => ProblemKind::Gen,
        _ => ProblemKind::Other,
    }
}

/// Whether `message` is a quota warning: it mentions a quota, or a limit that
/// isn't the date-*range* limit (`gen.api`'s "range limited to 90 days").
fn mentions_quota(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("quota") || (lower.contains("limit") && !lower.contains("range"))
}

fn parse_notices(root: &Map<String, Value>) -> Vec<String> {
    let notices: Vec<&str> = match root.get("x-api-message") {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        Some(Value::String(one)) => vec![one.as_str()],
        _ => Vec::new(),
    };
    notices
        .into_iter()
        .map(sanitize)
        .filter(|notice| !notice.is_empty())
        .collect()
}

/// A problem this crate found while reading, rather than one the provider sent.
fn read_problem(message: String, account_id: Option<&str>) -> FeedProblem {
    FeedProblem {
        kind: ProblemKind::Other,
        code: String::new(),
        message: sanitize(&message),
        conn_id: None,
        account_id: account_id.map(str::to_string),
    }
}

/// One account, or nothing, plus whatever was dropped from it.
fn parse_account(
    entry: &Value,
    connections: &[FeedConnection],
) -> (Option<FeedAccount>, Vec<FeedProblem>) {
    let Some(object) = entry.as_object() else {
        return (
            None,
            vec![read_problem(
                "SimpleFIN sent an account Ledgeline couldn't read, so it was skipped.".into(),
                None,
            )],
        );
    };
    let Some(id) = nonempty(object, &["id"]) else {
        return (
            None,
            vec![read_problem(
                "SimpleFIN sent an account with no id, so it was skipped.".into(),
                None,
            )],
        );
    };
    let name = nonempty(object, &["name"])
        .map(|name| sanitize(&name))
        .unwrap_or_else(|| sanitize(&id));
    let conn_id = nonempty(object, &["conn_id"]);
    let connection = conn_id
        .as_deref()
        .and_then(|conn| connections.iter().find(|c| c.id == conn));
    let org = field(object, &["org"]).and_then(Value::as_object);
    let org_name = connection
        .map(|c| c.name.clone())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            org.and_then(|o| nonempty(o, &["name"]))
                .map(|n| sanitize(&n))
        });
    let org_url = connection.and_then(|c| c.org_url.clone()).or_else(|| {
        org.and_then(|o| {
            nonempty(o, &["url"])
                .and_then(|u| web_url(&u))
                .or_else(|| nonempty(o, &["domain"]).and_then(|d| domain_url(&d)))
        })
    });
    let currency = match nonempty(object, &["currency"]) {
        Some(url) if url.starts_with("https://") || url.starts_with("http://") => {
            FeedCurrency::Url(url)
        }
        Some(code) => FeedCurrency::Code(sanitize(&code)),
        None => FeedCurrency::Code(String::new()),
    };

    let mut problems = Vec::new();
    let mut checked = |key: &[&str], what: &str| -> Option<String> {
        let raw = field(object, key).and_then(text)?;
        if is_decimal_text(&raw) {
            Some(raw)
        } else {
            problems.push(read_problem(
                format!("SimpleFIN sent {what} for \u{201c}{name}\u{201d} that isn't a number: \u{201c}{raw}\u{201d}."),
                Some(&id),
            ));
            None
        }
    };
    let balance = checked(&["balance"], "a balance").unwrap_or_default();
    let available_balance = checked(
        &["available-balance", "available_balance"],
        "an available balance",
    );

    let (transactions, dropped): (Vec<_>, Vec<_>) = field(object, &["transactions"])
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|txn| parse_txn(txn, &id, &name))
        .partition(Result::is_ok);
    problems.extend(dropped.into_iter().filter_map(Result::err));

    let account = FeedAccount {
        balance_date: field(object, &["balance-date", "balance_date"]).and_then(epoch),
        transactions: transactions.into_iter().filter_map(Result::ok).collect(),
        id,
        name,
        conn_id,
        org_name,
        org_url,
        currency,
        balance,
        available_balance,
    };
    (Some(account), problems)
}

fn parse_txn(entry: &Value, account_id: &str, account_name: &str) -> Result<FeedTxn, FeedProblem> {
    let skipped = |why: String| read_problem(why, Some(account_id));
    let object = entry.as_object().ok_or_else(|| {
        skipped(format!(
            "A transaction in \u{201c}{account_name}\u{201d} couldn't be read, so it was skipped."
        ))
    })?;
    let id = nonempty(object, &["id"]).ok_or_else(|| {
        skipped(format!(
            "A transaction in \u{201c}{account_name}\u{201d} had no id, so it was skipped."
        ))
    })?;
    let raw_amount = field(object, &["amount"])
        .and_then(text)
        .unwrap_or_default();
    if !is_decimal_text(&raw_amount) {
        return Err(skipped(format!(
            "Skipped transaction {id} in \u{201c}{account_name}\u{201d}: its amount \u{201c}{raw_amount}\u{201d} isn't a plain number."
        )));
    }
    Ok(FeedTxn {
        posted: field(object, &["posted"]).and_then(epoch).unwrap_or(0),
        amount: raw_amount,
        description: field(object, &["description"])
            .and_then(text)
            .unwrap_or_default(),
        payee: nonempty(object, &["payee"]),
        memo: nonempty(object, &["memo"]),
        transacted_at: field(object, &["transacted_at", "transacted-at"]).and_then(epoch),
        pending: field(object, &["pending"])
            .and_then(Value::as_bool)
            .unwrap_or(false),
        mcc: field(object, &["mcc"])
            .and_then(text)
            .and_then(|raw| super::mcc::normalize_code(&raw)),
        extra: field(object, &["extra"])
            .and_then(Value::as_object)
            .filter(|extra| !extra.is_empty())
            .map(|extra| Value::Object(extra.clone()).to_string()),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(url: &str) -> String {
        STANDARD_PAD_INDIFFERENT.encode(url)
    }

    const CLAIM: &str = "https://beta-bridge.simplefin.org/simplefin/claim/DEMO";

    #[test]
    fn a_standard_token_decodes_to_its_claim_url_and_host() {
        let decoded = decode_setup_token(&token(CLAIM)).expect("valid token");
        assert_eq!(decoded.claim_url, CLAIM);
        assert_eq!(decoded.host, "beta-bridge.simplefin.org");
    }

    #[test]
    fn padding_alphabet_and_whitespace_do_not_matter() {
        // A URL whose base64 needs padding and contains `+`/`/` in the
        // standard alphabet, so the URL-safe form genuinely differs.
        let url = "https://bridge.example/claim/>>~~ab";
        let standard = base64::engine::general_purpose::STANDARD.encode(url);
        let url_safe = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url);
        assert!(standard.ends_with('='));
        assert_ne!(standard.trim_end_matches('='), url_safe);
        let wrapped = format!("  {}\n{}  \n", &standard[..10], &standard[10..]);
        for input in [
            standard.clone(),
            standard.trim_end_matches('=').to_string(),
            url_safe.clone(),
            base64::engine::general_purpose::URL_SAFE.encode(url),
            wrapped,
        ] {
            let decoded = decode_setup_token(&input)
                .unwrap_or_else(|e| panic!("{input:?} should decode: {e}"));
            assert_eq!(decoded.host, "bridge.example");
        }
    }

    #[test]
    fn junk_is_not_base64() {
        for junk in ["", "   ", "not a token!", "%%%%", CLAIM, "ab$cd"] {
            assert_eq!(
                decode_setup_token(junk),
                Err(TokenError::NotBase64),
                "{junk:?}"
            );
        }
    }

    #[test]
    fn decoded_text_must_be_a_url() {
        assert_eq!(
            decode_setup_token(&token("hello world")),
            Err(TokenError::NotUrl)
        );
        assert_eq!(
            decode_setup_token(&STANDARD_PAD_INDIFFERENT.encode([0xff, 0xfe, 0x00])),
            Err(TokenError::NotUrl)
        );
    }

    #[test]
    fn http_and_other_schemes_are_refused() {
        for url in [
            "http://bridge.example/claim/X",
            "ftp://bridge.example/claim/X",
            "javascript://bridge.example/claim/X",
            "http://127.0.0.1/claim/X",
        ] {
            assert_eq!(
                decode_setup_token(&token(url)),
                Err(TokenError::NotHttps),
                "{url}"
            );
        }
    }

    #[test]
    fn loopback_http_is_allowed_only_when_asked() {
        for url in [
            "http://127.0.0.1:8123/claim/X",
            "http://localhost:8123/claim/X",
            "http://[::1]:8123/claim/X",
        ] {
            assert_eq!(decode_setup_token(&token(url)), Err(TokenError::NotHttps));
            assert!(decode_setup_token_with(&token(url), true).is_ok(), "{url}");
        }
        // The escape is for loopback only.
        assert_eq!(
            decode_setup_token_with(&token("http://bridge.example/claim/X"), true),
            Err(TokenError::NotHttps)
        );
        assert_eq!(
            decode_setup_token_with(&token("http://127.0.0.1.evil.example/claim/X"), true),
            Err(TokenError::NotHttps)
        );
    }

    #[test]
    fn the_path_must_claim_something() {
        for url in [
            "https://bridge.example/simplefin",
            "https://bridge.example/claim/",
            "https://bridge.example/?x=/claim/X",
        ] {
            assert_eq!(
                decode_setup_token(&token(url)),
                Err(TokenError::NotAClaimUrl),
                "{url}"
            );
        }
    }

    #[test]
    fn a_setup_token_debug_hides_the_claim_url() {
        let decoded = decode_setup_token(&token(CLAIM)).expect("valid");
        let shown = format!("{decoded:?}");
        assert!(!shown.contains("DEMO"), "{shown}");
        assert!(shown.contains("beta-bridge.simplefin.org"));
    }

    const ACCESS: &str = "https://s3cr3tUser:p4ssw0rdXYZ@beta-bridge.simplefin.org/simplefin/";

    #[test]
    fn an_access_url_splits_into_base_and_credentials() {
        let access = AccessUrl::parse(ACCESS, false).expect("valid");
        assert_eq!(access.base, "https://beta-bridge.simplefin.org/simplefin");
        assert_eq!(access.username, "s3cr3tUser");
        assert_eq!(access.password, "p4ssw0rdXYZ");
        assert_eq!(access.host, "beta-bridge.simplefin.org");
        assert_eq!(
            access.endpoint("accounts"),
            "https://beta-bridge.simplefin.org/simplefin/accounts"
        );
    }

    #[test]
    fn debug_and_display_never_show_credentials() {
        let access = AccessUrl::parse(ACCESS, false).expect("valid");
        for shown in [
            format!("{access:?}"),
            format!("{access}"),
            format!("{access:#?}"),
        ] {
            assert!(!shown.contains("p4ssw0rdXYZ"), "{shown}");
            assert!(!shown.contains("s3cr3tUser"), "{shown}");
            assert!(shown.contains("beta-bridge.simplefin.org"), "{shown}");
        }
    }

    #[test]
    fn the_secret_string_round_trips_even_with_awkward_characters() {
        let text = "https://us%40er:p%3Ass%2F%25w@host.example:8443/sf";
        let access = AccessUrl::parse(text, false).expect("valid");
        assert_eq!(access.username, "us@er");
        assert_eq!(access.password, "p:ss/%w");
        assert_eq!(access.base, "https://host.example:8443/sf");
        let again = AccessUrl::parse(&access.to_secret_string(), false).expect("round trip");
        assert_eq!(again, access);
        let plain = AccessUrl::parse(ACCESS, false).expect("valid");
        assert_eq!(
            plain.to_secret_string(),
            "https://s3cr3tUser:p4ssw0rdXYZ@beta-bridge.simplefin.org/simplefin"
        );
    }

    #[test]
    fn an_access_url_needs_https_and_credentials() {
        assert_eq!(
            AccessUrl::parse("http://u:p@bridge.example/sf", false),
            Err(TokenError::NotHttps)
        );
        assert!(AccessUrl::parse("http://u:p@127.0.0.1:9/sf", true).is_ok());
        assert_eq!(
            AccessUrl::parse("https://bridge.example/sf", false),
            Err(TokenError::NoCredentials)
        );
        assert_eq!(
            AccessUrl::parse("https://u@bridge.example/sf", false),
            Err(TokenError::NoCredentials)
        );
        assert_eq!(AccessUrl::parse("nonsense", false), Err(TokenError::NotUrl));
    }

    #[test]
    fn codes_map_to_kinds() {
        let cases = [
            ("con.auth", "", ProblemKind::ConAuth),
            ("CON.AUTH", "", ProblemKind::ConAuth),
            ("act.failed", "", ProblemKind::ActFailed),
            ("act.missingdata", "", ProblemKind::ActMissingData),
            ("act.other", "", ProblemKind::Act),
            (
                "gen.api",
                "Date range limited to 90 days",
                ProblemKind::GenApi,
            ),
            ("gen.auth", "", ProblemKind::GenAuth),
            ("gen.other", "", ProblemKind::Gen),
            (
                "gen.other",
                "You are close to your daily request limit",
                ProblemKind::Quota,
            ),
            ("", "Quota exceeded", ProblemKind::Quota),
            ("weird", "", ProblemKind::Other),
            // An account-level failure that mentions a limit is still about
            // the account.
            ("act.failed", "limit", ProblemKind::ActFailed),
        ];
        for (code, message, kind) in cases {
            assert_eq!(kind_for_code(code, message), kind, "{code} {message}");
        }
    }

    #[test]
    fn v1_strings_are_classified_by_text() {
        let kind = |text: &str| v1_problem(text).map(|p| p.kind);
        assert_eq!(
            kind("Connection to Chase may need attention"),
            Some(ProblemKind::NeedsAttention)
        );
        assert_eq!(
            kind("You have exceeded your request quota"),
            Some(ProblemKind::Quota)
        );
        assert_eq!(kind("Something else"), Some(ProblemKind::Gen));
        assert_eq!(kind("  \n "), None);
    }

    #[test]
    fn errlist_wins_over_errors() {
        let batch = parse_accounts(
            br#"{"errors":["dup"],"errlist":[{"code":"gen.api","msg":"dup"}],"accounts":[]}"#,
        )
        .expect("parses");
        assert_eq!(batch.problems.len(), 1);
        assert_eq!(batch.problems[0].kind, ProblemKind::GenApi);
    }

    #[test]
    fn non_objects_are_refused_without_echoing_content() {
        assert_eq!(parse_accounts(b"[1,2]"), Err(ParseError::NotAnObject));
        let Err(error) = parse_accounts(b"{\"secret-bank-data\": ") else {
            panic!("truncated JSON must fail");
        };
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn server_urls_must_be_web_urls() {
        assert_eq!(web_url("javascript:alert(1)"), None);
        assert_eq!(web_url("file:///etc/passwd"), None);
        assert_eq!(
            web_url("https://www.chase.com"),
            Some("https://www.chase.com/".into())
        );
        assert_eq!(domain_url("chase.com"), Some("https://chase.com/".into()));
        assert_eq!(domain_url("evil.com/@x"), None);
    }

    #[test]
    fn epochs_tolerate_floats_and_strings() {
        assert_eq!(
            epoch(&serde_json::json!(1_700_000_000)),
            Some(1_700_000_000)
        );
        assert_eq!(
            epoch(&serde_json::json!(1_700_000_000.9)),
            Some(1_700_000_000)
        );
        assert_eq!(epoch(&serde_json::json!("1700000000")), Some(1_700_000_000));
        assert_eq!(epoch(&serde_json::json!(1e300)), None);
        assert_eq!(epoch(&serde_json::json!(true)), None);
    }
}
