//! Plan 35 Phase 1: transfer matching ([`ledgeline_core::matching`]) — the
//! candidates (D3), pairing (D4), assignment (D6), score (D7), links and
//! broken links (D5), rejections and their cleanup (D12), and the D10
//! findings. (`tests/matching.rs` is the unrelated rules-file matcher.)

use ledgeline_core::edit::{EditOp, JournalEditor};
use ledgeline_core::matching::{
    self, BrokenReason, MatchSettings, PostingRef, Proposal, TxnOps, clearing_accounts, forget_ops,
    ignore_ops, link_ops, links, propose, reject_ops, rejections, unlink_ops,
};
use ledgeline_core::model::{Journal, Tindex};
use ledgeline_core::parse::parse_journal;
use ledgeline_core::view::Classes;
use ledgeline_core::view::checks::local_findings;
use proptest::prelude::*;
use std::collections::HashSet;

const TODAY: &str = "2026-12-31";

const ACCOUNTS: &str = "\
account assets:checking   ; type: C
account assets:savings    ; type: C
account liabilities:amex  ; type: L
account expenses:food     ; type: X
account transfers         ; clearing:
";

fn journal(body: &str) -> Journal {
    parse_journal(&format!("{ACCOUNTS}\n{body}"), "inline.journal").expect("journal parses")
}

fn settings() -> MatchSettings {
    MatchSettings {
        lookback_days: 0,
        ..MatchSettings::default()
    }
}

/// `(tindex, tindex)` of each proposal, for terse assertions.
fn pairs(proposals: &[Proposal]) -> Vec<(u32, u32)> {
    proposals.iter().map(|p| (p.a.txn.0, p.b.txn.0)).collect()
}

/// A two-posting transfer half: `real` pays `amount` into `clearing`.
fn half(date: &str, description: &str, real: &str, clearing: &str, amount: &str) -> String {
    format!("{date} {description}\n    {clearing}  {amount}\n    {real}\n\n")
}

/// The clearing posting (posting 0 in [`half`]) of the transaction `tindex`.
fn at(journal: &Journal, tindex: u32) -> PostingRef {
    let position = journal
        .transactions
        .iter()
        .position(|txn| txn.index == Tindex(tindex))
        .expect("transaction exists");
    PostingRef {
        position,
        txn: Tindex(tindex),
        posting: 0,
    }
}

/// `text` after applying `ops` as one batch.
fn apply(text: &str, ops: &TxnOps) -> String {
    let mut editor =
        JournalEditor::from_text("/ledgeline-match/inline.journal", text).expect("journal parses");
    let batch: Vec<_> = ops
        .iter()
        .map(|(index, ops)| {
            let fingerprint = editor.transaction_fingerprint(*index).expect("exists");
            (*index, fingerprint, ops.clone())
        })
        .collect();
    editor.apply_batch(&batch).expect("batch applies");
    editor.text()
}

// ---------------------------------------------------------------------------
// Clearing accounts and candidates (D2, D3)
// ---------------------------------------------------------------------------

#[test]
fn clearing_tag_is_inherited_by_subaccounts_and_exact_too() {
    let j = journal(
        &(half("2026-12-01", "a", "assets:checking", "transfers:cc", "$5")
            + &half("2026-12-01", "b", "assets:checking", "transfers", "$5")
            + &half("2026-12-01", "c", "assets:checking", "other:cc", "$5")),
    );
    let clearing = clearing_accounts(&j);
    assert!(clearing.contains("transfers"));
    assert!(clearing.contains("transfers:cc"));
    assert!(!clearing.contains("other:cc"));
    assert!(!clearing.contains("assets:checking"));
}

#[test]
fn a_name_that_only_starts_like_a_clearing_account_is_not_one() {
    // `transfersx` shares a prefix with `transfers` but is not under it; a
    // deeper declared clearing account is a root of its own, and a declared
    // but never-posted subaccount is listed too.
    let j = parse_journal(
        &format!(
            "account transfers  ; clearing:\naccount escrow:hold  ; clearing:\n\
             account transfers:unused\n\n{}{}{}",
            half("2026-12-01", "a", "assets:checking", "transfersx", "$5"),
            half(
                "2026-12-01",
                "b",
                "assets:checking",
                "escrow:hold:tax",
                "$5"
            ),
            half("2026-12-01", "c", "assets:checking", "escrow", "$5"),
        ),
        "inline.journal",
    )
    .expect("parses");
    let clearing: Vec<String> = clearing_accounts(&j).into_iter().collect();
    assert_eq!(
        clearing,
        [
            "escrow:hold",
            "escrow:hold:tax",
            "transfers",
            "transfers:unused"
        ]
    );
}

#[test]
fn no_clearing_declaration_means_no_candidates() {
    let text = half("2026-12-01", "a", "assets:checking", "transfers:cc", "$5")
        + &half("2026-12-02", "b", "liabilities:amex", "transfers:cc", "$-5");
    let j = parse_journal(&text, "inline.journal").expect("parses");
    assert!(!matching::has_clearing_accounts(&j));
    assert!(propose(&j, &settings(), TODAY).is_empty());
}

#[test]
fn cost_assertion_virtual_and_match_tags_are_not_candidates() {
    let j = journal(
        "\
2026-12-01 cost
    transfers:x  1 EUR @ $1.10
    assets:checking

2026-12-01 assertion
    transfers:x  $5 = $5
    assets:checking

2026-12-01 virtual
    (transfers:x)  $5

2026-12-01 ignored
    transfers:x  $5  ; match:ignore
    assets:checking

2026-12-01 plain
    transfers:x  $5
    assets:checking
",
    );
    let found: Vec<&str> = matching::candidates(&j, &settings(), TODAY)
        .iter()
        .map(|c| j.transactions[c.at.position].description.as_str())
        .collect();
    assert_eq!(found, vec!["plain"]);
    let unusual: Vec<&str> = matching::unusual_clearing_postings(&j)
        .iter()
        .map(|at| j.transactions[at.position].description.as_str())
        .collect();
    assert_eq!(unusual, vec!["cost", "assertion"]);
}

