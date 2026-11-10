//! Transfer matching: find the two halves of a transfer on a clearing account
//! and link them with a posting tag (`plans/35-transfer-matching.md`).
//!
//! Each side of a transfer is imported from its own bank feed and lands on a
//! **clearing account** — one whose inherited account tags include
//! `clearing` (D2). This module:
//!
//! - lists the **candidates** (D3): real, written clearing postings with one
//!   plain amount and no `match` tag, inside the lookback;
//! - **proposes** pairs (D4, D6, D7): same clearing account, same commodity,
//!   exactly opposite amounts, effective dates within the window, not
//!   rejected; assigned by smallest date gap across the whole set, with the
//!   score breaking ties; then **suggested combinations** for what is left
//!   (Amendment A4): one posting and the unique 2–4 opposite postings that
//!   sum to its negation;
//! - lists what one transaction's clearing postings could link to
//!   ([`partners_for`], A2);
//! - indexes existing **links** (`match:ID` on two or more postings, A3) and
//!   the broken ones, and **rejections** (`nomatch:ID` shared by two
//!   postings, D12);
//! - builds the [`EditOp`]s that link, reject, unlink, forget a rejection or
//!   ignore a posting, so every caller writes exactly the same tags.
//!
//! Nothing here writes a file. The ops go through
//! [`crate::edit::JournalEditor::apply_batch`].

use crate::decimal::Dec;
use crate::edit::EditOp;
use crate::model::{Amount, Journal, Posting, PostingType, Tindex, Transaction};
use crate::projections::is_generated;
use crate::reports::periods::days_between;
use crate::reports::{AccountType, AccountTypes, account_decls};
use crate::wire::account_tag_map;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, RandomState};
use thiserror::Error;

/// The account directive tag that marks a clearing account (D2).
pub const CLEARING_TAG: &str = "clearing";
/// The posting tag that links two halves (D1).
pub const MATCH_TAG: &str = "match";
/// The posting tag that remembers a rejected pair (D12).
pub const NOMATCH_TAG: &str = "nomatch";
/// The `match` value that takes a posting out of matching for good (D1).
pub const IGNORE_VALUE: &str = "ignore";

/// Default match window, in days (D8).
pub const DEFAULT_WINDOW_DAYS: u32 = 10;
/// Default lookback, in days (D8). `0` means no limit.
pub const DEFAULT_LOOKBACK_DAYS: u32 = 365;
/// Default age, in days, after which an unmatched candidate is "in transit"
/// (D8, D10).
pub const DEFAULT_IN_TRANSIT_DAYS: u32 = 10;

/// The length of a generated `match`/`nomatch` id (D1).
const ID_LEN: usize = 6;
const ID_ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// What the matcher is asked for (D8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSettings {
    /// The most days two halves' effective dates may be apart.
    pub window_days: u32,
    /// Only postings dated within this many days before `today` are
    /// proposed. `0` means no limit.
    pub lookback_days: u32,
    /// When `Some`, keep only the proposals with at least one half in one of
    /// these transactions (the import entry point's "pairs involving new
    /// postings"). Assignment still runs over every candidate first, so a new
    /// posting never takes a partner a better pair would have had.
    pub involving: Option<HashSet<Tindex>>,
}

impl Default for MatchSettings {
    fn default() -> Self {
        Self {
            window_days: DEFAULT_WINDOW_DAYS,
            lookback_days: DEFAULT_LOOKBACK_DAYS,
            involving: None,
        }
    }
}

/// One posting: its transaction (by position in `journal.transactions` and by
/// `tindex`) and its 0-based position among the transaction's postings — the
/// WRITTEN-posting position the edit ops take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PostingRef {
    /// 0-based position in `journal.transactions`.
    pub position: usize,
    /// The transaction's `tindex`.
    pub txn: Tindex,
    /// 0-based posting position.
    pub posting: usize,
}

impl PostingRef {
    fn sort_key(&self) -> (usize, usize) {
        (self.position, self.posting)
    }
}

/// A candidate posting (D3).
#[derive(Debug, Clone)]
pub struct Candidate<'j> {
    /// Where it is.
    pub at: PostingRef,
    /// Its clearing account.
    pub account: &'j str,
    /// Its effective date: the posting's `date:`, else the transaction's.
    pub date: &'j str,
    /// Its one amount.
    pub amount: &'j Amount,
}

/// A proposed pair (D6, D7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// The earlier half (by effective date, then file order).
    pub a: PostingRef,
    /// The later half.
    pub b: PostingRef,
    /// Days between the halves' effective dates.
    pub gap_days: u32,
    /// The D7 score: a tie-breaker and the confidence label's input.
    pub score: i32,
    /// Unambiguous with a non-negative score: eligible for "Link all clear".
    pub clear: bool,
    /// One half had another legal partner with the same gap and score.
    pub ambiguous: bool,
    /// Every other legal pair that shares a half with this one, nearest
    /// first.
    pub alternatives: Vec<Alternative>,
    /// Each half's real (non-clearing) account, `[a, b]`, for display.
    pub real_accounts: [Option<String>; 2],
    /// Empty for a pair. For a **suggested combination** (Amendment A4): the
    /// many side, 2–4 postings in different transactions whose amounts sum
    /// to exactly `a`'s negation, nearest first. Then `a` is the single
    /// posting (not necessarily the earlier), `b` is `group[0]`, `gap_days`
    /// the widest gap between `a` and a member, and the proposal is never
    /// `clear` (so never in "Link all").
    pub group: Vec<PostingRef>,
}

impl Proposal {
    /// Whether this is a suggested combination rather than a pair.
    #[must_use]
    pub fn is_combination(&self) -> bool {
        !self.group.is_empty()
    }

    /// Every posting the proposal would link: `a` and `b` for a pair, `a`
    /// and the whole group for a combination.
    pub fn postings(&self) -> impl Iterator<Item = PostingRef> + '_ {
        let rest: &[PostingRef] = if self.group.is_empty() {
            std::slice::from_ref(&self.b)
        } else {
            &self.group
        };
        std::iter::once(self.a).chain(rest.iter().copied())
    }
}

/// Another legal pair sharing a half with a [`Proposal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alternative {
    /// The earlier half.
    pub a: PostingRef,
    /// The later half.
    pub b: PostingRef,
    /// Days between the halves.
    pub gap_days: u32,
    /// The D7 score.
    pub score: i32,
    /// The half this alternative does NOT share with the proposal is already
    /// in another proposal.
    pub taken: bool,
}

/// Why a `match` id is not a valid link (D5, D10, A3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokenReason {
    /// The id is on this many postings: fewer than two.
    Count(usize),
    /// A posting is not on a clearing account.
    NotClearing,
    /// The postings are on more than one clearing account.
    DifferentAccounts,
    /// Two of the postings are in one transaction.
    SameTransaction,
    /// The amounts are not single plain amounts of one commodity summing to
    /// exactly zero.
    DoNotCancel,
}

/// A `match` id that is not a valid link: N ≥ 2 clearing postings on one
/// account, in N transactions, whose amounts sum to exactly zero (A3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenLink {
    /// The id.
    pub id: String,
    /// Every posting carrying it, in file order.
    pub postings: Vec<PostingRef>,
    /// What is wrong.
    pub reason: BrokenReason,
}

/// Every `match` id in a journal: the valid links (pairs, and groups of
/// three or more, A3) and the broken ones.
#[derive(Debug, Clone, Default)]
pub struct LinkIndex {
    pairs: BTreeMap<String, (PostingRef, PostingRef)>,
    groups: BTreeMap<String, Vec<PostingRef>>,
    broken: Vec<BrokenLink>,
    by_position: HashMap<usize, Vec<String>>,
}

impl LinkIndex {
    /// The valid two-posting links, by id, each pair in file order.
    #[must_use]
    pub fn pairs(&self) -> &BTreeMap<String, (PostingRef, PostingRef)> {
        &self.pairs
    }

