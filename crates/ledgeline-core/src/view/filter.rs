//! The journal filter and the display order (plan 26 B5; a port of the SPA's
//! `journal/rowModel.ts` `filterTxns` + `sortTxnsDesc`).
//!
//! # Semantics (unchanged from the TypeScript)
//!
//! - The date range is inclusive on both ends, against the transaction's
//!   primary date (a posting date does not move it in or out).
//! - A transaction matches the account selection when ANY posting is in ANY
//!   selected subtree (`a` selects `a` and `a:…`); an empty selection is all.
//! - The free-text query is trimmed and lowercased, then matched as a
//!   substring of the transaction's [`haystack`](super::rows::haystack).
//! - Display order is date descending, then `tindex` descending — newest
//!   first, and within a day the last-written first. Ascending is its exact
//!   reverse.
//!
//! Results are POSITIONS into `journal.transactions` (`u32`: a journal holds
//! far fewer than four billion transactions), so a cached result is 4 bytes a
//! row and borrows nothing.

use super::wire::js_trim;
use crate::model::Journal;
use crate::reports::account_matches;

/// What the journal view is filtered to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct JournalFilter {
    /// Inclusive lower bound on the primary date.
    pub from: Option<String>,
    /// Inclusive upper bound on the primary date.
    pub to: Option<String>,
    /// Selected account subtree roots; empty selects every account.
    pub accounts: Vec<String>,
    /// The free-text query, already trimmed and lowercased (see [`Self::new`]).
    query: String,
}

impl JournalFilter {
    /// A filter over `[from, to]`, the subtrees `accounts` (blank entries
    /// dropped, order and duplicates irrelevant) and the raw `query`.
    #[must_use]
    pub fn new(
        from: Option<String>,
        to: Option<String>,
        accounts: impl IntoIterator<Item = String>,
        query: &str,
    ) -> Self {
        let mut accounts: Vec<String> = accounts
            .into_iter()
            .filter(|account| !account.is_empty())
            .collect();
        accounts.sort();
        accounts.dedup();
        Self {
            from: from.filter(|date| !date.is_empty()),
            to: to.filter(|date| !date.is_empty()),
            accounts,
            query: js_trim(query).to_lowercase(),
        }
    }

    /// The normalized query; empty when the filter has none.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether a posting on `account` counts under the account selection —
    /// the SPA's `postingIncluded` with no category.
    #[must_use]
    pub fn includes_account(&self, account: &str) -> bool {
        self.accounts.is_empty()
            || self
                .accounts
                .iter()
                .any(|selected| account_matches(selected, account))
    }
}

/// Every transaction position, in display order: date descending, then
/// `tindex` descending.
#[must_use]
pub fn display_order(journal: &Journal) -> Vec<u32> {
    let txns = &journal.transactions;
    let mut order: Vec<u32> = (0..txns.len())
        .map(|at| u32::try_from(at).unwrap_or(u32::MAX))
        .collect();
    order.sort_by(|a, b| {
        let (a, b) = (&txns[*a as usize], &txns[*b as usize]);
        b.date.cmp(&a.date).then_with(|| b.index.0.cmp(&a.index.0))
    });
    order
}

/// The positions in `order` that `filter` keeps, still in `order`'s order.
///
/// `haystacks` (one per transaction, indexed by position) is consulted only
/// when the filter has a query, so a caller may build it lazily and pass
/// `None` for query-free filters.
#[must_use]
pub fn filter_positions(
    journal: &Journal,
    order: &[u32],
    haystacks: Option<&[Box<str>]>,
    filter: &JournalFilter,
) -> Vec<u32> {
    let query = filter.query();
    order
        .iter()
        .copied()
        .filter(|position| {
            let txn = &journal.transactions[*position as usize];
            filter
                .from
                .as_deref()
                .is_none_or(|from| txn.date.as_str() >= from)
                && filter
                    .to
                    .as_deref()
                    .is_none_or(|to| txn.date.as_str() <= to)
                && (filter.accounts.is_empty()
                    || txn
                        .postings
                        .iter()
                        .any(|posting| filter.includes_account(&posting.account.0)))
                && (query.is_empty()
                    || haystacks
                        .and_then(|all| all.get(*position as usize))
                        .is_some_and(|text| text.contains(query)))
        })
        .collect()
}
