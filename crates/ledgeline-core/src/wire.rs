//! hledger-1.52-compatible JSON serialization.
//!
//! These serde structs mirror the shape emitted by `hledger-web`'s
//! `/transactions` endpoint. The model → wire mapping lives here so the domain
//! model in [`crate::model`] can stay serde-free. Field names match hledger
//! exactly (camelCase ones via `#[serde(rename = ...)]`).

use crate::holdings::{HoldingsScope, ScopeMode, WarningKind, compute_holdings};
use crate::model::{
    AccountDeclaration, AccountName, Amount, CommoditySide, CostKind, Journal, Posting,
    PostingType, PriceDirective, SourcePos, Status, Transaction,
};
use crate::title::{journal_title, main_file_name};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The hledger version Ledgeline emulates on the wire (served at `/version`).
pub const HLEDGER_VERSION: &str = "1.52";

/// Synthetic tree root account name (hledger's `/accounts` root node).
const ACCOUNT_ROOT: &str = "root";

/// The single, all-time period key hledger uses when no report interval is set.
const ALL_TIME_PERIOD: &str = "0000-01-01";

/// A serialized transaction (`Transaction` JSON object).
#[derive(Debug, Serialize)]
pub struct WireTransaction {
    tindex: u32,
    tdate: String,
    tdate2: Option<String>,
    tstatus: &'static str,
    tcode: String,
    tdescription: String,
    tcomment: String,
    tprecedingcomment: String,
    ttags: Vec<(String, String)>,
    tpostings: Vec<WirePosting>,
    tsourcepos: [WirePos; 2],
}

/// A serialized posting.
#[derive(Debug, Serialize)]
pub struct WirePosting {
    paccount: String,
    pamount: Vec<WireAmount>,
    pbalanceassertion: Option<WireBalanceAssertion>,
    pcomment: String,
    pdate: Option<String>,
    pdate2: Option<String>,
    poriginal: Option<()>,
    pstatus: &'static str,
    ptags: Vec<(String, String)>,
    ptransaction_: String,
    ptype: &'static str,
}

/// A serialized single-commodity amount.
#[derive(Debug, Serialize)]
pub struct WireAmount {
    acommodity: String,
    acost: Option<WireCost>,
    acostbasis: Option<()>,
    aquantity: WireQuantity,
    astyle: WireStyle,
}

/// A serialized cost (`{contents, tag}`).
#[derive(Debug, Serialize)]
pub struct WireCost {
    contents: Box<WireAmount>,
    tag: &'static str,
}

/// A serialized exact quantity.
#[derive(Debug, Serialize)]
pub struct WireQuantity {
    #[serde(rename = "decimalMantissa")]
    decimal_mantissa: i128,
    #[serde(rename = "decimalPlaces")]
    decimal_places: u32,
    #[serde(rename = "floatingPoint")]
    floating_point: f64,
}

/// A serialized amount style.
#[derive(Debug, Serialize)]
pub struct WireStyle {
    ascommodityside: &'static str,
    ascommodityspaced: bool,
    asdecimalmark: Option<String>,
    asdigitgroups: Option<(String, Vec<u8>)>,
    asprecision: u32,
    asrounding: &'static str,
}

/// A serialized balance assertion.
#[derive(Debug, Serialize)]
pub struct WireBalanceAssertion {
    baamount: WireAmount,
    bainclusive: bool,
    batotal: bool,
    baposition: WirePos,
}

/// A serialized source position (used by both `tsourcepos` and `baposition`).
#[derive(Debug, Serialize)]
pub struct WirePos {
    #[serde(rename = "sourceColumn")]
    source_column: u32,
    #[serde(rename = "sourceLine")]
    source_line: u32,
    #[serde(rename = "sourceName")]
    source_name: String,
}

