//! The `kpi:` account tag and the dimension tags — the vocabulary the startup
//! metrics on Insights read (`plans/37-startup-metrics.md`).
//!
//! Modelled on [`super::cash_flow_statement`]'s `cfsection:` set: one key, a
//! closed set of values, one value per account, inherited by subaccounts
//! (nearest declared wins, like `type:`). `kpi:` only adds what no existing tag
//! says — gross margin comes from `issection: cogs`, cash from `type: C` — and it
//! changes no statement. Its readers are Insights' startup section, the Problems
//! panel, Top Transactions' skip rule and the QuickBooks importer.

use super::account_groups::nearest_declared;
use super::account_types::{AccountType, AccountTypes, root_type};
use crate::model::AccountDeclaration;
use std::collections::BTreeMap;
use std::ops::Bound;

/// The `account` tag that names an account's role in the startup metrics.
pub const KPI_TAG: &str = "kpi";

/// The dimension keys a recurring-revenue posting is broken down by when the
/// caller names none.
pub const DIMENSION_KEYS_DEFAULT: [&str; 4] = ["segment", "product", "class", "region"];

/// The closed vocabulary of [`KPI_TAG`].
///
/// Closed and coded for the reason `IsSectionKind` gives: a classification that
/// decides membership must never match English words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KpiRole {
    /// Subscription revenue as recognised. MRR is its monthly total.
    Recurring,
    /// Metered or overage revenue. Shown beside MRR, never in it.
    Usage,
    /// Contract liability.
    DeferredRevenue,
    /// Sales and marketing spend.
    SalesMarketing,
    /// Research and development.
    Rnd,
    /// General and administrative.
    Ga,
    /// Trade receivables.
    Receivable,
    /// Trade payables.
    Payable,
}

impl KpiRole {
    /// Every role, in vocabulary order.
    pub const ALL: [KpiRole; 8] = [
        KpiRole::Recurring,
        KpiRole::Usage,
        KpiRole::DeferredRevenue,
        KpiRole::SalesMarketing,
        KpiRole::Rnd,
        KpiRole::Ga,
        KpiRole::Receivable,
        KpiRole::Payable,
    ];

    /// The tag value, as written in a journal and on the wire.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Recurring => "recurring",
            Self::Usage => "usage",
            Self::DeferredRevenue => "deferred-revenue",
            Self::SalesMarketing => "sales-marketing",
            Self::Rnd => "rnd",
            Self::Ga => "ga",
            Self::Receivable => "receivable",
            Self::Payable => "payable",
        }
    }
}

/// Parse a `kpi:` value, trimmed and ASCII case-insensitive; `None` for anything
/// outside the vocabulary.
#[must_use]
pub fn parse_kpi_tag(value: &str) -> Option<KpiRole> {
    let value = value.trim();
    KpiRole::ALL
        .into_iter()
        .find(|role| role.code().eq_ignore_ascii_case(value))
}

/// The root type an account carrying `role` is expected to resolve to — what
/// the `kpi-type-mismatch` Problems check compares against.
#[must_use]
pub fn expected_type(role: KpiRole) -> AccountType {
    match role {
        KpiRole::Recurring | KpiRole::Usage => AccountType::Revenue,
        KpiRole::DeferredRevenue | KpiRole::Payable => AccountType::Liability,
        KpiRole::SalesMarketing | KpiRole::Rnd | KpiRole::Ga => AccountType::Expense,
        KpiRole::Receivable => AccountType::Asset,
    }
}

/// Declared `kpi:` roles per account. Empty or unreadable values read as no
/// declaration, exactly as `cfsection:` does; the typo is reported by
/// [`journal_to_tag_diagnostics`](crate::wire::journal_to_tag_diagnostics).
#[must_use]
pub fn declared_kpi_roles(accounts: &[AccountDeclaration]) -> BTreeMap<String, KpiRole> {
    accounts
        .iter()
        .filter_map(|decl| {
            decl.tags
                .iter()
                .find(|(key, _)| key == KPI_TAG)
                .and_then(|(_, value)| parse_kpi_tag(value))
                .map(|role| (decl.name.0.clone(), role))
        })
        .collect()
}

/// True when any account declaration carries a `kpi:` tag, readable or not —
/// the "this journal is a business" signal.
#[must_use]
pub fn any_kpi_declared(accounts: &[AccountDeclaration]) -> bool {
    accounts
        .iter()
        .any(|decl| decl.tags.iter().any(|(key, _)| key == KPI_TAG))
}

/// True when a strict descendant of `account` declares a role.
fn has_declared_descendant(declared: &BTreeMap<String, KpiRole>, account: &str) -> bool {
    declared
        .range::<str, _>((Bound::Excluded(account), Bound::Unbounded))
        .take_while(|(name, _)| name.starts_with(account))
        .any(|(name, _)| name.as_bytes().get(account.len()) == Some(&b':'))
}

