//! Criterion coverage of every hot path CLEANUP.md's Phase 6 names, so each of
//! PERF-1 .. PERF-5 has a number to move.
//!
//! # Running
//!
//! ```text
//! # default corpus: 5k + 50k transactions (a few minutes)
//! nix develop -c cargo bench -p ledgeline-core
//!
//! # add the 200k corpus that Phase 6's table was measured on (~15 minutes)
//! LEDGELINE_BENCH_SIZES=5000,50000,200000 nix develop -c cargo bench -p ledgeline-core
//!
//! # one group, one size
//! nix develop -c cargo bench -p ledgeline-core -- 'reports/net_worth'
//!
//! # record a baseline, then diff a later run against it
//! nix develop -c cargo bench -p ledgeline-core -- --save-baseline before
//! # ... make the change ...
//! nix develop -c cargo bench -p ledgeline-core -- --baseline before
//! ```
//!
//! Journals come from the deterministic generator in `benches/corpus.rs` and are
//! cached under `target/perf/`, so two runs on the same machine measure
//! byte-identical input.
//!
//! # What guards what
//!
//! | Bench | Finding | Expected effect of the fix |
//! |---|---|---|
//! | `wire/journal_to_value`, `wire/clone_value`, `wire/snapshot_from_journal` | PERF-1 | **DONE**: `snapshot_from_journal` now serializes to bytes, so it tracks `serialize_bytes`, not `journal_to_value`. `journal_to_value`/`clone_value`/`to_string` are kept as the *old* cost for reference — nothing on the serving path calls them any more |
//! | `wire/clone_journal` | PERF-1b | **DONE**: gone from the snapshot path; kept as the size of what was removed |
//! | `reports/net_worth/count_{12,24,60}` | PERF-5 | **must go flat in bucket count** |
//! | `holdings/holdings_series_12` | PERF-5b | 1,567 → ~150 ms at 200k |
//! | `reports/insights` | PERF-5c | 1,197 → ~250 ms at 200k |
//! | `prices/lookup_{early,late}_date` | PERF-5d | **the pair must converge** |
//! | `accounts/resolve_account_type_sweep` | PERF-5e | memoization should make it ~free |
//! | `aggregate/roll_up` | PERF-5f | in-place accumulate instead of clone-per-add — but see `docs/development/perf-baseline.md`: this one is FLAT in journal size, so the win is ~0.15 ms per call, not the 3–10× the framing implies |

#[path = "corpus.rs"]
mod corpus;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ledgeline_core::decimal::Dec;
use ledgeline_core::holdings::SeriesWindow;
use ledgeline_core::holdings::{HoldingsScope, ScopeMode, compute_holdings, holdings_series};
use ledgeline_core::model::{Commodity, Journal, PriceDirective};
use ledgeline_core::projections::{
    AccountsOpts, JournalFacts, ProjectionOpts, RealKey, RealSide, Scenario, StatementOpts,
    merged_prices, merged_transactions, project, project_cached, projected_accounts,
    projected_statements, serialize,
};
use ledgeline_core::reports::{
    AccountDecl, AccountPeriodsOpts, AccountType, BalanceMode, BsOpts, BudgetOpts, CfOpts, CfTags,
    IS_GROUP_TAG, InsightsOpts, Interval, IsOpts, MixedAmount, NetWorthOpts, PostingFilter,
    PriceDb, SubscriptionOpts, Valuation, account_decls, account_groups, account_periods,
    account_sections, account_totals, balance_sheet, balance_sheet_columns, balance_sheet_grouped,
    bs_terms, budget_report, cash_flow, cash_flow_statement, cash_predicate, declared_groups,
    declared_types, detect_subscriptions, income_statement, income_statement_columns,
    infer_market_prices, insights, net_worth, range_buckets, resolve_account_type, roll_up,
};
use ledgeline_core::{parse_journal, wire};
use std::collections::{BTreeMap, BTreeSet};
use std::hint::black_box;
use std::time::Duration;

