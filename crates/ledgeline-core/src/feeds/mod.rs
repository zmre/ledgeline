//! Bank feeds: the pure half of bank sync (see `plans/43-bank-sync.md` and
//! its contract, `plans/43-bank-sync-contract.md`, section WP-A).
//!
//! Nothing in this module touches the network, the filesystem or the clock.
//! `ledgeline-server` fetches, stores secrets and writes; this module only
//! turns bytes into facts and facts into text:
//!
//! - [`simplefin`] — the setup token, the access URL (whose `Debug` and
//!   `Display` never show the credentials), and the `/accounts` body, v1 and v2;
//! - [`normalize`] — one [`FeedAccount`] as the CSV the account's sync rules
//!   file reads, so the import itself is the existing Pipeline A;
//! - [`dupes`] — which normalized rows the journal already holds by id, and
//!   which look like a transaction already there under another id ("held");
//! - [`config`] — the `[sync]` tables of `ledgeline.toml`, parsed tolerantly
//!   and rewritten with `toml_edit` so nothing else in the file moves.
//!
//! # Trust
//!
//! Every string in a [`FeedBatch`] came from a server we don't control, so the
//! ones a person will read ([`FeedProblem::message`], names, notices) are
//! passed through [`sanitize`] on the way in: control and bidi-override
//! characters are removed and the length is capped. Transaction text is kept as
//! sent and cleaned by [`normalize`] on its way into a CSV.

pub mod dupes;
pub mod normalize;
pub mod simplefin;

/// Provider-neutral result of one fetch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedBatch {
    /// The connections (one per institution login) the provider reported.
    /// Empty for SimpleFIN v1, which has no connection objects.
    pub connections: Vec<FeedConnection>,
    /// Every account in the response, in the order sent.
    pub accounts: Vec<FeedAccount>,
    /// Errors the provider reported, plus anything this crate dropped while
    /// reading the response (an amount that isn't a decimal, an account with
    /// no id).
    pub problems: Vec<FeedProblem>,
    /// `x-api-message` and the like: text the provider wants shown, sanitized.
    pub notices: Vec<String>,
}

/// One institution login at the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedConnection {
    /// The provider's connection id (`conn_id`).
    pub id: String,
    /// The institution's name, sanitized.
    pub name: String,
    /// The institution's site, for "Fix at SimpleFIN" context.
    pub org_url: Option<String>,
}

/// One account at the provider, with whatever transactions the fetch asked
/// for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedAccount {
    /// Provider account id.
    pub id: String,
    /// The account's name, sanitized.
    pub name: String,
    /// The connection it belongs to (v2 only).
    pub conn_id: Option<String>,
    /// v2 connection name / v1 `org.name`, sanitized.
    pub org_name: Option<String>,
    /// The institution's domain or URL, for "Fix at SimpleFIN".
    pub org_url: Option<String>,
    /// The account's currency.
    pub currency: FeedCurrency,
    /// Decimal text exactly as sent. **Empty** when the provider sent none or
    /// sent something that isn't a decimal; that is also reported as an
    /// [`ProblemKind::Other`] problem.
    pub balance: String,
    /// Decimal text exactly as sent, when present and valid.
    pub available_balance: Option<String>,
    /// When `balance` was true, in epoch seconds.
    pub balance_date: Option<i64>,
    /// Transactions in the order sent. Every `amount` is valid decimal text.
    pub transactions: Vec<FeedTxn>,
}

/// An account's currency as the provider names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedCurrency {
    /// An ISO 4217 code such as `USD`.
    Code(String),
    /// A SimpleFIN custom currency: a URL describing it (points, miles). The
    /// server fetches it only from the connection's own host (plan §7).
    Url(String),
}

/// One transaction as the provider sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedTxn {
    /// Provider transaction id, unique only **within its account**.
    pub id: String,
    /// When it posted, epoch seconds; `0` = not posted.
    pub posted: i64,
    /// Decimal text, `+` = money in. Always valid: rows whose amount isn't a
    /// decimal are dropped while parsing and reported.
    pub amount: String,
    /// The bank's description, as sent.
    pub description: String,
    /// A cleaner merchant name, when the provider has one.
    pub payee: Option<String>,
    /// Extra text, when the provider has any.
    pub memo: Option<String>,
    /// When the purchase happened, epoch seconds.
    pub transacted_at: Option<i64>,
    /// Still pending at the bank.
    pub pending: bool,
}