/// The last `:` segment of `account` contains `needle` (ASCII
/// case-insensitive; `needle` is lowercase).
fn last_segment_contains(account: &str, needle: &str) -> bool {
    let segment = account.rsplit(':').next().unwrap_or(account);
    segment.to_ascii_lowercase().contains(needle)
}

/// The documented receivable/payable name fallback: with no `kpi:` anywhere on
/// the account, its ancestors or its descendants, an Asset-typed account whose
/// last segment contains `receivable` is a receivable and a Liability-typed one
/// whose last segment contains `payable` is a payable. Nothing else is ever
/// inferred from a name.
fn inferred_role(
    declared: &BTreeMap<String, KpiRole>,
    types: &AccountTypes,
    account: &str,
) -> Option<KpiRole> {
    if has_declared_descendant(declared, account) {
        return None;
    }
    let root = types.resolve(account).map(root_type)?;
    match root {
        AccountType::Asset if last_segment_contains(account, "receivable") => {
            Some(KpiRole::Receivable)
        }
        AccountType::Liability if last_segment_contains(account, "payable") => {
            Some(KpiRole::Payable)
        }
        _ => None,
    }
}

/// The role resolver: the nearest declared `kpi:` on the account or an
/// ancestor, else the receivable/payable name inference.
pub fn kpi_resolver<'a>(
    declared: &'a BTreeMap<String, KpiRole>,
    types: &'a AccountTypes,
) -> impl Fn(&str) -> Option<KpiRole> + 'a {
    move |account: &str| {
        nearest_declared(declared, account)
            .copied()
            .or_else(|| inferred_role(declared, types, account))
    }
}

/// Whether `account` (or an ancestor) DECLARES its role, as opposed to having
/// none or an inferred one. The Problems check uses this so an inferred
/// receivable never fires a type mismatch.
#[must_use]
pub fn role_is_declared(declared: &BTreeMap<String, KpiRole>, account: &str) -> bool {
    nearest_declared(declared, account).is_some()
}

/// Declared dimension tags per account: `account → key → value`, for the keys
/// in `keys` only. Empty values read as no declaration.
#[must_use]
pub fn declared_dimension_tags(
    accounts: &[AccountDeclaration],
    keys: &[String],
) -> BTreeMap<String, BTreeMap<String, String>> {
    accounts
        .iter()
        .filter_map(|decl| {
            let dims: BTreeMap<String, String> = decl
                .tags
                .iter()
                .filter(|(key, _)| keys.iter().any(|wanted| wanted == key))
                .map(|(key, value)| (key.clone(), value.trim().to_string()))
                .filter(|(_, value)| !value.is_empty())
                .collect();
            (!dims.is_empty()).then(|| (decl.name.0.clone(), dims))
        })
        .collect()
}