/// Report end date: the corpus's last transaction date. Fixed, so bucket math is
/// identical on every run and every machine.
const AS_OF: &str = "2025-12-31";
/// Insights compares two halves of this span (the corpus's last two years).
const INSIGHTS_START: &str = "2024-01-01";
/// First month of the periodic balance-sheet bench: 30 month ends up to `AS_OF`.
const BS_COLUMNS_FROM: &str = "2023-07-01";
/// A date near the START of the 30-year price series: `PriceDb::latest`'s
/// reverse scan has to walk the whole list to reach it (PERF-5d).
const EARLY_DATE: &str = "1996-06-30";
/// A date at the END of the series: the reverse scan hits on its first step.
const LATE_DATE: &str = "2025-12-31";
/// Account depth used by every report bench — deep enough that depth clamping is
/// not what is being measured.
const DEPTH: usize = 4;

/// Everything derived from one corpus, computed once and shared by every bench
/// so setup cost never lands inside a measurement.
struct Fixture {
    id: String,
    txns: usize,
    text: String,
    journal: Journal,
    decls: Vec<AccountDecl>,
    declared: BTreeMap<String, AccountType>,
    /// Cost-inferred prices followed by the explicit `P` directives — exactly
    /// what `net_worth` and `insights` build a `PriceDb` from.
    all_prices: Vec<PriceDirective>,
    price_db: PriceDb,
    /// One `account_totals` pass, reused as `roll_up`'s input.
    totals: BTreeMap<String, MixedAmount>,
    /// Every posting's account name, in file order — the `O(T·P)` call sequence
    /// PERF-5e is about.
    posting_accounts: Vec<String>,
    scope: HoldingsScope,
}

impl Fixture {
    fn load(shape: &corpus::Shape) -> Self {
        let txns = shape.txns;
        let path = corpus::ensure_journal_for(shape);
        let source = path.to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let journal = parse_journal(&text, &source)
            .unwrap_or_else(|e| panic!("generated corpus must parse: {e}"));

        let decls = account_decls(&journal);
        let declared = declared_types(&decls);
        let mut all_prices = infer_market_prices(&journal.transactions)
            .expect("cost-inferred prices must not overflow");
        all_prices.extend_from_slice(&journal.prices);
        let price_db = PriceDb::build(&all_prices);
        let totals = account_totals(&journal.transactions, &PostingFilter::default())
            .expect("account totals must not overflow");
        let posting_accounts = journal
            .transactions
            .iter()
            .flat_map(|txn| txn.postings.iter().map(|p| p.account.0.clone()))
            .collect();
        let scope = HoldingsScope {
            accounts: BTreeSet::new(),
            mode: ScopeMode::Include,
            as_of: AS_OF.to_string(),
            gain_since: None,
            value_in: None,
        };

        Self {
            id: corpus::shape_id(shape),
            txns,
            text,
            journal,
            decls,
            declared,
            all_prices,
            price_db,
            totals,
            posting_accounts,
            scope,
        }
    }

    fn id(&self) -> String {
        self.id.clone()
    }
}

/// Load every requested corpus once. Parsing 200k costs ~200 ms and building the
/// derived tables costs seconds; doing it per group would dominate the run.
///
/// Defaults to the v1 sizes, so a plain `cargo bench` still produces exactly the
/// benchmark ids every recorded baseline holds. `LEDGELINE_BENCH_SHAPES=v2,v2fx`
/// swaps in the large-repo fixtures, whose ids carry their own prefix and so
/// cannot be confused with a v1 number.
fn fixtures() -> Vec<Fixture> {
    corpus::bench_shapes().iter().map(Fixture::load).collect()
}

/// Criterion's defaults (100 samples × 5 s warm-up) are unusable for a bench that
/// takes a second per iteration. Ten samples still gives a mean and a confidence
/// interval; it just will not detect a 2% regression — which is fine, every
/// finding here is claimed to be worth 3–10×.
fn heavy(group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>) {
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(5));
}

// ---------------------------------------------------------------------------
// parse — the baseline everything else is compared against
// ---------------------------------------------------------------------------

