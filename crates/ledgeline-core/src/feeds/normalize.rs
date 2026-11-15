//! One [`FeedAccount`] as the CSV its sync rules file reads (plan 43 §3.2).
//!
//! Sync doesn't import anything itself. It writes a CSV with fixed columns and
//! hands it to the existing Pipeline A (`hledger import` + the account's rules
//! file + the id index), so a sync and a manual import of the same CSV give
//! the same journal text, and `hledger import` from a terminal does too.
//!
//! What this step decides:
//!
//! - **Pending rows never get in.** `pending: true` or `posted: 0` is dropped,
//!   which sidesteps SimpleFIN's pending/posted id mismatch at the cost of a
//!   day or two of delay.
//! - **The date** is the posted date. An epoch at exactly midnight UTC is a
//!   date the bank meant (that UTC date); anything else is a moment, read at
//!   the user's offset. `transacted` (hledger's secondary date) is filled only
//!   when it falls on a different day.
//! - **Ids** are prefixed with [`account_short`], because SimpleFIN ids are
//!   unique only within an account, and made tag-safe exactly as OFX `FITID`s
//!   are.
//! - **Zero amounts** are dropped, and so is anything before the account's
//!   sync start date, which protects history imported by hand without ids.

use super::{FeedAccount, FeedTxn, clean_text, is_decimal_text};
use crate::convert::ofx::tag_safe;
use crate::reports::periods::iso_from_days;
use std::collections::HashSet;

/// The sync CSV's header names, in column order. The defaults are shown on
/// each field; rules sharing (plan 43 §3.2) may rename them to borrow a manual
/// rules file's names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedColumns {
    /// `"date"`.
    pub date: String,
    /// `"amount"`.
    pub amount: String,
    /// `"payee"`.
    pub payee: String,
    /// `"description"`.
    pub description: String,
    /// `"memo"`.
    pub memo: String,
    /// `"id"`.
    pub id: String,
    /// `"transacted"`.
    pub transacted: String,
}

impl Default for FeedColumns {
    fn default() -> Self {
        Self {
            date: "date".into(),
            amount: "amount".into(),
            payee: "payee".into(),
            description: "description".into(),
            memo: "memo".into(),
            id: "id".into(),
            transacted: "transacted".into(),
        }
    }
}

impl FeedColumns {
    /// The names in column order: the order of the CSV header and of the
    /// rules file's `fields` line.
    #[must_use]
    pub fn in_order(&self) -> [&str; 7] {
        [
            &self.date,
            &self.amount,
            &self.payee,
            &self.description,
            &self.memo,
            &self.id,
            &self.transacted,
        ]
    }
}

/// How to normalize one account.
#[derive(Debug, Clone, Copy)]
pub struct NormalizeOptions<'a> {
    /// ISO date; rows dated before it are dropped. `""` drops nothing.
    pub start_date: &'a str,
    /// The id prefix, from [`account_short`].
    pub account_short: &'a str,
    /// The header names. Normalization doesn't read them; they travel with the
    /// options so the caller hands one value to both this and [`to_csv`].
    pub columns: &'a FeedColumns,
    /// The user's UTC offset in seconds, for epochs that aren't midnight UTC.
    pub utc_offset_seconds: i32,
}

/// One CSV row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NormRow {
    /// Posted date, ISO.
    pub date: String,
    /// Decimal text as the provider sent it.
    pub amount: String,
    /// The payee, or the description when the provider had none.
    pub payee: String,
    /// The bank's description.
    pub description: String,
    /// Extra text, or `""`.
    pub memo: String,
    /// `{account_short}-{tag-safe provider id}`.
    pub id: String,
    /// The transacted date, ISO, or `""` when it is the same day as `date`.
    pub transacted: String,
}

/// The rows, and how many were dropped for each reason.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Normalized {
    /// Sorted by `(date, id)`.
    pub rows: Vec<NormRow>,
    /// `pending: true` or `posted: 0`.
    pub dropped_pending: usize,
    /// Dated before the start date.
    pub dropped_before_start: usize,
    /// Amount exactly zero.
    pub dropped_zero: usize,
    /// An amount that isn't a plain decimal, or an id that is empty once made
    /// tag-safe. [`parse_accounts`](super::simplefin::parse_accounts) already
    /// drops and reports such rows, so this is non-zero only for a
    /// [`FeedAccount`] built some other way.
    pub dropped_invalid: usize,
}

/// Why a transaction didn't become a row.
enum Drop {
    Pending,
    BeforeStart,
    Zero,
    Invalid,
}