/// The value of dimension `key` declared on `account` or its NEAREST declaring
/// ancestor (nearest wins per key, like `type:`). An ancestor that declares
/// other keys but not `key` is skipped over.
#[must_use]
pub fn nearest_dimension<'a>(
    declared: &'a BTreeMap<String, BTreeMap<String, String>>,
    account: &str,
    key: &str,
) -> Option<&'a str> {
    let mut name = account;
    loop {
        if let Some(value) = declared.get(name).and_then(|dims| dims.get(key)) {
            return Some(value.as_str());
        }
        name = &name[..name.rfind(':')?];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountName, SourcePos};
    use crate::reports::account_types::{account_decls_from, declared_types};
    use std::path::PathBuf;

    fn decl(name: &str, tags: &[(&str, &str)]) -> AccountDeclaration {
        AccountDeclaration {
            name: AccountName(name.to_string()),
            tags: tags
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            comment: String::new(),
            position: SourcePos { line: 1, column: 1 },
            source_file: PathBuf::new(),
        }
    }

    fn resolve_all(accounts: &[AccountDeclaration], queries: &[&str]) -> Vec<Option<KpiRole>> {
        let declared = declared_kpi_roles(accounts);
        let types = AccountTypes::from_declared(declared_types(&account_decls_from(accounts)));
        let resolve = kpi_resolver(&declared, &types);
        queries.iter().map(|account| resolve(account)).collect()
    }

    #[test]
    fn every_value_parses_with_case_and_whitespace() {
        for role in KpiRole::ALL {
            assert_eq!(parse_kpi_tag(role.code()), Some(role));
            assert_eq!(
                parse_kpi_tag(&format!("  {}\t", role.code().to_uppercase())),
                Some(role)
            );
        }
        assert_eq!(
            parse_kpi_tag("Deferred-Revenue"),
            Some(KpiRole::DeferredRevenue)
        );
    }

    #[test]
    fn unknown_values_are_rejected() {
        for bad in ["", "cogs", "recurrent", "deferred revenue", "r&d", "arr"] {
            assert_eq!(parse_kpi_tag(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn declarations_skip_empty_and_unreadable_values() {
        let accounts = [
            decl("revenue:subs", &[("kpi", "recurring")]),
            decl("revenue:other", &[("kpi", "")]),
            decl("revenue:typo", &[("kpi", "recuring")]),
        ];
        let declared = declared_kpi_roles(&accounts);
        assert_eq!(declared.len(), 1);
        assert_eq!(declared["revenue:subs"], KpiRole::Recurring);
        assert!(any_kpi_declared(&accounts));
        assert!(!any_kpi_declared(&[decl("assets:bank", &[("type", "C")])]));
    }

    #[test]
    fn nearest_declared_wins_and_a_child_overrides_its_parent() {
        let accounts = [
            decl("revenue", &[("type", "R"), ("kpi", "recurring")]),
            decl("revenue:usage", &[("kpi", "usage")]),
        ];
        assert_eq!(
            resolve_all(
                &accounts,
                &[
                    "revenue",
                    "revenue:subs:smb",
                    "revenue:usage",
                    "revenue:usage:api"
                ]
            ),
            vec![
                Some(KpiRole::Recurring),
                Some(KpiRole::Recurring),
                Some(KpiRole::Usage),
                Some(KpiRole::Usage),
            ]
        );
    }

    #[test]
    fn name_inference_fires_only_without_a_declaration_in_the_subtree() {
        // No kpi anywhere: inferred by name, asset and liability alike.
        let plain = [decl("assets:accounts receivable", &[("type", "A")])];
        assert_eq!(
            resolve_all(
                &plain,
                &["assets:accounts receivable", "liabilities:accounts payable"]
            ),
            vec![Some(KpiRole::Receivable), Some(KpiRole::Payable)]
        );
        // A declared descendant switches the parent's inference off.
        let tagged_child = [decl("assets:receivable:trade", &[("kpi", "receivable")])];
        assert_eq!(
            resolve_all(
                &tagged_child,
                &["assets:receivable", "assets:receivable:trade"]
            ),
            vec![None, Some(KpiRole::Receivable)]
        );
        // A declared ancestor role wins over the name.
        let tagged_parent = [decl("assets:receivable", &[("kpi", "ga")])];
        assert_eq!(
            resolve_all(&tagged_parent, &["assets:receivable:x"]),
            vec![Some(KpiRole::Ga)]
        );
    }

    #[test]
    fn name_inference_never_fires_off_type() {
        // A "receivable" that is not an asset, and a "payable" that is not a
        // liability, are never inferred.
        let accounts = [
            decl("expenses:receivable fees", &[("type", "X")]),
            decl("assets:payable float", &[("type", "A")]),
        ];
        assert_eq!(
            resolve_all(
                &accounts,
                &[
                    "expenses:receivable fees",
                    "assets:payable float",
                    "revenue:receivable"
                ]
            ),
            vec![None, None, None]
        );
        // Only the LAST segment counts.
        assert_eq!(resolve_all(&[], &["assets:receivable:bank"]), vec![None]);
    }

    #[test]
    fn expected_types_are_roots() {
        assert_eq!(expected_type(KpiRole::Recurring), AccountType::Revenue);
        assert_eq!(expected_type(KpiRole::Usage), AccountType::Revenue);
        assert_eq!(
            expected_type(KpiRole::DeferredRevenue),
            AccountType::Liability
        );
        assert_eq!(expected_type(KpiRole::Payable), AccountType::Liability);
        assert_eq!(expected_type(KpiRole::Receivable), AccountType::Asset);
        for role in [KpiRole::SalesMarketing, KpiRole::Rnd, KpiRole::Ga] {
            assert_eq!(expected_type(role), AccountType::Expense);
        }
    }

    #[test]
    fn dimensions_resolve_nearest_per_key() {
        let keys: Vec<String> = DIMENSION_KEYS_DEFAULT
            .iter()
            .map(|k| (*k).to_string())
            .collect();
        let accounts = [
            decl("revenue:subs", &[("segment", "smb"), ("product", "core")]),
            decl("revenue:subs:ent", &[("segment", "enterprise")]),
            decl("revenue:subs:blank", &[("segment", " ")]),
            decl("revenue:other", &[("color", "blue")]),
        ];
        let declared = declared_dimension_tags(&accounts, &keys);
        assert!(!declared.contains_key("revenue:other"));
        assert!(!declared.contains_key("revenue:subs:blank"));
        assert_eq!(
            nearest_dimension(&declared, "revenue:subs:ent:x", "segment"),
            Some("enterprise")
        );
        // The child declares only `segment`; `product` comes from the parent.
        assert_eq!(
            nearest_dimension(&declared, "revenue:subs:ent", "product"),
            Some("core")
        );
        assert_eq!(
            nearest_dimension(&declared, "revenue:subs:blank", "segment"),
            Some("smb")
        );
        assert_eq!(
            nearest_dimension(&declared, "revenue:other", "segment"),
            None
        );
    }
}
