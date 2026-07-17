//! Account-name utilities — port of the report-relevant parts of
//! `web/src/lib/domain/accounts.ts`.

/// The hledger-convention root category of an account, by its first segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootCategory {
    /// `assets*`
    Asset,
    /// `liabilit*`
    Liability,
    /// `equity*`
    Equity,
    /// `revenue*` / `income*`
    Revenue,
    /// `expense*`
    Expense,
    /// Anything else.
    Other,
}

/// Categorize by hledger-convention root account name
/// (`assets*`, `liabilities*`, `equity*`, `revenues|income*`, `expenses*`).
#[must_use]
pub fn categorize(account: &str) -> RootCategory {
    let root = account.split(':').next().unwrap_or("").to_lowercase();
    if root.starts_with("asset") {
        RootCategory::Asset
    } else if root.starts_with("liabilit") {
        RootCategory::Liability
    } else if root.starts_with("equity") {
        RootCategory::Equity
    } else if root.starts_with("revenue") || root.starts_with("income") {
        RootCategory::Revenue
    } else if root.starts_with("expense") {
        RootCategory::Expense
    } else {
        RootCategory::Other
    }
}

/// Clamp an account name to `depth` segments: `("a:b:c", 2) → "a:b"`.
#[must_use]
pub fn clamp_account(name: &str, depth: usize) -> String {
    name.split(':').take(depth).collect::<Vec<_>>().join(":")
}

/// True when `account` is `selected` itself or any of its sub-accounts.
#[must_use]
pub fn account_matches(selected: &str, account: &str) -> bool {
    account == selected || account.starts_with(&format!("{selected}:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_by_root_segment_case_insensitively() {
        assert_eq!(categorize("assets:bank:checking"), RootCategory::Asset);
        assert_eq!(categorize("liabilities:cc:visa"), RootCategory::Liability);
        assert_eq!(categorize("equity:opening"), RootCategory::Equity);
        assert_eq!(categorize("income:salary"), RootCategory::Revenue);
        assert_eq!(categorize("revenues:consulting"), RootCategory::Revenue);
        assert_eq!(categorize("expenses:food"), RootCategory::Expense);
        assert_eq!(categorize("Assets:Bank"), RootCategory::Asset);
        assert_eq!(categorize("misc"), RootCategory::Other);
        assert_eq!(categorize(""), RootCategory::Other);
    }

    #[test]
    fn clamps_to_depth() {
        assert_eq!(clamp_account("a:b:c", 2), "a:b");
        assert_eq!(clamp_account("a:b:c", 1), "a");
        assert_eq!(clamp_account("a:b:c", 5), "a:b:c");
        assert_eq!(clamp_account("a", 2), "a");
    }

    #[test]
    fn matches_self_and_subtree_only() {
        assert!(account_matches("assets", "assets"));
        assert!(account_matches("assets", "assets:bank"));
        assert!(!account_matches("assets", "assetsx"));
        assert!(!account_matches("assets:bank", "assets"));
    }
}
