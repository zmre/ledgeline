//! Recurring-subscription detection — finds the monthly and annual charges
//! quietly leaking money out of a journal.
//!
//! Nothing in a plain-text journal marks a charge as a subscription, so this
//! infers them from the shape of the spending. The pipeline is:
//!
//! 1. **Group by payee.** The payee is the part of a transaction's description
//!    before `|` (hledger's `payee | note` convention). Only the expense side of
//!    each transaction counts, valued in the base commodity.
//! 2. **Cluster by amount within each payee.** This is what separates a real
//!    subscription from ordinary shopping at the same merchant. Apple is the
//!    motivating case: a steady $9.99/month iCloud charge and a scatter of
//!    one-off app purchases share a payee, but only the former forms a tight
//!    amount cluster. A merchant whose amounts vary wildly (Amazon, Costco)
//!    never produces a cluster big enough to qualify.
//! 3. **Test each cluster's cadence.** A subscription recurs on roughly the same
//!    day of the month (or the same date each year), so we require BOTH a
//!    plausible median gap AND a consistent day-of-month. The gap check is what
//!    keeps quarterly charges (also day-of-month-consistent) from being reported
//!    as monthly, and weekly/biweekly patterns out entirely.
//! 4. **Require enough repetitions** — [`SubscriptionOpts::min_monthly`] /
//!    [`min_annual`](SubscriptionOpts::min_annual) — so a couple of coincidental
//!    look-alikes are not promoted to a standing charge.
//!
//! Every threshold lives in [`SubscriptionOpts`] rather than being hard-coded,
//! so they can be tuned (and later moved into a config file) without touching
//! the algorithm. Everything is exact [`Dec`] — including the amount tolerance,
//! which used to be compared in `f64` and so let a charge landing exactly on the
//! boundary decide whether a subscription was detected (see
//! [`cluster_by_amount`]).

use super::ReportError;
use super::account_types::{AccountType, account_decls, declared_types, resolve_account_type};
use super::periods::{add_months, days_between, parts};
use super::prices::base_target;
use crate::decimal::Dec;
use crate::model::{Commodity, Journal, Transaction};
use std::collections::BTreeMap;

/// How often a detected subscription recurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cadence {
    /// Roughly every month, on a consistent day.
    Monthly,
    /// Roughly every year, on a consistent date.
    Annual,
}

/// The tag key that overrides detection: `subscription:true` forces a charge
/// onto the list, `subscription:false` takes it off.
pub const OVERRIDE_TAG: &str = "subscription";

/// A hand-written verdict on a transaction, read from its [`OVERRIDE_TAG`].
///
/// Detection is a heuristic, and heuristics are wrong at the edges: a charge
/// whose amount swings too much to cluster never surfaces, and a merchant you
/// have stopped caring about keeps surfacing. These let the journal say so
/// directly, and an explicit verdict always beats an inferred one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Override {
    /// `subscription:true` (also `yes`/`monthly`, or `annual`/`yearly`).
    Include(Cadence),
    /// `subscription:false` (also `no`).
    Exclude,
}

/// Read a transaction's subscription verdict from its own tags or its postings'.
/// Unrecognized values are ignored rather than guessed at.
fn subscription_override(txn: &Transaction) -> Option<Override> {
    let tags = txn
        .tags
        .iter()
        .chain(txn.postings.iter().flat_map(|posting| posting.tags.iter()));
    for (key, value) in tags {
        if !key.eq_ignore_ascii_case(OVERRIDE_TAG) {
            continue;
        }
        return match value.trim().to_lowercase().as_str() {
            "true" | "yes" | "monthly" => Some(Override::Include(Cadence::Monthly)),
            "annual" | "yearly" => Some(Override::Include(Cadence::Annual)),
            "false" | "no" => Some(Override::Exclude),
            _ => None,
        };
    }
    None
}

/// One detected recurring charge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    /// Payee as written in the journal (description before `|`).
    pub payee: String,
    /// Detected recurrence.
    pub cadence: Cadence,
    /// The representative charge (median of the cluster), in the base commodity.
    pub typical_amount: Dec,
    /// Cost per year at this cadence: `typical × 12` monthly, `typical` annual.
    pub annualized_cost: Dec,
    /// How many charges were matched.
    pub occurrences: usize,
    /// Date of the first matched charge.
    pub first_seen: String,
    /// Date of the most recent matched charge.
    pub last_seen: String,
    /// When the next charge is due, projected from `last_seen`.
    pub next_expected: String,
    /// Expense accounts the charges posted to (sorted, deduped) — the hook for
    /// per-category ignore rules later.
    pub accounts: Vec<String>,
    /// True when this entry exists because a transaction was tagged
    /// `subscription:true`, rather than because the detector found a pattern.
    pub manual: bool,
}