#[test]
fn lookback_limits_what_is_proposed() {
    let text = half(
        "2025-01-01",
        "old out",
        "assets:checking",
        "transfers:b",
        "$5",
    ) + &half(
        "2025-01-02",
        "old in",
        "assets:savings",
        "transfers:b",
        "$-5",
    ) + &half(
        "2026-12-01",
        "new out",
        "assets:checking",
        "transfers:b",
        "$7",
    ) + &half(
        "2026-12-02",
        "new in",
        "assets:savings",
        "transfers:b",
        "$-7",
    );
    let j = journal(&text);
    let limited = MatchSettings::default();
    assert_eq!(pairs(&propose(&j, &limited, TODAY)), vec![(3, 4)]);
    assert_eq!(
        pairs(&propose(&j, &settings(), TODAY)),
        vec![(1, 2), (3, 4)]
    );
}

// ---------------------------------------------------------------------------
// Pairing and assignment (D4, D6, D7)
// ---------------------------------------------------------------------------

#[test]
fn a_simple_transfer_pairs_and_is_clear() {
    let text = half(
        "2026-12-30",
        "AMEX EPAYMENT",
        "assets:checking",
        "transfers:cc",
        "$500",
    ) + &half(
        "2027-01-02",
        "PAYMENT THANK YOU",
        "liabilities:amex",
        "transfers:cc",
        "$-500",
    );
    let j = journal(&text);
    let proposals = propose(&j, &settings(), TODAY);
    assert_eq!(pairs(&proposals), vec![(1, 2)]);
    let p = &proposals[0];
    assert_eq!(p.gap_days, 3);
    // cash→card (+1) and the description names `amex` (+1).
    assert_eq!(p.score, 2);
    assert!(p.clear && !p.ambiguous && p.alternatives.is_empty());
    assert_eq!(
        p.real_accounts,
        [
            Some("assets:checking".to_string()),
            Some("liabilities:amex".to_string())
        ]
    );
}

#[test]
fn recurring_same_amount_transfers_pair_month_by_month() {
    let text: String = (1..=6)
        .flat_map(|month| {
            [
                half(
                    &format!("2026-{month:02}-01"),
                    "to savings",
                    "assets:checking",
                    "transfers:bank",
                    "$500",
                ),
                half(
                    &format!("2026-{month:02}-03"),
                    "from checking",
                    "assets:savings",
                    "transfers:bank",
                    "$-500",
                ),
            ]
        })
        .collect();
    let j = journal(&text);
    let proposals = propose(&j, &settings(), TODAY);
    assert_eq!(
        pairs(&proposals),
        vec![(1, 2), (3, 4), (5, 6), (7, 8), (9, 10), (11, 12)]
    );
    assert!(proposals.iter().all(|p| p.gap_days == 2 && p.clear));
}

#[test]
fn smallest_gap_wins_over_file_order() {
    // Greedy first-fit would pair 1 with 2; 3 is closer to 2.
    let text = half(
        "2026-12-01",
        "out early",
        "assets:checking",
        "transfers:b",
        "$9",
    ) + &half("2026-12-09", "in", "assets:savings", "transfers:b", "$-9")
        + &half(
            "2026-12-08",
            "out late",
            "assets:checking",
            "transfers:b",
            "$9",
        );
    let j = journal(&text);
    let proposals = propose(&j, &settings(), TODAY);
    assert_eq!(pairs(&proposals), vec![(3, 2)]);
    assert!(!proposals[0].ambiguous);
    assert_eq!(proposals[0].alternatives.len(), 1);
    assert!(!proposals[0].alternatives[0].taken);
}

#[test]
fn ties_are_ambiguous_and_never_clear() {
    let text = half(
        "2026-12-01",
        "out one",
        "assets:checking",
        "transfers:b",
        "$9",
    ) + &half("2026-12-03", "in", "assets:savings", "transfers:b", "$-9")
        + &half(
            "2026-12-05",
            "out two",
            "assets:checking",
            "transfers:b",
            "$9",
        );
    let j = journal(&text);
    let proposals = propose(&j, &settings(), TODAY);
    assert_eq!(proposals.len(), 1);
    let p = &proposals[0];
    assert!(p.ambiguous && !p.clear);
    assert_eq!(p.alternatives.len(), 1);
}

#[test]
fn the_score_breaks_a_tie_in_gap() {
    // Both outs are 2 days from the in; the expense-funded one scores lower.
    let text = half("2026-12-01", "out", "expenses:food", "transfers:b", "$9")
        + &half("2026-12-03", "in", "assets:savings", "transfers:b", "$-9")
        + &half("2026-12-05", "out", "assets:checking", "transfers:b", "$9");
    let j = journal(&text);
    let proposals = propose(&j, &settings(), TODAY);
    assert_eq!(pairs(&proposals), vec![(2, 3)]);
    assert!(!proposals[0].ambiguous && proposals[0].clear);
}

#[test]
fn a_same_account_pair_scores_negative_and_needs_checking() {
    let text = half("2026-12-01", "x", "assets:checking", "transfers:b", "$9")
        + &half("2026-12-02", "y", "assets:checking", "transfers:b", "$-9");
    let p = &propose(&journal(&text), &settings(), TODAY)[0];
    assert_eq!(p.score, -1);
    assert!(!p.clear);
}

#[test]
fn posting_dates_decide_the_window() {
    // The transactions are 20 days apart, the posting dates 2.
    let text = "\
2026-11-01 out
    transfers:b  $9  ; date:2026-11-20
    assets:checking

2026-11-21 in
    transfers:b  $-9  ; date:2026-11-22
    assets:savings

2026-12-01 far out
    transfers:b  $4
    assets:checking

2026-12-01 far in
    transfers:b  $-4  ; date:2026-12-20
    assets:savings
";
    let proposals = propose(&journal(text), &settings(), TODAY);
    assert_eq!(pairs(&proposals), vec![(1, 2)]);
    assert_eq!(proposals[0].gap_days, 2);
}

#[test]
fn window_is_inclusive_and_configurable() {
    let text = half("2026-12-01", "out", "assets:checking", "transfers:b", "$9")
        + &half("2026-12-11", "in", "assets:savings", "transfers:b", "$-9");
    let j = journal(&text);
    assert_eq!(propose(&j, &settings(), TODAY).len(), 1);
    let narrow = MatchSettings {
        window_days: 9,
        ..settings()
    };
    assert!(propose(&j, &narrow, TODAY).is_empty());
}