    /// The valid links of three or more postings (A3), by id, each in file
    /// order. They never collapse into a transfer row.
    #[must_use]
    pub fn groups(&self) -> &BTreeMap<String, Vec<PostingRef>> {
        &self.groups
    }

    /// The valid two-posting link with `id`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<(PostingRef, PostingRef)> {
        self.pairs.get(id).copied()
    }

    /// Every posting of the valid link `id`, pair or group, in file order.
    #[must_use]
    pub fn members(&self, id: &str) -> Option<Vec<PostingRef>> {
        self.pairs
            .get(id)
            .map(|&(a, b)| vec![a, b])
            .or_else(|| self.groups.get(id).cloned())
    }

    /// The broken ids, in id order.
    #[must_use]
    pub fn broken(&self) -> &[BrokenLink] {
        &self.broken
    }

    /// The ids of the valid links with a half in the transaction at
    /// `position`, in id order. A transaction in more than one link is not
    /// collapsed into a transfer row (plan 35 Risks).
    #[must_use]
    pub fn links_at(&self, position: usize) -> &[String] {
        self.by_position.get(&position).map_or(&[], Vec::as_slice)
    }

    /// How many valid links have a half in the transaction at `position`.
    #[must_use]
    pub fn link_count(&self, position: usize) -> usize {
        self.links_at(position).len()
    }
}

/// Every `nomatch` value in a journal and the postings carrying it (D12).
#[derive(Debug, Clone, Default)]
pub struct Rejections {
    by_value: BTreeMap<String, Vec<PostingRef>>,
}

impl Rejections {
    /// Every value with the postings that carry it, in value order.
    pub fn values(&self) -> impl Iterator<Item = (&str, &[PostingRef])> {
        self.by_value
            .iter()
            .map(|(value, postings)| (value.as_str(), postings.as_slice()))
    }

    /// The rejected pairs: the values on exactly two postings. A value on one
    /// posting is a stale rejection; it is harmless and not listed here.
    pub fn pairs(&self) -> impl Iterator<Item = (&str, PostingRef, PostingRef)> {
        self.values()
            .filter_map(|(value, postings)| match postings {
                [a, b] => Some((value, *a, *b)),
                _ => None,
            })
    }

    /// The postings carrying `value`.
    #[must_use]
    pub fn postings_of(&self, value: &str) -> &[PostingRef] {
        self.by_value.get(value).map_or(&[], Vec::as_slice)
    }

    /// Whether `a` and `b` share a `nomatch` value.
    #[must_use]
    pub fn is_rejected(&self, a: PostingRef, b: PostingRef) -> bool {
        self.by_value
            .values()
            .any(|postings| postings.contains(&a) && postings.contains(&b))
    }
}

/// Why a link, reject or ignore cannot be built.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MatchError {
    /// No such transaction or posting.
    #[error("posting #{} of transaction #{} not found", .0.posting, .0.txn.0)]
    PostingNotFound(PostingRef),
    /// The posting cannot take part in matching.
    #[error("posting #{} of transaction #{} cannot be matched: {reason}", .at.posting, .at.txn.0)]
    NotMatchable {
        /// The posting.
        at: PostingRef,
        /// Why not.
        reason: String,
    },
    /// The two postings are not a legal pair.
    #[error("these postings cannot be paired: {0}")]
    NotAPair(String),
    /// No posting carries this id.
    #[error("no posting carries `{name}:{value}`")]
    UnknownId {
        /// The tag name.
        name: &'static str,
        /// The value.
        value: String,
    },
    /// The id is not one hledger could read back as a single tag value.
    #[error("`{0}` is not a usable match id")]
    BadId(String),
}

/// The edits one matching action writes: per transaction, in file order, its
/// `tindex` and ops — ready for [`crate::edit::JournalEditor::apply_batch`]
/// once each is paired with its fingerprint.
pub type TxnOps = Vec<(Tindex, Vec<EditOp>)>;

// ---------------------------------------------------------------------------
// Clearing accounts and candidates
// ---------------------------------------------------------------------------

/// Every account — declared or posted to — whose inherited account tags
/// include `clearing` (D2). Subaccounts inherit the tag, as in hledger (the
/// ancestor walk `wire::inherited_account_tags` does), never only the
/// exact-name [`Journal::account_tags`].
#[must_use]
pub fn clearing_accounts(journal: &Journal) -> BTreeSet<String> {
    // Inherited tags are the account's own declared tags plus each
    // ancestor's (`inherited_account_tags`), so an account is a clearing
    // account exactly when it, or an ancestor, declares `clearing` itself.
    // Testing names against those few roots by prefix, rather than walking
    // each posting's ancestors through a hash set, keeps this a plain scan
    // (~30 ms → a few ms on the 200k corpus).
    let tags = account_tag_map(&journal.accounts);
    let roots: Vec<&str> = tags
        .iter()
        .filter(|(_, declared)| declared.iter().any(|(name, _)| name == CLEARING_TAG))
        .map(|(name, _)| *name)
        .collect();
    if roots.is_empty() {
        return BTreeSet::new();
    }
    let under_a_root = |account: &str| {
        roots.iter().any(|root| {
            account
                .strip_prefix(root)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
        })
    };
    let names: BTreeSet<&str> = journal
        .accounts
        .iter()
        .map(|decl| decl.name.0.as_str())
        .chain(journal.transactions.iter().flat_map(|txn| {
            txn.postings
                .iter()
                .map(|posting| posting.account.0.as_str())
        }))
        .filter(|account| under_a_root(account))
        .collect();
    names.into_iter().map(str::to_string).collect()
}

/// Whether any account directive declares the `clearing` tag. With none,
/// Find Matches is disabled (D2).
#[must_use]
pub fn has_clearing_accounts(journal: &Journal) -> bool {
    journal
        .accounts
        .iter()
        .any(|decl| decl.tags.iter().any(|(name, _)| name == CLEARING_TAG))
}

/// A posting's effective date: its own `date:`, else its transaction's.
fn effective_date<'j>(txn: &'j Transaction, posting: &'j Posting) -> &'j str {
    posting.date.as_deref().unwrap_or(&txn.date)
}

/// Days since 1970-01-01, for window and lookback arithmetic.
fn day_number(date: &str) -> i64 {
    days_between("1970-01-01", date)
}

/// The written, real postings of the journal's real transactions, with
/// where each is. Forecast transactions and `=`-rule postings are nobody's
/// to match.
fn real_postings(journal: &Journal) -> impl Iterator<Item = (PostingRef, &Transaction, &Posting)> {
    journal
        .transactions
        .iter()
        .enumerate()
        .filter(|(_, txn)| !is_generated(txn))
        .flat_map(|(position, txn)| {
            txn.postings
                .iter()
                .enumerate()
                .filter(|(_, posting)| {
                    posting.generated.is_none() && posting.ptype == PostingType::Regular
                })
                .map(move |(at, posting)| {
                    (
                        PostingRef {
                            position,
                            txn: txn.index,
                            posting: at,
                        },
                        txn,
                        posting,
                    )
                })
        })
}

fn tag_values<'p>(posting: &'p Posting, name: &'p str) -> impl Iterator<Item = &'p str> {
    posting
        .tags
        .iter()
        .filter(move |(tag, _)| tag == name)
        .map(|(_, value)| value.as_str())
}

/// Why `posting` (on a clearing account) can never be paired, or `None` when
/// it can: D3 less the lookback.
fn unmatchable(posting: &Posting) -> Option<&'static str> {
    if tag_values(posting, MATCH_TAG).next().is_some() {
        return Some("it already has a `match` tag");
    }
    match posting.amounts.as_slice() {
        [amount] if amount.cost.is_some() => Some("it has a cost"),
        [amount] if amount.quantity.mantissa == 0 => Some("its amount is zero"),
        [_] if posting.balance_assertion.is_some() => Some("it has a balance assertion"),
        [_] => None,
        _ => Some("it does not have exactly one amount"),
    }
}

