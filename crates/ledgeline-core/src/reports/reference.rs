//! What one account actually did, period by period — the figures shown beside
//! the amount box when a budget goal is being set.
//!
//! Setting a budget number without seeing the history is guesswork, so the
//! editor puts the last few periods on screen: "you spent $612, $548 and $701 on
//! groceries over the last three months, and $389 so far this month". This
//! module produces exactly that list, for any account and any of hledger's fixed
//! intervals.
//!
//! # Subaccount-INCLUSIVE, deliberately
//!
//! `expenses:food` reports food *and* `food:dining` *and* `food:groceries`. That
//! is not a convenience: it is what makes the number comparable to the thing the
//! user is about to set it against. The budget report aggregates a parent's goal
//! from its children and shows the parent's inclusive actual
//! (`reports::budget`), so a reference figure that excluded subaccounts would
//! disagree with the very bar it is meant to inform — worst of all quietly, and
//! only for the users who have subaccounts.
//!
//! # The last period is partial, and says so
//!
//! The newest period is almost always still running. It is reported, because
//! "$389 so far this month" is useful and hiding it would be strange, but it is
//! flagged ([`ReferencePeriod::complete`]) so the caller can label it rather than
//! let the reader mistake a third of a month for a whole one.
//!
//! # The average covers the COMPLETE periods only
//!
//! [`AccountHistory::average`] is the figure a budget is actually set from — "I
//! typically spend about this much" — so it must average whole periods. Folding
//! in a month that is four days old would drag the mean down by however far
//! through the month you happen to be, which is a number that changes every day
//! for reasons that have nothing to do with spending.
//!
//! [`AccountHistory::averaged`] says how many periods went into it, so a caller
//! can label the figure honestly and can tell "no complete periods yet" (where
//! there is no average to show) from "an average of zero".
//!
//! # Signs are left exactly as the journal writes them
//!
//! Income is negative in hledger, and this module does not argue with that. The
//! decision to show an income budget as a positive magnitude is a presentation
//! one that depends on the account's *type* — a fact that lives in the journal's
//! declarations, not in its postings — so it is made once, by the caller, and not
//! half-made here.

use super::ReportError;
use super::aggregate::{PostingFilter, account_totals};
use super::mixed_amount::MixedAmount;
use super::periods::{
    Interval, bucket_end, bucket_label, bucket_span, compare_iso, last_n_buckets,
};
use crate::model::Transaction;
use std::cmp::Ordering;

/// One period's actuals for one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencePeriod {
    /// The bucket key, e.g. `2026-08` / `2026Q3` / `2026` / `2026-W35`.
    pub key: String,
    /// The key rendered for a person, e.g. `Aug 2026`.
    pub label: String,
    /// Inclusive period start, `YYYY-MM-DD`.
    pub start: String,
    /// Inclusive period end, **clamped to the as-of date** — so a partial
    /// period's end is the as-of date, and the figure and the range agree.
    pub end: String,
    /// The account's subaccount-inclusive total for the period, signed exactly
    /// as the journal writes it.
    pub total: MixedAmount,
    /// Whether the period has finished. `false` for the newest one whenever the
    /// as-of date falls inside it.
    pub complete: bool,
}

/// One account's recent history: the periods, and the mean of the complete ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountHistory {
    /// The periods, oldest → newest.
    pub periods: Vec<ReferencePeriod>,
    /// The mean over the periods with [`ReferencePeriod::complete`] set, per
    /// commodity. Empty when there are none — see the module docs.
    pub average: MixedAmount,
    /// How many periods [`average`](Self::average) covers. Zero means there is
    /// no average, which is a different fact from an average of zero.
    pub averaged: usize,
}

/// Which account, over which periods.
#[derive(Debug, Clone)]
pub struct ReferenceOpts<'a> {
    /// The account, matched against itself and every subaccount.
    pub account: &'a str,
    /// The period length to bucket by.
    pub interval: Interval,
    /// How many periods, ending with the one containing [`as_of`](Self::as_of).
    pub count: usize,
    /// The inclusive "today" the newest period ends at.
    pub as_of: &'a str,
}

