//! Budget report — hledger `bal -M --budget`.
//!
//! This is NET-NEW logic (there is no TypeScript engine to port): budgets are
//! impossible via the hledger-web JSON API, so this is the native engine's
//! headline capability. The output is verified byte-for-byte against committed
//! hledger goldens (`fixtures/budget/*.budget.json`) in `tests/budget_golden.rs`.
//!
//! # hledger semantics reproduced here (inferred from the CLI + JSON oracle)
//!
//! Goals come from the journal's `~` periodic rules (fed in as
//! [`PeriodicTransaction`]s). The report is computed as two multi-period balance
//! reports that are then combined:
//!
//! - **Budgeted account set** — the union, over *all* periodic rules (regardless
//!   of `--budget=DESCPAT`), of every rule-posting account and its ancestors. It
//!   controls how actual postings are re-homed and is *not* narrowed by
//!   `DESCPAT`.
//! - **Actuals** — a normal per-bucket balance of the real transactions, but with
//!   every posting's account remapped: an account that is itself budgeted stays;
//!   otherwise it moves to its nearest budgeted ancestor; failing that, to a
//!   single synthetic `<unbudgeted>` account. Displayed amounts are
//!   subaccount-INCLUSIVE (even in flat mode).
//! - **Goals** — generated only from the rules selected by `DESCPAT`
//!   (case-insensitive substring of the rule description; all rules when absent).
//!   Each selected rule contributes its posting amounts once per occurrence of
//!   *its own* interval within the report span, bucketed by the *report*
//!   interval; parent goals are the aggregate of their children's.
//! - **Rows shown** — every account with a non-zero OWN amount (remapped actual
//!   or goal) in some bucket. Boring pass-through parents (no own amount) are
//!   elided; a shown row still displays its inclusive total. A cell's goal is
//!   `Some` iff the account is part of the selected goal tree (else `None`, as
//!   for `<unbudgeted>`); a budgeted account with no goal this bucket shows an
//!   empty (`Some`) goal.
//! - **Totals** — the per-bucket sum of every remapped actual / every goal.
//!
//! All money math is exact (`Dec` via [`MixedAmount`]); every fallible step is
//! surfaced through [`ReportError`].

use super::ReportError;
use super::aggregate::{PostingFilter, account_totals, roll_up};
use super::mixed_amount::MixedAmount;
use super::periods::{
    Interval, bucket_end, bucket_key, bucket_start, compare_iso, last_n_buckets, next_bucket,
};
use crate::model::{PeriodExpr, PeriodicTransaction, Transaction};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// The synthetic account collecting actuals with no budgeted ancestor. Matches
/// hledger's literal `<unbudgeted>` pseudo-account name.
pub const UNBUDGETED: &str = "<unbudgeted>";

/// One account × bucket budget cell: the actual balance and, when the account is
/// part of the selected goal tree, its goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetCell {
    /// Subaccount-inclusive actual for the bucket.
    pub actual: MixedAmount,
    /// Subaccount-inclusive goal, or `None` when the account has no goal
    /// (e.g. `<unbudgeted>`, or an account budgeted only by a non-selected rule).
    pub goal: Option<MixedAmount>,
}

/// One budget report row: an account and its per-bucket cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetRow {
    /// Full, colon-delimited account name (clamped to the report depth; the
    /// synthetic [`UNBUDGETED`] name for the catch-all row).
    pub account: String,
    /// Number of `:`-separated segments in `account`.
    pub depth: usize,
    /// One cell per bucket, oldest → newest.
    pub cells: Vec<BudgetCell>,
}

/// A budget report: bucket keys, rows (union of shown accounts, sorted), and a
/// grand-total row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetReport {
    /// Bucket keys, oldest → newest.
    pub buckets: Vec<String>,
    /// Rows, sorted by account name (`<unbudgeted>` sorts first).
    pub rows: Vec<BudgetRow>,
    /// One grand-total cell per bucket.
    pub totals: Vec<BudgetCell>,
}

/// Budget report parameters.
#[derive(Debug, Clone)]
pub struct BudgetOpts<'a> {
    /// Inclusive report end (hledger's `-e DATE` minus one day).
    pub end: &'a str,
    /// Report bucketing interval.
    pub interval: Interval,
    /// Number of buckets ending with the one containing `end`.
    pub count: usize,
    /// Account depth limit (deeper accounts aggregate into their depth-`depth`
    /// ancestor). Use a large value for "no limit".
    pub depth: usize,
    /// `--budget=DESCPAT`: keep only rules whose description contains this
    /// (case-insensitive) substring. `None` selects all rules.
    pub budget_desc: Option<&'a str>,
}