fn candidates_in<'j>(
    journal: &'j Journal,
    clearing: &BTreeSet<String>,
    lookback_days: u32,
    today: &str,
) -> Vec<Candidate<'j>> {
    if clearing.is_empty() {
        return Vec::new();
    }
    let cutoff = (lookback_days > 0).then(|| day_number(today) - i64::from(lookback_days));
    real_postings(journal)
        .filter(|(_, _, posting)| clearing.contains(&posting.account.0))
        .filter(|(_, _, posting)| unmatchable(posting).is_none())
        .map(|(at, txn, posting)| Candidate {
            at,
            account: &posting.account.0,
            date: effective_date(txn, posting),
            amount: &posting.amounts[0],
        })
        .filter(|candidate| cutoff.is_none_or(|cutoff| day_number(candidate.date) >= cutoff))
        .collect()
}

/// The candidate postings (D3), in file order: real written postings on a
/// clearing account, with no `match` tag (`match:ignore` included), exactly
/// one amount with no cost, no balance assertion, a non-zero quantity, and an
/// effective date inside `settings.lookback_days` before `today`.
#[must_use]
pub fn candidates<'j>(
    journal: &'j Journal,
    settings: &MatchSettings,
    today: &str,
) -> Vec<Candidate<'j>> {
    candidates_in(
        journal,
        &clearing_accounts(journal),
        settings.lookback_days,
        today,
    )
}

/// The clearing postings that can never be matched because they carry a cost
/// or a balance assertion (D10 "Unusual clearing posting"). A posting marked
/// `match:ignore` is the user's to keep and is not listed.
#[must_use]
pub fn unusual_clearing_postings(journal: &Journal) -> Vec<PostingRef> {
    let clearing = clearing_accounts(journal);
    if clearing.is_empty() {
        return Vec::new();
    }
    real_postings(journal)
        .filter(|(_, _, posting)| clearing.contains(&posting.account.0))
        .filter(|(_, _, posting)| tag_values(posting, MATCH_TAG).all(|value| value != IGNORE_VALUE))
        .filter(|(_, _, posting)| {
            posting.balance_assertion.is_some()
                || posting.amounts.iter().any(|amount| amount.cost.is_some())
        })
        .map(|(at, _, _)| at)
        .collect()
}

// ---------------------------------------------------------------------------
// Proposals
// ---------------------------------------------------------------------------

/// A legal pair, by candidate index; `a` is the earlier half.
#[derive(Debug, Clone, Copy)]
struct Pair {
    a: usize,
    b: usize,
    gap: u32,
    score: i32,
}

/// The candidates' grouping key: clearing account, commodity, magnitude.
type Bucket<'j> = (&'j str, &'j str, u128, u32);

fn bucket_of<'j>(candidate: &Candidate<'j>) -> Bucket<'j> {
    let quantity: Dec = candidate.amount.quantity.normalized();
    (
        candidate.account,
        &candidate.amount.commodity.0,
        quantity.mantissa.unsigned_abs(),
        quantity.places,
    )
}

/// What the scorer needs to know about the journal, built once.
struct Scorer<'j> {
    journal: &'j Journal,
    clearing: &'j BTreeSet<String>,
    types: AccountTypes,
}

impl<'j> Scorer<'j> {
    fn transaction(&self, at: PostingRef) -> &'j Transaction {
        &self.journal.transactions[at.position]
    }

    fn real_account(&self, at: PostingRef) -> Option<&'j str> {
        real_account(self.journal, self.clearing, at)
    }

    /// Whether `description` names `account`: its last segment, or the
    /// left-hand side of a plain `alias` that maps to it, case-insensitively.
    fn names(&self, description: &str, account: &str) -> bool {
        let description = description.to_lowercase();
        let mentions = |word: &str| {
            let word = word.trim().to_lowercase();
            word.chars().count() >= 3 && description.contains(&word)
        };
        let last = account.rsplit(':').next().unwrap_or(account);
        mentions(last)
            || self.journal.aliases.iter().any(|alias| {
                !alias.regex
                    && alias.replacement == account
                    && mentions(alias.pattern.rsplit(':').next().unwrap_or(&alias.pattern))
            })
    }

    /// The D7 score of pairing `a` with `b`.
    fn score(&self, a: PostingRef, b: PostingRef) -> i32 {
        let accounts = [self.real_account(a), self.real_account(b)];
        let is = |account: &str, category| self.types.is_type(account, category);
        let mut score = 0;
        if let [Some(left), Some(right)] = accounts {
            if left == right {
                score -= 1;
            } else if [left, right].iter().all(|account| {
                is(account, AccountType::Asset) || is(account, AccountType::Liability)
            }) {
                score += 1;
            }
        }
        if accounts
            .iter()
            .flatten()
            .any(|account| is(account, AccountType::Expense) || is(account, AccountType::Revenue))
        {
            score -= 1;
        }
        let named = |own: PostingRef, other: Option<&str>| {
            other.is_some_and(|other| self.names(&self.transaction(own).description, other))
        };
        if named(a, accounts[1]) || named(b, accounts[0]) {
            score += 1;
        }
        score
    }
}

/// The real account on the other side of the clearing posting `at`: the
/// non-clearing real posting that exactly offsets it, else the first
/// non-clearing real posting; `None` when the transaction has neither.
#[must_use]
pub fn real_account<'j>(
    journal: &'j Journal,
    clearing: &BTreeSet<String>,
    at: PostingRef,
) -> Option<&'j str> {
    let txn = journal.transactions.get(at.position)?;
    let own = txn.postings.get(at.posting)?.amounts.first();
    let others: Vec<&Posting> = txn
        .postings
        .iter()
        .enumerate()
        .filter(|(index, posting)| {
            *index != at.posting
                && posting.ptype == PostingType::Regular
                && !clearing.contains(&posting.account.0)
        })
        .map(|(_, posting)| posting)
        .collect();
    let offsets = |posting: &&&Posting| match (own, posting.amounts.as_slice()) {
        (Some(own), [amount]) => {
            amount.commodity == own.commodity
                && own
                    .quantity
                    .neg()
                    .is_ok_and(|negated| negated == amount.quantity)
        }
        _ => false,
    };
    others
        .iter()
        .find(offsets)
        .or_else(|| others.first())
        .map(|posting| posting.account.0.as_str())
}

/// `[source, destination]` of the pair `a`/`b` (D11 "Direction"): the source
/// is the half whose real posting is negative, so its clearing posting is the
/// positive one. A pair whose clearing amounts are not one positive and one
/// negative keeps its given order.
#[must_use]
pub fn direction(journal: &Journal, a: PostingRef, b: PostingRef) -> [PostingRef; 2] {
    let positive = |at: PostingRef| {
        posting_at(journal, at)
            .and_then(|posting| posting.amounts.first())
            .is_some_and(|amount| amount.quantity.mantissa > 0)
    };
    if positive(b) && !positive(a) {
        [b, a]
    } else {
        [a, b]
    }
}

/// Short names for accounts in the "Checking → Amex" labels (D9, D11).
///
/// Each account is named by its shortest run of trailing segments that no
/// other account in the journal ends with. A lone `assets:bank:checking`
/// reads `checking`; with `assets:bank1:checking` beside it, both read
/// `bank1:checking` / `bank2:checking`. When even the full name is a suffix
/// of another (`checking` next to `assets:checking`), the full name is used.
/// Built once per journal; every declared account and every posted one counts.
#[derive(Debug, Clone, Default)]
pub struct AccountLabels(HashMap<String, String>);

/// `account` and each shorter run of its trailing segments, longest first.
fn suffixes(account: &str) -> impl Iterator<Item = &str> {
    std::iter::once(account).chain(
        account
            .match_indices(':')
            .map(move |(at, _)| &account[at + 1..]),
    )
}