/// The last `count` periods of `account`'s activity, oldest → newest, with the
/// mean of the complete ones.
///
/// Returns an empty history for `count == 0`, which is what `last_n_buckets`
/// documents and the only sensible answer for a zero-period window.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or unrecognized bucket math.
pub fn account_reference(
    txns: &[Transaction],
    opts: &ReferenceOpts,
) -> Result<AccountHistory, ReportError> {
    let buckets = last_n_buckets(opts.as_of, opts.interval, opts.count)?;
    // One selected account, which `account_totals` matches inclusively — the
    // same `account_matches` every other report selects with, so "inclusive"
    // means here exactly what it means there.
    let selected = [opts.account.to_string()];

    let periods = buckets
        .into_iter()
        .map(|key| {
            let (start, end) = bucket_span(&key, opts.as_of)?;
            let totals = account_totals(
                txns,
                &PostingFilter {
                    from: Some(&start),
                    to: Some(&end),
                    accounts: Some(&selected),
                    ..PostingFilter::default()
                },
            )?;
            // `account_totals` keys by FULL account name, so the inclusive
            // figure is the sum over every selected name — the subaccounts are
            // separate keys, not a rolled-up one.
            let mut total = MixedAmount::new();
            for amounts in totals.values() {
                total = total.ma_add(amounts)?;
            }
            Ok(ReferencePeriod {
                label: bucket_label(&key),
                complete: compare_iso(&bucket_end(&key)?, opts.as_of) != Ordering::Greater,
                key,
                start,
                end,
                total,
            })
        })
        .collect::<Result<Vec<ReferencePeriod>, ReportError>>()?;

    Ok(AccountHistory {
        average: mean_of_complete(&periods)?,
        averaged: periods.iter().filter(|period| period.complete).count(),
        periods,
    })
}