#[test]
fn commodity_account_and_exact_amount_must_agree() {
    let text = half(
        "2026-12-01",
        "eur",
        "assets:checking",
        "transfers:b",
        "9 EUR",
    ) + &half("2026-12-01", "usd", "assets:savings", "transfers:b", "$-9")
        + &half(
            "2026-12-01",
            "other clearing",
            "assets:savings",
            "transfers:c",
            "-9 EUR",
        )
        + &half(
            "2026-12-01",
            "off by a cent",
            "assets:savings",
            "transfers:b",
            "-9.01 EUR",
        )
        + &half(
            "2026-12-01",
            "same sign",
            "assets:savings",
            "transfers:b",
            "9.00 EUR",
        );
    assert!(propose(&journal(&text), &settings(), TODAY).is_empty());
    // Scale does not matter: 9 EUR and -9.00 EUR cancel.
    let text = half("2026-12-01", "a", "assets:checking", "transfers:b", "9 EUR")
        + &half(
            "2026-12-01",
            "b",
            "assets:savings",
            "transfers:b",
            "-9.00 EUR",
        );
    assert_eq!(
        pairs(&propose(&journal(&text), &settings(), TODAY)),
        vec![(1, 2)]
    );
}

#[test]
fn two_clearing_postings_in_one_transaction_never_pair_with_each_other() {
    let text = "\
2026-12-01 split
    transfers:b  $9
    transfers:b  $-9

2026-12-02 in
    transfers:b  $-9
    assets:savings
";
    let proposals = propose(&journal(text), &settings(), TODAY);
    assert_eq!(pairs(&proposals), vec![(1, 2)]);
    assert_eq!(proposals[0].a.posting, 0);
}

#[test]
fn involving_scopes_after_global_assignment() {
    let text = half("2026-12-01", "out", "assets:checking", "transfers:b", "$9")
        + &half("2026-12-02", "in", "assets:savings", "transfers:b", "$-9")
        + &half(
            "2026-12-09",
            "new out",
            "assets:checking",
            "transfers:b",
            "$9",
        );
    let j = journal(&text);
    let scoped = MatchSettings {
        involving: Some(HashSet::from([Tindex(3)])),
        ..settings()
    };
    // The new posting's only partner is already better paired with 1.
    assert!(propose(&j, &scoped, TODAY).is_empty());
    let scoped = MatchSettings {
        involving: Some(HashSet::from([Tindex(2)])),
        ..settings()
    };
    assert_eq!(pairs(&propose(&j, &scoped, TODAY)), vec![(1, 2)]);
}

// ---------------------------------------------------------------------------
// Links (D1, D5)
// ---------------------------------------------------------------------------

#[test]
fn link_then_unlink_restores_the_bytes() {
    let text = format!(
        "{ACCOUNTS}\n{}{}",
        half(
            "2026-12-30",
            "out",
            "assets:checking",
            "transfers:cc",
            "$500"
        ),
        half(
            "2027-01-02",
            "in",
            "liabilities:amex",
            "transfers:cc",
            "$-500"
        )
    );
    let j = parse_journal(&text, "inline.journal").expect("parses");
    let ops = link_ops(&j, at(&j, 1), at(&j, 2), "k7f3a2").expect("legal");
    let linked = apply(&text, &ops);
    assert_eq!(
        linked,
        text.replace(
            "transfers:cc  $500\n",
            "transfers:cc  $500  ; match:k7f3a2\n"
        )
        .replace(
            "transfers:cc  $-500\n",
            "transfers:cc  $-500  ; match:k7f3a2\n"
        )
    );
    let lj = parse_journal(&linked, "inline.journal").expect("parses");
    let index = links(&lj);
    assert_eq!(index.get("k7f3a2"), Some((at(&lj, 1), at(&lj, 2))));
    assert_eq!(index.link_count(at(&lj, 1).position), 1);
    assert!(index.broken().is_empty());
    // Linked postings are no longer proposed.
    assert!(propose(&lj, &settings(), TODAY).is_empty());

    let unlinked = apply(&linked, &unlink_ops(&lj, "k7f3a2").expect("known"));
    assert_eq!(unlinked, text);
}

#[test]
fn link_ops_refuse_illegal_pairs() {
    let text = half("2026-12-01", "a", "assets:checking", "transfers:b", "$9")
        + &half("2026-12-02", "b", "assets:savings", "transfers:b", "$-8")
        + &half("2026-12-02", "c", "assets:savings", "transfers:c", "$-9")
        + &half("2026-12-02", "d", "assets:savings", "assets:other", "$-9");
    let j = journal(&text);
    for other in [2, 3, 4] {
        assert!(link_ops(&j, at(&j, 1), at(&j, other), "abc").is_err());
    }
    assert!(link_ops(&j, at(&j, 1), at(&j, 1), "abc").is_err());
}

#[test]
fn broken_links_are_detected() {
    let text = "\
2026-12-01 lonely
    transfers:b  $9  ; match:one
    assets:checking

2026-12-01 bad sum a
    transfers:b  $9  ; match:sum
    assets:checking

2026-12-02 bad sum b
    transfers:b  $-8  ; match:sum
    assets:savings

2026-12-01 off clearing a
    assets:checking  $9  ; match:off
    assets:savings

2026-12-02 off clearing b
    transfers:b  $-9  ; match:off
    assets:savings

2026-12-03 good a
    transfers:b  $5  ; match:good
    assets:checking

2026-12-03 good b
    transfers:b  $-5  ; match:good
    assets:savings

2026-12-04 crowd
    transfers:b  $5  ; match:three
    transfers:b  $5  ; match:three
    transfers:b  $-10  ; match:three
";
    let j = journal(text);
    let index = links(&j);
    let broken: Vec<(&str, BrokenReason)> = index
        .broken()
        .iter()
        .map(|link| (link.id.as_str(), link.reason))
        .collect();
    assert_eq!(
        broken,
        vec![
            ("off", BrokenReason::NotClearing),
            ("one", BrokenReason::Count(1)),
            ("sum", BrokenReason::DoNotCancel),
            ("three", BrokenReason::SameTransaction),
        ]
    );
    assert_eq!(index.pairs().keys().collect::<Vec<_>>(), vec!["good"]);

    let findings = local_findings(&j, &Classes::of(&j), TODAY);
    let broken_rows: Vec<usize> = findings
        .iter()
        .filter(|f| f.rule == "broken-match")
        .filter_map(|f| f.position)
        .collect();
    assert_eq!(broken_rows, vec![3, 4, 0, 1, 2, 7]);
}

