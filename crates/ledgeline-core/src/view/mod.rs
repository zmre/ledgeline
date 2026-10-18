//! Journal views for thin clients (plan 26 B3–B5): the computations the SPA
//! used to run over the whole downloaded journal, moved into the engine so any
//! UI — the web SPA, a native desktop shell, an iOS app — asks for exactly the
//! slice it draws.
//!
//! - [`cash`] — cash and short-term-liability balances (the journal tab's
//!   Balances view), formerly `web/src/lib/reports/cashBalances.ts`.
//! - [`checks`] — the five local check rules (pending, uncategorized,
//!   negative cash, missing description, future date), formerly
//!   `web/src/lib/checks/rules.ts`, folded in with the engine diagnostics.
//! - [`rows`] — one transaction in the SPA's domain shape, plus the search text.
//! - [`filter`] — the journal filter (date range, account subtrees, free text)
//!   and the display order, formerly `journal/rowModel.ts`.
//! - [`series`] — the insights summary and charts, formerly
//!   `insights/series.ts`.
//!
//! Every module is a PORT, held to goldens captured from the TypeScript before
//! it was deleted (`fixtures/journal-view/*.json`). Behaviour changes belong in
//! a separate, deliberate commit.

pub mod cash;
pub mod wire;

use crate::model::Journal;
use crate::reports::account_types::is_category;
use crate::reports::{
    AccountClass, AccountType, BsTerm, account_classes, account_decls_from, declared_types,
    resolve_account_type,
};
use std::collections::BTreeMap;

/// Every account's engine classification (type and balance-sheet term), the
/// table `/api/journal` serves as `accounts`, with the SPA's
/// `domain/accountClass.ts` lookups over it.
///
/// `Sync`, unlike [`crate::reports::AccountTypes`] (whose memo is a
/// `RefCell`), so one instance can be shared by every request against a
/// snapshot.
#[derive(Debug)]
pub struct Classes {
    known: BTreeMap<String, AccountClass>,
    declared: BTreeMap<String, AccountType>,
}

impl Classes {
    /// Classify every account `journal` knows.
    #[must_use]
    pub fn of(journal: &Journal) -> Self {
        Self {
            known: account_classes(journal),
            declared: declared_types(&account_decls_from(&journal.accounts)),
        }
    }

    /// The effective type of `account` (`accountTypeOf`). Every posted account
    /// is in the table; a name the engine never saw resolves the way the engine
    /// would resolve it.
    #[must_use]
    pub fn account_type(&self, account: &str) -> Option<AccountType> {
        match self.known.get(account) {
            Some(class) => class.account_type,
            None => resolve_account_type(account, &self.declared),
        }
    }

    /// Whether `account` belongs to `category`, subtypes included
    /// (`isAccountType`).
    #[must_use]
    pub fn is_type(&self, account: &str, category: AccountType) -> bool {
        is_category(self.account_type(account), category)
    }

    /// The engine's balance-sheet term and whether it was a name guess
    /// (`accountTermOf`); `(None, false)` for a name it never classified.
    #[must_use]
    pub fn term(&self, account: &str) -> (Option<BsTerm>, bool) {
        self.known
            .get(account)
            .map_or((None, false), |class| (class.term, class.term_guessed))
    }
}