/// The per-commodity mean of the COMPLETE periods, or an empty bag when there
/// are none.
///
/// Summed then divided, never accumulated as a running mean: the sum is exact
/// and the single division at the end rounds once, so the figure is the same
/// whichever order the periods arrive in.
fn mean_of_complete(periods: &[ReferencePeriod]) -> Result<MixedAmount, ReportError> {
    let complete: Vec<&ReferencePeriod> = periods.iter().filter(|period| period.complete).collect();
    let Ok(count) = u32::try_from(complete.len()) else {
        // Unreachable in practice — the caller caps `count` far below this — but
        // an empty average is the honest answer rather than a panic.
        return Ok(MixedAmount::new());
    };
    if count == 0 {
        return Ok(MixedAmount::new());
    }
    let mut sum = MixedAmount::new();
    for period in complete {
        sum = sum.ma_add(&period.total)?;
    }
    let mut average = MixedAmount::new();
    for (commodity, quantity) in sum.iter() {
        average.accumulate(commodity, quantity.div_int(count)?)?;
    }
    average.drop_zeros();
    Ok(average)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{txn, usd};
    use super::*;
    use crate::decimal::Dec;
    use crate::model::Commodity;

    /// An `Amount` of `cents` EUR, for the multi-commodity case.
    fn eur(cents: i128) -> crate::model::Amount {
        crate::model::Amount {
            commodity: Commodity("EUR".into()),
            quantity: Dec::new(cents, 2),
            style: crate::model::AmountStyle {
                side: crate::model::CommoditySide::Right,
                spaced: true,
                decimal_mark: Some('.'),
                digit_groups: None,
                precision: 2,
            },
            cost: None,
        }
    }

    /// A `MixedAmount` of `cents` USD.
    fn usd_ma(cents: i128) -> MixedAmount {
        MixedAmount::single(Commodity("$".into()), Dec::new(cents, 2))
    }

    fn ledger() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2026-06-04",
                vec![
                    ("expenses:food", vec![usd(41_000)]),
                    ("assets:checking", vec![usd(-41_000)]),
                ],
            ),
            // A SUBACCOUNT, which the inclusive figure must pick up.
            txn(
                2,
                "2026-06-19",
                vec![
                    ("expenses:food:dining", vec![usd(20_200)]),
                    ("assets:checking", vec![usd(-20_200)]),
                ],
            ),
            txn(
                3,
                "2026-07-11",
                vec![
                    ("expenses:food", vec![usd(54_800)]),
                    ("assets:checking", vec![usd(-54_800)]),
                ],
            ),
            // In the partial month, and AFTER the as-of date below, so the last
            // period must exclude it.
            txn(
                4,
                "2026-08-03",
                vec![
                    ("expenses:food", vec![usd(38_900)]),
                    ("assets:checking", vec![usd(-38_900)]),
                ],
            ),
            txn(
                5,
                "2026-08-28",
                vec![
                    ("expenses:food", vec![usd(9_900)]),
                    ("assets:checking", vec![usd(-9_900)]),
                ],
            ),
            // A sibling that is NOT under expenses:food and must never appear.
            txn(
                6,
                "2026-07-02",
                vec![
                    ("expenses:foodstuffs", vec![usd(5_000)]),
                    ("assets:checking", vec![usd(-5_000)]),
                ],
            ),
        ]
    }

    #[test]
    fn reports_the_last_periods_inclusive_of_subaccounts() {
        let history = account_reference(
            &ledger(),
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Monthly,
                count: 3,
                as_of: "2026-08-15",
            },
        )
        .unwrap();
        let periods = &history.periods;

        assert_eq!(
            periods.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(),
            ["2026-06", "2026-07", "2026-08"]
        );
        // June = $410 + $202 dining. The subaccount is the whole point.
        assert_eq!(periods[0].total, usd_ma(61_200));
        // July = $548. `expenses:foodstuffs` is a different account, not a
        // subaccount, and `account_matches` knows the difference.
        assert_eq!(periods[1].total, usd_ma(54_800));
        // August, to the 15th: the $389 on the 3rd, not the $99 on the 28th.
        assert_eq!(periods[2].total, usd_ma(38_900));
    }

    #[test]
    fn the_running_period_is_flagged_and_clamped() {
        let history = account_reference(
            &ledger(),
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Monthly,
                count: 3,
                as_of: "2026-08-15",
            },
        )
        .unwrap();
        let periods = &history.periods;
        assert!(periods[0].complete);
        assert!(periods[1].complete);
        assert!(!periods[2].complete, "August is still running on the 15th");
        // The range agrees with the figure: it ends where the count stopped.
        assert_eq!(periods[2].start, "2026-08-01");
        assert_eq!(periods[2].end, "2026-08-15");
        assert_eq!(periods[1].end, "2026-07-31");
        assert_eq!(periods[0].label, "Jun 2026");
    }

    /// An as-of date on the last day of a period completes it — the boundary
    /// where "still running" flips.
    #[test]
    fn a_period_ending_exactly_on_the_as_of_date_is_complete() {
        let history = account_reference(
            &ledger(),
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Monthly,
                count: 1,
                as_of: "2026-07-31",
            },
        )
        .unwrap();
        let periods = &history.periods;
        assert!(periods[0].complete);
        assert_eq!(periods[0].end, "2026-07-31");
    }

    /// Yearly is the interval an income budget is set on, and it is the same
    /// walk — "this year plus two prior" is three yearly buckets.
    #[test]
    fn yearly_periods_work_the_same_way() {
        let history = account_reference(
            &ledger(),
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Yearly,
                count: 3,
                as_of: "2026-08-15",
            },
        )
        .unwrap();
        let periods = &history.periods;
        assert_eq!(
            periods.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(),
            ["2024", "2025", "2026"]
        );
        assert_eq!(periods[0].total, MixedAmount::new());
        assert_eq!(periods[2].total, usd_ma(154_900));
        assert!(!periods[2].complete);
    }

    /// An account with no postings reports zeros rather than nothing: the
    /// periods are what the user asked to see, and "nothing here yet" is an
    /// answer.
    #[test]
    fn an_account_with_no_activity_reports_empty_periods() {
        let history = account_reference(
            &ledger(),
            &ReferenceOpts {
                account: "expenses:travel",
                interval: Interval::Monthly,
                count: 2,
                as_of: "2026-08-15",
            },
        )
        .unwrap();
        let periods = &history.periods;
        assert_eq!(periods.len(), 2);
        assert!(periods.iter().all(|p| p.total.is_zero()));
    }

    /// The headline of this change: the mean of the COMPLETE periods, which is
    /// the number a budget actually gets set from.
    #[test]
    fn the_average_covers_the_complete_periods_only() {
        let history = account_reference(
            &ledger(),
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Monthly,
                count: 3,
                as_of: "2026-08-15",
            },
        )
        .unwrap();
        // June $612, July $548 — and NOT August's part-month $389, which would
        // drag the mean down by however far through the month we happen to be.
        // ($612.00 + $548.00) / 2 = $580.00.
        assert_eq!(history.average, usd_ma(58_000));
        assert_eq!(history.averaged, 2);
    }

    /// A history whose newest period is finished averages all of them.
    #[test]
    fn a_history_of_whole_periods_averages_all_of_them() {
        let history = account_reference(
            &ledger(),
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Monthly,
                count: 2,
                as_of: "2026-07-31",
            },
        )
        .unwrap();
        // June $612 + July $548 = $1,160 over two whole months.
        assert_eq!(history.averaged, 2);
        assert_eq!(history.average, usd_ma(58_000));
    }

    /// With nothing complete there is no average — and that is reported as an
    /// absence, not as zero.
    #[test]
    fn a_window_with_no_complete_period_has_no_average() {
        let history = account_reference(
            &ledger(),
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Monthly,
                count: 1,
                as_of: "2026-08-15",
            },
        )
        .unwrap();
        assert_eq!(history.averaged, 0);
        assert!(history.average.is_zero());
    }

    /// Each commodity is averaged on its own, over the same period count — the
    /// months a commodity did not appear in still count, because "nothing" is a
    /// real month of spending in it.
    #[test]
    fn a_multi_commodity_history_averages_per_commodity() {
        let mut txns = ledger();
        txns.push(txn(
            7,
            "2026-06-10",
            vec![
                ("expenses:food", vec![eur(30_000)]),
                ("assets:checking", vec![eur(-30_000)]),
            ],
        ));
        let history = account_reference(
            &txns,
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Monthly,
                count: 3,
                as_of: "2026-08-15",
            },
        )
        .unwrap();
        assert_eq!(history.averaged, 2);
        // $580 as before; €300 over the SAME two months, not over the one it
        // appeared in.
        assert_eq!(
            history.average.get(&Commodity("$".into())),
            Some(Dec::new(58_000, 2))
        );
        assert_eq!(
            history.average.get(&Commodity("EUR".into())),
            Some(Dec::new(15_000, 2))
        );
    }

    /// The rounding `Dec::div_int` promises, reached through a real history.
    #[test]
    fn an_average_that_does_not_divide_evenly_rounds_half_away_from_zero() {
        let txns = vec![
            txn(
                1,
                "2026-01-10",
                vec![
                    ("expenses:food", vec![usd(10_000)]),
                    ("assets:checking", vec![usd(-10_000)]),
                ],
            ),
            txn(
                2,
                "2026-02-10",
                vec![
                    ("expenses:food", vec![usd(10_000)]),
                    ("assets:checking", vec![usd(-10_000)]),
                ],
            ),
            txn(
                3,
                "2026-03-10",
                vec![
                    ("expenses:food", vec![usd(10_100)]),
                    ("assets:checking", vec![usd(-10_100)]),
                ],
            ),
        ];
        let history = account_reference(
            &txns,
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Monthly,
                count: 3,
                as_of: "2026-03-31",
            },
        )
        .unwrap();
        // $301.00 / 3 = $100.333… → $100.33, at the cent the inputs were written to.
        assert_eq!(history.average, usd_ma(10_033));
    }

    #[test]
    fn zero_count_is_an_empty_list_not_a_panic() {
        let history = account_reference(
            &ledger(),
            &ReferenceOpts {
                account: "expenses:food",
                interval: Interval::Monthly,
                count: 0,
                as_of: "2026-08-15",
            },
        )
        .unwrap();
        let periods = &history.periods;
        assert!(periods.is_empty());
    }
}
