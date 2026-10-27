//! What a journal's `account` directives say about how the balance sheet and
//! the income statement are laid out.

use super::account_groups::{BsTerm, IS_GROUP_TAG, account_groups, bs_terms, declared_groups};
use super::account_types::{AccountType, account_decls, declared_types};
use super::income_statement::{IsSectionKind, account_sections};
use crate::model::Journal;
use std::collections::BTreeMap;

/// Declared types, `bsgroup:`/`bsterm:` and `issection:`/`isgroup:`. (The cash
/// flow statement reads its own, [`super::CfTags::from_journal`].)
///
/// ONE constructor, read by the Reports handlers and by the projected
/// statements (plan 28 Phase 4, plan 29), so a projected balance sheet cannot
/// be grouped, banded or typed differently from the one on the Reports tab.
/// Every field is a walk over the `account` directives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatementShape {
    /// Declared `type:` per account.
    pub declared: BTreeMap<String, AccountType>,
    /// Declared `bsgroup:` per account.
    pub bs_groups: BTreeMap<String, String>,
    /// Declared `bsterm:` per account.
    pub bs_terms: BTreeMap<String, BsTerm>,
    /// Declared `issection:` per account.
    pub is_sections: BTreeMap<String, IsSectionKind>,
    /// Declared `isgroup:` per account.
    pub is_groups: BTreeMap<String, String>,
}

impl StatementShape {
    /// Read every declaration the two statements use off `journal`.
    #[must_use]
    pub fn of(journal: &Journal) -> Self {
        Self {
            declared: declared_types(&account_decls(journal)),
            bs_groups: account_groups(journal),
            bs_terms: bs_terms(journal),
            is_sections: account_sections(journal),
            is_groups: declared_groups(journal, IS_GROUP_TAG),
        }
    }
}