/// Map a rule's [`PeriodExpr`] to the report [`Interval`] used to step its
/// occurrences.
fn period_interval(period: PeriodExpr) -> Interval {
    match period {
        PeriodExpr::Daily => Interval::Daily,
        PeriodExpr::Weekly => Interval::Weekly,
        PeriodExpr::Monthly => Interval::Monthly,
        PeriodExpr::Quarterly => Interval::Quarterly,
        PeriodExpr::Yearly => Interval::Yearly,
    }
}

/// An account's proper ancestors, nearest first (`a:b:c` → `["a:b", "a"]`).
fn parent_accounts(account: &str) -> Vec<String> {
    let segments: Vec<&str> = account.split(':').collect();
    (1..segments.len())
        .rev()
        .map(|n| segments[..n].join(":"))
        .collect()
}

/// Clamp a full account name to at most `depth` segments (`min` 1). Deeper
/// accounts collapse onto their depth-`depth` ancestor.
fn clip(account: &str, depth: usize) -> String {
    account
        .split(':')
        .take(depth.max(1))
        .collect::<Vec<_>>()
        .join(":")
}

/// Re-home an actual account under the budget tree: keep it if budgeted, else
/// move it to its nearest budgeted ancestor, else to [`UNBUDGETED`].
fn remap_account(account: &str, budgeted: &BTreeSet<String>) -> String {
    if budgeted.contains(account) {
        return account.to_string();
    }
    parent_accounts(account)
        .into_iter()
        .find(|ancestor| budgeted.contains(ancestor))
        .unwrap_or_else(|| UNBUDGETED.to_string())
}

/// The dates a rule of interval `ri` fires on within the inclusive report span
/// `[start, end]`: each occurrence is the start of an `ri`-period whose boundary
/// falls within the span. hledger does NOT include a partial first period whose
/// boundary precedes the report start — e.g. for a weekly rule reported from a
/// non-Monday `start`, the ISO week that merely *contains* `start` (and begins
/// the prior week) does not count; only the boundaries at or after `start` do.
fn occurrences(start: &str, end: &str, ri: Interval) -> Result<Vec<String>, ReportError> {
    let mut out = Vec::new();
    let mut key = bucket_key(start, ri);
    loop {
        let period_start = bucket_start(&key)?;
        if compare_iso(&period_start, end) == Ordering::Greater {
            break;
        }
        if compare_iso(&period_start, start) != Ordering::Less {
            out.push(period_start);
        }
        key = next_bucket(&key, ri)?;
    }
    Ok(out)
}

/// Add every commodity of `src` into `dst` (in place), preserving zeros for a
/// later single prune.
fn accumulate_into(dst: &mut MixedAmount, src: &MixedAmount) -> Result<(), ReportError> {
    for (commodity, qty) in src.iter() {
        dst.accumulate(commodity, *qty)?;
    }
    Ok(())
}