/// Serialize a journal's transactions to the hledger wire representation.
#[must_use]
pub fn journal_to_transactions(journal: &Journal) -> Vec<WireTransaction> {
    let account_tags = account_tag_map(&journal.accounts);
    let commodity_tags: HashMap<&str, &[(String, String)]> = journal
        .commodity_tags
        .iter()
        .map(|(commodity, tags)| (commodity.0.as_str(), tags.as_slice()))
        .collect();

    journal
        .transactions
        .iter()
        .map(|transaction| {
            transaction_to_wire(
                transaction,
                &account_tags,
                &commodity_tags,
                &journal.source_name,
            )
        })
        .collect()
}

/// Serialize a journal's transactions to a `serde_json::Value` array.
///
/// # Errors
/// Returns any error from `serde_json` while building the value (none is
/// expected for finite, well-formed input).
pub fn journal_to_value(journal: &Journal) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(journal_to_transactions(journal))
}

fn transaction_to_wire(
    transaction: &Transaction,
    account_tags: &HashMap<&str, &[(String, String)]>,
    commodity_tags: &HashMap<&str, &[(String, String)]>,
    source_name: &str,
) -> WireTransaction {
    let tindex = transaction.index.0;
    WireTransaction {
        tindex,
        tdate: transaction.date.clone(),
        tdate2: transaction.date2.clone(),
        tstatus: status_str(transaction.status),
        tcode: transaction.code.clone(),
        tdescription: transaction.description.clone(),
        tcomment: transaction.comment.clone(),
        tprecedingcomment: transaction.preceding_comment.clone(),
        ttags: transaction.tags.clone(),
        tpostings: transaction
            .postings
            .iter()
            .map(|posting| {
                posting_to_wire(posting, tindex, account_tags, commodity_tags, source_name)
            })
            .collect(),
        tsourcepos: [
            pos_to_wire(transaction.source_span.0, source_name),
            pos_to_wire(transaction.source_span.1, source_name),
        ],
    }
}

fn posting_to_wire(
    posting: &Posting,
    tindex: u32,
    account_tags: &HashMap<&str, &[(String, String)]>,
    commodity_tags: &HashMap<&str, &[(String, String)]>,
    source_name: &str,
) -> WirePosting {
    WirePosting {
        paccount: posting.account.0.clone(),
        pamount: posting.amounts.iter().map(amount_to_wire).collect(),
        pbalanceassertion: posting.balance_assertion.as_ref().map(|assertion| {
            WireBalanceAssertion {
                baamount: amount_to_wire(&assertion.amount),
                bainclusive: assertion.inclusive,
                batotal: assertion.total,
                baposition: pos_to_wire(assertion.position, source_name),
            }
        }),
        pcomment: posting.comment.clone(),
        pdate: posting.date.clone(),
        pdate2: posting.date2.clone(),
        poriginal: None,
        pstatus: status_str(posting.status),
        ptags: posting_tags(posting, account_tags, commodity_tags),
        ptransaction_: tindex.to_string(),
        ptype: ptype_str(posting.ptype),
    }
}

fn ptype_str(ptype: PostingType) -> &'static str {
    match ptype {
        PostingType::Regular => "RegularPosting",
        PostingType::Virtual => "VirtualPosting",
        PostingType::BalancedVirtual => "BalancedVirtualPosting",
    }
}

fn amount_to_wire(amount: &Amount) -> WireAmount {
    WireAmount {
        acommodity: amount.commodity.0.clone(),
        acost: amount.cost.as_ref().map(|cost| WireCost {
            contents: Box::new(amount_to_wire(&cost.amount)),
            tag: match cost.kind {
                CostKind::Unit => "UnitCost",
                CostKind::Total => "TotalCost",
            },
        }),
        acostbasis: None,
        aquantity: WireQuantity {
            decimal_mantissa: amount.quantity.mantissa,
            decimal_places: amount.quantity.places,
            floating_point: amount.quantity.floating_point(),
        },
        astyle: WireStyle {
            ascommodityside: match amount.style.side {
                CommoditySide::Left => "L",
                CommoditySide::Right => "R",
            },
            ascommodityspaced: amount.style.spaced,
            asdecimalmark: amount.style.decimal_mark.map(|mark| mark.to_string()),
            asdigitgroups: amount
                .style
                .digit_groups
                .as_ref()
                .map(|groups| (groups.mark.to_string(), groups.sizes.clone())),
            asprecision: amount.style.precision,
            asrounding: "NoRounding",
        },
    }
}

