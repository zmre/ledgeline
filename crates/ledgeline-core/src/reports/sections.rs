//! Internal helper shared by the sectioned reports — port of
//! `web/src/lib/reports/sections.ts`. Not part of the public contract.

use super::account_types::{AccountType, is_account_type};
use super::aggregate::roll_up;
use super::mixed_amount::MixedAmount;
use super::types::{ReportRow, Section};
use crate::decimal::DecError;
use std::collections::BTreeMap;

/// Build one report section from aggregated totals.
///
/// - `direct`     — full-account-name direct totals (`account_totals` output).
/// - `depth_mask` — the journal-wide rolled totals already clamped to the report
///   depth; used ONLY as a key set to clamp this section's rows (see below).
/// - `declared`   — declared account types; membership is decided by EFFECTIVE
///   type (declaration → nearest declared ancestor → name → agreeing declared
///   descendants), so a chart of accounts rooted at `cogs:` or in another
///   language still lands in the right section. `Cash` counts as `Asset` (see
///   [`is_account_type`]).
/// - `flip`       — present sign-flipped (liabilities on bs, revenues on is:
///   internally negative, displayed positive, hledger-style).
///
/// # Ordering: filter, THEN roll up
///
/// Membership is decided on the DIRECT totals, before any roll-up (RPT-2, and
/// what `cash_flow` already did). Rolling up first lets a parent net in children
/// of a different effective type: with `assets ; type: A` holding
/// `assets:bank $1000` and `assets:receivable ; type: L` at `-$300`, the old
/// order put a fabricated `assets = $700` row in the Assets section — a number
/// hledger never prints. Rolling up WITHIN the section instead gives `assets`
/// its own section's subtotal in each section ($1000 under Assets, $300 under
/// Liabilities), which is exactly what `hledger bs --depth 1` shows.
///
/// The section's rolled accounts are always a subset of the journal-wide rolled
/// accounts (both are the ancestor closure of a set of direct accounts, and the
/// section's members are a subset of all direct accounts), so intersecting with
/// `depth_mask` clamps them to the report depth without needing the depth
/// itself.
///
/// # The total
///
/// Summed over the section's MEMBERS — equivalently, over the maximal
/// type-matching rows, since every member rolls into exactly one of them. It
/// used to be summed over `depth == 1` rows, which read ZERO for any chart of
/// accounts whose typed accounts sit below depth 1 (RPT-1). Summing the members
/// also makes the total depth-independent, matching hledger, so `?depth=0`
/// reports totals rather than zeros.
///
/// # Errors
/// Returns [`DecError`] on decimal overflow.
pub fn build_section(
    title: &str,
    category: AccountType,
    direct: &BTreeMap<String, MixedAmount>,
    depth_mask: &BTreeMap<String, MixedAmount>,
    declared: &BTreeMap<String, AccountType>,
    flip: bool,
) -> Result<Section, DecError> {
    let members: BTreeMap<String, MixedAmount> = direct
        .iter()
        .filter(|(account, _)| is_account_type(account, declared, category))
        .map(|(account, ma)| (account.clone(), ma.clone()))
        .collect();

    let total = members
        .values()
        .try_fold(MixedAmount::new(), |acc, ma| acc.ma_add(ma))?;

    // `roll_up` yields a BTreeMap → rows come out sorted by account name.
    let rows: Vec<ReportRow> = roll_up(&members)?
        .into_iter()
        .filter(|(account, _)| depth_mask.contains_key(account))
        .map(|(account, inclusive)| {
            // `own` reads from `members`, not `direct`: a parent that belongs to
            // another section (`assets ; type: A` under Liabilities) contributes
            // none of its own postings here.
            let own = members.get(&account).cloned().unwrap_or_default();
            let depth = account.split(':').count();
            Ok(ReportRow {
                account,
                depth,
                own: if flip { own.ma_neg()? } else { own },
                inclusive: if flip { inclusive.ma_neg()? } else { inclusive },
            })
        })
        .collect::<Result<_, DecError>>()?;

    let total = if flip { total.ma_neg()? } else { total };
    Ok(Section {
        title: title.to_string(),
        rows,
        total,
    })
}