/// Normalize `account`'s transactions into CSV rows.
#[must_use]
pub fn normalize(account: &FeedAccount, opts: &NormalizeOptions) -> Normalized {
    let mut out = account.transactions.iter().map(|txn| row(txn, opts)).fold(
        Normalized::default(),
        |mut out, result| {
            match result {
                Ok(row) => out.rows.push(row),
                Err(Drop::Pending) => out.dropped_pending += 1,
                Err(Drop::BeforeStart) => out.dropped_before_start += 1,
                Err(Drop::Zero) => out.dropped_zero += 1,
                Err(Drop::Invalid) => out.dropped_invalid += 1,
            }
            out
        },
    );
    out.rows
        .sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.id.cmp(&b.id)));
    out
}

fn row(txn: &FeedTxn, opts: &NormalizeOptions) -> Result<NormRow, Drop> {
    if txn.pending || txn.posted == 0 {
        return Err(Drop::Pending);
    }
    let date = epoch_to_date(txn.posted, opts.utc_offset_seconds);
    if date.as_str() < opts.start_date {
        return Err(Drop::BeforeStart);
    }
    if !is_decimal_text(&txn.amount) {
        return Err(Drop::Invalid);
    }
    if crate::Dec::parse(&txn.amount, '.').is_ok_and(|amount| amount.is_zero()) {
        return Err(Drop::Zero);
    }
    let safe_id = tag_safe(&txn.id);
    if safe_id.is_empty() {
        return Err(Drop::Invalid);
    }
    let transacted = txn
        .transacted_at
        .map(|at| epoch_to_date(at, opts.utc_offset_seconds))
        .filter(|day| *day != date)
        .unwrap_or_default();
    let description = clean_text(&txn.description);
    let payee = txn
        .payee
        .as_deref()
        .map(clean_text)
        .filter(|payee| !payee.is_empty())
        .unwrap_or_else(|| description.clone());
    Ok(NormRow {
        date,
        amount: txn.amount.clone(),
        payee,
        description,
        memo: txn.memo.as_deref().map(clean_text).unwrap_or_default(),
        id: format!("{}-{safe_id}", opts.account_short),
        transacted,
    })
}

/// The id a row from transaction `txn_id` gets: `{account_short}-{tag_safe}`.
#[must_use]
pub fn prefixed_id(account_short: &str, txn_id: &str) -> String {
    format!("{account_short}-{}", tag_safe(txn_id))
}

/// Every transaction's [`prefixed_id`] in `account`, **before** any filtering
/// (pending, start date, zero): the `fetched_ids` that
/// [`dupes::split`](super::dupes::split) needs.
#[must_use]
pub fn fetched_ids(account: &FeedAccount, account_short: &str) -> HashSet<String> {
    account
        .transactions
        .iter()
        .map(|txn| prefixed_id(account_short, &txn.id))
        .collect()
}

/// `rows` as CSV text: a header of `columns`' names, then one line per row,
/// `\n`-terminated. A field is quoted (RFC 4180, `"` doubled) when it holds a
/// comma, a quote, a line break, or leading/trailing whitespace.
#[must_use]
pub fn to_csv(rows: &[NormRow], columns: &FeedColumns) -> String {
    let line = |fields: [&str; 7]| {
        let mut line = fields.map(csv_field).join(",");
        line.push('\n');
        line
    };
    std::iter::once(line(columns.in_order()))
        .chain(rows.iter().map(|row| {
            line([
                &row.date,
                &row.amount,
                &row.payee,
                &row.description,
                &row.memo,
                &row.id,
                &row.transacted,
            ])
        }))
        .collect()
}