/// Map each declared account name to its declaration's tag slice, for the
/// account-directive tag inheritance shared by the wire `ptags` computation and
/// the holdings name resolution.
pub(crate) fn account_tag_map(
    accounts: &[AccountDeclaration],
) -> HashMap<&str, &[(String, String)]> {
    accounts
        .iter()
        .map(|decl| (decl.name.0.as_str(), decl.tags.as_slice()))
        .collect()
}

/// The declared tags a posting on `account` inherits: the account's own declared
/// tags, then each ancestor's, most-specific first (hledger's account-directive
/// tag inheritance). This is the single implementation of the ancestor walk,
/// shared by [`posting_tags`] and the holdings engine's name resolution.
pub(crate) fn inherited_account_tags(
    account: &AccountName,
    account_tags: &HashMap<&str, &[(String, String)]>,
) -> Vec<(String, String)> {
    account
        .self_and_ancestors()
        .iter()
        .filter_map(|name| account_tags.get(name.as_str()).copied())
        .flat_map(|declared| declared.iter().cloned())
        .collect()
}

/// Compute a posting's `ptags`: its own comment tags first, then account tags
/// inherited from the account and its ancestors (most-specific first),
/// de-duplicated on exact `(key, value)` keeping the first occurrence. Finally,
/// tags from the `commodity` directives of the posting's amounts are appended,
/// but only for names not already present (a posting or account tag of the same
/// name takes precedence, matching hledger).
fn posting_tags(
    posting: &Posting,
    account_tags: &HashMap<&str, &[(String, String)]>,
    commodity_tags: &HashMap<&str, &[(String, String)]>,
) -> Vec<(String, String)> {
    let mut tags: Vec<(String, String)> = posting.tags.clone();
    tags.extend(inherited_account_tags(&posting.account, account_tags));

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut result: Vec<(String, String)> = tags
        .into_iter()
        .filter(|pair| seen.insert(pair.clone()))
        .collect();

    let existing_names: HashSet<String> = result.iter().map(|(name, _)| name.clone()).collect();
    let mut added: HashSet<(String, String)> = HashSet::new();
    for amount in &posting.amounts {
        let Some(declared) = commodity_tags.get(amount.commodity.0.as_str()) else {
            continue;
        };
        for pair in declared.iter() {
            if !existing_names.contains(&pair.0) && added.insert(pair.clone()) {
                result.push(pair.clone());
            }
        }
    }
    result
}

fn pos_to_wire(pos: SourcePos, source_name: &str) -> WirePos {
    WirePos {
        source_column: pos.column,
        source_line: pos.line,
        source_name: source_name.to_string(),
    }
}

fn status_str(status: Status) -> &'static str {
    match status {
        Status::Unmarked => "Unmarked",
        Status::Pending => "Pending",
        Status::Cleared => "Cleared",
    }
}

// ===========================================================================
// /api/diagnostics
// ===========================================================================

/// A journal-wide diagnostic, shaped to drop straight into the SPA's existing
/// `Problem` type (`web/src/lib/checks/engine.ts`) so the checks UI needs no new
/// type to render one.
///
/// An unbalanced transaction and a failed balance assertion are DIAGNOSTICS, not
/// parse errors: the journal always opens, and neither ever blocks opening,
/// editing or reloading. See [`crate::parse::check_transaction_balances`] and
/// [`crate::assertions::check_balance_assertions`] for why.
///
/// The same shape also carries the three STOCK findings — see
/// [`journal_to_stock_diagnostics`], which is why `rule` and `severity` are no
/// longer a two-value and a one-value enum.
#[derive(Debug, Serialize)]
pub struct WireDiagnostic {
    /// 0-based index into the SAME transactions array `/transactions` serves,
    /// so the UI can flag the row.
    #[serde(rename = "txnIndex")]
    txn_index: usize,
    /// One of [`DIAGNOSTIC_RULES`]. The SPA allow-lists exactly this set
    /// (`normalizeDiagnostics` in `web/src/lib/api/normalize.ts`) and drops
    /// anything else, so the two lists must be grown together.
    rule: &'static str,
    /// `"error"` or `"warning"`; the SPA's `Severity` is `info|warning|error`.
    severity: &'static str,
    /// Human-readable, hledger-style.
    message: String,
}

