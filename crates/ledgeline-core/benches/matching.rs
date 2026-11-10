//! Criterion coverage of transfer matching (plan 35) on the plan-26 corpus,
//! with its transfers routed through a clearing account.
//!
//! The corpus writes each transfer as one transaction (`to $X` / `from`). Here
//! every one is split into the two halves two bank feeds would import, each
//! through `transfers:bank` (declared `clearing:`), in two variants:
//!
//! - **unlinked**: no half carries a `match` tag, so every half is a
//!   candidate — a first Find Matches run over an old journal, the worst case
//!   for [`matching::propose`];
//! - **half linked**: every other pair carries `match:ID` on both halves —
//!   the worst realistic case for the journal's collapsed rows;
//! - **bill pay**: unlinked, with every fourth transfer's incoming half
//!   arriving as two postings (two thirds and the rest), so a quarter of the
//!   transfers are many-to-one — the suggested-combination search
//!   (Amendment A4) at work, as on a bill-pay journal.
//!
//! | Bench | What it is | Paid |
//! |---|---|---|
//! | `matching/propose_all` | `GET /api/matches?lookback=0` | per request |
//! | `matching/propose_365` | `GET /api/matches` (default lookback) | per request |
//! | `matching/count_all` | `older`: the no-lookback count `GET /api/matches` adds | per request with a lookback |
//! | `matching/billpay_propose_all`, `…_365` | as above, with combinations to find | per request |
//! | `matching/billpay_partners_for` | `GET /api/matches/for/{index}` on a split debit | per request |
//! | `matching/in_transit` | the D10 Problems finding | per generation and date |
//! | `matching/clearing_accounts` | the accounts tagged `clearing:` | inside each of the above |
//! | `matching/links` | every `match` id, valid and broken | inside `transfers_of` |
//! | `rows/transfers_of` | [`Transfers::of`] | once per generation (prewarmed) |
//! | `rows/collapse` | every position → rows, no filter | per new filter (cached) |
//!
//! # Running
//!
//! ```text
//! nix develop -c cargo bench -p ledgeline-core --bench matching
//! LEDGELINE_BENCH_SIZES=200000 nix develop -c cargo bench -p ledgeline-core --bench matching
//! ```
//!
//! The variants are written beside the corpus cache
//! (`synthetic-…-matching-{unlinked,linked}.journal`) so the engine can be
//! pointed at them for end-to-end timings of the routes.

#[path = "corpus.rs"]
mod corpus;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ledgeline_core::matching::{self, MatchSettings};
use ledgeline_core::model::Journal;
use ledgeline_core::view::filter::display_order;
use ledgeline_core::view::rows::Transfers;
use std::fmt::Write as _;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The corpus's last transaction date: "today".
const TODAY: &str = "2025-12-31";
/// The clearing account every split transfer goes through.
const CLEARING: &str = "transfers:bank";

/// A 6-character base-36 id, unique per `n`.
fn match_id(n: usize) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut rest = n;
    let mut id = [b'0'; 6];
    for slot in id.iter_mut().rev() {
        *slot = DIGITS[rest % 36];
        rest /= 36;
    }
    String::from_utf8(id.to_vec()).expect("ascii")
}

/// `$1,234.56` → `$-1,234.56`.
fn negated(amount: &str) -> String {
    amount
        .strip_prefix('$')
        .map_or_else(|| format!("-{amount}"), |rest| format!("$-{rest}"))
}

/// `$1,234.56` → `123456` (cents); `None` for anything else.
fn cents(amount: &str) -> Option<i64> {
    let digits = amount.strip_prefix('$')?.replace(',', "");
    let (whole, fraction) = digits.split_once('.').unwrap_or((&digits, "00"));
    if fraction.len() != 2 {
        return None;
    }
    Some(whole.parse::<i64>().ok()? * 100 + fraction.parse::<i64>().ok()?)
}

/// `123456` → `$1234.56` (non-negative).
fn dollars(cents: i64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

/// The corpus with every transfer split into two halves through
/// [`CLEARING`]; pair `i` is linked when `linked(i)`.
fn split_transfers(text: &str, linked: impl Fn(usize) -> bool) -> String {
    split_transfers_with(text, linked, |_| false)
}

/// [`split_transfers`], with pair `i`'s incoming half written as two
/// transactions (two thirds and the rest) when `many(i)`.
fn split_transfers_with(
    text: &str,
    linked: impl Fn(usize) -> bool,
    many: impl Fn(usize) -> bool,
) -> String {
    let mut out = String::with_capacity(text.len() * 11 / 10);
    let _ = writeln!(out, "account transfers  ; clearing:\n");
    let mut pairs = 0usize;
    for block in text.split_inclusive("\n\n") {
        let mut lines = block.lines();
        let header = lines.next().unwrap_or_default();
        let note = header
            .split_once(" | ")
            .map(|(_, rest)| rest.split_once("  ;").map_or(rest, |(note, _)| note));
        let postings: Vec<&str> = lines.filter(|line| !line.trim().is_empty()).collect();
        let [to_line, from_line] = postings.as_slice() else {
            out.push_str(block);
            continue;
        };
        if note != Some("transfer") {
            out.push_str(block);
            continue;
        }
        let Some((to, amount)) = to_line.trim().split_once("  ") else {
            out.push_str(block);
            continue;
        };
        let (to, amount, from) = (to.trim(), amount.trim(), from_line.trim());
        let (head, _) = header.split_once(" | ").expect("checked above");
        let tag = if linked(pairs) {
            format!("  ; match:{}", match_id(pairs))
        } else {
            String::new()
        };
        let back = negated(amount);
        let _ = write!(
            out,
            "{head} | transfer out\n    {from}  {back}\n    {CLEARING}  {amount}{tag}\n\n"
        );
        match cents(amount).filter(|&total| many(pairs) && total >= 3) {
            Some(total) => {
                let first = total * 2 / 3;
                for part in [first, total - first] {
                    let part = dollars(part);
                    let back = negated(&part);
                    let _ = write!(
                        out,
                        "{head} | transfer in part\n    {to}  {part}\n    {CLEARING}  {back}\n\n"
                    );
                }
            }
            None => {
                let _ = write!(
                    out,
                    "{head} | transfer in\n    {to}  {amount}\n    {CLEARING}  {back}{tag}\n\n"
                );
            }
        }
        pairs += 1;
    }
    out
}

/// Write `text` beside the corpus as `<corpus stem>-matching-<name>.journal`
/// (only when it changed) and parse it.
fn variant(corpus: &Path, name: &str, text: &str) -> (PathBuf, Journal) {
    let stem = corpus
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .expect("corpus has a stem");
    let path = corpus.with_file_name(format!("{stem}-matching-{name}.journal"));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }
    let journal = ledgeline_core::parse_journal(text, &path.to_string_lossy())
        .unwrap_or_else(|e| panic!("the {name} variant must parse: {e}"));
    (path, journal)
}