impl AccountLabels {
    /// The labels for every account `journal` declares or posts to.
    #[must_use]
    pub fn of(journal: &Journal) -> Self {
        let names: BTreeSet<&str> = journal
            .accounts
            .iter()
            .map(|decl| decl.name.0.as_str())
            .chain(journal.transactions.iter().flat_map(|txn| {
                txn.postings
                    .iter()
                    .map(|posting| posting.account.0.as_str())
            }))
            .collect();
        let counts = names.iter().flat_map(|name| suffixes(name)).fold(
            HashMap::<&str, usize>::new(),
            |mut counts, suffix| {
                *counts.entry(suffix).or_default() += 1;
                counts
            },
        );
        Self(
            names
                .iter()
                .map(|name| {
                    let shortest = suffixes(name)
                        .filter(|suffix| counts.get(suffix) == Some(&1))
                        .last()
                        .unwrap_or(name);
                    ((*name).to_string(), shortest.to_string())
                })
                .collect(),
        )
    }

    /// `account`'s label: its shortest unique suffix, or its last segment for
    /// an account this journal has never seen.
    #[must_use]
    pub fn label(&self, account: &str) -> String {
        self.0
            .get(account)
            .cloned()
            .unwrap_or_else(|| account.rsplit(':').next().unwrap_or(account).to_string())
    }

    /// The labels for a transfer's two real accounts, source first. Two
    /// labels that would still read the same (one account on both sides)
    /// fall back to the full names.
    #[must_use]
    pub fn pair(&self, from: Option<&str>, to: Option<&str>) -> [Option<String>; 2] {
        let labels = [from.map(|f| self.label(f)), to.map(|t| self.label(t))];
        match (from, to, &labels) {
            (Some(from), Some(to), [Some(a), Some(b)]) if a == b => {
                [Some(from.to_string()), Some(to.to_string())]
            }
            _ => labels,
        }
    }
}

/// Every legal pair among `candidates` (D4), `a` the earlier half.
fn legal_pairs(
    candidates: &[Candidate<'_>],
    days: &[i64],
    nomatch: &[Vec<&str>],
    window_days: u32,
    scorer: &Scorer<'_>,
) -> Vec<Pair> {
    // Group by (account, commodity, |qty|), split by sign; the negatives of
    // each bucket sorted by day so each positive scans only its window.
    let mut buckets: HashMap<Bucket<'_>, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let (positive, negative) = buckets.entry(bucket_of(candidate)).or_default();
        if candidate.amount.quantity.mantissa > 0 {
            positive.push(index);
        } else {
            negative.push(index);
        }
    }
    let window = i64::from(window_days);
    let mut pairs = Vec::new();
    for (positive, mut negative) in buckets.into_values() {
        negative.sort_by_key(|&index| days[index]);
        for &p in &positive {
            let from = negative.partition_point(|&n| days[n] < days[p] - window);
            for &n in negative[from..]
                .iter()
                .take_while(|&&n| days[n] <= days[p] + window)
            {
                if candidates[p].at.position == candidates[n].at.position
                    || shares_a_value(&nomatch[p], &nomatch[n])
                {
                    continue;
                }
                let (a, b) = if (days[p], candidates[p].at.sort_key())
                    <= (days[n], candidates[n].at.sort_key())
                {
                    (p, n)
                } else {
                    (n, p)
                };
                let gap = u32::try_from((days[p] - days[n]).unsigned_abs()).unwrap_or(u32::MAX);
                pairs.push(Pair {
                    a,
                    b,
                    gap,
                    score: scorer.score(candidates[a].at, candidates[b].at),
                });
            }
        }
    }
    pairs
}

/// The smallest and largest many side of a suggested combination (A4).
const COMBINATION_SIZES: std::ops::RangeInclusive<usize> = 2..=4;
/// How many opposite-sign postings, nearest first, a combination is sought
/// among (A4). `C(20,2) + C(20,3) + C(20,4)` ≈ 6k subsets at most, pruned by
/// amount.
const COMBINATION_POOL: usize = 20;

/// A suggested combination, by candidate index (A4).
#[derive(Debug, Clone)]
struct Combination {
    single: usize,
    /// Nearest first.
    members: Vec<usize>,
    /// The widest gap between `single` and a member.
    gap: u32,
}

/// The one-to-one assignment (D6) and the combinations (A4) over one set of
/// candidates: what [`propose_in`] reports and [`count_proposals`] counts.
struct Assignment<'j> {
    candidates: Vec<Candidate<'j>>,
    pairs: Vec<Pair>,
    /// Indices into `pairs`, in assignment order.
    chosen: Vec<usize>,
    /// Every candidate in a chosen pair.
    taken: Vec<bool>,
    /// Per candidate, every pair touching it, in sorted order.
    touching: Vec<Vec<usize>>,
    combinations: Vec<Combination>,
}

impl Assignment<'_> {
    fn involved(&self, settings: &MatchSettings, mut indices: impl Iterator<Item = usize>) -> bool {
        settings
            .involving
            .as_ref()
            .is_none_or(|txns| indices.any(|index| txns.contains(&self.candidates[index].at.txn)))
    }

    fn pair_involved(&self, settings: &MatchSettings, pair: &Pair) -> bool {
        self.involved(settings, [pair.a, pair.b].into_iter())
    }

    fn combination_involved(&self, settings: &MatchSettings, combination: &Combination) -> bool {
        self.involved(
            settings,
            std::iter::once(combination.single).chain(combination.members.iter().copied()),
        )
    }
}