/// Diagnostic severity for the two hledger-level rules: both are errors.
const DIAGNOSTIC_ERROR: &str = "error";

/// Diagnostic severity for the three stock rules. They describe a journal
/// hledger itself accepts — an unknown cost basis is a gap in the data, not a
/// broken entry — so they are warnings, and never promoted to errors.
const DIAGNOSTIC_WARNING: &str = "warning";

/// Every `rule` value `/api/diagnostics` can emit, in the order the SPA's
/// drawer groups them. The first two are hledger-level errors; the last three
/// are the stock findings from the holdings engine.
///
/// This is the wire contract the SPA's `DIAGNOSTIC_RULES` allow-list mirrors.
/// The SPA drops an unrecognized rule SILENTLY, so the two are pinned together
/// by `web/src/lib/checks/stock-diagnostics.test.ts`, which reads both files —
/// it lives on that side because `nix build .#tests` builds from
/// `cleanCargoSource`, which does not contain `web/`.
pub const DIAGNOSTIC_RULES: [&str; 5] = [
    "unbalanced",
    "assertion",
    "stock-missing-basis",
    "stock-negative",
    "stock-unpriced",
];

/// Every diagnostic in `journal`: the unbalanced transactions first (in file
/// order), then the failed balance assertions (in hledger's evaluation order,
/// which is date order and deliberately preserved).
///
/// A check whose arithmetic overflows `i128` — unreachable for a real journal,
/// since a literal that large is already rejected at parse — contributes
/// nothing rather than aborting the payload. The journal must always open, and
/// the frozen wire contract has no third `rule` to report the failure under.
#[must_use]
pub fn journal_to_diagnostics(journal: &Journal) -> Vec<WireDiagnostic> {
    let unbalanced = crate::parse::check_transaction_balances(journal)
        .unwrap_or_default()
        .into_iter()
        .map(|failure| WireDiagnostic {
            txn_index: failure.transaction_index,
            rule: "unbalanced",
            severity: DIAGNOSTIC_ERROR,
            message: failure.message(),
        });
    let assertions = crate::assertions::check_balance_assertions(journal)
        .unwrap_or_default()
        .into_iter()
        .map(|failure| WireDiagnostic {
            txn_index: failure.transaction_index,
            rule: "assertion",
            severity: DIAGNOSTIC_ERROR,
            message: failure.message(),
        });
    unbalanced.chain(assertions).collect()
}

/// The as-of date the stock diagnostics are computed at: far enough in the
/// future to include the whole journal.
///
/// The TypeScript check rules this replaces valued at `today()`. A journal-wide
/// diagnostic must not, for three reasons:
///
/// 1. **It is cached.** The payload is built once per snapshot and served under
///    an `ETag` (see `Snapshot::from_journal`). A clock-dependent body silently
///    goes stale at midnight with nothing to invalidate it.
/// 2. **It must be reproducible.** Golden fixtures and the endpoint tests would
///    otherwise depend on the day they run.
/// 3. **It is the better answer.** A future-dated purchase with no cost
///    annotation is a data-entry mistake today, not one that starts mattering on
///    its settlement date. Hiding it until then is exactly the kind of finding a
///    checks drawer exists to surface early.
///
/// Comparisons against it are lexical on zero-padded ISO dates, the same
/// convention the rest of the engine uses.
const STOCK_DIAGNOSTICS_AS_OF: &str = "9999-12-31";