fn csv_field(field: &str) -> String {
    let needs_quotes = field.contains([',', '"', '\n', '\r'])
        || field.starts_with(char::is_whitespace)
        || field.ends_with(char::is_whitespace);
    if needs_quotes {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// FNV-1a, 64-bit. Written out rather than taken from a crate because
/// [`account_short`] must never change, and a dependency's hash could.
fn fnv1a_64(bytes: impl IntoIterator<Item = u8>) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.into_iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

/// The 10-lowercase-hex id prefix for a linked account.
///
/// **Stable forever:** it is written into journals as the start of every
/// synced `id:` tag, and it keys the account in `sync-state.json`. It is the
/// first 40 bits of FNV-1a 64 over `connection_id`, a NUL, then
/// `remote_account_id` (the NUL keeps `("ab","c")` and `("a","bc")` apart).
/// Never `DefaultHasher`, whose output may change between Rust releases.
#[must_use]
pub fn account_short(connection_id: &str, remote_account_id: &str) -> String {
    let bytes = connection_id
        .bytes()
        .chain(std::iter::once(0))
        .chain(remote_account_id.bytes());
    format!("{:016x}", fnv1a_64(bytes))[..10].to_string()
}

const SECONDS_PER_DAY: i64 = 86_400;

/// The ISO date of `epoch`: the UTC date when `epoch` is exactly midnight UTC
/// (a bank stating a date), otherwise the date at `utc_offset_seconds`.
#[must_use]
pub fn epoch_to_date(epoch: i64, utc_offset_seconds: i32) -> String {
    let local = if epoch.rem_euclid(SECONDS_PER_DAY) == 0 {
        epoch
    } else {
        epoch.saturating_add(i64::from(utc_offset_seconds))
    };
    iso_from_days(local.div_euclid(SECONDS_PER_DAY))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feeds::FeedCurrency;

    /// 2026-11-10T00:00:00Z.
    const MIDNIGHT: i64 = 1_794_268_800;
    const HOUR: i64 = 3_600;
    const PACIFIC: i32 = -8 * 3_600;

    fn txn(id: &str, posted: i64, amount: &str) -> FeedTxn {
        FeedTxn {
            id: id.into(),
            posted,
            amount: amount.into(),
            description: "COFFEE SHOP #42".into(),
            payee: None,
            memo: None,
            transacted_at: None,
            pending: false,
        }
    }

    fn account(transactions: Vec<FeedTxn>) -> FeedAccount {
        FeedAccount {
            id: "ACT-1".into(),
            name: "Checking".into(),
            conn_id: None,
            org_name: None,
            org_url: None,
            currency: FeedCurrency::Code("USD".into()),
            balance: "0".into(),
            available_balance: None,
            balance_date: None,
            transactions,
        }
    }

    fn run(transactions: Vec<FeedTxn>, start: &str) -> Normalized {
        let columns = FeedColumns::default();
        normalize(
            &account(transactions),
            &NormalizeOptions {
                start_date: start,
                account_short: "abcdef0123",
                columns: &columns,
                utc_offset_seconds: PACIFIC,
            },
        )
    }

    #[test]
    fn account_short_is_pinned_forever() {
        // If this fails, every synced id in every journal stops matching.
        // Don't update the expected values; restore the function.
        assert_eq!(account_short("CON-1", "ACT-1"), "5b803be016");
        assert_eq!(
            account_short("4f6c1a2e-0000-4000-8000-000000000001", "ACT-12ab"),
            "2562b58c16"
        );
        assert_eq!(account_short("", ""), "af63bd4c86");
    }

    #[test]
    fn account_short_is_ten_lowercase_hex_and_separates_its_inputs() {
        let short = account_short("conn", "acct");
        assert_eq!(short.len(), 10);
        assert!(
            short
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_ne!(account_short("ab", "c"), account_short("a", "bc"));
        assert_ne!(account_short("c1", "a"), account_short("c2", "a"));
    }

    #[test]
    fn midnight_utc_is_that_date_and_anything_else_is_local() {
        assert_eq!(epoch_to_date(MIDNIGHT, PACIFIC), "2026-11-10");
        // 03:00 UTC is still the 9th in California.
        assert_eq!(epoch_to_date(MIDNIGHT + 3 * HOUR, PACIFIC), "2026-11-09");
        assert_eq!(epoch_to_date(MIDNIGHT + 3 * HOUR, 0), "2026-11-10");
        // 23:00 UTC is the next day in Tokyo.
        assert_eq!(epoch_to_date(MIDNIGHT + 23 * HOUR, 9 * 3_600), "2026-11-11");
        assert_eq!(epoch_to_date(0, 0), "1970-01-01");
        assert_eq!(epoch_to_date(-1, 0), "1969-12-31");
    }

    #[test]
    fn extreme_epochs_do_not_panic() {
        for epoch in [i64::MIN, i64::MAX, i64::MAX - 1, i64::MIN + 1] {
            for offset in [i32::MIN, 0, i32::MAX] {
                let _ = epoch_to_date(epoch, offset);
            }
        }
    }

    #[test]
    fn pending_and_unposted_rows_are_dropped() {
        let pending = FeedTxn {
            pending: true,
            ..txn("P", MIDNIGHT, "-1.00")
        };
        let out = run(
            vec![pending, txn("U", 0, "-1.00"), txn("K", MIDNIGHT, "-1.00")],
            "",
        );
        assert_eq!(out.dropped_pending, 2);
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].id, "abcdef0123-K");
    }

    #[test]
    fn zero_amounts_and_rows_before_the_start_are_dropped() {
        let out = run(
            vec![
                txn("Z", MIDNIGHT, "0.00"),
                txn("Z2", MIDNIGHT, "-0"),
                txn("OLD", MIDNIGHT - 2 * SECONDS_PER_DAY, "-5"),
                txn("ON", MIDNIGHT - SECONDS_PER_DAY, "-5"),
            ],
            "2026-11-09",
        );
        assert_eq!(out.dropped_zero, 2);
        assert_eq!(out.dropped_before_start, 1);
        assert_eq!(
            out.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["abcdef0123-ON"]
        );
    }

    #[test]
    fn invalid_amounts_and_empty_ids_are_counted() {
        let out = run(
            vec![txn("A", MIDNIGHT, "1,000.00"), txn(" , ", MIDNIGHT, "-1")],
            "",
        );
        assert_eq!(out.dropped_invalid, 2);
        assert!(out.rows.is_empty());
    }

    #[test]
    fn transacted_is_filled_only_on_a_different_day() {
        let same = FeedTxn {
            transacted_at: Some(MIDNIGHT + 20 * HOUR),
            ..txn("S", MIDNIGHT + 22 * HOUR, "-1")
        };
        let earlier = FeedTxn {
            transacted_at: Some(MIDNIGHT - 30 * HOUR),
            ..txn("E", MIDNIGHT, "-1")
        };
        let out = run(vec![same, earlier], "");
        let by_id = |id: &str| {
            out.rows
                .iter()
                .find(|r| r.id.ends_with(id))
                .map(|r| (r.date.clone(), r.transacted.clone()))
        };
        assert_eq!(by_id("-S"), Some(("2026-11-10".into(), String::new())));
        assert_eq!(
            by_id("-E"),
            Some(("2026-11-10".into(), "2026-11-08".into()))
        );
    }

    #[test]
    fn ids_are_prefixed_and_tag_safe() {
        let out = run(vec![txn("TRN 1,234\n 5", MIDNIGHT, "-1")], "");
        assert_eq!(out.rows[0].id, "abcdef0123-TRN 1234 5");
        assert_eq!(prefixed_id("abcdef0123", "TRN 1,234\n 5"), out.rows[0].id);
    }

    #[test]
    fn payee_falls_back_to_description_and_text_is_cleaned() {
        let with_payee = FeedTxn {
            payee: Some("  Blue\nBottle ".into()),
            memo: Some("tab\there".into()),
            ..txn("A", MIDNIGHT, "-1")
        };
        let blank_payee = FeedTxn {
            payee: Some("   ".into()),
            ..txn("B", MIDNIGHT, "-2")
        };
        let out = run(vec![with_payee, blank_payee, txn("C", MIDNIGHT, "-3")], "");
        let payees: Vec<&str> = out.rows.iter().map(|r| r.payee.as_str()).collect();
        assert_eq!(
            payees,
            vec!["Blue Bottle", "COFFEE SHOP #42", "COFFEE SHOP #42"]
        );
        assert_eq!(out.rows[0].memo, "tab here");
        assert_eq!(out.rows[1].memo, "");
    }

    #[test]
    fn rows_sort_by_date_then_id() {
        let out = run(
            vec![
                txn("b", MIDNIGHT + SECONDS_PER_DAY, "-1"),
                txn("c", MIDNIGHT, "-1"),
                txn("a", MIDNIGHT + SECONDS_PER_DAY, "-1"),
            ],
            "",
        );
        let order: Vec<&str> = out.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(order, vec!["abcdef0123-c", "abcdef0123-a", "abcdef0123-b"]);
    }

    #[test]
    fn fetched_ids_cover_every_transaction_before_filtering() {
        let pending = FeedTxn {
            pending: true,
            ..txn("P", MIDNIGHT, "-1")
        };
        let ids = fetched_ids(&account(vec![pending, txn("Z", MIDNIGHT, "0")]), "s");
        assert_eq!(ids, HashSet::from(["s-P".to_string(), "s-Z".to_string()]));
    }

    #[test]
    fn csv_quotes_what_needs_quoting() {
        let row = NormRow {
            date: "2026-11-10".into(),
            amount: "-1.50".into(),
            payee: "Smith, Jones & Co".into(),
            description: "He said \"hi\"".into(),
            memo: "line\nbreak".into(),
            id: "abc-1".into(),
            transacted: String::new(),
        };
        let columns = FeedColumns {
            payee: "name".into(),
            ..FeedColumns::default()
        };
        assert_eq!(
            to_csv(&[row], &columns),
            "date,amount,name,description,memo,id,transacted\n\
             2026-11-10,-1.50,\"Smith, Jones & Co\",\"He said \"\"hi\"\"\",\"line\nbreak\",abc-1,\n"
        );
        assert_eq!(
            to_csv(&[], &FeedColumns::default()),
            "date,amount,payee,description,memo,id,transacted\n"
        );
        assert_eq!(csv_field(" lead"), "\" lead\"");
        assert_eq!(csv_field("plain"), "plain");
    }
}