/// Detected subscriptions, split by cadence and sorted by annual cost desc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionsReport {
    /// Inclusive end of the scanned window.
    pub as_of: String,
    /// Inclusive start of the scanned window.
    pub lookback_start: String,
    /// Monthly charges, most expensive per year first.
    pub monthly: Vec<Subscription>,
    /// Annual charges, most expensive first.
    pub annual: Vec<Subscription>,
}

/// Tunables for [`detect_subscriptions`]. Defaults come from [`Default`].
#[derive(Debug, Clone)]
pub struct SubscriptionOpts<'a> {
    /// Inclusive end of the scan window (usually today).
    pub as_of: &'a str,
    /// How many months back to scan.
    pub lookback_months: i64,
    /// How far two charges may differ (percent of the smaller) and still count
    /// as the same recurring amount — absorbs price rises and FX drift.
    ///
    /// Exact `Dec`, not `f64`: this is a threshold, and a threshold decides
    /// whether a subscription is reported. See [`cluster_by_amount`].
    pub amount_tolerance_pct: Dec,
    /// How far the billing day may drift and still count as the same cadence.
    pub date_tolerance_days: i64,
    /// Charges needed before a monthly cadence is believed.
    pub min_monthly: usize,
    /// Charges needed before an annual cadence is believed.
    pub min_annual: usize,
    /// How long a charge may go unseen before it is treated as cancelled.
    ///
    /// Measured NOT against `as_of` but against how far the data actually
    /// reaches — see [`detect_subscriptions`]. Monthly charges get this much
    /// grace; annual charges get a full year on top, so one missed renewal plus
    /// the same grace retires them.
    pub stale_months: i64,
    /// Case-insensitive substrings that disqualify a transaction outright.
    ///
    /// Matched against the WHOLE description, not just the payee: a mortgage is
    /// typically written `Wells Fargo | mortgage`, so the telling word lives in
    /// the note rather than the payee the detector groups by. This is the escape
    /// hatch for charges that recur perfectly but are not subscriptions in the
    /// "what could I cancel" sense — debt service, rent-like obligations.
    pub exclude_desc: &'a [String],
}

/// Descriptions excluded unless the caller says otherwise: a mortgage recurs as
/// reliably as any subscription but is debt service, not something to cancel.
pub const DEFAULT_EXCLUDE_DESC: &[&str] = &["mortgage"];

impl Default for SubscriptionOpts<'_> {
    fn default() -> Self {
        Self {
            as_of: "",
            lookback_months: 24,
            amount_tolerance_pct: Dec::new(15, 0),
            date_tolerance_days: 4,
            min_monthly: 5,
            min_annual: 2,
            stale_months: 3,
            exclude_desc: &[],
        }
    }
}

/// Plausible median gap (days) between consecutive monthly charges. Bounded
/// tightly enough to exclude biweekly (~14) below and quarterly (~91) above.
const MONTHLY_GAP: std::ops::RangeInclusive<i64> = 26..=35;
/// Plausible median gap (days) between consecutive annual charges.
const ANNUAL_GAP: std::ops::RangeInclusive<i64> = 330..=400;

/// Percent of a payee's charges an ANNUAL candidate must account for.
///
/// Two charges a year apart is weak evidence on its own — at a merchant you
/// visit constantly (a supermarket, a utility billed monthly at varying
/// amounts), some pair will always happen to sit ~365 days apart at a similar
/// price. Requiring the pattern to explain nearly all of that payee's activity
/// encodes what an annual subscription actually is: a payee you hear from once
/// a year. Monthly candidates deliberately face no such test — a dozen evenly
/// spaced charges are strong evidence by themselves, and genuine monthly
/// subscriptions routinely coexist with one-off purchases at the same merchant
/// (the Apple case).
const ANNUAL_DOMINANCE_PCT: usize = 80;