/// Rank a stock rule for report order, so the SPA drawer groups the three in a
/// stable, meaningful sequence rather than in symbol-major order.
fn stock_rule_rank(rule: &str) -> usize {
    DIAGNOSTIC_RULES
        .iter()
        .position(|known| *known == rule)
        .unwrap_or(usize::MAX)
}

/// The `rule` a holdings warning reports under. 1:1 with [`WarningKind`], which
/// is the mapping `HoldingsWarning`'s doc has always claimed.
fn stock_rule_of(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::MissingBasis => "stock-missing-basis",
        WarningKind::NegativeShares => "stock-negative",
        WarningKind::Unpriced => "stock-unpriced",
    }
}

/// The journal's stock findings — unknown cost basis, net-negative shares, and
/// unpriced positions — as diagnostics anchored to the transactions that caused
/// them.
///
/// This is the Rust half of DRY-1: the SPA used to recompute all three from its
/// own copy of the average-cost pools (`web/src/lib/holdings/engine.ts`), which
/// had drifted from this engine — most visibly on a stock split, where the two
/// pages reported opposite answers for the same journal. There is now one
/// engine, and the drawer reads it.
///
/// Scope is the WHOLE journal (every account, no `valueIn` override), matching
/// the unscoped rules this replaces; the as-of date is
/// [`STOCK_DIAGNOSTICS_AS_OF`]. A warning naming several transactions — a symbol
/// acquired cost-lessly more than once — becomes one diagnostic per transaction,
/// so every offending row is flagged. Output is ordered by rule, then symbol,
/// then transaction.
///
/// A holdings computation that overflows `i128` contributes nothing rather than
/// aborting the payload, exactly as the balance checks do: the journal must
/// always open.
#[must_use]
pub fn journal_to_stock_diagnostics(journal: &Journal) -> Vec<WireDiagnostic> {
    let scope = HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include, // include + empty set = every account
        as_of: STOCK_DIAGNOSTICS_AS_OF.to_string(),
        gain_since: None,
        value_in: None,
    };
    let Ok(report) = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
    ) else {
        return Vec::new();
    };

    // `Tindex` is 1-based file order; the wire wants a 0-based POSITION into the
    // transactions array as served. They differ whenever the journal was built
    // any way other than a single straight parse, so the map is built rather
    // than assumed.
    let positions: HashMap<u32, usize> = journal
        .transactions
        .iter()
        .enumerate()
        .map(|(position, txn)| (txn.index.0, position))
        .collect();

    let mut diagnostics: Vec<(usize, &str, WireDiagnostic)> = report
        .warnings
        .iter()
        .flat_map(|warning| {
            // An unanchored warning would vanish here rather than reach the
            // drawer, and nothing downstream could tell. `HoldingsWarning.txns`
            // is documented as never empty; this is what keeps that true.
            debug_assert!(
                !warning.txns.is_empty(),
                "holdings warning with no transaction to anchor to: {warning:?}"
            );
            let rule = stock_rule_of(warning.kind);
            let positions = &positions;
            warning.txns.iter().filter_map(move |txn| {
                positions.get(&txn.0).map(|&position| {
                    (
                        stock_rule_rank(rule),
                        warning.symbol.as_str(),
                        WireDiagnostic {
                            txn_index: position,
                            rule,
                            severity: DIAGNOSTIC_WARNING,
                            message: warning.message.clone(),
                        },
                    )
                })
            })
        })
        .collect();
    diagnostics.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(b.1))
            .then_with(|| a.2.txn_index.cmp(&b.2.txn_index))
    });
    diagnostics
        .into_iter()
        .map(|(_, _, diagnostic)| diagnostic)
        .collect()
}

/// Every diagnostic `/api/diagnostics` serves: the hledger-level errors from
/// [`journal_to_diagnostics`] first, then the stock warnings from
/// [`journal_to_stock_diagnostics`].
///
/// The errors lead deliberately. The SPA's drawer groups by rule in
/// FIRST-APPEARANCE order, so a journal that both fails to balance and holds an
/// unpriced security shows the balance failure at the top.
#[must_use]
pub fn journal_to_all_diagnostics(journal: &Journal) -> Vec<WireDiagnostic> {
    let mut all = journal_to_diagnostics(journal);
    all.extend(journal_to_stock_diagnostics(journal));
    all
}