/// Each candidate's `nomatch` values (D12).
fn nomatch_values<'j>(journal: &'j Journal, candidates: &[Candidate<'j>]) -> Vec<Vec<&'j str>> {
    candidates
        .iter()
        .map(|c| {
            let txn = &journal.transactions[c.at.position];
            tag_values(&txn.postings[c.at.posting], NOMATCH_TAG).collect()
        })
        .collect()
}

fn shares_a_value(x: &[&str], y: &[&str]) -> bool {
    x.iter().any(|value| y.contains(value))
}

/// `quantity` as an integer count of `10^-places`, or `None` when it has more
/// places or overflows.
fn scaled(quantity: Dec, places: u32) -> Option<i128> {
    let extra = places.checked_sub(quantity.places)?;
    10_i128
        .checked_pow(extra)
        .and_then(|factor| quantity.mantissa.checked_mul(factor))
}

/// The subsets of `values` (magnitudes, sorted ascending, all positive) of
/// [`COMBINATION_SIZES`] that sum to `target`, members in different
/// transactions (`txns`): at most two are collected, since a second one
/// already makes the combination a guess.
fn subsets_summing_to(values: &[i128], txns: &[usize], target: i128) -> Vec<Vec<usize>> {
    fn walk(
        values: &[i128],
        txns: &[usize],
        target: i128,
        from: usize,
        chosen: &mut Vec<usize>,
        sum: i128,
        found: &mut Vec<Vec<usize>>,
    ) {
        if found.len() >= 2 {
            return;
        }
        if sum == target && COMBINATION_SIZES.contains(&chosen.len()) {
            found.push(chosen.clone());
            return;
        }
        if chosen.len() == *COMBINATION_SIZES.end() {
            return;
        }
        for index in from..values.len() {
            let next = sum + values[index];
            if next > target {
                // Ascending: every later value overshoots too.
                break;
            }
            if chosen.iter().any(|&other| txns[other] == txns[index]) {
                continue;
            }
            chosen.push(index);
            walk(values, txns, target, index + 1, chosen, next, found);
            chosen.pop();
            if found.len() >= 2 {
                return;
            }
        }
    }
    let mut found = Vec::new();
    walk(values, txns, target, 0, &mut Vec::new(), 0, &mut found);
    found
}

/// A4: for each candidate the one-to-one pass left without a partner, the
/// one subset (2–4) of the nearest [`COMBINATION_POOL`] opposite-sign
/// untaken candidates on its account and commodity, within the window, that
/// sums to exactly its negation. A posting with two or more such subsets
/// gets none (it would be a guess); a subset holding a posting the single
/// one shares a `nomatch` value with is not one. The survivors are then
/// taken by widest gap, smallest first, so no posting is in two.
fn combinations(
    candidates: &[Candidate<'_>],
    days: &[i64],
    nomatch: &[Vec<&str>],
    taken: &[bool],
    window_days: u32,
) -> Vec<Combination> {
    // The untaken candidates by (account, commodity, positive), by day.
    let mut sides: HashMap<(&str, &str, bool), Vec<usize>> = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if !taken[index] {
            sides
                .entry((
                    candidate.account,
                    &candidate.amount.commodity.0,
                    candidate.amount.quantity.mantissa > 0,
                ))
                .or_default()
                .push(index);
        }
    }
    for side in sides.values_mut() {
        side.sort_by_key(|&index| (days[index], candidates[index].at.sort_key()));
    }
    let window = u64::from(window_days);
    let gap = |x: usize, y: usize| (days[x] - days[y]).unsigned_abs();
    let mut found: Vec<Combination> = Vec::new();
    for (&(account, commodity, positive), singles) in &sides {
        let Some(others) = sides.get(&(account, commodity, !positive)) else {
            continue;
        };
        for &single in singles {
            let own = &candidates[single];
            // The nearest untaken opposite postings in the window: walk out
            // from the single's day on both sides, nearer first.
            let mid = others.partition_point(|&n| days[n] < days[single]);
            let (mut left, mut right) = (mid, mid);
            let mut pool: Vec<usize> = Vec::with_capacity(COMBINATION_POOL);
            while pool.len() < COMBINATION_POOL {
                let before = left.checked_sub(1).map(|at| others[at]);
                let after = others.get(right).copied();
                let next = match (before, after) {
                    (Some(b), Some(a)) => {
                        if (gap(single, b), candidates[b].at.sort_key())
                            <= (gap(single, a), candidates[a].at.sort_key())
                        {
                            left -= 1;
                            b
                        } else {
                            right += 1;
                            a
                        }
                    }
                    (Some(b), None) => {
                        left -= 1;
                        b
                    }
                    (None, Some(a)) => {
                        right += 1;
                        a
                    }
                    (None, None) => break,
                };
                if gap(single, next) > window {
                    // `next` is the nearer of the two fronts: both are out.
                    break;
                }
                if candidates[next].at.position != own.at.position
                    && !shares_a_value(&nomatch[single], &nomatch[next])
                {
                    pool.push(next);
                }
            }
            if pool.len() < *COMBINATION_SIZES.start() {
                continue;
            }
            let places = pool
                .iter()
                .map(|&index| candidates[index].amount.quantity.places)
                .chain([own.amount.quantity.places])
                .max()
                .unwrap_or(0);
            let Some(target) = scaled(own.amount.quantity, places).and_then(i128::checked_abs)
            else {
                continue;
            };
            let mut magnitudes: Vec<(i128, usize)> = pool
                .iter()
                .filter_map(|&index| {
                    let value = scaled(candidates[index].amount.quantity, places)?.checked_abs()?;
                    Some((value, index))
                })
                .collect();
            if magnitudes.len() != pool.len() {
                continue;
            }
            magnitudes.sort_unstable();
            let values: Vec<i128> = magnitudes.iter().map(|(value, _)| *value).collect();
            let txns: Vec<usize> = magnitudes
                .iter()
                .map(|(_, index)| candidates[*index].at.position)
                .collect();
            let subsets = subsets_summing_to(&values, &txns, target);
            let [subset] = subsets.as_slice() else {
                continue;
            };
            let mut members: Vec<usize> = subset.iter().map(|&at| magnitudes[at].1).collect();
            members.sort_by_key(|&member| (gap(single, member), candidates[member].at.sort_key()));
            let widest = members
                .iter()
                .map(|&member| gap(single, member))
                .max()
                .unwrap_or(0);
            found.push(Combination {
                single,
                members,
                gap: u32::try_from(widest).unwrap_or(u32::MAX),
            });
        }
    }
    found.sort_by_key(|combination| {
        (
            combination.gap,
            candidates[combination.single].at.sort_key(),
        )
    });
    let mut used = vec![false; candidates.len()];
    found
        .into_iter()
        .filter(|combination| {
            let all = || std::iter::once(&combination.single).chain(&combination.members);
            if all().any(|&index| used[index]) {
                return false;
            }
            all().for_each(|&index| used[index] = true);
            true
        })
        .collect()
}

fn assign<'j>(
    journal: &'j Journal,
    clearing: &BTreeSet<String>,
    settings: &MatchSettings,
    today: &str,
    scorer: &Scorer<'_>,
) -> Assignment<'j> {
    let candidates = candidates_in(journal, clearing, settings.lookback_days, today);
    let days: Vec<i64> = candidates.iter().map(|c| day_number(c.date)).collect();
    let nomatch = nomatch_values(journal, &candidates);
    let mut pairs = legal_pairs(&candidates, &days, &nomatch, settings.window_days, scorer);
    let at = |index: usize| candidates[index].at.sort_key();
    pairs.sort_by_key(|pair| (pair.gap, -pair.score, at(pair.a), at(pair.b)));

    // D6: smallest gap first, score breaking ties; first come, first served.
    let mut taken: Vec<bool> = vec![false; candidates.len()];
    let mut chosen: Vec<usize> = Vec::new();
    let mut touching: Vec<Vec<usize>> = vec![Vec::new(); candidates.len()];
    for (index, pair) in pairs.iter().enumerate() {
        touching[pair.a].push(index);
        touching[pair.b].push(index);
        if !taken[pair.a] && !taken[pair.b] {
            taken[pair.a] = true;
            taken[pair.b] = true;
            chosen.push(index);
        }
    }
    let combinations = combinations(&candidates, &days, &nomatch, &taken, settings.window_days);
    Assignment {
        candidates,
        pairs,
        chosen,
        taken,
        touching,
        combinations,
    }
}

fn scorer<'j>(journal: &'j Journal, clearing: &'j BTreeSet<String>) -> Scorer<'j> {
    Scorer {
        journal,
        clearing,
        types: AccountTypes::new(&account_decls(journal)),
    }
}

fn propose_in(
    journal: &Journal,
    clearing: &BTreeSet<String>,
    settings: &MatchSettings,
    today: &str,
) -> Vec<Proposal> {
    let scorer = scorer(journal, clearing);
    let assignment = assign(journal, clearing, settings, today, &scorer);
    let Assignment {
        candidates,
        pairs,
        chosen,
        taken,
        touching,
        combinations,
    } = &assignment;
    let real_accounts = |a: PostingRef, b: PostingRef| {
        [
            scorer.real_account(a).map(str::to_string),
            scorer.real_account(b).map(str::to_string),
        ]
    };
    let mut proposals: Vec<Proposal> = chosen
        .iter()
        .copied()
        .filter(|&index| assignment.pair_involved(settings, &pairs[index]))
        .map(|index| {
            let pair = pairs[index];
            // `touching` lists pairs in sorted order, so merging the two
            // halves' lists by index keeps the alternatives nearest first.
            let mut others: Vec<usize> = touching[pair.a]
                .iter()
                .chain(&touching[pair.b])
                .copied()
                .filter(|&other| other != index)
                .collect();
            others.sort_unstable();
            others.dedup();
            let ambiguous = others
                .iter()
                .any(|&other| pairs[other].gap == pair.gap && pairs[other].score == pair.score);
            let alternatives = others
                .iter()
                .map(|&other| {
                    let alt = pairs[other];
                    let foreign = if alt.a == pair.a || alt.a == pair.b {
                        alt.b
                    } else {
                        alt.a
                    };
                    Alternative {
                        a: candidates[alt.a].at,
                        b: candidates[alt.b].at,
                        gap_days: alt.gap,
                        score: alt.score,
                        taken: taken[foreign],
                    }
                })
                .collect();
            let (a, b) = (candidates[pair.a].at, candidates[pair.b].at);
            Proposal {
                a,
                b,
                gap_days: pair.gap,
                score: pair.score,
                clear: !ambiguous && pair.score >= 0,
                ambiguous,
                alternatives,
                real_accounts: real_accounts(a, b),
                group: Vec::new(),
            }
        })
        .collect();
    proposals.extend(
        combinations
            .iter()
            .filter(|combination| assignment.combination_involved(settings, combination))
            .map(|combination| {
                let a = candidates[combination.single].at;
                let group: Vec<PostingRef> = combination
                    .members
                    .iter()
                    .map(|&member| candidates[member].at)
                    .collect();
                let b = group[0];
                Proposal {
                    a,
                    b,
                    gap_days: combination.gap,
                    score: 0,
                    clear: false,
                    ambiguous: false,
                    alternatives: Vec::new(),
                    real_accounts: real_accounts(a, b),
                    group,
                }
            }),
    );
    // Date order, not file order: with one file per bank feed, file order
    // walks one feed's pairs and then the next's.
    let date_of: HashMap<PostingRef, &str> = candidates.iter().map(|c| (c.at, c.date)).collect();
    proposals.sort_by(|x, y| {
        (date_of.get(&x.a), x.a.sort_key()).cmp(&(date_of.get(&y.a), y.a.sort_key()))
    });
    proposals
}