#[test]
fn a_transaction_in_two_links_reports_both() {
    let text = "\
2026-12-01 two legs
    transfers:b  $9  ; match:aaa
    transfers:b  $5  ; match:bbb
    assets:checking

2026-12-02 in one
    transfers:b  $-9  ; match:aaa
    assets:savings

2026-12-02 in two
    transfers:b  $-5  ; match:bbb
    assets:savings
";
    let j = journal(text);
    let index = links(&j);
    assert_eq!(index.link_count(0), 2);
    assert_eq!(index.links_at(0), ["aaa".to_string(), "bbb".to_string()]);
    assert_eq!(index.link_count(1), 1);
}

// ---------------------------------------------------------------------------
// Rejections (D12)
// ---------------------------------------------------------------------------

#[test]
fn a_rejected_pair_is_not_proposed_but_each_half_still_pairs_elsewhere() {
    let text = half("2026-12-01", "out", "assets:checking", "transfers:b", "$9")
        + &half(
            "2026-12-02",
            "in near",
            "assets:savings",
            "transfers:b",
            "$-9",
        )
        + &half(
            "2026-12-06",
            "in far",
            "assets:savings",
            "transfers:b",
            "$-9",
        );
    let full = format!("{ACCOUNTS}\n{text}");
    let j = parse_journal(&full, "inline.journal").expect("parses");
    assert_eq!(pairs(&propose(&j, &settings(), TODAY)), vec![(1, 2)]);

    let rejected = apply(
        &full,
        &reject_ops(&j, at(&j, 1), at(&j, 2), "r4x9").expect("legal"),
    );
    let rj = parse_journal(&rejected, "inline.journal").expect("parses");
    assert_eq!(pairs(&propose(&rj, &settings(), TODAY)), vec![(1, 3)]);
    let r = rejections(&rj);
    assert!(r.is_rejected(at(&rj, 1), at(&rj, 2)));
    assert_eq!(r.pairs().count(), 1);

    // Forgetting puts the file back.
    assert_eq!(
        apply(&rejected, &forget_ops(&rj, "r4x9").expect("known")),
        full
    );
}

#[test]
fn linking_cleans_up_related_rejections_on_both_sides_only() {
    let full = format!(
        "{ACCOUNTS}\n\
2026-12-01 out
    transfers:b  $9  ; nomatch:aaa111, keep:me
    assets:checking

2026-12-02 rejected partner
    transfers:b  $-9  ; nomatch:aaa111, nomatch:zzz999
    assets:savings

2026-12-03 in
    transfers:b  $-9
    assets:savings

2026-12-04 unrelated
    transfers:b  $-9  ; nomatch:zzz999
    assets:savings
"
    );
    let j = parse_journal(&full, "inline.journal").expect("parses");
    let ops = link_ops(&j, at(&j, 1), at(&j, 3), "k1").expect("legal");
    let out = apply(&full, &ops);
    assert_eq!(
        out,
        full.replace("$9  ; nomatch:aaa111, keep:me", "$9  ; keep:me, match:k1")
            .replace(
                "$-9  ; nomatch:aaa111, nomatch:zzz999",
                "$-9  ; nomatch:zzz999"
            )
            .replace(
                "2026-12-03 in\n    transfers:b  $-9\n",
                "2026-12-03 in\n    transfers:b  $-9  ; match:k1\n"
            )
    );
    // Both postings of each op touched transaction appear once, in file order.
    let tindexes: Vec<u32> = ops.iter().map(|(t, _)| t.0).collect();
    assert_eq!(tindexes, vec![1, 2, 3]);
}

#[test]
fn ignore_marks_the_posting_and_clears_its_rejections() {
    let full = format!(
        "{ACCOUNTS}\n\
2026-12-01 out
    transfers:b  $9  ; nomatch:aaa111
    assets:checking

2026-12-02 partner
    transfers:b  $-9  ; nomatch:aaa111
    assets:savings
"
    );
    let j = parse_journal(&full, "inline.journal").expect("parses");
    let out = apply(&full, &ignore_ops(&j, at(&j, 1)).expect("matchable"));
    assert_eq!(
        out,
        full.replace("$9  ; nomatch:aaa111", "$9  ; match:ignore")
            .replace("$-9  ; nomatch:aaa111", "$-9")
    );
    let ij = parse_journal(&out, "inline.journal").expect("parses");
    assert!(matching::candidates(&ij, &settings(), TODAY).len() == 1);
    assert!(links(&ij).broken().is_empty(), "ignore is not a link");
}

#[test]
fn ops_are_posting_tag_ops_only() {
    let text = half("2026-12-01", "a", "assets:checking", "transfers:b", "$9")
        + &half("2026-12-02", "b", "assets:savings", "transfers:b", "$-9");
    let j = journal(&text);
    let ops = link_ops(&j, at(&j, 1), at(&j, 2), "abc123").expect("legal");
    assert!(ops.iter().flat_map(|(_, ops)| ops).all(|op| matches!(
        op,
        EditOp::SetPostingTag { .. } | EditOp::RemovePostingTagValue { .. }
    )));
}

#[test]
fn new_ids_do_not_collide_with_existing_values() {
    let j = journal("2026-12-01 a\n    transfers:b  $9  ; match:abc\n    assets:checking\n");
    let id = matching::new_match_id(&j);
    assert_eq!(id.len(), 6);
    assert_ne!(id, "abc");
}

// ---------------------------------------------------------------------------
// Findings (D10)
// ---------------------------------------------------------------------------

