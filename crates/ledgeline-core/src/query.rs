//! hledger's query language, as an `= QUERY` auto-posting rule reads it: does
//! this query match this POSTING (of this transaction)?
//!
//! # What hledger 1.52 does, and therefore what this does
//!
//! Each fact below was checked by driving `hledger print --auto` over a
//! scratch journal, one query at a time (`plans/30-auto-postings-and-period-grammar.md`):
//!
//! - A bare word is `acct:` — a **case-insensitive, infix** regular
//!   expression on the account name. `Food` matches `expenses:food` and
//!   `(budget:food)`; `^exp.*d$` matches `expenses:food`.
//! - `desc:`, `payee:` (the description before `|`), `note:` (after it, or the
//!   whole description), `code:` — the same kind of match on the transaction.
//! - `tag:NAME[=VALUE]` — both halves infix regexes, over the posting's own
//!   tags, its transaction's, and any the caller passes as inherited (account
//!   and commodity tags). Hidden tags count: `tag:_generated-transaction` is
//!   how a rule limits itself to forecast transactions.
//! - `amt:N`, `amt:<N`, `amt:<=N`, `amt:>N`, `amt:>=N` — a signed comparison
//!   when `N` is written with a sign (or is zero), otherwise a comparison of
//!   MAGNITUDES. A posting holding more than one commodity matches when any
//!   one of its amounts does (not "always", as hledger's manual says); an
//!   amountless posting compares as zero.
//! - `cur:` — a regex that must match the WHOLE commodity symbol.
//! - `status:` / `status:!` / `status:*`, `depth:N`, `date:PERIOD`.
//! - `real:` and `real:1` select real postings; any other value (`real:0`,
//!   but also `real:yes`, `real:true`) selects virtual ones.
//! - `not:` negates any term.
//! - Several terms: the positive `desc:` terms are ORed, the positive account
//!   terms are ORed, the positive `status:` terms are ORed, and those groups
//!   and every other term (each `not:` included) are ANDed.
//!
//! # What is refused, and how
//!
//! `expr:`/`any:`/`all:` boolean queries, `date2:`, `type:`, open-ended dates
//! and a regex the `regex` crate cannot compile are not guessed at. The query
//! is [`Query::unsupported`] and says why; a rule holding one is kept, shown
//! and never fires — the same "still parsed, still carried, locked" contract
//! plan 21 set for period expressions. A query this module misread would
//! silently move money, which is worse than a rule that visibly does nothing.

use crate::decimal::Dec;
use crate::model::{Posting, PostingType, Status, Transaction};
use regex::{Regex, RegexBuilder};
use std::cmp::Ordering;

/// The largest compiled program one query term may build. A query is typed
/// into a journal file; a pattern that wants more than this is not one.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// A parsed hledger query.
///
/// Equality is equality of the text it was parsed from: two queries written
/// the same way ARE the same query, and a compiled regex has no equality of
/// its own.
#[derive(Debug, Clone)]
pub struct Query {
    raw: String,
    compiled: Result<Compiled, String>,
}

impl PartialEq for Query {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl Eq for Query {}

#[derive(Debug, Clone)]
struct Compiled {
    /// Positive `desc:` terms, ORed.
    descriptions: Vec<Regex>,
    /// Positive account terms (bare words and `acct:`), ORed.
    accounts: Vec<Regex>,
    /// Positive `status:` terms, ORed.
    statuses: Vec<Status>,
    /// Every other term, each negated or not, ANDed.
    others: Vec<(bool, Term)>,
}

#[derive(Debug, Clone)]
enum Term {
    Acct(Regex),
    Desc(Regex),
    Payee(Regex),
    Note(Regex),
    Code(Regex),
    Tag(Regex, Option<Regex>),
    Amt(Ordering, bool, Dec),
    AmtOr(Ordering, Ordering, bool, Dec),
    Cur(Regex),
    Status(Status),
    Real(bool),
    Depth(usize),
    Date(Option<String>, Option<String>),
}

impl Query {
    /// Parse `raw`. Infallible: a query this module cannot read is
    /// [`unsupported`](Self::unsupported), with the reason.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim().to_string();
        let compiled = compile(&raw);
        Self { raw, compiled }
    }

    /// The query as written.
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Why this query cannot be evaluated, or `None` when it can.
    #[must_use]
    pub fn unsupported(&self) -> Option<&str> {
        self.compiled.as_ref().err().map(String::as_str)
    }