/// [`journal_to_all_diagnostics`] as the `{"diagnostics": [...]}` payload. The
/// array is always present and is `[]` — never null, never absent — when the
/// journal is clean.
///
/// # Errors
/// Propagates any `serde_json` error (never expected for these records).
pub fn journal_to_diagnostics_value(
    journal: &Journal,
) -> Result<serde_json::Value, serde_json::Error> {
    Ok(serde_json::json!({
        "diagnostics": serde_json::to_value(journal_to_all_diagnostics(journal))?,
    }))
}

// ===========================================================================
// /version
// ===========================================================================

/// The `/version` payload: the emulated hledger version as a bare JSON string.
#[must_use]
pub fn version_value() -> serde_json::Value {
    serde_json::Value::String(HLEDGER_VERSION.to_string())
}

// ===========================================================================
// /api/journal
// ===========================================================================

/// The `/api/journal` payload: which journal the user is looking at.
///
/// A NATIVE route, so the field names are camelCase like the rest of the
/// `/api/*` wire (both happen to be single words, and the rename is declared
/// anyway so adding a second-word field cannot silently break the convention).
///
/// Both fields are nullable and independently so: a journal can be titled with
/// no file behind it (an in-memory journal that opens with a comment) and a file
/// can be there with nothing to call it. `null` is served explicitly rather than
/// the key being omitted, so the SPA never has to tell "absent" from "unknown".
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireJournalInfo {
    /// The derived display title — see [`crate::title`] for how, and why.
    title: Option<String>,
    /// The BARE filename of the main journal file, never a path.
    file: Option<String>,
}

/// The open journal's derived title and the bare name of its main file.
///
/// Cheap: both halves read data the parse already captured. No filesystem
/// access, no pass over the transactions.
#[must_use]
pub fn journal_to_info(journal: &Journal) -> WireJournalInfo {
    WireJournalInfo {
        title: journal_title(journal),
        file: main_file_name(journal),
    }
}

// ===========================================================================
// /accountnames
// ===========================================================================

/// Every real account name — the union of accounts named in postings, accounts
/// named in `account` directives, and all of their implied ancestors — sorted
/// lexically. The synthetic root is not a real account and is excluded.
#[must_use]
pub fn journal_to_accountnames(journal: &Journal) -> Vec<String> {
    let from_postings = journal
        .transactions
        .iter()
        .flat_map(|transaction| &transaction.postings)
        .flat_map(|posting| posting.account.self_and_ancestors());
    let from_declarations = journal
        .accounts
        .iter()
        .flat_map(|declaration| declaration.name.self_and_ancestors());
    // A BTreeSet dedupes and yields the names in lexical order in one pass.
    from_postings
        .chain(from_declarations)
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}

/// [`journal_to_accountnames`] as a JSON array value.
///
/// # Errors
/// Propagates any `serde_json` error (never expected for these string arrays).
pub fn journal_to_accountnames_value(
    journal: &Journal,
) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(journal_to_accountnames(journal))
}

// ===========================================================================
// /commodities
// ===========================================================================

/// Every commodity symbol appearing in an amount (including nested costs), a
/// balance assertion, a `commodity` directive, or a price directive — sorted
/// lexically (so `"$"` precedes the alphabetic symbols).
#[must_use]
pub fn journal_to_commodities(journal: &Journal) -> Vec<String> {
    let from_directives = journal
        .commodity_styles
        .iter()
        .map(|(commodity, _)| commodity.0.clone());
    let from_amounts = journal
        .transactions
        .iter()
        .flat_map(|transaction| &transaction.postings)
        .flat_map(|posting| {
            posting
                .amounts
                .iter()
                .chain(posting.balance_assertion.as_ref().map(|a| &a.amount))
        })
        .flat_map(amount_commodities);
    let from_prices = journal.prices.iter().flat_map(|price| {
        std::iter::once(price.commodity.0.clone()).chain(amount_commodities(&price.price))
    });
    from_directives
        .chain(from_amounts)
        .chain(from_prices)
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}