/// Propose pairs among the candidates (D4, D6, D7), then suggested
/// combinations (A4), ordered by `a`'s effective date, then its file
/// position.
///
/// Every legal pair is built (same clearing account, same commodity, exactly
/// opposite amounts, effective dates at most `window_days` apart, different
/// transactions, no shared `nomatch` value), sorted by `(gap, -score)`, and
/// taken in that order unless a half is already taken. Each candidate left
/// over may then get a combination ([`Proposal::group`]). No posting is in
/// two proposals.
#[must_use]
pub fn propose(journal: &Journal, settings: &MatchSettings, today: &str) -> Vec<Proposal> {
    propose_in(journal, &clearing_accounts(journal), settings, today)
}

/// How many proposals [`propose`] would return, without building them (no
/// alternatives, no accounts): `GET /api/matches`' `older` count runs the
/// matcher a second time with no lookback (A1).
#[must_use]
pub fn count_proposals(journal: &Journal, settings: &MatchSettings, today: &str) -> usize {
    let clearing = clearing_accounts(journal);
    let scorer = scorer(journal, &clearing);
    let assignment = assign(journal, &clearing, settings, today, &scorer);
    let pairs = assignment
        .chosen
        .iter()
        .filter(|&&index| assignment.pair_involved(settings, &assignment.pairs[index]))
        .count();
    let combinations = assignment
        .combinations
        .iter()
        .filter(|combination| assignment.combination_involved(settings, combination))
        .count();
    pairs + combinations
}

/// The candidates (no lookback) older than `flag_days` before `today` that
/// [`propose`] finds no partner for — neither a pair nor a suggested
/// combination (A5) — with no lookback limit either (D10 "In transit"). In
/// file order.
#[must_use]
pub fn in_transit<'j>(
    journal: &'j Journal,
    window_days: u32,
    flag_days: u32,
    today: &str,
) -> Vec<Candidate<'j>> {
    let clearing = clearing_accounts(journal);
    let settings = MatchSettings {
        window_days,
        lookback_days: 0,
        involving: None,
    };
    let paired: HashSet<PostingRef> = propose_in(journal, &clearing, &settings, today)
        .iter()
        .flat_map(Proposal::postings)
        .collect();
    let today = day_number(today);
    candidates_in(journal, &clearing, 0, "")
        .into_iter()
        .filter(|candidate| !paired.contains(&candidate.at))
        .filter(|candidate| today - day_number(candidate.date) > i64::from(flag_days))
        .collect()
}

/// The most exact partners [`partners_for`] lists per posting (A2).
pub const MAX_EXACT_PARTNERS: usize = 20;
/// The most nearby postings [`partners_for`] lists per posting (A2).
pub const MAX_NEARBY: usize = 50;

/// Another posting, with its gap in days from the one asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Partner {
    /// Where it is.
    pub at: PostingRef,
    /// Days between the two effective dates.
    pub gap_days: u32,
}

/// What the matcher can offer one clearing posting (A2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostingPartners {
    /// The posting asked about.
    pub at: PostingRef,
    /// Candidates on the same clearing account with exactly the opposite
    /// amount, at ANY gap (no window, no lookback), not rejected with it,
    /// in other transactions: nearest first, at most
    /// [`MAX_EXACT_PARTNERS`].
    pub exact: Vec<Partner>,
    /// The suggested combinations (A4, no lookback) that include it, as the
    /// single posting or a member.
    pub combinations: Vec<Proposal>,
    /// Candidates on the same clearing account and commodity with the
    /// opposite sign, within ± the window, in other transactions: nearest
    /// first, at most [`MAX_NEARBY`] — for picking a group by hand.
    pub nearby: Vec<Partner>,
}