fn bench_size(c: &mut Criterion, txns: usize) {
    let corpus_path = corpus::ensure_journal(txns);
    let label = corpus::shape_id(&corpus::V1.with_txns(txns));
    let text = std::fs::read_to_string(&corpus_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", corpus_path.display()));
    let (_, unlinked) = variant(&corpus_path, "unlinked", &split_transfers(&text, |_| false));
    let (_, linked) = variant(
        &corpus_path,
        "linked",
        &split_transfers(&text, |pair| pair.is_multiple_of(2)),
    );

    let (_, billpay) = variant(
        &corpus_path,
        "billpay",
        &split_transfers_with(&text, |_| false, |pair| pair.is_multiple_of(4)),
    );

    let all = MatchSettings {
        lookback_days: 0,
        ..MatchSettings::default()
    };
    let proposed = matching::propose(&unlinked, &all, TODAY).len();
    let proposed_365 = matching::propose(&unlinked, &MatchSettings::default(), TODAY).len();
    let links = matching::links(&linked).pairs().len();
    let billpay_proposals = matching::propose(&billpay, &all, TODAY);
    let combinations = billpay_proposals
        .iter()
        .filter(|proposal| proposal.is_combination())
        .count();
    // A split debit for `partners_for`: the last combination's single side.
    let debit = billpay_proposals
        .iter()
        .rev()
        .find(|proposal| proposal.is_combination())
        .map_or(0, |proposal| proposal.a.position);
    eprintln!(
        "matching/{label}: {} transactions unlinked, {proposed} proposals (all), \
         {proposed_365} (365 days); {} transactions half linked, {links} links; \
         {} transactions bill pay, {} proposals (all), {combinations} combinations",
        unlinked.transactions.len(),
        linked.transactions.len(),
        billpay.transactions.len(),
        billpay_proposals.len(),
    );

    let mut group = c.benchmark_group("matching");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function(BenchmarkId::new("propose_all", &label), |b| {
        b.iter(|| matching::propose(black_box(&unlinked), &all, TODAY));
    });
    group.bench_function(BenchmarkId::new("propose_365", &label), |b| {
        b.iter(|| matching::propose(black_box(&unlinked), &MatchSettings::default(), TODAY));
    });
    group.bench_function(BenchmarkId::new("count_all", &label), |b| {
        b.iter(|| matching::count_proposals(black_box(&unlinked), &all, TODAY));
    });
    group.bench_function(BenchmarkId::new("billpay_propose_all", &label), |b| {
        b.iter(|| matching::propose(black_box(&billpay), &all, TODAY));
    });
    group.bench_function(BenchmarkId::new("billpay_propose_365", &label), |b| {
        b.iter(|| matching::propose(black_box(&billpay), &MatchSettings::default(), TODAY));
    });
    group.bench_function(BenchmarkId::new("billpay_partners_for", &label), |b| {
        b.iter(|| {
            matching::partners_for(
                black_box(&billpay),
                debit,
                matching::DEFAULT_WINDOW_DAYS,
                TODAY,
            )
        });
    });
    group.bench_function(BenchmarkId::new("in_transit", &label), |b| {
        b.iter(|| {
            matching::in_transit(
                black_box(&unlinked),
                matching::DEFAULT_WINDOW_DAYS,
                matching::DEFAULT_IN_TRANSIT_DAYS,
                TODAY,
            )
        });
    });
    group.bench_function(BenchmarkId::new("clearing_accounts", &label), |b| {
        b.iter(|| matching::clearing_accounts(black_box(&linked)));
    });
    group.bench_function(BenchmarkId::new("links", &label), |b| {
        b.iter(|| matching::links(black_box(&linked)));
    });
    group.finish();

    let mut group = c.benchmark_group("rows");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function(BenchmarkId::new("transfers_of", &label), |b| {
        b.iter(|| Transfers::of(black_box(&linked)));
    });
    let transfers = Transfers::of(&linked);
    let order = display_order(&linked);
    group.bench_function(BenchmarkId::new("collapse", &label), |b| {
        b.iter(|| transfers.collapse(black_box(&order)));
    });
    group.finish();
}

fn all(c: &mut Criterion) {
    for txns in corpus::bench_sizes() {
        bench_size(c, txns);
    }
}

criterion_group!(benches, all);
criterion_main!(benches);
