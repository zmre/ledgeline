//! Balance-sheet groups — the presentation buckets the grouped balance sheet
//! collapses accounts into ("Cash and cash equivalents", "Investments", …).
//!
//! Resolution mirrors [`super::account_types::resolve_account_type`]'s shape —
//! own declaration → nearest declared ancestor → inference — with the inference
//! step split into the three signals below.
//!
//! # Membership is never decided by an English name
//!
//! Classification that reads account NAMES is how reports come to read zero: a
//! chart of accounts rooted at `cogs:`, or written in Spanish, matches no
//! English word list and silently falls out of every bucket (the RPT-1 /
//! `account-type-not-name` failure). So the only signals that decide which group
//! an account BELONGS to are:
//!
//! 1. an explicit `bsgroup:` tag (the user said so),
//! 2. the same tag on the nearest declared ancestor (it inherits, like `type:`),
//! 3. the account's effective TYPE being `Cash` (type-driven),
//! 4. the account holding a commodity other than the base one (commodity-driven),
//! 5. the account's SECOND PATH SEGMENT (tree-position-driven).
//!
//! Every one of those is language-neutral, and step 5 always succeeds, so no
//! account can fall out of the report. [`alias`] may prettify step 5's LABEL
//! (`cc` → "Credit cards"), and that is all it may ever do: a cosmetic table
//! that cannot reach membership cannot cause the reads-zero failure.

use super::ReportError;
use super::account_types::{AccountType, AccountTypes};
use super::accounts::ascii_or_lowercased;
use super::mixed_amount::MixedAmount;
use crate::model::{AccountDeclaration, Commodity, Journal};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Which of the five resolution steps named a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupSource {
    /// A `bsgroup:` tag on the account or a declared ancestor.
    Tag,
    /// The account's effective type (`Cash`).
    Type,
    /// The account holds a commodity other than the base one.
    Commodity,
    /// The account's second path segment.
    Segment,
    /// A synthetic line the report computes rather than a bucket of accounts.
    Computed,
}

/// Built-in group for accounts whose effective type is `Cash`.
pub const CASH_GROUP: &str = "Cash and cash equivalents";
/// Built-in group for asset accounts holding a non-base commodity.
pub const INVESTMENTS_GROUP: &str = "Investments";
/// Synthetic equity line: revenues − expenses through the as-of date.
pub const RETAINED_EARNINGS_GROUP: &str = "Retained earnings";
/// Synthetic equity line: the balance sheet on the DISPLAY basis minus the same
/// accounts at COST, per commodity.
///
/// In the ordinary case — a single currency with every holding priced — that
/// difference IS the unrealized holding gain, which is why the figure looks
/// familiar. The line is deliberately not named that, because the same
/// subtraction also absorbs two things no gain can be:
///
/// - commodities no price reaches, which stay as share counts (so the line can
///   read `5 GLD`), and
/// - revaluation of balances held in another currency (so it can read
///   `933.25 EUR`).
///
/// A balance sheet cannot say "unrealized gains of 5 GLD and 933,25 EUR", and a
/// wrong label on a financial statement is worse than a dull correct one.
/// "Valuation adjustment" is precisely what the number is on every basis.
pub const VALUATION_ADJUSTMENT_GROUP: &str = "Valuation adjustment";

/// Presentation order for the group names the engine itself can produce:
/// current assets before non-current, current liabilities before long-term.
/// Everything else sorts alphabetically after these, and the two synthetic
/// equity lines sort after that (see [`group_rank`]).
const BUILT_IN_ORDER: [&str; 5] = [
    CASH_GROUP,
    "Accounts receivable",
    INVESTMENTS_GROUP,
    "Credit cards",
    "Accounts payable",
];

/// Sort rank for a group. Pair it with the group NAME as a secondary key to get
/// "known groups in balance-sheet order, then everything else alphabetically,
/// then the synthetic equity lines".
#[must_use]
pub fn group_rank(name: &str, source: GroupSource) -> usize {
    if source == GroupSource::Computed {
        // Retained earnings, then the valuation adjustment — both after every
        // bucket of real accounts.
        return BUILT_IN_ORDER.len()
            + if name == RETAINED_EARNINGS_GROUP {
                1
            } else {
                2
            };
    }
    BUILT_IN_ORDER
        .iter()
        .position(|built_in| *built_in == name)
        .unwrap_or(BUILT_IN_ORDER.len())
}