/// A2: for each candidate posting (D3 with no lookback: unlinked, not
/// `match:ignore`) of the transaction at `position`, its exact partners, the
/// suggested combinations it is in, and the nearby opposite postings. Empty
/// when the transaction has no such posting (or does not exist).
#[must_use]
pub fn partners_for(
    journal: &Journal,
    position: usize,
    window_days: u32,
    today: &str,
) -> Vec<PostingPartners> {
    let clearing = clearing_accounts(journal);
    let candidates = candidates_in(journal, &clearing, 0, today);
    let own: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.at.position == position)
        .map(|(index, _)| index)
        .collect();
    if own.is_empty() {
        return Vec::new();
    }
    let days: Vec<i64> = candidates.iter().map(|c| day_number(c.date)).collect();
    let nomatch = nomatch_values(journal, &candidates);
    let settings = MatchSettings {
        window_days,
        lookback_days: 0,
        involving: None,
    };
    let combinations: Vec<Proposal> = propose_in(journal, &clearing, &settings, today)
        .into_iter()
        .filter(Proposal::is_combination)
        .collect();
    let window = u64::from(window_days);
    own.into_iter()
        .map(|single| {
            let me = &candidates[single];
            let partner = |other: usize| Partner {
                at: candidates[other].at,
                gap_days: u32::try_from((days[single] - days[other]).unsigned_abs())
                    .unwrap_or(u32::MAX),
            };
            let sorted = |mut partners: Vec<Partner>, limit: usize| {
                partners.sort_by_key(|p| (p.gap_days, p.at.sort_key()));
                partners.truncate(limit);
                partners
            };
            let opposite: Vec<usize> = candidates
                .iter()
                .enumerate()
                .filter(|(_, other)| {
                    other.at.position != position
                        && other.account == me.account
                        && other.amount.commodity == me.amount.commodity
                        && (other.amount.quantity.mantissa > 0) != (me.amount.quantity.mantissa > 0)
                })
                .map(|(index, _)| index)
                .collect();
            let exact = opposite
                .iter()
                .copied()
                .filter(|&other| {
                    me.amount
                        .quantity
                        .neg()
                        .is_ok_and(|negated| negated == candidates[other].amount.quantity)
                        && !shares_a_value(&nomatch[single], &nomatch[other])
                })
                .map(partner)
                .collect();
            let nearby = opposite
                .iter()
                .copied()
                .map(partner)
                .filter(|p| u64::from(p.gap_days) <= window)
                .collect();
            PostingPartners {
                at: me.at,
                exact: sorted(exact, MAX_EXACT_PARTNERS),
                combinations: combinations
                    .iter()
                    .filter(|proposal| proposal.postings().any(|at| at == me.at))
                    .cloned()
                    .collect(),
                nearby: sorted(nearby, MAX_NEARBY),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Links and rejections
// ---------------------------------------------------------------------------

/// Every value of the tag `name` on a real written posting, with the
/// postings carrying it in file order. A posting holding one value twice is
/// listed once.
fn tag_index(journal: &Journal, name: &str) -> BTreeMap<String, Vec<PostingRef>> {
    let mut index: BTreeMap<String, Vec<PostingRef>> = BTreeMap::new();
    for (at, _, posting) in real_postings(journal) {
        for value in tag_values(posting, name) {
            let postings = index.entry(value.to_string()).or_default();
            if postings.last() != Some(&at) {
                postings.push(at);
            }
        }
    }
    index
}

fn posting_at(journal: &Journal, at: PostingRef) -> Option<&Posting> {
    journal
        .transactions
        .get(at.position)
        .filter(|txn| txn.index == at.txn)?
        .postings
        .get(at.posting)
}

/// Why `postings` (2 or more, all real) are not one valid link, or `None`
/// when they are (A3): every one on the same clearing account, in its own
/// transaction, with one cost-free amount of one commodity, the amounts
/// summing to exactly zero.
fn broken_reason(
    clearing: &BTreeSet<String>,
    postings: &[(PostingRef, &Posting)],
) -> Option<BrokenReason> {
    let Some(((_, first), rest)) = postings.split_first() else {
        return Some(BrokenReason::Count(0));
    };
    if rest.is_empty() {
        return Some(BrokenReason::Count(1));
    }
    if postings
        .iter()
        .any(|(_, posting)| !clearing.contains(&posting.account.0))
    {
        return Some(BrokenReason::NotClearing);
    }
    if rest
        .iter()
        .any(|(_, posting)| posting.account != first.account)
    {
        return Some(BrokenReason::DifferentAccounts);
    }
    let positions: HashSet<usize> = postings.iter().map(|(at, _)| at.position).collect();
    if positions.len() != postings.len() {
        return Some(BrokenReason::SameTransaction);
    }
    (!sums_to_zero(postings.iter().map(|(_, posting)| *posting)))
        .then_some(BrokenReason::DoNotCancel)
}

/// Whether every posting has one cost-free amount, all in one commodity, and
/// they sum to exactly zero.
fn sums_to_zero<'p>(postings: impl IntoIterator<Item = &'p Posting>) -> bool {
    let mut commodity = None;
    let mut sum = Dec::zero();
    for posting in postings {
        let [amount] = posting.amounts.as_slice() else {
            return false;
        };
        if amount.cost.is_some() || *commodity.get_or_insert(&amount.commodity) != &amount.commodity
        {
            return false;
        }
        let Ok(next) = sum.add(amount.quantity) else {
            return false;
        };
        sum = next;
    }
    sum.is_zero()
}

/// Every `match` id: the valid links (A3: N ≥ 2 clearing postings on one
/// account, in N transactions, summing to exactly zero) and the broken ones
/// (D5, D10). `match:ignore` is not a link.
#[must_use]
pub fn links(journal: &Journal) -> LinkIndex {
    let clearing = clearing_accounts(journal);
    let mut index = LinkIndex::default();
    for (id, postings) in tag_index(journal, MATCH_TAG) {
        if id == IGNORE_VALUE {
            continue;
        }
        let Some(resolved) = postings
            .iter()
            .map(|&at| posting_at(journal, at).map(|posting| (at, posting)))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if let Some(reason) = broken_reason(&clearing, &resolved) {
            index.broken.push(BrokenLink {
                id,
                postings,
                reason,
            });
            continue;
        }
        for position in postings
            .iter()
            .map(|at| at.position)
            .collect::<BTreeSet<_>>()
        {
            index
                .by_position
                .entry(position)
                .or_default()
                .push(id.clone());
        }
        match postings.as_slice() {
            [a, b] => {
                index.pairs.insert(id, (*a, *b));
            }
            _ => {
                index.groups.insert(id, postings);
            }
        }
    }
    index
}

/// Every `nomatch` value and the postings carrying it (D12).
#[must_use]
pub fn rejections(journal: &Journal) -> Rejections {
    Rejections {
        by_value: tag_index(journal, NOMATCH_TAG),
    }
}

// ---------------------------------------------------------------------------
// Ids
// ---------------------------------------------------------------------------

/// Fresh `match`/`nomatch` ids: 6 base-36 characters, unique across every
/// `match` and `nomatch` value in the journal and every id this generator has
/// already handed out (so a bulk link can draw many from one journal).
#[derive(Debug)]
pub struct MatchIds {
    used: HashSet<String>,
    hasher: RandomState,
    counter: u64,
}

impl MatchIds {
    /// A generator that avoids every id already in `journal`.
    #[must_use]
    pub fn new(journal: &Journal) -> Self {
        let used = real_postings(journal)
            .flat_map(|(_, _, posting)| {
                tag_values(posting, MATCH_TAG).chain(tag_values(posting, NOMATCH_TAG))
            })
            .map(str::to_string)
            .collect();
        Self {
            used,
            hasher: RandomState::new(),
            counter: 0,
        }
    }

    /// The next fresh id.
    pub fn next_id(&mut self) -> String {
        let space = 36_u64.pow(ID_LEN as u32);
        loop {
            self.counter += 1;
            let mut n = self.hasher.hash_one(self.counter) % space;
            let id: String = (0..ID_LEN)
                .map(|_| {
                    let digit = ID_ALPHABET[(n % 36) as usize];
                    n /= 36;
                    char::from(digit)
                })
                .collect();
            if id != IGNORE_VALUE && self.used.insert(id.clone()) {
                return id;
            }
        }
    }
}

/// One fresh id for `journal` (see [`MatchIds`]).
#[must_use]
pub fn new_match_id(journal: &Journal) -> String {
    MatchIds::new(journal).next_id()
}

// ---------------------------------------------------------------------------
// Ops
// ---------------------------------------------------------------------------

/// Collects ops per transaction, emitted in file order.
#[derive(Default)]
struct OpsBuilder {
    by_position: BTreeMap<usize, (Tindex, Vec<EditOp>)>,
}

impl OpsBuilder {
    fn push(&mut self, at: PostingRef, op: EditOp) {
        self.by_position
            .entry(at.position)
            .or_insert_with(|| (at.txn, Vec::new()))
            .1
            .push(op);
    }

    fn set(&mut self, at: PostingRef, name: &str, value: Option<&str>) {
        self.push(
            at,
            EditOp::SetPostingTag {
                posting: at.posting,
                name: name.to_string(),
                value: value.map(str::to_string),
            },
        );
    }

    fn add(&mut self, at: PostingRef, name: &str, value: &str) {
        self.push(
            at,
            EditOp::AddPostingTagValue {
                posting: at.posting,
                name: name.to_string(),
                value: value.to_string(),
            },
        );
    }

    fn remove(&mut self, at: PostingRef, name: &str, value: &str) {
        self.push(
            at,
            EditOp::RemovePostingTagValue {
                posting: at.posting,
                name: name.to_string(),
                value: value.to_string(),
            },
        );
    }

    /// D12 rule 3: every `nomatch` value on any of `postings`, removed from
    /// every posting carrying it.
    fn forget_rejections_of(
        &mut self,
        rejections: &Rejections,
        postings: &[(PostingRef, &Posting)],
    ) {
        let values: BTreeSet<&str> = postings
            .iter()
            .flat_map(|(_, posting)| tag_values(posting, NOMATCH_TAG))
            .collect();
        for value in values {
            for &at in rejections.postings_of(value) {
                self.remove(at, NOMATCH_TAG, value);
            }
        }
    }

    fn build(self) -> TxnOps {
        self.by_position.into_values().collect()
    }
}

fn check_id(id: &str) -> Result<(), MatchError> {
    let ok = !id.is_empty()
        && id != IGNORE_VALUE
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(MatchError::BadId(id.to_string()))
    }
}

/// A posting that may be paired: a real, written clearing posting that
/// passes D3 (the lookback aside).
fn matchable<'j>(
    journal: &'j Journal,
    clearing: &BTreeSet<String>,
    at: PostingRef,
) -> Result<&'j Posting, MatchError> {
    let txn = journal
        .transactions
        .get(at.position)
        .filter(|txn| txn.index == at.txn)
        .ok_or(MatchError::PostingNotFound(at))?;
    let posting = txn
        .postings
        .get(at.posting)
        .ok_or(MatchError::PostingNotFound(at))?;
    let not = |reason: &str| MatchError::NotMatchable {
        at,
        reason: reason.to_string(),
    };
    if is_generated(txn) || posting.generated.is_some() {
        return Err(not("it is generated by a rule"));
    }
    if posting.ptype != PostingType::Regular {
        return Err(not("it is a virtual posting"));
    }
    if !clearing.contains(&posting.account.0) {
        return Err(not("its account is not a clearing account"));
    }
    match unmatchable(posting) {
        Some(reason) => Err(not(reason)),
        None => Ok(posting),
    }
}