fn bench_parse(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("parse");
    heavy(&mut group);
    for fixture in fixtures {
        group.throughput(Throughput::Bytes(fixture.text.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("parse_journal", fixture.id()),
            fixture,
            |b, fixture| {
                b.iter(|| parse_journal(black_box(&fixture.text), "bench.journal").unwrap());
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// wire / snapshot — PERF-1 and PERF-1b
// ---------------------------------------------------------------------------

fn bench_wire(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("wire");
    heavy(&mut group);
    for fixture in fixtures {
        let id = fixture.id();
        group.throughput(Throughput::Elements(fixture.txns as u64));

        // What `Snapshot` does today: build a `serde_json::Value` tree and hold
        // it resident.
        group.bench_with_input(
            BenchmarkId::new("journal_to_value", &id),
            fixture,
            |b, fixture| {
                b.iter(|| wire::journal_to_value(black_box(&fixture.journal)).unwrap());
            },
        );

        // What PERF-1 proposes instead: serialize straight to bytes, once.
        group.bench_with_input(
            BenchmarkId::new("serialize_bytes", &id),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    serde_json::to_vec(&wire::journal_to_transactions(black_box(&fixture.journal)))
                        .unwrap()
                });
            },
        );

        // The per-request costs PERF-1 attributes to the `Value` tree: a deep
        // clone, then serde re-walking it. Both become an `Arc` bump + a memcpy
        // once the snapshot stores bytes.
        let value = wire::journal_to_value(&fixture.journal).unwrap();
        group.bench_with_input(BenchmarkId::new("clone_value", &id), &value, |b, value| {
            b.iter(|| black_box(value).clone());
        });
        group.bench_with_input(BenchmarkId::new("to_string", &id), &value, |b, value| {
            b.iter(|| serde_json::to_string(black_box(value)).unwrap());
        });

        // PERF-1b: `Snapshot::from_journal` deep-clones the journal for nothing.
        group.bench_with_input(
            BenchmarkId::new("clone_journal", &id),
            fixture,
            |b, fixture| {
                b.iter(|| black_box(&fixture.journal).clone());
            },
        );

        // The whole of `Snapshot::from_journal`, replicated: the 85%-of-startup
        // number. Kept in sync with `ledgeline-server`'s `Snapshot::from_journal`
        // by hand — that type is crate-private, so this bench cannot call it.
        // Post-PERF-1 it serializes straight to bytes and shares the journal.
        group.bench_with_input(
            BenchmarkId::new("snapshot_from_journal", &id),
            fixture,
            |b, fixture| {
                b.iter(|| snapshot_payloads(black_box(&fixture.journal)));
            },
        );
    }
    group.finish();
}

/// The `{"diagnostics": [...]}` envelope, matching `ledgeline-server`'s
/// `DiagnosticsBody`. `wire` only exposes the array and a pre-built `Value`, and
/// building the `Value` is exactly the cost PERF-1 removed.
#[derive(serde::Serialize)]
struct DiagnosticsBody<'a> {
    diagnostics: &'a [wire::WireDiagnostic],
}

/// What `ledgeline::Snapshot::from_journal` builds, replicated.
///
/// Post-PERF-1 this is seven `serde_json::to_vec` calls and NO journal clone —
/// the snapshot shares the editor's `Arc<Journal>` (PERF-1b), which is O(1) and
/// so contributes nothing worth modelling here. `/transactions` is about half
/// the total and `/api/diagnostics` is the next largest (it runs the balance
/// checks AND, since DRY-1, a holdings pass), so the server splits those two
/// onto threads of their own and runs the remaining five alongside; this mirrors
/// that split, because a sequential replica would report a snapshot build the
/// server no longer performs.
///
/// **Hand-kept copy.** `Snapshot` is `pub(crate)` in the server crate, so this
/// cannot call the real thing. If `Snapshot::from_journal` gains or loses a
/// payload, or stops overlapping them, update this or the number silently stops
/// meaning what it says.
fn snapshot_payloads(journal: &Journal) -> [Vec<u8>; 7] {
    let (transactions, diagnostics, rest) = std::thread::scope(|scope| {
        let transactions =
            scope.spawn(|| serde_json::to_vec(&wire::journal_to_transactions(journal)).unwrap());
        let diagnostics = scope.spawn(|| {
            serde_json::to_vec(&DiagnosticsBody {
                diagnostics: &wire::journal_to_all_diagnostics(journal),
            })
            .unwrap()
        });
        let rest = [
            serde_json::to_vec(&wire::version_value()).unwrap(),
            serde_json::to_vec(&wire::journal_to_accountnames(journal)).unwrap(),
            serde_json::to_vec(&wire::journal_to_prices(journal)).unwrap(),
            serde_json::to_vec(&wire::journal_to_commodities(journal)).unwrap(),
            serde_json::to_vec(&wire::journal_to_accounts(journal)).unwrap(),
        ];
        (
            transactions.join().unwrap(),
            diagnostics.join().unwrap(),
            rest,
        )
    });
    let [version, accountnames, prices, commodities, accounts] = rest;
    [
        version,
        accountnames,
        transactions,
        prices,
        commodities,
        accounts,
        diagnostics,
    ]
}

// ---------------------------------------------------------------------------
// reports — PERF-5 (bucket-count scaling) and PERF-5c (insights)
// ---------------------------------------------------------------------------

fn bench_reports(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("reports");
    heavy(&mut group);
    for fixture in fixtures {
        let id = fixture.id();
        group.throughput(Throughput::Elements(fixture.txns as u64));

        // PERF-5 claims clean LINEAR scaling in bucket count (545 ms at 12 →
        // 2,547 ms at 60). Three points make the slope visible; after the
        // single-pass rewrite they must be roughly equal.
        for count in [12usize, 24, 60] {
            group.bench_with_input(
                BenchmarkId::new(format!("net_worth/count_{count}"), &id),
                fixture,
                |b, fixture| {
                    let opts = NetWorthOpts {
                        end: AS_OF,
                        interval: Interval::Monthly,
                        count,
                        depth: DEPTH,
                        value_in: None,
                        declared: &fixture.declared,
                    };
                    b.iter(|| {
                        net_worth(
                            black_box(&fixture.journal.transactions),
                            &fixture.journal.prices,
                            &opts,
                        )
                        .unwrap()
                    });
                },
            );
        }

        for count in [12usize, 60] {
            group.bench_with_input(
                BenchmarkId::new(format!("cash_flow/count_{count}"), &id),
                fixture,
                |b, fixture| {
                    let is_cash = cash_predicate(&fixture.decls);
                    b.iter(|| {
                        cash_flow(
                            black_box(&fixture.journal.transactions),
                            AS_OF,
                            Interval::Monthly,
                            count,
                            DEPTH,
                            Some(&is_cash),
                        )
                        .unwrap()
                    });
                },
            );

            group.bench_with_input(
                BenchmarkId::new(format!("budget/count_{count}"), &id),
                fixture,
                |b, fixture| {
                    let opts = BudgetOpts {
                        end: AS_OF,
                        interval: Interval::Monthly,
                        count,
                        depth: DEPTH,
                        budget_desc: None,
                    };
                    b.iter(|| {
                        budget_report(
                            black_box(&fixture.journal.transactions),
                            &fixture.journal.periodic_transactions,
                            &opts,
                        )
                        .unwrap()
                    });
                },
            );
        }

        group.bench_with_input(
            BenchmarkId::new("balance_sheet", &id),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    balance_sheet(
                        black_box(&fixture.journal.transactions),
                        AS_OF,
                        DEPTH,
                        &fixture.declared,
                    )
                    .unwrap()
                });
            },
        );

        // Plan 28 Phase 3: a periodic grouped balance sheet. `n_calls` is the
        // fallback (one `balance_sheet_grouped` per column, O(columns × txns));
        // `one_pass` is `balance_sheet_columns`. Same 30 month ends, same output.
        let ends: Vec<String> = range_buckets(BS_COLUMNS_FROM, AS_OF, Some(Interval::Monthly))
            .unwrap()
            .into_iter()
            .map(|span| span.to)
            .collect();
        let bs_groups = account_groups(&fixture.journal);
        let bs_terms_map = bs_terms(&fixture.journal);
        let bs_opts = BsOpts {
            as_of: AS_OF,
            depth: None,
            value: Valuation::Market,
            value_in: None,
        };
        group.bench_with_input(
            BenchmarkId::new("balance_sheet_columns/one_pass_30", &id),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    balance_sheet_columns(
                        black_box(&fixture.journal.transactions),
                        &fixture.journal.prices,
                        &bs_opts,
                        &ends,
                        &fixture.declared,
                        &bs_groups,
                        &bs_terms_map,
                    )
                    .unwrap()
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("balance_sheet_columns/n_calls_30", &id),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    ends.iter()
                        .map(|as_of| {
                            balance_sheet_grouped(
                                black_box(&fixture.journal.transactions),
                                &fixture.journal.prices,
                                &BsOpts {
                                    as_of,
                                    ..bs_opts.clone()
                                },
                                &fixture.declared,
                                &bs_groups,
                                &bs_terms_map,
                            )
                            .unwrap()
                        })
                        .collect::<Vec<_>>()
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("income_statement", &id),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    income_statement(
                        black_box(&fixture.journal.transactions),
                        INSIGHTS_START,
                        AS_OF,
                        DEPTH,
                        &fixture.declared,
                    )
                    .unwrap()
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("subscriptions", &id),
            fixture,
            |b, fixture| {
                let exclude: Vec<String> = ledgeline_core::reports::DEFAULT_EXCLUDE_DESC
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect();
                let opts = SubscriptionOpts {
                    as_of: AS_OF,
                    exclude_desc: &exclude,
                    ..SubscriptionOpts::default()
                };
                b.iter(|| detect_subscriptions(black_box(&fixture.journal), &opts).unwrap());
            },
        );

        // PERF-5c: ~20 full passes, 1,066 ms at 200k.
        group.bench_with_input(BenchmarkId::new("insights", &id), fixture, |b, fixture| {
            let opts = InsightsOpts {
                start: INSIGHTS_START,
                end: AS_OF,
                cost_exclude: &[],
                change_min: Dec::zero(),
            };
            b.iter(|| insights(black_box(&fixture.journal), &opts).unwrap());
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// holdings — PERF-5b
// ---------------------------------------------------------------------------

fn bench_holdings(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("holdings");
    heavy(&mut group);
    for fixture in fixtures {
        let id = fixture.id();
        group.throughput(Throughput::Elements(fixture.txns as u64));

        group.bench_with_input(
            BenchmarkId::new("compute_holdings", &id),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    compute_holdings(
                        black_box(&fixture.journal.transactions),
                        &fixture.journal.prices,
                        &fixture.journal.accounts,
                        &fixture.journal.commodity_tags,
                        &fixture.scope,
                    )
                    .unwrap()
                });
            },
        );

        // The slowest endpoint measured (1,599 ms at 200k): `compute_holdings`
        // re-run per point. Should approach `compute_holdings` itself once the
        // pools are replayed once in date order.
        //
        // Both point counts are measured because the POINT of PERF-5b is that
        // cost stops scaling with the point count: `12` alone cannot tell a 12×
        // replay apart from a single one that happens to be slow. The pair is
        // the gate — `60 / 12` must collapse from ~5× to ~1×.
        for count in [12usize, 60] {
            group.bench_with_input(
                BenchmarkId::new(format!("holdings_series_{count}"), &id),
                fixture,
                |b, fixture| {
                    b.iter(|| {
                        holdings_series(
                            black_box(&fixture.journal.transactions),
                            &fixture.journal.prices,
                            &fixture.journal.accounts,
                            &fixture.journal.commodity_tags,
                            &fixture.scope,
                            &SeriesWindow::counted(Interval::Monthly, count),
                        )
                        .unwrap()
                    });
                },
            );
        }
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// prices — PERF-5d
// ---------------------------------------------------------------------------

fn bench_prices(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("prices");
    // Everything here is µs-to-ms scale, so a fuller sample is affordable — and
    // the early/late lookup pair needs the resolution to show convergence.
    group.sample_size(50);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    for fixture in fixtures {
        let id = fixture.id();

        // `PriceDb::build` deep-clones every directive, and is rebuilt per
        // `compute_holdings` and per `net_worth`.
        group.bench_with_input(
            BenchmarkId::new("PriceDb_build", &id),
            fixture,
            |b, fixture| {
                b.iter(|| PriceDb::build(black_box(&fixture.all_prices)));
            },
        );

        // The other half of "build the whole price database": an N-transaction
        // sort plus a `PriceDirective` (three `String` clones) per cost
        // annotation. `infer_market_prices` + `PriceDb_build` is what a caller
        // that only wants the base commodity — `subscriptions` — used to pay,
        // so the pair bounds what skipping the build can possibly win.
        group.bench_with_input(
            BenchmarkId::new("infer_market_prices", &id),
            fixture,
            |b, fixture| {
                b.iter(|| infer_market_prices(black_box(&fixture.journal.transactions)).unwrap());
            },
        );

        // The pair PERF-5d says must converge: 11.2 µs for an early date (a full
        // reverse scan of the ascending list) vs 0.004 µs for a recent one.
        let commodity = Commodity("AAPL".to_string());
        for (label, as_of) in [("early_date", EARLY_DATE), ("late_date", LATE_DATE)] {
            group.bench_with_input(
                BenchmarkId::new(format!("lookup_{label}"), &id),
                fixture,
                |b, fixture| {
                    b.iter(|| {
                        black_box(&fixture.price_db).lookup(black_box(&commodity), black_box(as_of))
                    });
                },
            );
        }
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// aggregate + account types — PERF-5e and PERF-5f
// ---------------------------------------------------------------------------

fn bench_aggregate(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("aggregate");
    heavy(&mut group);
    for fixture in fixtures {
        let id = fixture.id();
        group.throughput(Throughput::Elements(fixture.txns as u64));

        group.bench_with_input(
            BenchmarkId::new("account_totals", &id),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    account_totals(
                        black_box(&fixture.journal.transactions),
                        &PostingFilter::default(),
                    )
                    .unwrap()
                });
            },
        );

        // PERF-5f: `roll_up` clones the accumulator on every addition, so a
        // root's map is cloned once per descendant.
        group.bench_with_input(BenchmarkId::new("roll_up", &id), fixture, |b, fixture| {
            b.iter(|| roll_up(black_box(&fixture.totals)).unwrap());
        });
    }
    group.finish();

    let mut group = c.benchmark_group("accounts");
    heavy(&mut group);
    for fixture in fixtures {
        let id = fixture.id();
        group.throughput(Throughput::Elements(fixture.posting_accounts.len() as u64));

        // PERF-5e: 2–3 `String` allocations per call, called once per posting by
        // `subscriptions`, `net_worth`, `insights` and `sections`. Memoizing on
        // the snapshot should make this collapse to a map lookup.
        group.bench_with_input(
            BenchmarkId::new("resolve_account_type_sweep", &id),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    let mut hits = 0usize;
                    for account in black_box(&fixture.posting_accounts) {
                        if resolve_account_type(account, &fixture.declared).is_some() {
                            hits += 1;
                        }
                    }
                    hits
                });
            },
        );

        // Rebuilt on EVERY HTTP request today, though the snapshot is immutable.
        group.bench_with_input(
            BenchmarkId::new("declared_types", &id),
            fixture,
            |b, fixture| {
                b.iter(|| declared_types(black_box(&fixture.decls)));
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------

/// A small balanced scenario over the corpus's own accounts: what a seeded
/// projection looks like to the engine (a handful of monthly rules, each with
/// its counter-account), so the run is dominated by the journal, not the rules.
const PROJECTION_SCENARIO: &str = "; projection: bench
~ monthly  projection
    revenues:salary          $-9000.00
    assets:bank:chase:checking

~ monthly  projection
    expenses:housing:rent     $2500.00
    assets:bank:chase:checking

~ monthly  projection
    expenses:food:groceries    $800.00
    assets:bank:chase:checking

~ monthly  projection
    expenses:utilities:power   $150.00
    assets:bank:chase:checking
";
/// After the corpus's last transaction, so the cutoff is 2026-01-01 and the
/// whole history is "real before the cutoff" — the case the merge copies most.
const PROJECTION_TODAY: &str = "2026-01-10";

/// The projection run's cost, and how much of it is copying the journal.
///
/// - `clone_history`: `journal.transactions` deep-cloned into a new `Vec`,
///   which is what `merged_transactions` does with every real transaction
///   before the cutoff on every run.
/// - `merged_transactions`: the real function, over a 24-month forecast.
/// - `project_24m`: the whole `project()` call (forecast expansion, openings,
///   lookback, series, runway) — not the three statements the server then
///   builds over the merged list.
/// - `run_reference` / `run_cold` / `run_warm`: the SERVER-EQUIVALENT run
///   (plan 29): the projection plus the balance sheet columns, the income
///   statement columns, the cash flow statement and the Accounts reading
///   (Change). `run_reference` is the full scan the handler made before plan
///   29 (`project` and every reading over `merged_transactions`); `run_cold`
///   is a first request (`JournalFacts::new`, `RealSide::build`, then the
///   run); `run_warm` is every edit after it (the run against a built
///   `RealSide`). The `before` baseline recorded both cold and warm as the
///   full scan, which is what they were then.
fn bench_projections(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("projections");
    heavy(&mut group);
    for fx in fixtures {
        let scenario = serialize::scenario_from_text(
            PROJECTION_SCENARIO,
            "bench.journal",
            "bench",
            &fx.declared,
        )
        .expect("bench scenario must read");
        let opts = ProjectionOpts {
            today: PROJECTION_TODAY,
            interval: Interval::Monthly,
            count: 24,
            lookback: 2,
            depth: DEPTH,
            declared: &fx.declared,
            value_in: None,
        };
        let projection = project(
            &scenario,
            &fx.journal.transactions,
            &fx.journal.prices,
            &opts,
        )
        .expect("bench projection must run");
        group.throughput(Throughput::Elements(fx.txns as u64));
        group.bench_with_input(BenchmarkId::new("clone_history", fx.id()), fx, |b, fx| {
            b.iter(|| black_box(fx.journal.transactions.clone()));
        });
        group.bench_with_input(
            BenchmarkId::new("merged_transactions", fx.id()),
            fx,
            |b, fx| {
                b.iter(|| {
                    black_box(merged_transactions(
                        &fx.journal.transactions,
                        &projection.forecast,
                    ))
                });
            },
        );
        group.bench_with_input(BenchmarkId::new("project_24m", fx.id()), fx, |b, fx| {
            b.iter(|| {
                black_box(
                    project(
                        &scenario,
                        &fx.journal.transactions,
                        &fx.journal.prices,
                        &opts,
                    )
                    .expect("run"),
                )
            });
        });
        group.bench_with_input(BenchmarkId::new("run_reference", fx.id()), fx, |b, fx| {
            b.iter(|| black_box(full_scan_run(fx, &scenario)));
        });
        group.bench_with_input(BenchmarkId::new("run_cold", fx.id()), fx, |b, fx| {
            b.iter(|| {
                let facts = JournalFacts::new(&fx.journal).expect("facts");
                let real = RealSide::build(&fx.journal, &facts, &run_key(&facts)).expect("real");
                black_box(cached_run(&facts, &real, &scenario))
            });
        });
        let facts = JournalFacts::new(&fx.journal).expect("facts");
        let real = RealSide::build(&fx.journal, &facts, &run_key(&facts)).expect("real");
        group.bench_with_input(BenchmarkId::new("run_warm", fx.id()), fx, |b, _| {
            b.iter(|| black_box(cached_run(&facts, &real, &scenario)));
        });
    }
    group.finish();
}

/// The run's real-side key: the server's default request.
fn run_key(facts: &JournalFacts) -> RealKey {
    facts
        .key(PROJECTION_TODAY, Interval::Monthly, 2, None)
        .expect("key")
}

/// The run handler after plan 29: the forecast against a cached real side.
fn cached_run(facts: &JournalFacts, real: &RealSide, scenario: &Scenario) -> usize {
    let projection = project_cached(scenario, real, 24, DEPTH).expect("run");
    let statements = projected_statements(
        real,
        facts,
        &projection.forecast,
        &StatementOpts {
            valuation: Valuation::Market,
            bs_depth: None,
            cf_depth: DEPTH,
        },
    )
    .expect("statements");
    let accounts = projected_accounts(
        real,
        facts,
        &projection.forecast,
        &AccountsOpts {
            mode: BalanceMode::Change,
            depth: DEPTH,
            empty: false,
        },
    )
    .expect("accounts");
    statements.balance_sheet.len()
        + statements.income_statement.sections.len()
        + statements.cash_flow.sections.len()
        + accounts.rows.len()
}

/// The run handler before plan 29: `project`, the merged lists, and every
/// reading over them, each a full scan.
fn full_scan_run(fx: &Fixture, scenario: &Scenario) -> usize {
    let journal = &fx.journal;
    let declared = declared_types(&account_decls(journal));
    let bs_groups = account_groups(journal);
    let terms = bs_terms(journal);
    let sections = account_sections(journal);
    let is_groups = declared_groups(journal, IS_GROUP_TAG);
    let projection = project(
        scenario,
        &journal.transactions,
        &journal.prices,
        &ProjectionOpts {
            today: PROJECTION_TODAY,
            interval: Interval::Monthly,
            count: 24,
            lookback: 2,
            depth: DEPTH,
            declared: &declared,
            value_in: None,
        },
    )
    .expect("run");
    let forecast = &projection.forecast;
    let window = &forecast.window;
    let txns = merged_transactions(&journal.transactions, forecast);
    let prices = merged_prices(&journal.prices, forecast);
    let spans = range_buckets(&window.from, &window.end, Some(window.interval)).expect("spans");
    let ends: Vec<String> = spans.iter().map(|span| span.to.clone()).collect();
    let sheets = balance_sheet_columns(
        &txns,
        &prices,
        &BsOpts {
            as_of: &window.end,
            depth: None,
            value: Valuation::Market,
            value_in: forecast.target.clone(),
        },
        &ends,
        &declared,
        &bs_groups,
        &terms,
    )
    .expect("bs");
    let income = income_statement_columns(
        &txns,
        &prices,
        &IsOpts {
            from: &window.from,
            to: &window.end,
            value: Valuation::Market,
            value_in: forecast.target.clone(),
            compare: false,
        },
        window.interval,
        &declared,
        &sections,
        &is_groups,
    )
    .expect("is");
    let cash_flow = cash_flow_statement(
        &txns,
        &CfOpts {
            from: &window.from,
            to: &window.end,
            interval: Some(window.interval),
            depth: DEPTH,
        },
        &CfTags::from_journal(journal),
    )
    .expect("cf");
    let accounts = account_periods(
        &txns,
        &AccountPeriodsOpts {
            from: &window.from,
            to: &window.end,
            interval: window.interval,
            depth: DEPTH,
            mode: BalanceMode::Change,
            known: None,
        },
    )
    .expect("accounts");
    sheets.len() + income.sections.len() + cash_flow.sections.len() + accounts.rows.len()
}

// ---------------------------------------------------------------------------

fn all(c: &mut Criterion) {
    let fixtures = fixtures();
    bench_parse(c, &fixtures);
    bench_wire(c, &fixtures);
    bench_aggregate(c, &fixtures);
    bench_prices(c, &fixtures);
    bench_reports(c, &fixtures);
    bench_holdings(c, &fixtures);
    bench_projections(c, &fixtures);
}

criterion_group!(benches, all);
criterion_main!(benches);