    /// Whether this query matches `posting` of `txn`. `inherited` is the tags
    /// the posting inherits beyond its own and its transaction's (its
    /// account's and commodity's). An unsupported query matches nothing.
    #[must_use]
    pub fn matches(
        &self,
        txn: &Transaction,
        posting: &Posting,
        inherited: &[(String, String)],
    ) -> bool {
        let Ok(compiled) = &self.compiled else {
            return false;
        };
        let any = |patterns: &[Regex], text: &str| {
            patterns.is_empty() || patterns.iter().any(|p| p.is_match(text))
        };
        any(&compiled.descriptions, &txn.description)
            && any(&compiled.accounts, &posting.account.0)
            && (compiled.statuses.is_empty()
                || compiled.statuses.contains(&effective_status(txn, posting)))
            && compiled
                .others
                .iter()
                .all(|(negated, term)| term_matches(term, txn, posting, inherited) != *negated)
    }
}

/// A posting's status, or its transaction's when it has none of its own.
fn effective_status(txn: &Transaction, posting: &Posting) -> Status {
    if posting.status == Status::Unmarked {
        txn.status
    } else {
        posting.status
    }
}

fn term_matches(
    term: &Term,
    txn: &Transaction,
    posting: &Posting,
    inherited: &[(String, String)],
) -> bool {
    match term {
        Term::Acct(pattern) => pattern.is_match(&posting.account.0),
        Term::Desc(pattern) => pattern.is_match(&txn.description),
        Term::Payee(pattern) => pattern.is_match(payee(&txn.description)),
        Term::Note(pattern) => pattern.is_match(note(&txn.description)),
        Term::Code(pattern) => pattern.is_match(&txn.code),
        Term::Tag(name, value) => {
            posting
                .tags
                .iter()
                .chain(&txn.tags)
                .chain(inherited)
                .any(|(tag, tag_value)| {
                    name.is_match(tag) && value.as_ref().is_none_or(|v| v.is_match(tag_value))
                })
        }
        Term::Amt(ordering, signed, n) => amount_matches(posting, *signed, *n, |o| o == *ordering),
        Term::AmtOr(a, b, signed, n) => {
            amount_matches(posting, *signed, *n, |o| o == *a || o == *b)
        }
        Term::Cur(pattern) => posting
            .amounts
            .iter()
            .any(|amount| pattern.is_match(&amount.commodity.0)),
        Term::Status(status) => effective_status(txn, posting) == *status,
        Term::Real(real) => (posting.ptype == PostingType::Regular) == *real,
        Term::Depth(depth) => posting.account.0.split(':').count() <= *depth,
        Term::Date(start, end) => {
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            start.as_deref().is_none_or(|s| date >= s) && end.as_deref().is_none_or(|e| date < e)
        }
    }
}

/// `amt:` — each of the posting's amounts compared against `n`, signed or by
/// magnitude; the term matches when ANY of them is accepted. An amountless
/// posting compares as zero. (hledger's manual says a multi-commodity
/// posting always matches; hledger 1.52 does this instead.)
fn amount_matches(
    posting: &Posting,
    signed: bool,
    n: Dec,
    accept: impl Fn(Ordering) -> bool,
) -> bool {
    let amountless = posting.amounts.is_empty().then(Dec::zero);
    let mut quantities = posting
        .amounts
        .iter()
        .map(|amount| amount.quantity)
        .chain(amountless);
    quantities.any(|quantity| {
        let compared = if signed { Ok(quantity) } else { quantity.abs() };
        compared.is_ok_and(|q| accept(q.cmp(&n)))
    })
}

/// The description before `|`, trimmed; the whole description when there is
/// no `|`.
fn payee(description: &str) -> &str {
    description
        .split_once('|')
        .map_or(description, |(payee, _)| payee.trim())
}

/// The description after `|`, trimmed; the whole description when there is
/// no `|`.
fn note(description: &str) -> &str {
    description
        .split_once('|')
        .map_or(description, |(_, note)| note.trim())
}

fn compile(raw: &str) -> Result<Compiled, String> {
    let mut compiled = Compiled {
        descriptions: Vec::new(),
        accounts: Vec::new(),
        statuses: Vec::new(),
        others: Vec::new(),
    };
    for token in tokens(raw)? {
        let (negated, term) = term(&token)?;
        match (negated, term) {
            (false, Term::Desc(pattern)) => compiled.descriptions.push(pattern),
            (false, Term::Acct(pattern)) => compiled.accounts.push(pattern),
            (false, Term::Status(status)) => compiled.statuses.push(status),
            other => compiled.others.push(other),
        }
    }
    Ok(compiled)
}