/// The account tag naming a BALANCE-SHEET group.
pub const BS_GROUP_TAG: &str = "bsgroup";
/// The account tag splitting a balance-sheet box into current and non-current.
pub const BS_TERM_TAG: &str = "bsterm";
/// The account tag naming an INCOME-STATEMENT group.
pub const IS_GROUP_TAG: &str = "isgroup";

/// Which half of a balance-sheet box an account sits in.
///
/// The standard current/non-current split, and a THIRD tag rather than a value
/// of `bsgroup:` for the reason this codebase keeps separating: `bsgroup:` names
/// the LINE an account prints on, `bsterm:` names the SUBHEADING that line sits
/// under. `type:` picks the box, `bsterm:` picks the half, `bsgroup:` picks the
/// line. Three questions, three tags, none of them able to answer another's.
///
/// Equity is never split — the distinction is about when an asset converts to
/// cash or when a debt comes due, and neither question is asked of capital.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BsTerm {
    /// Realizable (or payable) within the operating cycle — normally a year.
    Current,
    /// Everything else: property, long-term investments, the mortgage.
    NonCurrent,
}

impl BsTerm {
    /// The subheading printed above this half of a box.
    #[must_use]
    pub fn heading(self) -> &'static str {
        match self {
            Self::Current => "Current",
            Self::NonCurrent => "Non-current",
        }
    }

    /// Presentation order: current first, as every statement prints it.
    #[must_use]
    pub fn rank(self) -> usize {
        match self {
            Self::Current => 0,
            Self::NonCurrent => 1,
        }
    }

    /// The wire spelling.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::NonCurrent => "noncurrent",
        }
    }
}

/// Parse a `bsterm:` tag value; `None` for anything outside the vocabulary.
///
/// The canonical spellings — the wire codes, and the only two
/// [`ReportError::UnknownBsTerm`] names — are `current` and `noncurrent`.
/// Accepted on top of them, trimmed and case-insensitively: `short`,
/// `shortterm` and `short-term` for current; `non-current`, `long`, `longterm`
/// and `long-term` for non-current. The synonyms are documented in
/// `docs/balance-sheet.md` and journals rely on them, so "aligning" this match
/// with the error message's two-word vocabulary is not a cleanup — dropping a
/// spelling turns every account tagged with it into an [`ReportError::UnknownBsTerm`]
/// refusal at report time.
#[must_use]
pub fn parse_bs_term_tag(value: &str) -> Option<BsTerm> {
    match value.trim().to_lowercase().as_str() {
        "current" | "short" | "shortterm" | "short-term" => Some(BsTerm::Current),
        "noncurrent" | "non-current" | "long" | "longterm" | "long-term" => {
            Some(BsTerm::NonCurrent)
        }
        _ => None,
    }
}

/// [`declared_bs_terms`] over a whole parsed journal.
///
/// # Errors
/// Returns [`ReportError::UnknownBsTerm`] for an unrecognized value.
pub fn bs_terms(journal: &Journal) -> Result<BTreeMap<String, BsTerm>, ReportError> {
    declared_bs_terms(&journal.accounts)
}

/// Declared `bsterm:` values per account. Untagged accounts are absent.
///
/// # Errors
/// Returns [`ReportError::UnknownBsTerm`] for a value outside the closed
/// vocabulary, following `issection:` rather than `type:`: a misspelt term files
/// the account under the wrong subheading and its balance into the wrong
/// subtotal, which is a plausible statement rather than a visibly broken one.
pub fn declared_bs_terms(
    accounts: &[AccountDeclaration],
) -> Result<BTreeMap<String, BsTerm>, ReportError> {
    accounts
        .iter()
        .filter_map(|decl| {
            decl.tags
                .iter()
                .find(|(key, _)| key == BS_TERM_TAG)
                .map(|(_, value)| value.trim())
                .filter(|value| !value.is_empty())
                .map(|value| {
                    parse_bs_term_tag(value)
                        .map(|term| (decl.name.0.clone(), term))
                        .ok_or_else(|| ReportError::UnknownBsTerm {
                            account: decl.name.0.clone(),
                            value: value.to_string(),
                        })
                })
        })
        .collect()
}

