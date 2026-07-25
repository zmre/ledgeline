//! Internal helper shared by the sectioned reports — port of
//! `web/src/lib/reports/sections.ts`. Not part of the public contract.

use super::account_types::{AccountType, is_account_type};
use super::mixed_amount::MixedAmount;
use super::types::{ReportRow, Section};
use crate::decimal::DecError;
use std::collections::BTreeMap;

/// Build one report section from aggregated totals.
///
/// - `direct`   — full-account-name direct totals (`account_totals` output).
/// - `clamped`  — rolled-up totals already clamped to the report depth.
/// - `declared` — declared account types; membership is decided by EFFECTIVE
///   type (declaration → nearest declared ancestor → name), so a chart of
///   accounts rooted at `cogs:` or in another language still lands in the right
///   section. `Cash` counts as `Asset` (see [`is_account_type`]).
/// - `flip`     — present sign-flipped (liabilities on bs, revenues on is:
///   internally negative, displayed positive, hledger-style).
///
/// # Errors
/// Returns [`DecError`] on decimal overflow.
pub fn build_section(
    title: &str,
    category: AccountType,
    direct: &BTreeMap<String, MixedAmount>,
    clamped: &BTreeMap<String, MixedAmount>,
    declared: &BTreeMap<String, AccountType>,
    flip: bool,
) -> Result<Section, DecError> {
    let mut rows: Vec<ReportRow> = Vec::new();
    let mut total = MixedAmount::new();
    // `clamped` is a BTreeMap → keys already sorted lexically (matches TS .sort()).
    for (account, inclusive) in clamped {
        if !is_account_type(account, declared, category) {
            continue;
        }
        let depth = account.split(':').count();
        let own = direct.get(account).cloned().unwrap_or_default();
        rows.push(ReportRow {
            account: account.clone(),
            depth,
            own: if flip { own.ma_neg()? } else { own },
            inclusive: if flip {
                inclusive.ma_neg()?
            } else {
                inclusive.clone()
            },
        });
        if depth == 1 {
            total = total.ma_add(inclusive)?; // roots carry the whole subtree
        }
    }
    let total = if flip { total.ma_neg()? } else { total };
    Ok(Section {
        title: title.to_string(),
        rows,
        total,
    })
}