#[test]
fn in_transit_flags_old_unpartnered_candidates_only() {
    let text = half(
        "2026-11-01",
        "old alone",
        "assets:checking",
        "transfers:b",
        "$9",
    ) + &half(
        "2026-12-28",
        "recent alone",
        "assets:checking",
        "transfers:b",
        "$7",
    ) + &half(
        "2026-10-01",
        "old paired out",
        "assets:checking",
        "transfers:b",
        "$5",
    ) + &half(
        "2026-10-02",
        "old paired in",
        "assets:savings",
        "transfers:b",
        "$-5",
    ) + "2026-10-01 ignored\n    transfers:b  $3  ; match:ignore\n    assets:checking\n\n"
        + "2026-10-01 unusual\n    transfers:b  $3 = $3\n    assets:checking\n";
    let j = journal(&text);
    let findings = local_findings(&j, &Classes::of(&j), TODAY);
    let rows = |rule: &str| -> Vec<&str> {
        findings
            .iter()
            .filter(|f| f.rule == rule)
            .map(|f| {
                j.transactions[f.position.expect("txn")]
                    .description
                    .as_str()
            })
            .collect()
    };
    assert_eq!(rows("in-transit"), vec!["old alone"]);
    assert_eq!(rows("unusual-clearing"), vec!["unusual"]);
    assert!(rows("broken-match").is_empty());
}

#[test]
fn a_journal_without_clearing_accounts_gets_no_matching_findings() {
    let text = half("2020-01-01", "a", "assets:checking", "transfers:b", "$9");
    let j = parse_journal(&text, "inline.journal").expect("parses");
    let findings = local_findings(&j, &Classes::of(&j), TODAY);
    assert!(
        findings
            .iter()
            .all(|f| !matches!(f.rule, "in-transit" | "broken-match" | "unusual-clearing"))
    );
}