/// [`journal_to_commodities`] as a JSON array value.
///
/// # Errors
/// Propagates any `serde_json` error (never expected for these string arrays).
pub fn journal_to_commodities_value(
    journal: &Journal,
) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(journal_to_commodities(journal))
}

/// An amount's own commodity plus every commodity nested in its cost chain.
fn amount_commodities(amount: &Amount) -> Vec<String> {
    let mut names = vec![amount.commodity.0.clone()];
    if let Some(cost) = &amount.cost {
        names.extend(amount_commodities(&cost.amount));
    }
    names
}

// ===========================================================================
// /prices
// ===========================================================================

/// A serialized `MarketPrice` (hledger's `/prices` element). `mprate` is a bare
/// quantity — no `astyle`, matching the 1.52 snapshot.
#[derive(Debug, Serialize)]
pub struct WireMarketPrice {
    mpdate: String,
    mpfrom: String,
    mprate: WireQuantity,
    mpto: String,
}

/// The `P` price directives in file order as hledger `MarketPrice` records.
#[must_use]
pub fn journal_to_prices(journal: &Journal) -> Vec<WireMarketPrice> {
    journal.prices.iter().map(price_to_wire).collect()
}

/// [`journal_to_prices`] as a JSON array value.
///
/// # Errors
/// Propagates any `serde_json` error (never expected for finite input).
pub fn journal_to_prices_value(journal: &Journal) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(journal_to_prices(journal))
}

fn price_to_wire(price: &PriceDirective) -> WireMarketPrice {
    WireMarketPrice {
        mpdate: price.date.clone(),
        // `P DATE FROM PRICE` prices commodity FROM in the price's commodity.
        mpfrom: price.commodity.0.clone(),
        mprate: WireQuantity {
            decimal_mantissa: price.price.quantity.mantissa,
            decimal_places: price.price.quantity.places,
            floating_point: price.price.quantity.floating_point(),
        },
        mpto: price.price.commodity.0.clone(),
    }
}

// ===========================================================================
// /accounts
// ===========================================================================

/// A serialized account-tree node. The SPA reads only `aname` and
/// `adeclarationinfo.aditags`; the remaining fields are reproduced for wire
/// fidelity except `adata`, whose real balances are deferred to Phase 3.
#[derive(Debug, Serialize)]
pub struct WireAccount {
    aname: String,
    adeclarationinfo: Option<WireDeclarationInfo>,
    aparent_: String,
    asubs_: Vec<String>,
    asubs: Vec<serde_json::Value>,
    aboring: bool,
    adata: WireAccountData,
}

/// A serialized `account` declaration (hledger's `adeclarationinfo`).
#[derive(Debug, Serialize)]
pub struct WireDeclarationInfo {
    adicomment: String,
    adideclarationorder: u32,
    adisourcepos: WirePos,
    aditags: Vec<(String, String)>,
}

/// A serialized per-account balance tree (`adata`).
#[derive(Debug, Serialize)]
pub struct WireAccountData {
    pdperiods: Vec<(String, WireBalanceData)>,
    pdpre: WireBalanceData,
}

/// A serialized single-period balance bucket.
#[derive(Debug, Serialize)]
pub struct WireBalanceData {
    bdexcludingsubs: Vec<serde_json::Value>,
    bdincludingsubs: Vec<serde_json::Value>,
    bdnumpostings: u32,
}

impl WireBalanceData {
    fn empty() -> Self {
        Self {
            bdexcludingsubs: Vec::new(),
            bdincludingsubs: Vec::new(),
            bdnumpostings: 0,
        }
    }
}