/// The effective term for an account printing on the group named `group_name`.
///
/// Its own declared tag, else the nearest declared ancestor's, else a default
/// taken from the GROUP: [`INVESTMENTS_GROUP`] is non-current and everything
/// else is current. That default is not invented here — it is the assumption
/// [`BUILT_IN_ORDER`] has always encoded, now made to mean something.
///
/// Untagged custom groups land in `Current` because that is what a journal that
/// bothers to tag is saying: you tag the house and the mortgage, and leave the
/// everyday accounts alone.
#[must_use]
pub fn resolve_bs_term(
    account: &str,
    group_name: &str,
    declared: &BTreeMap<String, BsTerm>,
) -> BsTerm {
    nearest_declared(declared, account).copied().unwrap_or({
        if group_name == INVESTMENTS_GROUP {
            BsTerm::NonCurrent
        } else {
            BsTerm::Current
        }
    })
}

/// Declared `bsgroup:` values per account, read off a parsed journal's `account`
/// directives — the group analogue of
/// [`super::account_types::declared_types`].
///
/// Deliberately NOT a new field on
/// [`AccountDecl`](super::account_types::AccountDecl): that type is built by
/// hand in ~25 test literals, and widening it would be pure churn for a value
/// only this report reads.
#[must_use]
pub fn account_groups(journal: &Journal) -> BTreeMap<String, String> {
    account_groups_from(&journal.accounts)
}

/// [`account_groups`] over a bare slice of declarations.
#[must_use]
pub fn account_groups_from(accounts: &[AccountDeclaration]) -> BTreeMap<String, String> {
    declared_groups_from(accounts, BS_GROUP_TAG)
}

/// Declared group names per account for ONE tag — [`BS_GROUP_TAG`] or
/// [`IS_GROUP_TAG`].
///
/// The two statements' group tags are the same idea read off the same
/// directives, so they are the same function with the tag name as a parameter
/// rather than two copies that can drift on trimming, on the empty-value rule,
/// or on hledger's own "a tag value ends at the next comma" gotcha (which is the
/// parser's business, and is therefore shared for free).
#[must_use]
pub fn declared_groups(journal: &Journal, tag: &str) -> BTreeMap<String, String> {
    declared_groups_from(&journal.accounts, tag)
}

/// [`declared_groups`] over a bare slice of declarations.
#[must_use]
pub fn declared_groups_from(
    accounts: &[AccountDeclaration],
    tag: &str,
) -> BTreeMap<String, String> {
    accounts
        .iter()
        .filter_map(|decl| {
            decl.tags
                .iter()
                .find(|(key, _)| key == tag)
                .map(|(_, value)| value.trim())
                .filter(|value| !value.is_empty())
                .map(|value| (decl.name.0.clone(), value.to_string()))
        })
        .collect()
}

/// The value declared for `account` itself, else the nearest declared
/// ANCESTOR's — steps 1 and 2 of every group resolution in this crate.
///
/// Walking up by `rfind(':')` is [`super::account_types::resolve_account_type`]'s
/// own loop, so a group tag inherits down a subtree exactly like `type:` does.
/// Shared by [`AccountGroups::resolve_uncached`] and by the income statement's
/// section and group resolution, all four of which are the same walk.
pub(super) fn nearest_declared<'a, T>(
    declared: &'a BTreeMap<String, T>,
    account: &str,
) -> Option<&'a T> {
    let mut name = account;
    loop {
        if let Some(found) = declared.get(name) {
            return Some(found);
        }
        name = &name[..name.rfind(':')?];
    }
}