/// The most postings one link may hold: far more than a bill-pay debit
/// covers, it only bounds a hostile request.
pub const MAX_LINK_MEMBERS: usize = 16;

/// Every member matchable, in its own transaction, on one clearing account,
/// summing to exactly zero in one commodity (D4 and A3, less the window and
/// the rejections).
fn legal_group<'j>(
    journal: &'j Journal,
    clearing: &BTreeSet<String>,
    members: &[PostingRef],
) -> Result<Vec<(PostingRef, &'j Posting)>, MatchError> {
    if !(2..=MAX_LINK_MEMBERS).contains(&members.len()) {
        return Err(MatchError::NotAPair(format!(
            "a link needs 2 to {MAX_LINK_MEMBERS} postings, got {}",
            members.len()
        )));
    }
    let resolved = members
        .iter()
        .map(|&at| matchable(journal, clearing, at).map(|posting| (at, posting)))
        .collect::<Result<Vec<_>, _>>()?;
    match broken_reason(clearing, &resolved) {
        None => Ok(resolved),
        Some(BrokenReason::SameTransaction) => Err(MatchError::NotAPair(
            "two of the postings are in one transaction".to_string(),
        )),
        Some(BrokenReason::DifferentAccounts) => Err(MatchError::NotAPair(
            "the postings are on different clearing accounts".to_string(),
        )),
        Some(_) => Err(MatchError::NotAPair(
            "the amounts do not cancel exactly".to_string(),
        )),
    }
}

/// Link `a` and `b` with `match:id` (D1), and clean up every rejection
/// either carries, on both sides of each (D12 rule 3).
///
/// # Errors
/// [`MatchError`] when either posting is missing or not a candidate, the two
/// are not a legal pair (D4, window and rejections aside — linking by hand is
/// the user's call), or `id` is unusable.
pub fn link_ops(
    journal: &Journal,
    a: PostingRef,
    b: PostingRef,
    id: &str,
) -> Result<TxnOps, MatchError> {
    Linker::new(journal).link_ops(a, b, id)
}

/// Link every posting of `members` (2 to [`MAX_LINK_MEMBERS`]) with one
/// `match:id` (A3): a many-to-one group, or a pair. Every rejection any
/// member carries is cleaned up on both sides (D12 rule 3); `nomatch` stays
/// pairwise.
///
/// # Errors
/// As [`link_ops`]: a member missing or not a candidate, two in one
/// transaction, on different clearing accounts, or amounts not summing to
/// exactly zero in one commodity.
pub fn link_group_ops(
    journal: &Journal,
    members: &[PostingRef],
    id: &str,
) -> Result<TxnOps, MatchError> {
    Linker::new(journal).link_group_ops(members, id)
}

/// [`link_ops`] for many pairs over one journal: the clearing accounts and
/// the rejections are read once rather than once per pair, which is what
/// keeps a bulk link of thousands of pairs linear.
#[derive(Debug)]
pub struct Linker<'j> {
    journal: &'j Journal,
    clearing: BTreeSet<String>,
    rejections: Rejections,
}

impl<'j> Linker<'j> {
    /// A linker over `journal`.
    #[must_use]
    pub fn new(journal: &'j Journal) -> Self {
        Self {
            journal,
            clearing: clearing_accounts(journal),
            rejections: rejections(journal),
        }
    }

    /// As [`link_ops`].
    ///
    /// # Errors
    /// As [`link_ops`].
    pub fn link_ops(&self, a: PostingRef, b: PostingRef, id: &str) -> Result<TxnOps, MatchError> {
        self.link_group_ops(&[a, b], id)
    }

    /// As [`link_group_ops`].
    ///
    /// # Errors
    /// As [`link_group_ops`].
    pub fn link_group_ops(&self, members: &[PostingRef], id: &str) -> Result<TxnOps, MatchError> {
        check_id(id)?;
        let resolved = legal_group(self.journal, &self.clearing, members)?;
        let mut ops = OpsBuilder::default();
        for &at in members {
            ops.set(at, MATCH_TAG, Some(id));
        }
        ops.forget_rejections_of(&self.rejections, &resolved);
        Ok(ops.build())
    }
}

/// Reject the pair `a`/`b` ("Not a match", D12): add `nomatch:id` to both.
///
/// # Errors
/// As [`link_ops`].
pub fn reject_ops(
    journal: &Journal,
    a: PostingRef,
    b: PostingRef,
    id: &str,
) -> Result<TxnOps, MatchError> {
    check_id(id)?;
    legal_group(journal, &clearing_accounts(journal), &[a, b])?;
    let mut ops = OpsBuilder::default();
    ops.add(a, NOMATCH_TAG, id);
    ops.add(b, NOMATCH_TAG, id);
    Ok(ops.build())
}

/// Unlink `id`: remove `match:id` from every posting carrying it, a broken
/// link's included (D5). Other `match` values stay.
///
/// # Errors
/// [`MatchError::UnknownId`] when no posting carries it.
pub fn unlink_ops(journal: &Journal, id: &str) -> Result<TxnOps, MatchError> {
    remove_everywhere(journal, MATCH_TAG, id)
}

/// Forget the rejection `value`: remove `nomatch:value` from every posting
/// carrying it (D12 Undo).
///
/// # Errors
/// [`MatchError::UnknownId`] when no posting carries it.
pub fn forget_ops(journal: &Journal, value: &str) -> Result<TxnOps, MatchError> {
    remove_everywhere(journal, NOMATCH_TAG, value)
}

fn remove_everywhere(
    journal: &Journal,
    name: &'static str,
    value: &str,
) -> Result<TxnOps, MatchError> {
    let postings = tag_index(journal, name).remove(value).unwrap_or_default();
    if postings.is_empty() || (name == MATCH_TAG && value == IGNORE_VALUE) {
        return Err(MatchError::UnknownId {
            name,
            value: value.to_string(),
        });
    }
    let mut ops = OpsBuilder::default();
    for at in postings {
        ops.remove(at, name, value);
    }
    Ok(ops.build())
}

/// Mark `at` `match:ignore` ("Ignore this side", D1, D9), cleaning up its
/// rejections on both sides as a link does (D12 rule 3).
///
/// # Errors
/// As [`link_ops`], for the one posting.
pub fn ignore_ops(journal: &Journal, at: PostingRef) -> Result<TxnOps, MatchError> {
    let posting = matchable(journal, &clearing_accounts(journal), at)?;
    let mut ops = OpsBuilder::default();
    ops.set(at, MATCH_TAG, Some(IGNORE_VALUE));
    ops.forget_rejections_of(&rejections(journal), &[(at, posting)]);
    Ok(ops.build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;

    #[test]
    fn ids_are_six_base36_chars_unique_and_never_ignore() {
        let journal = parse_journal("", "inline.journal").expect("parses");
        let mut ids = MatchIds::new(&journal);
        let drawn: HashSet<String> = (0..2000).map(|_| ids.next_id()).collect();
        assert_eq!(drawn.len(), 2000);
        assert!(drawn.iter().all(|id| {
            id.len() == ID_LEN
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
                && id != IGNORE_VALUE
        }));
    }

    #[test]
    fn ids_avoid_values_already_in_the_journal() {
        let journal = parse_journal(
            "2026-01-01 x\n    a  $1  ; match:abc123, nomatch:def456\n    b\n",
            "inline.journal",
        )
        .expect("parses");
        let ids = MatchIds::new(&journal);
        assert!(ids.used.contains("abc123") && ids.used.contains("def456"));
    }

    #[test]
    fn check_id_refuses_unreadable_values() {
        for id in ["", "ignore", "a,b", "a b", "x:y"] {
            assert!(check_id(id).is_err(), "{id}");
        }
        assert!(check_id("k7f3a2").is_ok());
    }
}