/// The payee part of a description: everything before the first `|`, trimmed.
/// Descriptions are stored whole in the model; this splits only for grouping.
fn payee_of(description: &str) -> &str {
    description
        .split('|')
        .next()
        .unwrap_or(description)
        .trim_end()
        .trim_start()
}

/// One matched charge: when, how much, and where it posted.
#[derive(Debug, Clone)]
struct Charge {
    date: String,
    amount: Dec,
    /// Expense accounts the charge hit (reported to the caller).
    accounts: Vec<String>,
    /// The non-expense side — the card or account the money came FROM. These
    /// decide how current the charge's data is: an import feeds an account, so
    /// the most recent activity on the funding account is the last date we could
    /// possibly have seen this charge.
    funding: Vec<String>,
    /// A hand-written verdict from this transaction's `subscription:` tag.
    verdict: Option<Override>,
}

/// The expense-side total of `txn` in `base`, with the accounts it hit.
///
/// Accounts are classified by their EFFECTIVE type — an `account … ; type: X`
/// declaration, else the nearest declared ancestor's, else the name — never by
/// the name alone. A chart of accounts that books costs outside an `expenses:`
/// root (`cogs:infrastructure`, say) is perfectly ordinary, and judging by name
/// would make every charge in it invisible: no expense side means no amount,
/// which means the transaction is skipped entirely and even an explicit
/// `subscription:true` on it does nothing.
///
/// Returns `None` when the transaction spends nothing (a refund or transfer) or
/// when it is really INCOME. The income guard matters more than it looks: a
/// paycheck carries payroll-tax withholding as expense postings, so without it
/// every employer is "detected" as a large monthly subscription. A subscription
/// is money you pay out, not a deduction from money coming in.
fn expense_charge(
    txn: &Transaction,
    base: &Commodity,
    declared: &BTreeMap<String, AccountType>,
) -> Result<Option<Charge>, ReportError> {
    let type_of = |account: &str| resolve_account_type(account, declared);
    if txn
        .postings
        .iter()
        .any(|posting| type_of(&posting.account.0) == Some(AccountType::Revenue))
    {
        return Ok(None);
    }
    let mut total = Dec::zero();
    let mut accounts: Vec<String> = Vec::new();
    let mut funding: Vec<String> = Vec::new();
    for posting in &txn.postings {
        if type_of(&posting.account.0) != Some(AccountType::Expense) {
            funding.push(posting.account.0.clone());
            continue;
        }
        for amount in &posting.amounts {
            if amount.commodity != *base {
                continue;
            }
            total = total.add(amount.quantity)?;
        }
        accounts.push(posting.account.0.clone());
    }
    if total.mantissa <= 0 {
        return Ok(None);
    }
    accounts.sort();
    accounts.dedup();
    funding.sort();
    funding.dedup();
    Ok(Some(Charge {
        date: txn.date.clone(),
        amount: total,
        accounts,
        funding,
        verdict: subscription_override(txn),
    }))
}

/// Split a payee's charges into groups of similar amount.
///
/// Charges are sorted ascending and cut wherever the next amount exceeds the
/// current group's smallest by more than `tolerance_pct`, so every group spans
/// at most that much — a $9.99 subscription never absorbs a $49.99 purchase.
///
/// The comparison is exact. It used to convert both amounts to `f64` and test
/// against `low × (1 + tol/100)`, where neither the amount nor the ratio is
/// binary-representable: at 15% tolerance a $6.00 anchor produced a ceiling of
/// 6.899999999999999467, so a $6.90 charge — a rise of exactly the tolerance,
/// the very thing the tolerance exists to absorb — fell outside the cluster.
/// That is not a rounding artifact in a displayed figure; it splits the cluster,
/// drops both halves below [`SubscriptionOpts::min_monthly`], and the
/// subscription is **not detected at all** (DRY-5).
fn cluster_by_amount(
    mut charges: Vec<Charge>,
    tolerance_pct: Dec,
) -> Result<Vec<Vec<Charge>>, ReportError> {
    charges.sort_by_key(|charge| charge.amount);
    // `charge <= anchor × (1 + tol/100)` with the division cleared: multiply
    // both sides by 100 and compare `charge × 100` against `anchor × (100 + tol)`.
    // Every term is then exact `Dec`, and `Dec`'s `Ord` is exact too.
    let hundred = Dec::new(100, 0);
    let ceiling_factor = hundred.add(tolerance_pct)?;
    let mut clusters: Vec<Vec<Charge>> = Vec::new();
    for charge in charges {
        let fits = match clusters.last().and_then(|cluster| cluster.first()) {
            Some(anchor) => charge.amount.mul(hundred)? <= anchor.amount.mul(ceiling_factor)?,
            None => false,
        };
        match (fits, clusters.last_mut()) {
            (true, Some(cluster)) => cluster.push(charge),
            _ => clusters.push(vec![charge]),
        }
    }
    Ok(clusters)
}