/// Group resolution over one immutable set of declarations, memoized exactly as
/// [`AccountTypes`] memoizes type resolution: reports ask about the same couple
/// of hundred distinct account names once per posting, and resolution walks the
/// ancestry on every miss.
///
/// Purity is intact — the cache observes nothing but its own inputs and the
/// names it is asked about — so for a given construction this is the same total
/// function, only cheaper. Interior mutability keeps it a `&self` API, which
/// makes it `Send` but not `Sync`: the right trade for a cache built per report.
#[derive(Debug)]
pub struct AccountGroups {
    /// `bsgroup:` tags, keyed by the declaring account.
    declared: BTreeMap<String, String>,
    /// Effective account types, for the `Cash` and `Asset` tests.
    types: AccountTypes,
    /// Accounts holding at least one commodity that is not the base one. Read
    /// from the account's AS-WRITTEN balance, never a valued one — otherwise
    /// valuing the report into `$` would erase the very signal that says
    /// "this account holds securities", and the groups would move when the
    /// user flipped the valuation toggle.
    non_base: BTreeSet<String>,
    memo: RefCell<HashMap<String, (String, GroupSource)>>,
}

impl AccountGroups {
    /// Build a resolver.
    ///
    /// `balances` are the AS-WRITTEN (never valued) per-account totals and
    /// `base` the commodity the report values into; together they decide step 4.
    /// With no `base` there is no commodity signal, and step 4 never fires.
    #[must_use]
    pub fn new(
        declared: BTreeMap<String, String>,
        types: AccountTypes,
        balances: &BTreeMap<String, MixedAmount>,
        base: Option<&Commodity>,
    ) -> Self {
        let non_base = match base {
            None => BTreeSet::new(),
            Some(base) => balances
                .iter()
                .filter(|(_, ma)| ma.iter().any(|(commodity, _)| commodity != base))
                .map(|(account, _)| account.clone())
                .collect(),
        };
        Self {
            declared,
            types,
            non_base,
            memo: RefCell::new(HashMap::new()),
        }
    }

    /// The group `account` belongs to, and which step named it.
    #[must_use]
    pub fn resolve(&self, account: &str) -> (String, GroupSource) {
        if let Some(hit) = self.memo.borrow().get(account) {
            return hit.clone();
        }
        let resolved = self.resolve_uncached(account);
        self.memo
            .borrow_mut()
            .insert(account.to_string(), resolved.clone());
        resolved
    }

    fn resolve_uncached(&self, account: &str) -> (String, GroupSource) {
        // 1 & 2 — the account's own `bsgroup:`, else the nearest declared
        // ancestor's, so a tag inherits down a subtree exactly like `type:`.
        if let Some(group) = nearest_declared(&self.declared, account) {
            return (group.clone(), GroupSource::Tag);
        }
        // 3 — type-driven. `Cash` is hledger's Asset subtype, the one
        // `cashflow` selects on, so it is exactly "money you can spend today".
        if self.types.is_cash(account) {
            return (CASH_GROUP.to_string(), GroupSource::Type);
        }
        // 4 — commodity-driven. An asset holding anything other than the base
        // commodity is a position, not a balance.
        if self.non_base.contains(account) && self.types.is_type(account, AccountType::Asset) {
            return (INVESTMENTS_GROUP.to_string(), GroupSource::Commodity);
        }
        // 5 — tree-position-driven, and total: every account has a name, so
        // nothing can fall out of the report here.
        (segment_label(account), GroupSource::Segment)
    }
}

/// The label for step 5: the account's second path segment, aliased or
/// capitalized. Accounts with a single segment fall back to that segment, so
/// even a bare `equity` posting gets a group.
fn segment_label(account: &str) -> String {
    let mut segments = account.split(':');
    let root = segments.next().unwrap_or("");
    let segment = segments.next().filter(|s| !s.is_empty()).unwrap_or(root);
    humanized_segment(segment)
}

/// One path segment as a group LABEL: the cosmetic [`alias`] when there is one,
/// else the segment with its first character upper-cased.
///
/// Shared with the income statement, whose untagged fallback picks a DIFFERENT
/// segment (see `income_statement::group_segment_index`) but humanizes it
/// identically — so `cc` reads "Credit cards" on both statements, and the alias
/// table has exactly one home.
pub(super) fn humanized_segment(segment: &str) -> String {
    alias(segment).map_or_else(|| capitalized(segment), str::to_string)
}