/// TODO(Phase 3): replace this structurally-valid EMPTY balance tree with the
/// real running balances (`bdexcludingsubs`/`bdincludingsubs`/`bdnumpostings`)
/// once the report engine lands. `adata` is deliberately EXCLUDED from the
/// `/accounts` parity test for exactly this reason.
fn empty_account_data() -> WireAccountData {
    WireAccountData {
        pdperiods: vec![(ALL_TIME_PERIOD.to_string(), WireBalanceData::empty())],
        pdpre: WireBalanceData::empty(),
    }
}

/// Borrowed lookups shared across the recursive account-tree walk.
struct AccountContext<'a> {
    /// Parent name (`"root"` for top-level) → lexically-sorted child full names.
    children: &'a BTreeMap<String, Vec<String>>,
    /// Account name → (1-based file-order declaration index, its declaration).
    declared: &'a HashMap<&'a str, (u32, &'a AccountDeclaration)>,
    /// Absolute journal path recorded in `adisourcepos.sourceName`.
    source_name: &'a str,
}

/// The `/accounts` tree: one node per posting-referenced account plus all of its
/// ancestors, plus the synthetic `root`, in a pre-order depth-first walk (root
/// first, each account before its lexically-sorted children).
///
/// This set matches hledger's `/accounts` — it is built from postings only, so a
/// declared-but-never-posted leaf is absent here even though `/accountnames`
/// still lists it.
#[must_use]
pub fn journal_to_accounts(journal: &Journal) -> Vec<WireAccount> {
    let names: BTreeSet<String> = journal
        .transactions
        .iter()
        .flat_map(|transaction| &transaction.postings)
        .flat_map(|posting| posting.account.self_and_ancestors())
        .collect();

    // Iterating `names` in lexical order keeps each child list sorted.
    let children = names
        .iter()
        .fold(BTreeMap::<String, Vec<String>>::new(), |mut map, name| {
            map.entry(parent_name(name)).or_default().push(name.clone());
            map
        });

    let declared: HashMap<&str, (u32, &AccountDeclaration)> = journal
        .accounts
        .iter()
        .enumerate()
        .map(|(index, declaration)| (declaration.name.0.as_str(), (to_order(index), declaration)))
        .collect();

    let context = AccountContext {
        children: &children,
        declared: &declared,
        source_name: &journal.source_name,
    };
    walk_account(ACCOUNT_ROOT, &context)
}

/// [`journal_to_accounts`] as a JSON array value.
///
/// # Errors
/// Propagates any `serde_json` error (never expected for finite input).
pub fn journal_to_accounts_value(
    journal: &Journal,
) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(journal_to_accounts(journal))
}

/// Pre-order DFS: this account's node followed by its children's subtrees.
fn walk_account(name: &str, context: &AccountContext) -> Vec<WireAccount> {
    let descendants = context
        .children
        .get(name)
        .into_iter()
        .flatten()
        .flat_map(|child| walk_account(child, context));
    std::iter::once(build_account_node(name, context))
        .chain(descendants)
        .collect()
}

fn build_account_node(name: &str, context: &AccountContext) -> WireAccount {
    let is_root = name == ACCOUNT_ROOT;
    let declaration = if is_root {
        None
    } else {
        context
            .declared
            .get(name)
            .map(|(order, declaration)| WireDeclarationInfo {
                adicomment: declaration.comment.clone(),
                adideclarationorder: *order,
                adisourcepos: pos_to_wire(declaration.position, context.source_name),
                aditags: declaration.tags.clone(),
            })
    };
    WireAccount {
        aname: name.to_string(),
        adeclarationinfo: declaration,
        aparent_: if is_root {
            String::new()
        } else {
            parent_name(name)
        },
        asubs_: context.children.get(name).cloned().unwrap_or_default(),
        asubs: Vec::new(),
        aboring: false,
        adata: empty_account_data(),
    }
}

/// The parent of a colon-delimited account name, or `"root"` for a top-level
/// account.
fn parent_name(name: &str) -> String {
    match name.rsplit_once(':') {
        Some((parent, _)) => parent.to_string(),
        None => ACCOUNT_ROOT.to_string(),
    }
}

/// A 0-based declaration index as hledger's 1-based `adideclarationorder`.
fn to_order(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}