/// Day of the month (1–31) of an ISO date.
///
/// Delegates to [`super::periods::parts`], the crate's one ISO field parser —
/// this used to re-slice `8..10` itself (DRY-2). The fallback is unchanged:
/// `parts` also yields 0 for a field it cannot parse.
fn day_of_month(date: &str) -> i64 {
    parts(date).2
}

/// Month (1–12) of an ISO date. Delegates to [`super::periods::parts`]; see
/// [`day_of_month`].
fn month_of(date: &str) -> i64 {
    parts(date).1
}

/// Distance between two days-of-month, wrapping around the end of the month so
/// a charge that bills on the 31st and slips to the 1st reads as 1 day apart,
/// not 30.
fn day_distance(a: i64, b: i64) -> i64 {
    let direct = (a - b).abs();
    direct.min(31 - direct)
}

/// The middle value of a sorted-by-value copy of `values`.
fn median(values: &[i64]) -> i64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 2).copied().unwrap_or(0)
}

/// The middle amount (exact — never averaged, so the reported figure is always
/// a charge that really occurred).
fn median_amount(amounts: &[Dec]) -> Dec {
    let mut sorted = amounts.to_vec();
    sorted.sort();
    sorted
        .get(sorted.len() / 2)
        .copied()
        .unwrap_or_else(Dec::zero)
}

/// Decide whether `dates` (ascending, deduped) recur monthly or annually.
///
/// Both cadences demand a plausible median gap AND a consistent billing day:
/// the gap alone would accept quarterly charges as monthly, and the day alone
/// would accept them too (a quarterly charge also lands on the same day).
fn detect_cadence(dates: &[String], opts: &SubscriptionOpts) -> Option<Cadence> {
    if dates.len() < 2 {
        return None;
    }
    let gaps: Vec<i64> = dates
        .windows(2)
        .map(|pair| days_between(&pair[0], &pair[1]))
        .collect();
    let typical_gap = median(&gaps);

    let days: Vec<i64> = dates.iter().map(|date| day_of_month(date)).collect();
    let typical_day = median(&days);
    let day_is_consistent = days
        .iter()
        .all(|day| day_distance(*day, typical_day) <= opts.date_tolerance_days);

    if dates.len() >= opts.min_monthly && MONTHLY_GAP.contains(&typical_gap) && day_is_consistent {
        return Some(Cadence::Monthly);
    }
    if dates.len() >= opts.min_annual && ANNUAL_GAP.contains(&typical_gap) && day_is_consistent {
        // An annual charge should also recur in the same month each year.
        let months: Vec<i64> = dates.iter().map(|date| month_of(date)).collect();
        let typical_month = median(&months);
        if months.iter().all(|m| (m - typical_month).abs() <= 1) {
            return Some(Cadence::Annual);
        }
    }
    None
}

/// Latest transaction date on each account, ignoring anything after `as_of`.
///
/// This is the "data horizon" per account: how far imports have actually been
/// loaded for it. Judging staleness against it — rather than against today —
/// is what separates *cancelled* from *not imported yet*.
fn account_horizons(txns: &[Transaction], as_of: &str) -> BTreeMap<String, String> {
    let mut latest: BTreeMap<String, String> = BTreeMap::new();
    for txn in txns {
        if txn.date.as_str() > as_of {
            continue;
        }
        for posting in &txn.postings {
            latest
                .entry(posting.account.0.clone())
                .and_modify(|current| {
                    if txn.date > *current {
                        current.clone_from(&txn.date);
                    }
                })
                .or_insert_with(|| txn.date.clone());
        }
    }
    latest
}