/// Cosmetic label aliases for conventional abbreviations.
///
/// This table renames and NOTHING else. Membership was already decided by
/// [`AccountGroups::resolve_uncached`] before it is consulted, so no entry here
/// can move an account between groups, and adding one can never reintroduce the
/// name-matching failure mode this module's doc comment is about.
fn alias(segment: &str) -> Option<&'static str> {
    match ascii_or_lowercased(segment).as_ref() {
        "cc" => Some("Credit cards"),
        "ar" => Some("Accounts receivable"),
        "ap" => Some("Accounts payable"),
        _ => None,
    }
}

/// `segment` with its first character upper-cased (`bank` → `Bank`), the rest
/// left exactly as the user wrote it.
fn capitalized(segment: &str) -> String {
    let mut chars = segment.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::super::account_types::{AccountDecl, declared_types};
    use super::*;
    use crate::decimal::Dec;

    fn c(symbol: &str) -> Commodity {
        Commodity(symbol.into())
    }

    fn decl(name: &str, ty: AccountType) -> AccountDecl {
        AccountDecl {
            name: name.into(),
            account_type: Some(ty),
        }
    }

    /// A resolver over declared types + `bsgroup:` tags + per-account balances.
    fn resolver(
        types: &[AccountDecl],
        groups: &[(&str, &str)],
        balances: &[(&str, &[(&str, i128)])],
        base: Option<&str>,
    ) -> AccountGroups {
        let balances: BTreeMap<String, MixedAmount> = balances
            .iter()
            .map(|(account, holdings)| {
                let mut ma = MixedAmount::new();
                for (commodity, qty) in *holdings {
                    ma.accumulate(&c(commodity), Dec::new(*qty, 0)).unwrap();
                }
                ((*account).to_string(), ma)
            })
            .collect();
        AccountGroups::new(
            groups
                .iter()
                .map(|(account, group)| ((*account).to_string(), (*group).to_string()))
                .collect(),
            AccountTypes::from_declared(declared_types(types)),
            &balances,
            base.map(c).as_ref(),
        )
    }

    #[test]
    fn own_tag_beats_every_other_signal() {
        let groups = resolver(
            &[decl("assets:bank:checking", AccountType::Cash)],
            &[("assets:bank:checking", "Operating cash")],
            &[("assets:bank:checking", &[("EUR", 100)])],
            Some("$"),
        );
        // Cash type AND a non-base commodity are both present and both lose.
        assert_eq!(
            groups.resolve("assets:bank:checking"),
            ("Operating cash".to_string(), GroupSource::Tag)
        );
    }

    #[test]
    fn tag_inherits_from_the_nearest_declared_ancestor() {
        let groups = resolver(
            &[decl("assets:broker", AccountType::Asset)],
            &[("assets:broker", "Brokerage"), ("assets", "Everything")],
            &[("assets:broker:taxable:aapl", &[("AAPL", 10)])],
            Some("$"),
        );
        // The NEARER ancestor wins, and it beats the commodity signal below.
        assert_eq!(
            groups.resolve("assets:broker:taxable:aapl"),
            ("Brokerage".to_string(), GroupSource::Tag)
        );
        assert_eq!(
            groups.resolve("assets:other"),
            ("Everything".to_string(), GroupSource::Tag)
        );
    }

    #[test]
    fn cash_type_beats_the_commodity_signal() {
        // A EUR cash account holds a non-base commodity, but it is still cash.
        let groups = resolver(
            &[decl("assets:bank:wise:eur", AccountType::Cash)],
            &[],
            &[("assets:bank:wise:eur", &[("EUR", 500)])],
            Some("$"),
        );
        assert_eq!(
            groups.resolve("assets:bank:wise:eur"),
            (CASH_GROUP.to_string(), GroupSource::Type)
        );
    }

    #[test]
    fn a_non_base_commodity_makes_an_asset_an_investment() {
        let groups = resolver(
            &[
                decl("assets:broker:taxable:aapl", AccountType::Asset),
                decl("liabilities:loan:margin", AccountType::Liability),
            ],
            &[],
            &[
                ("assets:broker:taxable:aapl", &[("AAPL", 10)]),
                // A LIABILITY in a foreign commodity is not an investment.
                ("liabilities:loan:margin", &[("EUR", -100)]),
            ],
            Some("$"),
        );
        assert_eq!(
            groups.resolve("assets:broker:taxable:aapl"),
            (INVESTMENTS_GROUP.to_string(), GroupSource::Commodity)
        );
        assert_eq!(
            groups.resolve("liabilities:loan:margin"),
            ("Loan".to_string(), GroupSource::Segment)
        );
    }

    /// Without a base commodity there is no "non-base" to test against, so
    /// step 4 must not fire (rather than calling everything an investment).
    #[test]
    fn no_base_commodity_disables_the_commodity_signal() {
        let groups = resolver(
            &[decl("assets:broker:taxable:aapl", AccountType::Asset)],
            &[],
            &[("assets:broker:taxable:aapl", &[("AAPL", 10)])],
            None,
        );
        assert_eq!(
            groups.resolve("assets:broker:taxable:aapl"),
            ("Broker".to_string(), GroupSource::Segment)
        );
    }

    #[test]
    fn falls_back_to_the_humanized_second_segment() {
        let groups = resolver(&[], &[], &[], Some("$"));
        for (account, want) in [
            ("assets:vault:gold", "Vault"),
            ("liabilities:cc:visa", "Credit cards"),
            ("liabilities:ap:acme", "Accounts payable"),
            ("assets:ar:customers", "Accounts receivable"),
            ("assets:Vault:gold", "Vault"),
            // A single-segment account still gets a group.
            ("equity", "Equity"),
            ("", ""),
        ] {
            assert_eq!(
                groups.resolve(account),
                (want.to_string(), GroupSource::Segment),
                "account {account:?}"
            );
        }
    }

    /// Step 3 runs before step 5, and an account's effective TYPE has its own
    /// name fallback (hledger's Cash heuristic) when nothing is declared. So an
    /// undeclared `assets:bank:chase` is Cash, and lands in the cash group
    /// rather than in a "Bank" segment group. That is the type talking, not this
    /// module: the group signal never reads the name itself.
    #[test]
    fn an_undeclared_cash_looking_account_is_cash_before_it_is_a_segment() {
        let groups = resolver(&[], &[], &[], Some("$"));
        assert_eq!(
            groups.resolve("assets:bank:chase"),
            (CASH_GROUP.to_string(), GroupSource::Type)
        );
        // Declaring the subtree something else moves it, name unchanged.
        let declared = resolver(
            &[decl("assets:bank", AccountType::Asset)],
            &[],
            &[],
            Some("$"),
        );
        assert_eq!(
            declared.resolve("assets:bank:chase"),
            ("Bank".to_string(), GroupSource::Segment)
        );
    }

    /// The alias table is a RENAME. `pasivo:cc:visa` and `liabilities:cc:visa`
    /// share a group because they share a tree position, not because either
    /// name is English — and a non-English segment that no alias knows still
    /// gets its own group rather than falling out of the report.
    #[test]
    fn aliases_rename_without_deciding_membership() {
        let groups = resolver(
            &[
                decl("pasivo:cc:visa", AccountType::Liability),
                decl("pasivo:tarjeta:oro", AccountType::Liability),
            ],
            &[],
            &[],
            Some("$"),
        );
        assert_eq!(
            groups.resolve("pasivo:cc:visa"),
            ("Credit cards".to_string(), GroupSource::Segment)
        );
        assert_eq!(
            groups.resolve("pasivo:tarjeta:oro"),
            ("Tarjeta".to_string(), GroupSource::Segment)
        );
    }

    /// The memo must answer identically on every repeat ask, on every path.
    #[test]
    fn memo_is_stable_across_repeat_asks() {
        let groups = resolver(
            &[
                decl("assets:bank:checking", AccountType::Cash),
                decl("assets:broker:taxable:aapl", AccountType::Asset),
            ],
            &[("assets:vault", "Bullion")],
            &[("assets:broker:taxable:aapl", &[("AAPL", 10)])],
            Some("$"),
        );
        let names = [
            "assets:bank:checking",
            "assets:broker:taxable:aapl",
            "assets:vault:gold",
            "liabilities:cc:visa",
            "misc",
        ];
        let first: Vec<_> = names.iter().map(|name| groups.resolve(name)).collect();
        let second: Vec<_> = names.iter().map(|name| groups.resolve(name)).collect();
        assert_eq!(first, second);
    }

    /// Every spelling `parse_bs_term_tag` accepts, pinned one by one. The plan
    /// locks the vocabulary to `current | noncurrent` — the two the
    /// `UnknownBsTerm` error names — and the synonyms are accepted on top of
    /// it, so an "align the parser with the error message" cleanup that
    /// dropped a spelling would break every journal using it (each tagged
    /// account becomes an `UnknownBsTerm` refusal). Conversely, nothing
    /// OUTSIDE this list may ever parse: a lenient fallback would file
    /// misspellings under `Current` silently.
    #[test]
    fn parse_bs_term_accepts_exactly_the_documented_spellings() {
        for spelling in ["current", "short", "shortterm", "short-term"] {
            assert_eq!(
                parse_bs_term_tag(spelling),
                Some(BsTerm::Current),
                "{spelling:?}"
            );
        }
        for spelling in ["noncurrent", "non-current", "long", "longterm", "long-term"] {
            assert_eq!(
                parse_bs_term_tag(spelling),
                Some(BsTerm::NonCurrent),
                "{spelling:?}"
            );
        }
        // Trimmed and case-insensitive, like every tag value on this sheet.
        assert_eq!(parse_bs_term_tag("  Short-Term "), Some(BsTerm::Current));
        assert_eq!(parse_bs_term_tag("LONG"), Some(BsTerm::NonCurrent));
        // Anything else is refused, never defaulted.
        for outside in ["", "curr", "currentt", "non current", "long-ish"] {
            assert_eq!(parse_bs_term_tag(outside), None, "{outside:?}");
        }
    }

    /// plans/12, decision 3: once the split is on, an untagged account on the
    /// BUILT-IN Investments group defaults to NON-current, and untagged
    /// accounts on every other group default to Current. Flipping (or
    /// deleting) the `INVESTMENTS_GROUP` branch in `resolve_bs_term` fails the
    /// first assertion.
    #[test]
    fn resolve_bs_term_defaults_investments_to_non_current_and_the_rest_to_current() {
        let untagged = BTreeMap::new();
        assert_eq!(
            resolve_bs_term("assets:broker:taxable:aapl", INVESTMENTS_GROUP, &untagged),
            BsTerm::NonCurrent,
            "the built-in Investments group is non-current by default"
        );
        for group in [CASH_GROUP, "Accounts receivable", "Cartera"] {
            assert_eq!(
                resolve_bs_term("assets:whatever", group, &untagged),
                BsTerm::Current,
                "group {group:?}"
            );
        }

        // A declared tag — own or inherited from the nearest declared
        // ancestor — beats the group default in BOTH directions.
        let declared: BTreeMap<String, BsTerm> = BTreeMap::from([
            ("assets:broker".to_string(), BsTerm::Current),
            ("assets:vault".to_string(), BsTerm::NonCurrent),
        ]);
        assert_eq!(
            resolve_bs_term("assets:broker:taxable:aapl", INVESTMENTS_GROUP, &declared),
            BsTerm::Current,
            "an inherited tag overrides the Investments default"
        );
        assert_eq!(
            resolve_bs_term("assets:vault:gold", CASH_GROUP, &declared),
            BsTerm::NonCurrent,
            "an inherited tag overrides the Current default"
        );
    }

    #[test]
    fn ranks_known_groups_first_and_synthetics_last() {
        let mut named: Vec<(&str, GroupSource)> = vec![
            ("Zebra", GroupSource::Segment),
            (VALUATION_ADJUSTMENT_GROUP, GroupSource::Computed),
            ("Credit cards", GroupSource::Segment),
            (RETAINED_EARNINGS_GROUP, GroupSource::Computed),
            (INVESTMENTS_GROUP, GroupSource::Commodity),
            ("Alpha", GroupSource::Segment),
            (CASH_GROUP, GroupSource::Type),
        ];
        named.sort_by_key(|(name, source)| (group_rank(name, *source), (*name).to_string()));
        assert_eq!(
            named.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            [
                CASH_GROUP,
                INVESTMENTS_GROUP,
                "Credit cards",
                "Alpha",
                "Zebra",
                RETAINED_EARNINGS_GROUP,
                VALUATION_ADJUSTMENT_GROUP,
            ]
        );
    }
}