/// Split a query into terms on whitespace, honouring single and double
/// quotes anywhere in a term (`'desc:team lunch'`, `desc:"team lunch"`). The
/// quotes themselves are dropped.
fn tokens(raw: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in raw.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("a quote in the query is never closed".to_string());
    }
    if started {
        out.push(current);
    }
    Ok(out)
}

/// One term, with its `not:` prefixes resolved.
fn term(token: &str) -> Result<(bool, Term), String> {
    let mut negated = false;
    let mut rest = token;
    while let Some(after) = rest.strip_prefix("not:") {
        negated = !negated;
        rest = after;
    }
    let (prefix, value) = rest.split_once(':').unwrap_or(("", rest));
    let term = match prefix {
        "acct" => Term::Acct(pattern(value)?),
        "desc" => Term::Desc(pattern(value)?),
        "payee" => Term::Payee(pattern(value)?),
        "note" => Term::Note(pattern(value)?),
        "code" => Term::Code(pattern(value)?),
        "cur" => Term::Cur(pattern(&format!("^(?:{value})$"))?),
        "tag" => match value.split_once('=') {
            Some((name, tag_value)) => Term::Tag(pattern(name)?, Some(pattern(tag_value)?)),
            None => Term::Tag(pattern(value)?, None),
        },
        "amt" => amount_term(value)?,
        "status" => Term::Status(match value {
            "" => Status::Unmarked,
            "!" => Status::Pending,
            "*" => Status::Cleared,
            other => return Err(format!("`status:{other}` is not a status hledger knows")),
        }),
        // hledger's parseBool: only `1` or nothing is true; `yes`, `true`
        // and every other spelling mean virtual.
        "real" => Term::Real(matches!(value, "" | "1")),
        "depth" => Term::Depth(
            value
                .parse()
                .map_err(|_| format!("`depth:{value}` is not a depth this engine reads"))?,
        ),
        "date" => {
            let (start, end) = crate::parse::date_query_bounds(value)
                .ok_or_else(|| format!("`date:{value}` is not a period this engine reads"))?;
            Term::Date(start, end)
        }
        "date2" | "type" | "expr" | "any" | "all" => {
            return Err(format!("`{prefix}:` queries are not supported yet"));
        }
        // Not a query prefix: an account name with colons in it, which is the
        // common case (`expenses:people-costs`).
        _ => Term::Acct(pattern(rest)?),
    };
    Ok((negated, term))
}

/// `amt:` — `[<|<=|>|>=|=]N`, signed when `N` is written with a sign or is
/// zero.
fn amount_term(value: &str) -> Result<Term, String> {
    let (operator, number) = ["<=", ">=", "<", ">", "="]
        .iter()
        .find_map(|op| value.strip_prefix(op).map(|rest| (*op, rest)))
        .unwrap_or(("=", value));
    let number = number.trim();
    let n = Dec::parse(number, '.')
        .map_err(|_| format!("`amt:{value}` is not a number this engine reads"))?;
    let signed = number.starts_with(['+', '-']) || n == Dec::new(0, 0);
    Ok(match operator {
        "<" => Term::Amt(Ordering::Less, signed, n),
        ">" => Term::Amt(Ordering::Greater, signed, n),
        "<=" => Term::AmtOr(Ordering::Less, Ordering::Equal, signed, n),
        ">=" => Term::AmtOr(Ordering::Greater, Ordering::Equal, signed, n),
        _ => Term::Amt(Ordering::Equal, signed, n),
    })
}