/// Find recurring monthly and annual charges in the journal's expense history.
///
/// A detected charge is dropped when it has not been seen recently enough to
/// still look live. "Recently" is measured against the latest activity on the
/// accounts that FUND the charge, not against `as_of`: a card whose statements
/// stop in March says nothing about April, so its subscriptions must not be
/// retired merely because the calendar moved on. A card that is current
/// through last week, on the other hand, genuinely has not been billed — and
/// after [`SubscriptionOpts::stale_months`] that charge is treated as cancelled
/// rather than left on the list as a phantom cost.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow (unreachable for realistic
/// journals, but never unwrapped).
pub fn detect_subscriptions(
    journal: &Journal,
    opts: &SubscriptionOpts,
) -> Result<SubscriptionsReport, ReportError> {
    // The most frequent price target, `$` when the journal prices nothing —
    // the same commodity `insights::base_commodity` used to hand back, but
    // COUNTED rather than derived from a freshly built price database.
    // Detection reads one string off it and nothing else, while
    // `infer_market_prices` + `PriceDb::build` charged a sort of the whole
    // journal plus a deep clone of every directive to produce it (11.5 ms +
    // 1.7 ms on the 200k corpus).
    let base = base_target(&journal.transactions, &journal.prices)?
        .unwrap_or_else(|| Commodity("$".to_string()));
    let lookback_start = add_months(opts.as_of, -opts.lookback_months);
    let horizons = account_horizons(&journal.transactions, opts.as_of);
    // Effective account types, so costs booked outside an `expenses:` root still
    // count (see [`expense_charge`]).
    let declared = declared_types(&account_decls(journal));

    // Lowercased once; the descriptions are compared case-insensitively.
    let excluded: Vec<String> = opts
        .exclude_desc
        .iter()
        .map(|pattern| pattern.to_lowercase())
        .filter(|pattern| !pattern.is_empty())
        .collect();

    // Payee → its charges inside the window, plus the subset each payee has
    // explicitly tagged onto the list.
    let mut by_payee: BTreeMap<String, Vec<Charge>> = BTreeMap::new();
    let mut tagged_in: BTreeMap<String, Vec<Charge>> = BTreeMap::new();
    for txn in &journal.transactions {
        if txn.date.as_str() < lookback_start.as_str() || txn.date.as_str() > opts.as_of {
            continue;
        }
        // Matched against the whole description (payee AND note) — see
        // [`SubscriptionOpts::exclude_desc`]. A transaction tagged
        // `subscription:true` is exempt: an explicit verdict in the journal
        // outranks a blanket default.
        let lowered = txn.description.to_lowercase();
        let is_tagged_in = matches!(subscription_override(txn), Some(Override::Include(_)));
        if !is_tagged_in && excluded.iter().any(|pattern| lowered.contains(pattern)) {
            continue;
        }
        let payee = payee_of(&txn.description);
        if payee.is_empty() {
            continue;
        }
        if let Some(charge) = expense_charge(txn, &base, &declared)? {
            if matches!(charge.verdict, Some(Override::Include(_))) {
                tagged_in
                    .entry(payee.to_string())
                    .or_default()
                    .push(charge.clone());
            }
            by_payee.entry(payee.to_string()).or_default().push(charge);
        }
    }

    let mut monthly: Vec<Subscription> = Vec::new();
    let mut annual: Vec<Subscription> = Vec::new();

    for (payee, charges) in by_payee {
        let payee_charges = charges.len();
        for cluster in cluster_by_amount(charges, opts.amount_tolerance_pct)? {
            // One charge per date: a repeated same-day, same-price purchase is
            // shopping, not an extra billing cycle.
            let mut dated: BTreeMap<String, &Charge> = BTreeMap::new();
            for charge in &cluster {
                dated.entry(charge.date.clone()).or_insert(charge);
            }
            let dates: Vec<String> = dated.keys().cloned().collect();
            // A `subscription:false` anywhere in this cluster takes it off the
            // list. Scoped to the cluster, not the whole payee, so marking a
            // one-off purchase does not also silence the real subscription
            // billed by the same merchant.
            if dated
                .values()
                .any(|charge| charge.verdict == Some(Override::Exclude))
            {
                continue;
            }
            let Some(cadence) = detect_cadence(&dates, opts) else {
                continue;
            };
            // An annual pattern must explain nearly all of this payee's activity
            // (see [`ANNUAL_DOMINANCE_PCT`]) — otherwise it is a coincidence
            // inside a merchant you deal with constantly.
            if cadence == Cadence::Annual
                && dates.len() * 100 < payee_charges * ANNUAL_DOMINANCE_PCT
            {
                continue;
            }

            let amounts: Vec<Dec> = dated.values().map(|charge| charge.amount).collect();
            let typical_amount = median_amount(&amounts);
            let annualized_cost = match cadence {
                Cadence::Monthly => typical_amount.mul(Dec::new(12, 0))?,
                Cadence::Annual => typical_amount,
            };
            let mut accounts: Vec<String> = dated
                .values()
                .flat_map(|charge| charge.accounts.iter().cloned())
                .collect();
            accounts.sort();
            accounts.dedup();

            let first_seen = dates.first().cloned().unwrap_or_default();
            let last_seen = dates.last().cloned().unwrap_or_default();
            let next_expected = match cadence {
                Cadence::Monthly => add_months(&last_seen, 1),
                Cadence::Annual => add_months(&last_seen, 12),
            };

            // Retire charges that have gone quiet, but only when the funding
            // accounts have data recent enough to prove the silence is real.
            // An annual charge gets a year of grace on top, so it is retired
            // one missed renewal later rather than mid-cycle.
            let horizon = dated
                .values()
                .flat_map(|charge| charge.funding.iter())
                .filter_map(|account| horizons.get(account))
                .max()
                .map_or(opts.as_of, String::as_str);
            let allowance = match cadence {
                Cadence::Monthly => opts.stale_months,
                Cadence::Annual => 12 + opts.stale_months,
            };
            if last_seen.as_str() < add_months(horizon, -allowance).as_str() {
                continue;
            }

            let subscription = Subscription {
                payee: payee.clone(),
                cadence,
                typical_amount,
                annualized_cost,
                occurrences: dates.len(),
                first_seen,
                last_seen,
                next_expected,
                accounts,
                manual: false,
            };
            match cadence {
                Cadence::Monthly => monthly.push(subscription),
                Cadence::Annual => annual.push(subscription),
            }
        }
    }

    // Charges the journal names outright. Detection cannot see a subscription
    // whose amount swings too much to cluster, so `subscription:true` puts it on
    // the list regardless — one entry per payee however many transactions carry
    // the tag, and never a duplicate of something already found.
    for (payee, charges) in &tagged_in {
        if monthly.iter().chain(&annual).any(|row| row.payee == *payee) {
            continue;
        }
        let cadence = charges
            .iter()
            .find_map(|charge| match charge.verdict {
                Some(Override::Include(cadence)) => Some(cadence),
                _ => None,
            })
            .unwrap_or(Cadence::Monthly);
        let amounts: Vec<Dec> = charges.iter().map(|charge| charge.amount).collect();
        let typical_amount = median_amount(&amounts);
        let annualized_cost = match cadence {
            Cadence::Monthly => typical_amount.mul(Dec::new(12, 0))?,
            Cadence::Annual => typical_amount,
        };
        let mut dates: Vec<&str> = charges.iter().map(|charge| charge.date.as_str()).collect();
        dates.sort_unstable();
        let first_seen = dates.first().copied().unwrap_or_default().to_string();
        let last_seen = dates.last().copied().unwrap_or_default().to_string();
        let mut accounts: Vec<String> = charges
            .iter()
            .flat_map(|charge| charge.accounts.iter().cloned())
            .collect();
        accounts.sort();
        accounts.dedup();
        let subscription = Subscription {
            payee: payee.clone(),
            cadence,
            typical_amount,
            annualized_cost,
            occurrences: charges.len(),
            first_seen,
            next_expected: match cadence {
                Cadence::Monthly => add_months(&last_seen, 1),
                Cadence::Annual => add_months(&last_seen, 12),
            },
            last_seen,
            accounts,
            manual: true,
        };
        match cadence {
            Cadence::Monthly => monthly.push(subscription),
            Cadence::Annual => annual.push(subscription),
        }
    }

    // Biggest annual bite first, so the most valuable thing to cancel leads.
    let by_cost = |a: &Subscription, b: &Subscription| {
        b.annualized_cost
            .cmp(&a.annualized_cost)
            .then_with(|| a.payee.cmp(&b.payee))
    };
    monthly.sort_by(by_cost);
    annual.sort_by(by_cost);

    Ok(SubscriptionsReport {
        as_of: opts.as_of.to_string(),
        lookback_start,
        monthly,
        annual,
    })
}
