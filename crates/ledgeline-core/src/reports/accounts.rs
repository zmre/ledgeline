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
    let root = account.split(':').next().unwrap_or("");
    let lowered = ascii_or_lowercased(root);
    let has_root = |prefix: &str| {
        lowered
            .as_bytes()
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
    };
    if has_root("asset") {
        RootCategory::Asset
    } else if has_root("liabilit") {
        RootCategory::Liability
    } else if has_root("equity") {
        RootCategory::Equity
    } else if has_root("revenue") || has_root("income") {
        RootCategory::Revenue
    } else if has_root("expense") {
        RootCategory::Expense
    } else {
        RootCategory::Other
    }
}

/// `s` itself when it is ASCII, else its Unicode lowercase.
///
/// Every comparison in this module is against an ASCII literal, and ASCII
/// lowering is byte-wise — so for an ASCII input (every realistic account name)
/// `eq_ignore_ascii_case` against the borrowed original gives exactly the answer
/// `s.to_lowercase()` would, without the per-call `String`. Non-ASCII keeps the
/// real Unicode lowering, so a name relying on e.g. `K` (U+212A KELVIN SIGN)
/// folding to `k` still classifies as it always did. `categorize` and
/// `matches_cash_name` sit under `resolve_account_type`, which reports call once
/// per POSTING, so this is two `String`s per posting removed (PERF-5e).
pub(super) fn ascii_or_lowercased(s: &str) -> std::borrow::Cow<'_, str> {
    if s.is_ascii() {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(s.to_lowercase())
    }
}

/// Clamp an account name to `depth` segments: `("a:b:c", 2) → "a:b"`.
#[must_use]
pub fn clamp_account(name: &str, depth: usize) -> String {
    name.split(':').take(depth).collect::<Vec<_>>().join(":")
}

/// True when `account` is `selected` itself or any of its sub-accounts.
///
/// The subtree test is "prefixed by `selected`, and the next byte is a `:`" —
/// `None` meaning the names are the same length, i.e. equal. Spelling it that
/// way rather than as `starts_with(&format!("{selected}:"))` matters because
/// `account_totals` calls this once per selected account PER POSTING (PERF-5e).
#[must_use]
pub fn account_matches(selected: &str, account: &str) -> bool {
    account.starts_with(selected)
        && matches!(account.as_bytes().get(selected.len()), None | Some(b':'))
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
        // The byte-wise subtree test must not mistake a shorter name for a
        // parent, nor a `:`-less continuation for a child.
        assert!(!account_matches("assets", "asset"));
        assert!(!account_matches("assets", ""));
        assert!(account_matches("", ""));
        assert!(account_matches("assets", "assets:bank:checking"));
    }

    /// Root categorization stays Unicode-correct on the non-ASCII path, where
    /// `to_lowercase` can do something `eq_ignore_ascii_case` cannot: U+212A
    /// KELVIN SIGN folds to a plain `k`.
    #[test]
    fn categorizes_non_ascii_roots_by_unicode_lowering() {
        assert_eq!(categorize("ASSETSÜ:banco"), RootCategory::Asset);
        assert_eq!(categorize("ЕQUITY:opening"), RootCategory::Other);
        assert_eq!(categorize("IN\u{212A}OME"), RootCategory::Other);
        assert_eq!(categorize("INCOME\u{212A}:salary"), RootCategory::Revenue);
    }
}