/// A case-insensitive infix regex, as every hledger query pattern is.
fn pattern(text: &str) -> Result<Regex, String> {
    RegexBuilder::new(text)
        .case_insensitive(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|_| format!("`{text}` is not a regular expression this engine reads"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;

    /// The two transactions every case below runs against, as the hledger
    /// check that established each expectation used them.
    const JOURNAL: &str = "\
2026-01-01 * (42) acme | team lunch  ; trip: paris
    expenses:food   $60
    assets:checking

2026-01-02 dinner
    expenses:rent   $40  ; trip: rome
    expenses:food   10 EUR
    (budget:food)   $-5
    assets:checking   $-40
    assets:checking   -10 EUR
";

    /// `date account` for every posting `query` matches, in journal order.
    fn matched(query: &str) -> Vec<String> {
        matched_in(JOURNAL, query)
    }

    /// [`matched`], over `text` instead of [`JOURNAL`].
    fn matched_in(text: &str, query: &str) -> Vec<String> {
        let journal = parse_journal(text, "q.journal").expect("parses");
        let query = Query::parse(query);
        assert_eq!(query.unsupported(), None, "{query:?}");
        journal
            .transactions
            .iter()
            .flat_map(|txn| {
                let query = &query;
                txn.postings
                    .iter()
                    .filter(move |posting| query.matches(txn, posting, &[]))
                    .map(move |posting| format!("{} {}", &txn.date[8..], posting.account.0))
            })
            .collect()
    }

    /// Every expectation is `hledger print --auto`'s, one query at a time
    /// (2026-10-28).
    #[test]
    fn each_query_matches_the_postings_hledger_matches() {
        let cases: &[(&str, &[&str])] = &[
            (
                "expenses not:rent",
                &["01 expenses:food", "02 expenses:food"],
            ),
            (
                "acct:food acct:rent",
                &[
                    "01 expenses:food",
                    "02 expenses:rent",
                    "02 expenses:food",
                    "02 budget:food",
                ],
            ),
            ("desc:lunch acct:food", &["01 expenses:food"]),
            (
                "Food",
                &["01 expenses:food", "02 expenses:food", "02 budget:food"],
            ),
            ("^exp.*d$", &["01 expenses:food", "02 expenses:food"]),
            ("amt:>50", &["01 expenses:food", "01 assets:checking"]),
            ("amt:50", &[]),
            ("amt:<-39", &["01 assets:checking", "02 assets:checking"]),
            ("amt:<=-40", &["01 assets:checking", "02 assets:checking"]),
            (
                r"cur:\$",
                &[
                    "01 expenses:food",
                    "01 assets:checking",
                    "02 expenses:rent",
                    "02 budget:food",
                    "02 assets:checking",
                ],
            ),
            ("cur:EU", &[]),
            (
                "tag:trip",
                &["01 expenses:food", "01 assets:checking", "02 expenses:rent"],
            ),
            ("tag:trip=par", &["01 expenses:food", "01 assets:checking"]),
            ("not:tag:trip expenses", &["02 expenses:food"]),
            ("payee:acme", &["01 expenses:food", "01 assets:checking"]),
            ("note:lunch", &["01 expenses:food", "01 assets:checking"]),
            ("real:0", &["02 budget:food"]),
            (
                "real:",
                &[
                    "01 expenses:food",
                    "01 assets:checking",
                    "02 expenses:rent",
                    "02 expenses:food",
                    "02 assets:checking",
                    "02 assets:checking",
                ],
            ),
            ("not:real:1", &["02 budget:food"]),
            ("status:*", &["01 expenses:food", "01 assets:checking"]),
            ("code:42", &["01 expenses:food", "01 assets:checking"]),
            (
                "'desc:team lunch'",
                &["01 expenses:food", "01 assets:checking"],
            ),
            (
                "\"desc:team lunch\"",
                &["01 expenses:food", "01 assets:checking"],
            ),
            (
                "not:acct:assets not:acct:budget",
                &["01 expenses:food", "02 expenses:rent", "02 expenses:food"],
            ),
            ("not:desc:dinner expenses", &["01 expenses:food"]),
            ("acct:food not:food", &[]),
            (
                "date:2026-01-02",
                &[
                    "02 expenses:rent",
                    "02 expenses:food",
                    "02 budget:food",
                    "02 assets:checking",
                    "02 assets:checking",
                ],
            ),
        ];
        for (query, want) in cases {
            assert_eq!(matched(query), *want, "`{query}`");
        }
    }

    /// `real:` reads its value as hledger's parseBool does: only `1` or
    /// nothing is true, so `real:yes` selects VIRTUAL postings (hledger 1.52
    /// `print --auto`, each spelling and its `not:` form, 2026-10-28).
    #[test]
    fn real_yes_and_other_spellings_select_virtual_postings_as_hledger_does() {
        let real = matched("real:1");
        let virtual_only: &[&str] = &["02 budget:food"];
        assert_eq!(matched("real:"), real);
        for value in [
            "0", "yes", "true", "y", "Y", "True", "no", "false", "01", "t", "2",
        ] {
            assert_eq!(
                matched(&format!("real:{value}")),
                virtual_only,
                "`real:{value}`"
            );
            assert_eq!(
                matched(&format!("not:real:{value}")),
                real,
                "`not:real:{value}`"
            );
        }
    }

    /// A leg whose amount is inferred in two commodities (`$-10, -5 EUR`) and
    /// a lone amountless leg: the cases [`JOURNAL`] has none of.
    const MULTI_COMMODITY_JOURNAL: &str = "\
2026-01-01 multi
    expenses:a   $10
    expenses:b   5 EUR
    assets:checking

2026-01-02 empty
    (virtual:x)
";

    /// `amt:` on a multi-commodity leg matches when SOME commodity's amount
    /// does, and an amountless leg compares as zero; it does not "always
    /// match" as hledger's manual says. Every expectation is `hledger print
    /// --auto`'s over [`MULTI_COMMODITY_JOURNAL`] (hledger 1.52, 2026-10-28).
    #[test]
    fn amt_on_a_multi_commodity_or_amountless_leg_matches_as_hledger_does() {
        let cases: &[(&str, &[&str])] = &[
            ("amt:>60", &[]),
            ("amt:<-20", &[]),
            (
                "amt:>-3",
                &["01 expenses:a", "01 expenses:b", "02 virtual:x"],
            ),
            ("amt:7", &[]),
            (
                "not:amt:>60",
                &[
                    "01 expenses:a",
                    "01 expenses:b",
                    "01 assets:checking",
                    "02 virtual:x",
                ],
            ),
            ("amt:0", &["02 virtual:x"]),
            ("amt:<1", &["02 virtual:x"]),
            ("amt:<=0", &["01 assets:checking", "02 virtual:x"]),
            ("amt:10", &["01 expenses:a", "01 assets:checking"]),
            ("amt:5", &["01 expenses:b", "01 assets:checking"]),
            ("amt:-5", &["01 assets:checking"]),
            ("amt:<-6", &["01 assets:checking"]),
            (
                "not:amt:<-6",
                &["01 expenses:a", "01 expenses:b", "02 virtual:x"],
            ),
        ];
        for (query, want) in cases {
            assert_eq!(
                matched_in(MULTI_COMMODITY_JOURNAL, query),
                *want,
                "`{query}`"
            );
        }
    }

    /// The two forms this commit does not evaluate are refused by name rather
    /// than guessed at, and an unreadable regex is refused too. None of them
    /// ever matches.
    #[test]
    fn a_query_this_engine_cannot_evaluate_says_why_and_matches_nothing() {
        for query in [
            "expr:'food and not lunch'",
            "type:X",
            "date2:2026",
            "(unclosed",
            "'open quote",
        ] {
            let parsed = Query::parse(query);
            assert!(parsed.unsupported().is_some(), "`{query}` is refused");
            let journal = parse_journal(JOURNAL, "q.journal").expect("parses");
            let txn = &journal.transactions[0];
            assert!(!parsed.matches(txn, &txn.postings[0], &[]));
        }
    }

    /// The forecast-only idiom: hidden tags are tags.
    #[test]
    fn a_hidden_tag_matches_like_any_other() {
        let journal = parse_journal(
            "2027-01-01 forecast  ; _generated-transaction: ~ monthly\n    revenue:sales  $-100\n    assets:checking\n",
            "g.journal",
        )
        .expect("parses");
        let txn = &journal.transactions[0];
        let query = Query::parse("revenue:sales tag:_generated-transaction");
        assert!(query.matches(txn, &txn.postings[0], &[]));
        assert!(!query.matches(txn, &txn.postings[1], &[]));
    }

    /// A rule on a parent account fires for its CHILD accounts too: account
    /// queries are infix (2026-10-28, the user's payroll-tax rule on
    /// per-person wage accounts). Infix also reaches a sibling that merely
    /// starts with the same name, as in hledger; an anchored query keeps to
    /// the subtree.
    #[test]
    fn an_account_query_matches_the_account_and_its_children() {
        let journal = parse_journal(
            "2027-01-01 payroll\n\
             \x20   expenses:people-costs:wages-salaries:george   $6000\n\
             \x20   expenses:people-costs:wages-salaries:john     $5000\n\
             \x20   expenses:people-costs:wages-salaries          $1000\n\
             \x20   expenses:people-costs:wages-salaries2          $100\n\
             \x20   expenses:people-costs:payroll-tax              $10\n\
             \x20   assets:mercury:checking\n",
            "w.journal",
        )
        .expect("parses");
        let txn = &journal.transactions[0];
        let hits = |query: &str| -> Vec<&str> {
            let query = Query::parse(query);
            txn.postings
                .iter()
                .filter(|posting| query.matches(txn, posting, &[]))
                .map(|posting| posting.account.0.as_str())
                .collect()
        };
        assert_eq!(
            hits("expenses:people-costs:wages-salaries"),
            [
                "expenses:people-costs:wages-salaries:george",
                "expenses:people-costs:wages-salaries:john",
                "expenses:people-costs:wages-salaries",
                "expenses:people-costs:wages-salaries2",
            ]
        );
        assert_eq!(
            hits("^expenses:people-costs:wages-salaries(:|$)"),
            [
                "expenses:people-costs:wages-salaries:george",
                "expenses:people-costs:wages-salaries:john",
                "expenses:people-costs:wages-salaries",
            ]
        );
    }
}