/// What kind of trouble a [`FeedProblem`] describes.
///
/// The SimpleFIN v2 codes map one-to-one (`gen.auth` → [`GenAuth`](Self::GenAuth)
/// and so on); v1 sends only strings, which are classified by their text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProblemKind {
    /// Any other `gen.*`, or a v1 string we can't classify.
    Gen,
    /// `gen.api`: the request was adjusted (for example the 90-day cap).
    GenApi,
    /// `gen.auth`: the provider refused the credentials.
    GenAuth,
    /// `con.auth`: the bank login needs fixing on the provider's site.
    ConAuth,
    /// Any other `act.*`.
    Act,
    /// `act.failed`: one account couldn't be fetched.
    ActFailed,
    /// `act.missingdata`: one account came back incomplete.
    ActMissingData,
    /// v1 text "… may need attention": the v1 spelling of `con.auth`.
    NeedsAttention,
    /// A quota warning: auto-sync must stop for the day.
    Quota,
    /// Anything else, including rows this crate dropped as unreadable.
    Other,
}

/// One problem, from the provider or from reading its response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedProblem {
    /// What kind of problem.
    pub kind: ProblemKind,
    /// Raw code (`con.auth`), sanitized; `""` for v1 strings and for problems
    /// this crate found itself.
    pub code: String,
    /// Sanitized: control characters stripped, at most [`MAX_TEXT_CHARS`]
    /// characters.
    pub message: String,
    /// The connection it concerns, when known.
    pub conn_id: Option<String>,
    /// The account it concerns, when known.
    pub account_id: Option<String>,
}

/// The most characters [`sanitize`] keeps.
pub const MAX_TEXT_CHARS: usize = 500;

/// `text` made safe to show: whitespace controls (newline, tab) become a
/// space, every other control or bidi-override character is removed, runs of
/// whitespace collapse to one space, the ends are trimmed, and at most
/// [`MAX_TEXT_CHARS`] characters are kept.
///
/// Bidi overrides (`U+202E` and friends) are removed because a server string
/// can use them to make text read differently from what it is.
#[must_use]
pub fn sanitize(text: &str) -> String {
    clean_text(text).chars().take(MAX_TEXT_CHARS).collect()
}

/// [`sanitize`] without the length cap: what transaction text gets before it
/// goes into a CSV, where a newline would end the journal entry it lands in.
#[must_use]
pub(crate) fn clean_text(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_whitespace() {
                Some(' ')
            } else if c.is_control() || is_bidi_control(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The invisible characters that change how surrounding text is ordered.
const fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Whether `text` is a plain decimal as SimpleFIN sends it: an optional `-`,
/// digits, and optionally `.` and more digits — and fits the crate's
/// [`Dec`](crate::Dec).
///
/// Deliberately stricter than [`Dec::parse`](crate::Dec::parse), which (like
/// hledger) reads `1,000.00` as a thousand. A feed amount with a comma is
/// ambiguous between locales, and money that might be read a thousand times too
/// big is dropped and reported instead.
#[must_use]
pub fn is_decimal_text(text: &str) -> bool {
    let body = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = match body.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (body, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    digits(whole) && fraction.is_none_or(digits) && crate::Dec::parse(text, '.').is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_controls_and_bidi_and_caps_length() {
        assert_eq!(sanitize("  a\n\tb\u{0007}c\u{202E}d  "), "a bcd");
        assert_eq!(sanitize("x\u{0000}y"), "xy");
        assert_eq!(sanitize(&"é".repeat(600)).chars().count(), MAX_TEXT_CHARS);
        assert_eq!(sanitize(""), "");
    }

    #[test]
    fn clean_text_keeps_length_and_collapses_whitespace() {
        let long = "word ".repeat(200);
        assert_eq!(clean_text(&long).len(), long.trim_end().len());
        assert_eq!(clean_text("a\r\n\r\nb"), "a b");
    }

    #[test]
    fn decimal_text_is_strict() {
        for good in ["5", "-0.1", "100.23", "-33293.43", "0", "0.00"] {
            assert!(is_decimal_text(good), "{good}");
        }
        for bad in [
            "",
            "-",
            "1,000.00",
            "1.",
            ".5",
            "+5",
            "1e3",
            " 5",
            "5 ",
            "1.2.3",
            "abc",
            "--1",
            "99999999999999999999999999999999999999999999",
        ] {
            assert!(!is_decimal_text(bad), "{bad}");
        }
    }
}