/// Compute the budget report for `txns`/`rules` under `opts`.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or unrecognized bucket math.
pub fn budget_report(
    txns: &[Transaction],
    rules: &[PeriodicTransaction],
    opts: &BudgetOpts,
) -> Result<BudgetReport, ReportError> {
    let buckets = last_n_buckets(opts.end, opts.interval, opts.count)?;

    // Budgeted account set: every rule-posting account + ancestors, ALL rules.
    let budgeted: BTreeSet<String> = rules
        .iter()
        .flat_map(|rule| &rule.postings)
        .flat_map(|posting| posting.account.self_and_ancestors())
        .collect();

    // Rules selected for goals by `--budget=DESCPAT` (all when absent/empty).
    let pattern = opts.budget_desc.map(str::to_lowercase);
    let selected: Vec<&PeriodicTransaction> = rules
        .iter()
        .filter(|rule| match &pattern {
            Some(pat) => rule.description.to_lowercase().contains(pat.as_str()),
            None => true,
        })
        .collect();

    // Accounts that make up the selected goal tree — controls goal `Some`/`None`.
    let goal_accts: BTreeSet<String> = selected
        .iter()
        .flat_map(|rule| &rule.postings)
        .flat_map(|posting| posting.account.self_and_ancestors())
        .collect();

    // --- Actuals: per bucket, remap + clip own totals, then roll up. ---
    let mut actual_own: Vec<BTreeMap<String, MixedAmount>> = Vec::with_capacity(buckets.len());
    let mut actual_incl: Vec<BTreeMap<String, MixedAmount>> = Vec::with_capacity(buckets.len());
    for key in &buckets {
        let start = bucket_start(key)?;
        let bucket_end_date = bucket_end(key)?;
        let to = if compare_iso(opts.end, &bucket_end_date) == Ordering::Less {
            opts.end
        } else {
            bucket_end_date.as_str()
        };
        let direct = account_totals(
            txns,
            &PostingFilter {
                from: Some(&start),
                to: Some(to),
                ..PostingFilter::default()
            },
        )?;
        let mut own: BTreeMap<String, MixedAmount> = BTreeMap::new();
        for (account, ma) in &direct {
            let remapped = clip(&remap_account(account, &budgeted), opts.depth);
            accumulate_into(own.entry(remapped).or_default(), ma)?;
        }
        for ma in own.values_mut() {
            ma.drop_zeros();
        }
        actual_incl.push(roll_up(&own)?);
        actual_own.push(own);
    }

    // --- Goals: per bucket, sum selected-rule occurrences, then roll up. ---
    let report_start = bucket_start(&buckets[0])?;
    let bucket_index: BTreeMap<&str, usize> = buckets
        .iter()
        .enumerate()
        .map(|(index, key)| (key.as_str(), index))
        .collect();
    let mut goal_own: Vec<BTreeMap<String, MixedAmount>> =
        (0..buckets.len()).map(|_| BTreeMap::new()).collect();
    for rule in &selected {
        let ri = period_interval(rule.period);
        for date in occurrences(&report_start, opts.end, ri)? {
            let Some(&index) = bucket_index.get(bucket_key(&date, opts.interval).as_str()) else {
                continue;
            };
            for posting in &rule.postings {
                let name = clip(&posting.account.0, opts.depth);
                let entry = goal_own[index].entry(name).or_default();
                for amount in &posting.amounts {
                    entry.accumulate(&amount.commodity, amount.quantity)?;
                }
            }
        }
    }
    let goal_incl: Vec<BTreeMap<String, MixedAmount>> =
        goal_own.iter().map(roll_up).collect::<Result<_, _>>()?;

    // --- Rows: accounts with any non-zero OWN amount (actual or goal). ---
    let row_accounts: BTreeSet<String> = actual_own
        .iter()
        .chain(&goal_own)
        .flat_map(|own| own.iter())
        .filter(|(_, ma)| !ma.is_zero())
        .map(|(account, _)| account.clone())
        .collect();

    let rows = row_accounts
        .into_iter()
        .map(|account| {
            let in_goal_tree = goal_accts.contains(&account);
            let cells = (0..buckets.len())
                .map(|bucket| BudgetCell {
                    actual: actual_incl[bucket]
                        .get(&account)
                        .cloned()
                        .unwrap_or_default(),
                    goal: in_goal_tree
                        .then(|| goal_incl[bucket].get(&account).cloned().unwrap_or_default()),
                })
                .collect();
            BudgetRow {
                depth: account.split(':').count(),
                account,
                cells,
            }
        })
        .collect();

    // --- Totals: per-bucket sum of every remapped actual / every goal. ---
    let has_goals = !goal_accts.is_empty();
    let totals = (0..buckets.len())
        .map(|bucket| {
            let mut actual = MixedAmount::new();
            for ma in actual_own[bucket].values() {
                actual = actual.ma_add(ma)?;
            }
            let goal = if has_goals {
                let mut sum = MixedAmount::new();
                for ma in goal_own[bucket].values() {
                    sum = sum.ma_add(ma)?;
                }
                Some(sum)
            } else {
                None
            };
            Ok(BudgetCell { actual, goal })
        })
        .collect::<Result<_, ReportError>>()?;

    Ok(BudgetReport {
        buckets,
        rows,
        totals,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{txn, usd};
    use super::*;
    use crate::decimal::Dec;
    use crate::model::{
        AccountName, Amount, AmountStyle, Commodity, CommoditySide, PeriodExpr, Posting,
        PostingType, Status,
    };

    /// A `MixedAmount` of `cents` USD (2 places).
    fn usd_ma(cents: i128) -> MixedAmount {
        MixedAmount::single(Commodity("$".into()), Dec::new(cents, 2))
    }

    /// A whole-dollar unbalanced-virtual (`(account)`) posting, mirroring how
    /// budget goals are written.
    fn goal_posting(account: &str, dollars: i128) -> Posting {
        Posting {
            status: Status::Unmarked,
            ptype: PostingType::Virtual,
            account: AccountName(account.to_string()),
            amounts: vec![Amount {
                commodity: Commodity("$".into()),
                quantity: Dec::new(dollars, 0),
                style: AmountStyle {
                    side: CommoditySide::Left,
                    spaced: false,
                    decimal_mark: Some('.'),
                    digit_groups: None,
                    precision: 0,
                },
                cost: None,
            }],
            balance_assertion: None,
            date: None,
            date2: None,
            comment: String::new(),
            tags: Vec::new(),
        }
    }

    fn rule(period: PeriodExpr, description: &str, postings: Vec<Posting>) -> PeriodicTransaction {
        PeriodicTransaction {
            period,
            description: description.to_string(),
            postings,
        }
    }

    fn row<'a>(report: &'a BudgetReport, account: &str) -> &'a BudgetRow {
        report
            .rows
            .iter()
            .find(|r| r.account == account)
            .unwrap_or_else(|| panic!("row {account} exists"))
    }

    fn opts<'a>(end: &'a str, count: usize, budget_desc: Option<&'a str>) -> BudgetOpts<'a> {
        BudgetOpts {
            end,
            interval: Interval::Monthly,
            count,
            depth: 99,
            budget_desc,
        }
    }

    /// A monthly food+bus goal with an unbudgeted cash leg: goals, an inclusive
    /// (subaccount-rolled) actual, and the `<unbudgeted>` catch-all.
    #[test]
    fn basic_goal_actual_and_unbudgeted() {
        let rules = vec![rule(
            PeriodExpr::Monthly,
            "household budget",
            vec![
                goal_posting("expenses:food", 400),
                goal_posting("expenses:bus", 20),
            ],
        )];
        let txns = vec![
            txn(
                1,
                "2026-01-05",
                vec![
                    ("expenses:food", vec![usd(35_200)]),
                    ("assets:checking", vec![usd(-35_200)]),
                ],
            ),
            txn(
                2,
                "2026-01-12",
                vec![
                    ("expenses:bus", vec![usd(2300)]),
                    ("assets:checking", vec![usd(-2300)]),
                ],
            ),
            // A subaccount without its own goal rolls into expenses:food.
            txn(
                3,
                "2026-01-20",
                vec![
                    ("expenses:food:dining", vec![usd(8000)]),
                    ("assets:checking", vec![usd(-8000)]),
                ],
            ),
        ];

        let report = budget_report(&txns, &rules, &opts("2026-01-31", 1, None)).unwrap();
        assert_eq!(report.buckets, ["2026-01"]);
        assert_eq!(
            report
                .rows
                .iter()
                .map(|r| r.account.as_str())
                .collect::<Vec<_>>(),
            ["<unbudgeted>", "expenses:bus", "expenses:food"]
        );

        // food actual is subaccount-inclusive ($352 + $80 dining) with a $400 goal.
        let food = &row(&report, "expenses:food").cells[0];
        assert_eq!(food.actual, usd_ma(43_200));
        assert_eq!(food.goal, Some(usd_ma(40_000)));

        let bus = &row(&report, "expenses:bus").cells[0];
        assert_eq!(bus.actual, usd_ma(2300));
        assert_eq!(bus.goal, Some(usd_ma(2000)));

        // <unbudgeted> = all cash, no goal.
        let unbudgeted = &row(&report, "<unbudgeted>").cells[0];
        assert_eq!(unbudgeted.actual, usd_ma(-45_500));
        assert_eq!(unbudgeted.goal, None);

        // Totals net to zero; goal is the sum of the goals.
        assert_eq!(report.totals[0].actual, MixedAmount::new());
        assert_eq!(report.totals[0].goal, Some(usd_ma(42_000)));
    }

    /// Goals on leaf accounts aggregate onto the parent row, whose inclusive
    /// actual includes an unbudgeted sibling remapped onto it.
    #[test]
    fn parent_goal_is_aggregated_from_children() {
        let rules = vec![rule(
            PeriodExpr::Monthly,
            "category goals",
            vec![
                goal_posting("expenses:food:groceries", 300),
                goal_posting("expenses:food:dining", 150),
            ],
        )];
        let txns = vec![
            txn(
                1,
                "2026-01-03",
                vec![
                    ("expenses:food:groceries", vec![usd(28_000)]),
                    ("assets:checking", vec![usd(-28_000)]),
                ],
            ),
            txn(
                2,
                "2026-01-09",
                vec![
                    ("expenses:food:dining", vec![usd(12_000)]),
                    ("assets:checking", vec![usd(-12_000)]),
                ],
            ),
            // Unbudgeted sibling: remaps onto expenses:food, keeping it visible.
            txn(
                3,
                "2026-01-14",
                vec![
                    ("expenses:food:snacks", vec![usd(2500)]),
                    ("assets:checking", vec![usd(-2500)]),
                ],
            ),
        ];

        let report = budget_report(&txns, &rules, &opts("2026-01-31", 1, None)).unwrap();
        // The parent row aggregates the children's goals ($300 + $150) and shows
        // the inclusive actual ($280 + $120 + $25 snacks).
        let food = &row(&report, "expenses:food").cells[0];
        assert_eq!(food.goal, Some(usd_ma(45_000)));
        assert_eq!(food.actual, usd_ma(42_500));
        // The bare `expenses` parent has no own amount → elided.
        assert!(report.rows.iter().all(|r| r.account != "expenses"));
    }

    /// `--budget=DESCPAT` narrows which rules supply goals; accounts budgeted
    /// only by a non-selected rule still show, but with a `None` goal.
    #[test]
    fn budget_descpat_selects_goals_only() {
        let rules = vec![
            rule(
                PeriodExpr::Monthly,
                "groceries and transit",
                vec![
                    goal_posting("expenses:food", 400),
                    goal_posting("expenses:bus", 30),
                ],
            ),
            rule(
                PeriodExpr::Monthly,
                "housing costs",
                vec![goal_posting("expenses:rent", 1500)],
            ),
        ];
        let txns = vec![
            txn(
                1,
                "2026-01-06",
                vec![
                    ("expenses:food", vec![usd(35_200)]),
                    ("assets:checking", vec![usd(-35_200)]),
                ],
            ),
            txn(
                2,
                "2026-01-15",
                vec![
                    ("expenses:rent", vec![usd(150_000)]),
                    ("assets:checking", vec![usd(-150_000)]),
                ],
            ),
        ];

        let report = budget_report(&txns, &rules, &opts("2026-01-31", 1, Some("housing"))).unwrap();
        // rent has a goal from the selected rule…
        assert_eq!(
            row(&report, "expenses:rent").cells[0].goal,
            Some(usd_ma(150_000))
        );
        // …food is still a row (budgeted by the other rule) but has no goal.
        let food = &row(&report, "expenses:food").cells[0];
        assert_eq!(food.actual, usd_ma(35_200));
        assert_eq!(food.goal, None);
        // Case-insensitive substring: "GROCER" matches "groceries and transit".
        let grocer = budget_report(&txns, &rules, &opts("2026-01-31", 1, Some("GROCER"))).unwrap();
        assert_eq!(
            row(&grocer, "expenses:food").cells[0].goal,
            Some(usd_ma(40_000))
        );
        assert_eq!(row(&grocer, "expenses:rent").cells[0].goal, None);
    }

    /// A weekly rule contributes once per occurrence within a monthly bucket.
    #[test]
    fn weekly_rule_sums_occurrences_into_monthly_bucket() {
        let rules = vec![rule(
            PeriodExpr::Weekly,
            "",
            vec![goal_posting("expenses:food", 100)],
        )];
        let report = budget_report(&[], &rules, &opts("2026-02-28", 2, None)).unwrap();
        // Jan and Feb 2026 each have 4 Monday occurrences (5/12/19/26 and
        // 2/9/16/23); hledger does NOT clip a partial first week, so both months
        // are 4 × $100 = $400 (verified against the weekly.journal golden).
        assert_eq!(report.buckets, ["2026-01", "2026-02"]);
        let food = row(&report, "expenses:food");
        assert_eq!(food.cells[0].goal, Some(usd_ma(40_000)));
        assert_eq!(food.cells[1].goal, Some(usd_ma(40_000)));
    }
}