// ---------------------------------------------------------------------------
// Property: proposals are disjoint and cancel exactly
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn proposals_are_disjoint_and_cancel(
        rows in prop::collection::vec(
            (0u32..40, 0usize..2, prop::sample::select(vec![-3i32, -2, 2, 3, 5, -5]), 0usize..3, any::<bool>()),
            1..40,
        )
    ) {
        let clearing = ["transfers:a", "transfers:b"];
        let real = ["assets:checking", "assets:savings", "liabilities:amex"];
        let body: String = rows
            .iter()
            .map(|(day, account, amount, other, euro)| {
                let date = format!("2026-{:02}-{:02}", 1 + day / 28, 1 + day % 28);
                let amount = if *euro { format!("{amount} EUR") } else { format!("${amount}") };
                half(&date, "t", real[*other], clearing[*account], &amount)
            })
            .collect();
        let j = journal(&body);
        let proposals = propose(&j, &settings(), TODAY);
        let mut seen: HashSet<PostingRef> = HashSet::new();
        for p in &proposals {
            let postings: Vec<PostingRef> = p.postings().collect();
            for at in &postings {
                prop_assert!(seen.insert(*at), "{:?} proposed twice", at);
            }
            let legs: Vec<_> = postings
                .iter()
                .map(|at| &j.transactions[at.position].postings[at.posting])
                .collect();
            let first = legs[0];
            let mut sum = ledgeline_core::decimal::Dec::zero();
            for leg in &legs {
                prop_assert_eq!(&leg.account, &first.account);
                prop_assert_eq!(&leg.amounts[0].commodity, &first.amounts[0].commodity);
                sum = sum.add(leg.amounts[0].quantity).expect("add");
            }
            prop_assert!(sum.is_zero(), "{:?} does not sum to zero", p);
            let positions: HashSet<usize> = postings.iter().map(|at| at.position).collect();
            prop_assert_eq!(positions.len(), postings.len());
            prop_assert!(p.gap_days <= settings().window_days);
            if p.is_combination() {
                prop_assert!((2..=4).contains(&p.group.len()));
                prop_assert_eq!(p.b, p.group[0]);
                prop_assert!(!p.clear);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Amendment A: group links (A3), suggested combinations (A4), in transit
// (A5), the `older` count (A1) and one transaction's partners (A2)
// ---------------------------------------------------------------------------

/// A Bill.com-like debit: #1 pays $2,247.50 into `transfers:bills`, and #2,
/// #3 take $2,000 and $247.50 out the same day.
fn bill_pay() -> String {
    half(
        "2026-12-01",
        "BILL.COM DEBIT",
        "assets:checking",
        "transfers:bills",
        "$2247.50",
    ) + &half(
        "2026-12-01",
        "Rent bill",
        "expenses:rent",
        "transfers:bills",
        "$-2000.00",
    ) + &half(
        "2026-12-01",
        "Hosting bill",
        "expenses:hosting",
        "transfers:bills",
        "$-247.50",
    )
}

/// `(single, [members])` tindexes of each combination.
fn combos(proposals: &[Proposal]) -> Vec<(u32, Vec<u32>)> {
    proposals
        .iter()
        .filter(|p| p.is_combination())
        .map(|p| (p.a.txn.0, p.group.iter().map(|at| at.txn.0).collect()))
        .collect()
}

#[test]
fn a_unique_combination_is_proposed_as_check() {
    let j = journal(&bill_pay());
    let proposals = propose(&j, &settings(), TODAY);
    assert_eq!(combos(&proposals), vec![(1, vec![2, 3])]);
    let combination = &proposals[0];
    assert_eq!(combination.b, combination.group[0]);
    assert!(!combination.clear && !combination.ambiguous);
    assert_eq!(combination.gap_days, 0);
    assert!(combination.alternatives.is_empty());
    assert_eq!(
        combination.real_accounts[0].as_deref(),
        Some("assets:checking")
    );
}

#[test]
fn members_are_listed_nearest_first_and_the_gap_is_the_widest() {
    let text = half(
        "2026-12-05",
        "debit",
        "assets:checking",
        "transfers:bills",
        "$30",
    ) + &half("2026-12-01", "far", "expenses:a", "transfers:bills", "$-10")
        + &half(
            "2026-12-04",
            "near",
            "expenses:b",
            "transfers:bills",
            "$-20",
        );
    let j = journal(&text);
    let proposals = propose(&j, &settings(), TODAY);
    assert_eq!(combos(&proposals), vec![(1, vec![3, 2])]);
    assert_eq!(proposals[0].gap_days, 4);
}

#[test]
fn two_subsets_summing_to_the_amount_are_a_guess_and_not_proposed() {
    // $300 = 100 + 200 = 150 + 150.
    let text = half(
        "2026-12-10",
        "debit",
        "assets:checking",
        "transfers:bills",
        "$300",
    ) + &half("2026-12-10", "a", "expenses:a", "transfers:bills", "$-100")
        + &half("2026-12-10", "b", "expenses:b", "transfers:bills", "$-200")
        + &half("2026-12-10", "c", "expenses:c", "transfers:bills", "$-150")
        + &half("2026-12-10", "d", "expenses:d", "transfers:bills", "$-150");
    let j = journal(&text);
    assert!(combos(&propose(&j, &settings(), TODAY)).is_empty());
    // Two equal amounts are two subsets too: $200 = 100 (#2) + 100 (#3).
    let text = half(
        "2026-12-10",
        "debit",
        "assets:checking",
        "transfers:bills",
        "$200",
    ) + &half("2026-12-10", "a", "expenses:a", "transfers:bills", "$-100")
        + &half("2026-12-10", "b", "expenses:b", "transfers:bills", "$-100")
        + &half("2026-12-10", "c", "expenses:c", "transfers:bills", "$-100");
    assert!(combos(&propose(&journal(&text), &settings(), TODAY)).is_empty());
}

#[test]
fn combinations_hold_two_to_four_members() {
    // Distinct powers of two: each total has exactly one subset.
    let set = |amounts: &[u32]| {
        let total: u32 = amounts.iter().sum();
        let mut text = half(
            "2026-12-10",
            "debit",
            "assets:checking",
            "transfers:bills",
            &format!("${total}"),
        );
        for amount in amounts {
            text += &half(
                "2026-12-10",
                "bill",
                "expenses:a",
                "transfers:bills",
                &format!("$-{amount}"),
            );
        }
        journal(&text)
    };
    for (amounts, expected) in [
        (&[1, 2, 4, 8][..], 1),
        (&[1, 2, 4][..], 1),
        (&[1, 2, 4, 8, 16][..], 0),
    ] {
        let found = combos(&propose(&set(amounts), &settings(), TODAY));
        assert_eq!(found.len(), expected, "{amounts:?}: {found:?}");
        if let Some((_, members)) = found.first() {
            assert_eq!(members.len(), amounts.len());
        }
    }
}

#[test]
fn a_posting_in_a_pair_is_never_in_a_combination() {
    // #4 pairs one-to-one with #2 ($-2000 / $2000); the debit's only
    // remaining subset would need #2, so it gets none.
    let text = bill_pay()
        + &half(
            "2026-12-01",
            "refund in",
            "assets:savings",
            "transfers:bills",
            "$2000.00",
        );
    let j = journal(&text);
    let proposals = propose(&j, &settings(), TODAY);
    assert_eq!(pairs(&proposals), vec![(2, 4)]);
    assert!(combos(&proposals).is_empty());
}

#[test]
fn no_posting_is_in_two_combinations_and_the_tighter_one_wins() {
    // Two $30 debits, #1 four days from the bills and #4 the same day: both
    // could take #2 + #3; #4's widest gap is smaller, so it wins.
    let text = half(
        "2026-12-01",
        "debit far",
        "assets:checking",
        "transfers:bills",
        "$30",
    ) + &half("2026-12-05", "a", "expenses:a", "transfers:bills", "$-10")
        + &half("2026-12-05", "b", "expenses:b", "transfers:bills", "$-20")
        + &half(
            "2026-12-05",
            "debit near",
            "assets:checking",
            "transfers:bills",
            "$30",
        );
    let j = journal(&text);
    assert_eq!(
        combos(&propose(&j, &settings(), TODAY)),
        vec![(4, vec![2, 3])]
    );
}

#[test]
fn a_rejected_member_keeps_its_subset_out() {
    let text = bill_pay()
        .replacen(
            "transfers:bills  $2247.50\n",
            "transfers:bills  $2247.50  ; nomatch:rej001\n",
            1,
        )
        .replacen(
            "transfers:bills  $-247.50\n",
            "transfers:bills  $-247.50  ; nomatch:rej001\n",
            1,
        );
    let j = journal(&text);
    assert!(combos(&propose(&j, &settings(), TODAY)).is_empty());
}

#[test]
fn members_outside_the_window_or_on_another_account_are_not_sought() {
    let far = bill_pay().replacen("2026-12-01 Hosting bill", "2026-12-20 Hosting bill", 1);
    assert!(combos(&propose(&journal(&far), &settings(), TODAY)).is_empty());
    let other = bill_pay().replacen("transfers:bills  $-247.50", "transfers:other  $-247.50", 1);
    assert!(combos(&propose(&journal(&other), &settings(), TODAY)).is_empty());
}

#[test]
fn an_imports_scope_keeps_a_combination_with_a_new_posting() {
    let j = journal(&bill_pay());
    let scoped = |txn: u32| MatchSettings {
        involving: Some(HashSet::from([Tindex(txn)])),
        ..settings()
    };
    assert_eq!(propose(&j, &scoped(3), TODAY).len(), 1);
    assert!(propose(&j, &scoped(9), TODAY).is_empty());
}

#[test]
fn a_posting_in_a_combination_is_not_in_transit() {
    // Old enough to be flagged; the ambiguous debit (#4) and its four
    // candidates are, the Bill.com three are not.
    let ambiguous = half(
        "2026-11-10",
        "debit",
        "assets:checking",
        "transfers:bills",
        "$300",
    ) + &half("2026-11-10", "a", "expenses:a", "transfers:bills", "$-100")
        + &half("2026-11-10", "b", "expenses:b", "transfers:bills", "$-200")
        + &half("2026-11-10", "c", "expenses:c", "transfers:bills", "$-150")
        + &half("2026-11-10", "d", "expenses:d", "transfers:bills", "$-150");
    let j = journal(&(bill_pay() + &ambiguous));
    let flagged: Vec<u32> = matching::in_transit(&j, 10, 10, TODAY)
        .iter()
        .map(|c| c.at.txn.0)
        .collect();
    assert_eq!(flagged, vec![4, 5, 6, 7, 8]);
}

#[test]
fn the_older_count_runs_the_matcher_again_with_no_lookback() {
    // #1/#2 are older than a 30-day lookback; #3/#4 are inside it. #5 is
    // inside, and its nearest partner #6 is outside: with no lookback it
    // pairs with #6, with the lookback it pairs with nobody — so the count
    // is not "the unlimited proposals with both halves inside".
    let text = half("2026-06-01", "old", "assets:checking", "transfers:b", "$5")
        + &half("2026-06-02", "old", "assets:savings", "transfers:b", "$-5")
        + &half("2026-12-10", "new", "assets:checking", "transfers:b", "$7")
        + &half("2026-12-11", "new", "assets:savings", "transfers:b", "$-7")
        + &half("2026-12-02", "edge", "assets:checking", "transfers:b", "$9")
        + &half("2026-11-28", "edge", "assets:savings", "transfers:b", "$-9");
    let j = journal(&text);
    let lookback = MatchSettings {
        lookback_days: 30,
        ..MatchSettings::default()
    };
    let all = settings();
    assert_eq!(matching::count_proposals(&j, &lookback, TODAY), 1);
    assert_eq!(propose(&j, &lookback, TODAY).len(), 1);
    assert_eq!(matching::count_proposals(&j, &all, TODAY), 3);
    assert_eq!(propose(&j, &all, TODAY).len(), 3);
    // The combination counts too.
    assert_eq!(
        matching::count_proposals(&journal(&bill_pay()), &all, TODAY),
        1
    );
}

// --- A3: group links ------------------------------------------------------

#[test]
fn a_group_link_writes_one_id_on_every_member_and_unlink_restores_the_bytes() {
    let text = format!("{ACCOUNTS}\n{}", bill_pay());
    let j = parse_journal(&text, "inline.journal").expect("parses");
    let members = [at(&j, 1), at(&j, 2), at(&j, 3)];
    let ops = matching::link_group_ops(&j, &members, "grp001").expect("a legal group");
    let linked = apply(&text, &ops);
    assert_eq!(
        linked,
        text.replace(
            "transfers:bills  $2247.50\n",
            "transfers:bills  $2247.50  ; match:grp001\n"
        )
        .replace(
            "transfers:bills  $-2000.00\n",
            "transfers:bills  $-2000.00  ; match:grp001\n"
        )
        .replace(
            "transfers:bills  $-247.50\n",
            "transfers:bills  $-247.50  ; match:grp001\n"
        )
    );
    let lj = parse_journal(&linked, "inline.journal").expect("parses");
    let index = links(&lj);
    assert!(index.pairs().is_empty() && index.broken().is_empty());
    assert_eq!(index.members("grp001"), Some(members.to_vec()));
    assert_eq!(index.groups().len(), 1);
    assert!(propose(&lj, &settings(), TODAY).is_empty());
    assert!(matching::in_transit(&lj, 10, 0, TODAY).is_empty());

    let unlinked = apply(&linked, &unlink_ops(&lj, "grp001").expect("known"));
    assert_eq!(unlinked, text);
}

#[test]
fn a_group_link_cleans_up_every_members_rejections() {
    // #2 was rejected with #4; #3 with #5. Linking the group removes both
    // values from both sides; #6's unrelated rejection stays.
    let text = format!(
        "{ACCOUNTS}\n{}{}{}{}",
        bill_pay()
            .replacen(
                "transfers:bills  $-2000.00\n",
                "transfers:bills  $-2000.00  ; nomatch:rej001\n",
                1
            )
            .replacen(
                "transfers:bills  $-247.50\n",
                "transfers:bills  $-247.50  ; nomatch:rej002\n",
                1
            ),
        half(
            "2026-12-02",
            "x",
            "assets:savings",
            "transfers:bills",
            "$2000.00"
        )
        .replace("$2000.00\n", "$2000.00  ; nomatch:rej001\n"),
        half(
            "2026-12-02",
            "y",
            "assets:savings",
            "transfers:bills",
            "$247.50"
        )
        .replace("$247.50\n", "$247.50  ; nomatch:rej002\n"),
        half("2026-12-02", "z", "assets:savings", "transfers:bills", "$1")
            .replace("$1\n", "$1  ; nomatch:rej003\n"),
    );
    let j = parse_journal(&text, "inline.journal").expect("parses");
    let ops =
        matching::link_group_ops(&j, &[at(&j, 1), at(&j, 2), at(&j, 3)], "grp001").expect("legal");
    let linked = apply(&text, &ops);
    assert!(!linked.contains("rej001") && !linked.contains("rej002"));
    assert!(linked.contains("nomatch:rej003"));
    assert_eq!(linked.matches("match:grp001").count(), 3);
}

#[test]
fn group_links_refuse_illegal_members() {
    let text = bill_pay()
        + &half(
            "2026-12-01",
            "other",
            "expenses:x",
            "transfers:other",
            "$-247.50",
        )
        + &half(
            "2026-12-01",
            "euro",
            "expenses:x",
            "transfers:bills",
            "-247.50 EUR",
        )
        + &half(
            "2026-12-01",
            "short",
            "expenses:x",
            "transfers:bills",
            "$-247.49",
        );
    let j = journal(&text);
    let group =
        |last: u32| matching::link_group_ops(&j, &[at(&j, 1), at(&j, 2), at(&j, last)], "g");
    assert!(group(3).is_ok());
    for last in [4, 5, 6] {
        assert!(group(last).is_err(), "#{last}");
    }
    // A member named twice, one member, and too many.
    assert!(matching::link_group_ops(&j, &[at(&j, 1), at(&j, 2), at(&j, 2)], "g").is_err());
    assert!(matching::link_group_ops(&j, &[at(&j, 1)], "g").is_err());
    let seventeen = vec![at(&j, 1); 17];
    assert!(matching::link_group_ops(&j, &seventeen, "g").is_err());
}

#[test]
fn group_links_are_valid_or_broken_by_the_a3_rule() {
    let text = "\
2026-12-01 good debit
    transfers:b  $30  ; match:good
    assets:checking

2026-12-01 good bill a
    transfers:b  $-10  ; match:good
    expenses:a

2026-12-01 good bill b
    transfers:b  $-20  ; match:good
    expenses:b

2026-12-01 split debit
    transfers:b  $30  ; match:split
    assets:checking

2026-12-01 split bill a
    transfers:c  $-10  ; match:split
    expenses:a

2026-12-01 split bill b
    transfers:b  $-20  ; match:split
    expenses:b

2026-12-01 short debit
    transfers:b  $30  ; match:short
    assets:checking

2026-12-01 short bill a
    transfers:b  $-10  ; match:short
    expenses:a

2026-12-01 short bill b
    transfers:b  $-19  ; match:short
    expenses:b

2026-12-01 mixed debit
    transfers:b  $30  ; match:mixed
    assets:checking

2026-12-01 mixed bill a
    transfers:b  -10 EUR  ; match:mixed
    expenses:a

2026-12-01 mixed bill b
    transfers:b  $-20  ; match:mixed
    expenses:b

2026-12-01 pair across accounts a
    transfers:b  $5  ; match:across
    assets:checking

2026-12-01 pair across accounts b
    transfers:c  $-5  ; match:across
    assets:savings
";
    let j = journal(text);
    let index = links(&j);
    let broken: Vec<(&str, BrokenReason)> = index
        .broken()
        .iter()
        .map(|link| (link.id.as_str(), link.reason))
        .collect();
    assert_eq!(
        broken,
        vec![
            ("across", BrokenReason::DifferentAccounts),
            ("mixed", BrokenReason::DoNotCancel),
            ("short", BrokenReason::DoNotCancel),
            ("split", BrokenReason::DifferentAccounts),
        ]
    );
    assert_eq!(index.groups().keys().collect::<Vec<_>>(), vec!["good"]);
    assert_eq!(index.link_count(0), 1);
    let findings = local_findings(&j, &Classes::of(&j), TODAY);
    assert!(
        findings
            .iter()
            .filter(|f| f.rule == "broken-match")
            .all(|f| f.position.is_some_and(|position| position >= 3))
    );
}

// --- A2: one transaction's partners ---------------------------------------

#[test]
fn partners_for_lists_exact_partners_at_any_gap_combinations_and_nearby() {
    // #1 ($75 in) has exact partners at 2 days (#3) and 200 days (#2, far
    // outside the window and a 30-day lookback), and a rejected one (#4);
    // nearby within the window: #3 and #5 ($-20), not #2.
    let text = half("2026-12-01", "in", "assets:checking", "transfers:b", "$75")
        .replace("$75\n", "$75  ; nomatch:rej001\n")
        + &half("2026-05-15", "old", "assets:savings", "transfers:b", "$-75")
        + &half(
            "2026-12-03",
            "near",
            "assets:savings",
            "transfers:b",
            "$-75",
        )
        + &half(
            "2026-12-01",
            "rejected",
            "assets:savings",
            "transfers:b",
            "$-75",
        )
        .replace("$-75\n", "$-75  ; nomatch:rej001\n")
        + &half(
            "2026-12-06",
            "small",
            "assets:savings",
            "transfers:b",
            "$-20",
        )
        + &half(
            "2026-12-01",
            "same sign",
            "assets:savings",
            "transfers:b",
            "$75",
        );
    let j = journal(&text);
    let found = matching::partners_for(&j, at(&j, 1).position, 10, TODAY);
    assert_eq!(found.len(), 1);
    let one = &found[0];
    assert_eq!(one.at, at(&j, 1));
    let txns = |partners: &[matching::Partner]| -> Vec<(u32, u32)> {
        partners.iter().map(|p| (p.at.txn.0, p.gap_days)).collect()
    };
    assert_eq!(txns(&one.exact), vec![(3, 2), (2, 200)]);
    assert_eq!(txns(&one.nearby), vec![(4, 0), (3, 2), (5, 5)]);
    assert!(one.combinations.is_empty());

    let bills = journal(&bill_pay());
    for txn in [1, 2] {
        let found = matching::partners_for(&bills, at(&bills, txn).position, 10, TODAY);
        assert_eq!(
            combos(&found[0].combinations),
            vec![(1, vec![2, 3])],
            "#{txn}"
        );
    }
    // Not a clearing posting, or linked: nothing to offer.
    let linked = journal(
        &half("2026-12-01", "x", "assets:checking", "transfers:b", "$5")
            .replace("$5\n", "$5  ; match:abc\n"),
    );
    assert!(matching::partners_for(&linked, 0, 10, TODAY).is_empty());
    assert!(matching::partners_for(&linked, 99, 10, TODAY).is_empty());
}

mod account_labels {
    use ledgeline_core::matching::AccountLabels;
    use ledgeline_core::parse::parse_journal;

    fn labels(text: &str) -> AccountLabels {
        AccountLabels::of(&parse_journal(text, "test.journal").expect("journal parses"))
    }

    #[test]
    fn a_unique_last_segment_is_the_label() {
        let labels = labels(
            "account assets:bank:checking:cash\naccount liabilities:venturex\naccount expenses:food\n",
        );
        assert_eq!(labels.label("assets:bank:checking:cash"), "cash");
        assert_eq!(labels.label("liabilities:venturex"), "venturex");
    }

    #[test]
    fn a_shared_last_segment_grows_until_unique() {
        let labels = labels(
            "account assets:bank1:checking\naccount assets:bank2:checking\naccount assets:savings\n",
        );
        assert_eq!(labels.label("assets:bank1:checking"), "bank1:checking");
        assert_eq!(labels.label("assets:bank2:checking"), "bank2:checking");
        assert_eq!(labels.label("assets:savings"), "savings");
    }

    #[test]
    fn posted_accounts_count_and_a_suffix_of_another_falls_back_to_the_full_name() {
        let labels =
            labels("account checking\n\n2026-01-01 x\n    assets:checking  $1\n    checking\n");
        assert_eq!(labels.label("checking"), "checking");
        assert_eq!(labels.label("assets:checking"), "assets:checking");
    }

    #[test]
    fn the_same_account_on_both_sides_reads_in_full() {
        let labels = labels("account assets:bank:cash\n");
        assert_eq!(
            labels.pair(Some("assets:bank:cash"), Some("assets:bank:cash")),
            [
                Some("assets:bank:cash".to_string()),
                Some("assets:bank:cash".to_string())
            ]
        );
    }
}
