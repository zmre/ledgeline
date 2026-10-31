//! The transaction-review model (plan 32, Phase 1): which transactions have
//! been reviewed, which are still uncategorised, which look alike, and what
//! category the journal's own history suggests for one.
//!
//! Everything here is computed from a parsed [`Journal`], because the client
//! never holds the whole journal (it reads 200-row blocks). [`ReviewIndex`]
//! is the per-snapshot index the server caches next to its other views; every
//! query against it is a hash lookup plus a sort of a few hundred items.
//!
//! # The vocabulary
//!
//! - **Review state** ([`ReviewState`], D1) lives in ONE tag on the
//!   transaction header — `reviewed:YYYY-MM-DD`, `reviewed:ignore`, or any
//!   other value. A `reviewed:` on a posting comment is ignored, so `hledger
//!   reg tag:reviewed` and `not:tag:reviewed` mean exactly what the UI means.
//! - **Unknown** ([`is_unknown`], D3) is the Problems drawer's
//!   `uncategorized` rule ([`crate::view::checks`]) over every posting of a
//!   real transaction, the ones an `=` rule generated included (amendment
//!   A): an auto rule that books to `expenses:unknown` has produced an
//!   uncategorised posting like any other, and the drawer counts it too.
//!   Forecast transactions (`~` rules, [`is_generated`]) are never unknown,
//!   here or in the drawer.
//! - **Similar** ([`similarity_key`], D10) is exact equality of a normalised
//!   payee, never fuzzy matching — a bulk edit is not the place for false
//!   positives, and an exact key can be indexed.
//!
//! # Confidence is integer basis points
//!
//! A suggestion's confidence decides whether it becomes the one-keystroke chip
//! (at least [`STRONG`]), and a threshold that decides something is compared
//! exactly in this engine (see `reports::subscriptions::cluster_by_amount` for
//! what an `f64` boundary cost there). So it is a `u32` out of [`CERTAIN`],
//! computed with integer arithmetic; [`Suggestion::confidence_ratio`] is for
//! display only.

use crate::decimal::Dec;
use crate::model::{Journal, Posting, Tindex, Transaction};
use crate::projections::is_generated;
use crate::reports::AccountType;
use crate::reports::periods::add_days;
use crate::reports::subscriptions::{
    Subscription, SubscriptionOpts, day_distance, detect_subscriptions, payee_of,
};
use crate::view::Classes;
use crate::view::checks::is_uncategorized;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

/// The tag that records a transaction's review (D1).
pub const REVIEWED_TAG: &str = "reviewed";

/// The [`REVIEWED_TAG`] value meaning "never ask about this one".
pub const IGNORE_VALUE: &str = "ignore";

/// The tag a rename from the review overlay records the ORIGINAL similarity
/// key in (D13a, Q12), so the rename can be suggested for the next
/// transaction with that key once the raw text is gone.
pub const PAYEE_KEY_TAG: &str = "payee-key";

/// The most candidates [`ReviewIndex::similar`] returns (D11).
pub const SIMILAR_CAP: usize = 500;

/// A confidence of 100%, in basis points.
pub const CERTAIN: u32 = 10_000;

/// The confidence a suggestion needs to be the one-keystroke chip (D13a:
/// 50%). Anything weaker is offered as an alternative only.
pub const STRONG: u32 = 5_000;

/// How many suggestions [`ReviewIndex::suggest`] returns: the chip and up to
/// two runners-up (D13).
pub const MAX_SUGGESTIONS: usize = 3;

/// The fewest leading key tokens a payee-prefix precedent may share (D13a
/// source 3): one shared word (`sq`, `amazon`) says nothing about a merchant.
const MIN_PREFIX_TOKENS: usize = 2;

/// A payee-prefix precedent's confidence is its share, scaled by how much of
/// the key it shares, times this percentage — a prefix is weaker evidence
/// than the whole key, however unanimous its history.
const PREFIX_DISCOUNT_PCT: u64 = 80;

/// The confidence of a recurring-charge match (D13a source 4): the same
/// funding account, amount and billing day as a known subscription is strong
/// evidence, shared out when several subscriptions fit.
const RECURRING_CONFIDENCE: u32 = 6_000;

/// How far a charge's amount may differ from a subscription's typical amount,
/// in percent of the typical amount, and still be that subscription (D13a: 1%).
const RECURRING_AMOUNT_TOLERANCE_PCT: i128 = 1;

/// How far a charge's day of month may be from a subscription's last billing
/// day — the subscription detector's own default tolerance.
const RECURRING_DAY_TOLERANCE: i64 = 4;

/// A transaction's review state, read from its HEADER tags only (D1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewState {
    /// No `reviewed` tag at all.
    Unreviewed,
    /// `reviewed:YYYY-MM-DD` — reviewed on that (local) day.
    Reviewed(String),
    /// `reviewed:ignore` — never ask; hidden from both queues and the markers.
    Ignored,
    /// `reviewed:` with any other value (`reviewed:yes`): treated as reviewed.
    ReviewedOther(String),
}

impl ReviewState {
    /// The state `txn`'s own tags record. The first `reviewed` tag wins, as
    /// it does for hledger's `tag:` query.
    #[must_use]
    pub fn of(txn: &Transaction) -> Self {
        let Some((_, value)) = txn.tags.iter().find(|(name, _)| name == REVIEWED_TAG) else {
            return Self::Unreviewed;
        };
        let value = value.trim();
        if value.eq_ignore_ascii_case(IGNORE_VALUE) {
            Self::Ignored
        } else if is_iso_date(value) {
            Self::Reviewed(value.to_string())
        } else {
            Self::ReviewedOther(value.to_string())
        }
    }

    /// Whether the transaction carries a `reviewed` tag of any value,
    /// `ignore` included — what keeps it out of Recents.
    #[must_use]
    pub fn is_tagged(&self) -> bool {
        !matches!(self, Self::Unreviewed)
    }

    /// Whether a human said yes to it: reviewed, but not merely ignored. A
    /// reviewed transaction's category weighs double as precedent (D13a).
    #[must_use]
    pub fn is_reviewed(&self) -> bool {
        matches!(self, Self::Reviewed(_) | Self::ReviewedOther(_))
    }
}

/// `YYYY-MM-DD`, by shape. The value is a stamp, not a date anything computes
/// with, so the shape is all that distinguishes it from `reviewed:yes`.
fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

/// The postings `txn`'s file writes. A posting an `=` rule generated is the
/// rule's choice of account, not the person's, so it never votes as precedent
/// or names a funding account — though it can still make `txn` unknown.
fn written(txn: &Transaction) -> impl Iterator<Item = &Posting> {
    txn.postings
        .iter()
        .filter(|posting| posting.generated.is_none())
}

/// Whether `txn` is **unknown** (D3): any posting — a generated one included
/// — to an account the Problems drawer flags as uncategorized, on a real
/// transaction. A forecast transaction is never unknown.
#[must_use]
pub fn is_unknown(txn: &Transaction, classes: &Classes) -> bool {
    !is_generated(txn)
        && txn
            .postings
            .iter()
            .any(|posting| is_uncategorized(&posting.account.0, classes))
}

/// The normalised payee two transactions must share to be "similar" (D10), or
/// `None` when nothing is left of it.
///
/// 1. The payee part (before `|`, as the subscription detector splits it).
/// 2. Unicode NFKC, then lowercased: compatibility forms a bank export emits
///    (fullwidth `ＳＱ`, the ligature `ﬁ`, `™`, the ideographic space) key the
///    same as their plain spellings. NFKC first, so the case mapping sees the
///    plain letters.
/// 3. Every whitespace-separated token containing a digit dropped (`#4412`,
///    `POS0923`, `10/12`, `X1234`).
/// 4. Punctuation turned into spaces, whitespace collapsed.
///
/// So `SQ *BLUE BOTTLE 4412 OAKLAND` and `SQ *BLUE BOTTLE 0917 OAKLAND` both
/// key to `sq blue bottle oakland`, and `12345` keys to nothing. A digit NFKC
/// produces (`①` → `1`) counts as a digit in step 3.
#[must_use]
pub fn similarity_key(description: &str) -> Option<String> {
    let folded: String = payee_of(description)
        .nfkc()
        .collect::<String>()
        .to_lowercase();
    let words: Vec<&str> = folded
        .split_whitespace()
        .filter(|token| !token.chars().any(char::is_numeric))
        .flat_map(|token| token.split(|c: char| !c.is_alphanumeric()))
        .filter(|word| !word.is_empty())
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// Which review queue (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    /// Every unknown transaction in the window except `reviewed:ignore`.
    Unknowns,
    /// Transactions in the window with no `reviewed` tag at all.
    Recents,
}

/// What to queue: the kind, the client's local today, and the paging.
#[derive(Debug, Clone, Copy)]
pub struct QueueOpts<'a> {
    /// Which queue.
    pub kind: QueueKind,
    /// The client's local date, `YYYY-MM-DD`. Later transactions are never
    /// queued.
    pub today: &'a str,
    /// The most items to return (Recents' `limit`, default 50; Unknowns'
    /// page size, 100). `usize::MAX` for all.
    pub limit: usize,
    /// How many days back the window reaches, inclusive: a transaction dated
    /// exactly `today − max_days` is in it. `0` means no limit.
    pub max_days: u32,
    /// How many queued items to skip first, for paging Unknowns.
    pub offset: usize,
}

impl<'a> QueueOpts<'a> {
    /// The D4 defaults for `kind` as of `today`: a 365-day window, 50 Recents,
    /// Unknowns paged at 100.
    #[must_use]
    pub fn new(kind: QueueKind, today: &'a str) -> Self {
        Self {
            kind,
            today,
            limit: match kind {
                QueueKind::Unknowns => 100,
                QueueKind::Recents => 50,
            },
            max_days: 365,
            offset: 0,
        }
    }
}

/// One queue page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueResult {
    /// The page, newest first.
    pub items: Vec<Tindex>,
    /// How many editable transactions the whole queue holds.
    pub total: usize,
    /// How many more would have qualified but live in a file the editor
    /// cannot write — left out of `items` and `total`, reported so the UI can
    /// say "N not editable".
    pub not_editable: usize,
}

/// The most common category among transactions sharing a key (D11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Precedent {
    /// The counter-account.
    pub account: String,
    /// How many of them used it.
    pub count: usize,
    /// How many categorised transactions there were to choose from.
    pub of: usize,
}

/// What [`ReviewIndex::similar`] found for one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Similar {
    /// Unreviewed, editable transactions with the same key, newest first,
    /// at most [`SIMILAR_CAP`].
    pub candidates: Vec<Tindex>,
    /// How many there were before the cap.
    pub total: usize,
    /// The most common counter-account among CATEGORISED transactions with
    /// the key, reviewed or not.
    pub precedent: Option<Precedent>,
}

/// Where a [`Suggestion`] came from (D13a), best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SuggestionSource {
    /// The most common counter-account among categorised transactions with
    /// the same similarity key.
    ExactKey,
    /// A rule in the import's rules file (Phase 4: import time only, never
    /// produced by [`ReviewIndex::suggest`]).
    RuleFile,
    /// The longest leading run of at least two key tokens shared with
    /// categorised transactions, when the exact key has no history.
    PayeePrefix,
    /// A known recurring charge with the same funding account, amount and
    /// billing day (a renamed subscription).
    Recurring,
}

/// One suggested category for a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The account to post the unknown leg to.
    pub account: String,
    /// A rename reviewed transactions with this key received, when there is
    /// one (`→ Blue Bottle Coffee · food:coffee`).
    pub description: Option<String>,
    /// Which source produced it.
    pub source: SuggestionSource,
    /// Basis points out of [`CERTAIN`].
    pub confidence: u32,
    /// How many transactions back it up.
    pub count: usize,
}

impl Suggestion {
    /// Whether it is strong enough for the one-keystroke chip (D13a).
    #[must_use]
    pub fn is_strong(&self) -> bool {
        self.confidence >= STRONG
    }

    /// The confidence as a fraction, for display.
    #[must_use]
    pub fn confidence_ratio(&self) -> f64 {
        f64::from(self.confidence) / f64::from(CERTAIN)
    }
}

/// The suggestion to put on the one-keystroke chip: the best one, if it is
/// [`STRONG`]. The rest are alternatives.
#[must_use]
pub fn top_chip(suggestions: &[Suggestion]) -> Option<&Suggestion> {
    suggestions
        .first()
        .filter(|suggestion| suggestion.is_strong())
}

/// The per-snapshot review index (D14): every transaction's review state,
/// whether it is unknown and editable, and the similarity-key buckets.
///
/// Built once per journal generation; the queries take the same journal back
/// (and the same [`Classes`] where they classify accounts), so the index
/// holds no copy of either. Handing it a different journal is a caller bug,
/// answered with empty results rather than a panic.
#[derive(Debug)]
pub struct ReviewIndex {
    /// Similarity key → the transactions with it, in file order.
    by_key: HashMap<String, Vec<Tindex>>,
    /// Original key (`payee-key:` tag) → the renamed transactions carrying it.
    by_payee_key: HashMap<String, Vec<Tindex>>,
    /// By position in `journal.transactions`.
    state: Vec<ReviewState>,
    /// By position: [`is_unknown`].
    unknown: Vec<bool>,
    /// By position: the file it lives in can be written.
    editable: Vec<bool>,
    /// By position: a real transaction (not one a forecast generated).
    real: Vec<bool>,
    /// Key prefix of at least [`MIN_PREFIX_TOKENS`] tokens → every key
    /// starting with it. Built on the first prefix lookup.
    prefixes: OnceLock<HashMap<String, Vec<String>>>,
    /// The journal's recurring charges, detected on the first recurring
    /// lookup.
    recurring: OnceLock<Vec<Subscription>>,
}

impl ReviewIndex {
    /// Index `journal`. `editable` says whether a transaction's source file
    /// can be written (asked once per file); a transaction in a file it
    /// refuses is never queued or offered as a similar candidate.
    #[must_use]
    pub fn build(journal: &Journal, classes: &Classes, editable: impl Fn(&Path) -> bool) -> Self {
        let count = journal.transactions.len();
        let mut by_key: HashMap<String, Vec<Tindex>> = HashMap::new();
        let mut by_payee_key: HashMap<String, Vec<Tindex>> = HashMap::new();
        let mut state = Vec::with_capacity(count);
        let mut unknown = Vec::with_capacity(count);
        let mut editable_at = Vec::with_capacity(count);
        let mut real = Vec::with_capacity(count);
        // Memoized: a few hundred accounts and files behind every transaction.
        let mut uncategorized: HashMap<&str, bool> = HashMap::new();
        let mut writable: HashMap<&Path, bool> = HashMap::new();
        for txn in &journal.transactions {
            if let Some(key) = similarity_key(&txn.description) {
                by_key.entry(key).or_default().push(txn.index);
            }
            if let Some((_, key)) = txn.tags.iter().find(|(name, _)| name == PAYEE_KEY_TAG) {
                let key = key.trim();
                if !key.is_empty() {
                    by_payee_key
                        .entry(key.to_string())
                        .or_default()
                        .push(txn.index);
                }
            }
            state.push(ReviewState::of(txn));
            unknown.push(
                !is_generated(txn)
                    && txn.postings.iter().any(|posting| {
                        let account = posting.account.0.as_str();
                        *uncategorized
                            .entry(account)
                            .or_insert_with(|| is_uncategorized(account, classes))
                    }),
            );
            editable_at.push(
                *writable
                    .entry(txn.source_file.as_path())
                    .or_insert_with(|| editable(&txn.source_file)),
            );
            real.push(!is_generated(txn));
        }
        Self {
            by_key,
            by_payee_key,
            state,
            unknown,
            editable: editable_at,
            real,
            prefixes: OnceLock::new(),
            recurring: OnceLock::new(),
        }
    }

    /// The review state of the transaction at `position` in
    /// `journal.transactions`.
    #[must_use]
    pub fn state_at(&self, position: usize) -> Option<&ReviewState> {
        self.state.get(position)
    }

    /// Whether the transaction at `position` is unknown (D3).
    #[must_use]
    pub fn unknown_at(&self, position: usize) -> bool {
        self.unknown.get(position).copied().unwrap_or(false)
    }

    /// One page of a review queue (D4), newest first (date, then file order,
    /// both descending — the journal's own display order).
    ///
    /// Both queues share one window: dated from `today − max_days` (inclusive)
    /// through `today`. Unknowns holds every unknown transaction except
    /// `reviewed:ignore` — a hand-tagged `reviewed:<date>` that is still
    /// unknown stays in it. Recents holds every transaction with no
    /// `reviewed` tag at all, unknown or not.
    #[must_use]
    pub fn queue(&self, journal: &Journal, opts: &QueueOpts) -> QueueResult {
        let start = (opts.max_days > 0).then(|| add_days(opts.today, -i64::from(opts.max_days)));
        let mut eligible: Vec<(&str, usize)> = Vec::new();
        let mut not_editable = 0;
        for (position, txn) in journal.transactions.iter().enumerate() {
            let date = txn.date.as_str();
            if date > opts.today || start.as_deref().is_some_and(|start| date < start) {
                continue;
            }
            let (Some(state), Some(&unknown), Some(&real)) = (
                self.state.get(position),
                self.unknown.get(position),
                self.real.get(position),
            ) else {
                continue;
            };
            let wanted = real
                && match opts.kind {
                    QueueKind::Unknowns => unknown && *state != ReviewState::Ignored,
                    QueueKind::Recents => !state.is_tagged(),
                };
            if !wanted {
                continue;
            }
            if self.editable.get(position) == Some(&true) {
                eligible.push((date, position));
            } else {
                not_editable += 1;
            }
        }
        eligible.sort_unstable_by(|a, b| b.cmp(a));
        QueueResult {
            total: eligible.len(),
            items: eligible
                .into_iter()
                .skip(opts.offset)
                .take(opts.limit)
                .map(|(_, position)| journal.transactions[position].index)
                .collect(),
            not_editable,
        }
    }

    /// The transactions similar to `index` (D11): its key is computed from its
    /// CURRENT description, which is the pre-edit one while the card is open.
    #[must_use]
    pub fn similar(&self, journal: &Journal, classes: &Classes, index: Tindex) -> Similar {
        let empty = Similar {
            candidates: Vec::new(),
            total: 0,
            precedent: None,
        };
        let Some(position) = position_of(journal, index) else {
            return empty;
        };
        let Some(key) = similarity_key(&journal.transactions[position].description) else {
            return empty;
        };
        let mut candidates: Vec<(&str, usize)> = self
            .positions(journal, self.by_key.get(&key))
            .filter(|&other| {
                other != position
                    && self.state.get(other) == Some(&ReviewState::Unreviewed)
                    && self.editable.get(other) == Some(&true)
                    && self.real.get(other) == Some(&true)
            })
            .map(|other| (journal.transactions[other].date.as_str(), other))
            .collect();
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        let total = candidates.len();
        let tally = self.tally(
            journal,
            classes,
            &self.key_pool(journal, &key),
            position,
            Some(&key),
        );
        Similar {
            candidates: candidates
                .into_iter()
                .take(SIMILAR_CAP)
                .map(|(_, other)| journal.transactions[other].index)
                .collect(),
            total,
            precedent: tally.ranked().into_iter().next().map(|vote| Precedent {
                account: vote.account.to_string(),
                count: vote.count,
                of: tally.total,
            }),
        }
    }

    /// Suggested categories for `index` (D13a), best first, at most
    /// [`MAX_SUGGESTIONS`]. Use [`top_chip`] for the one-keystroke chip.
    ///
    /// Ranked by confidence, then by source (best first), then by name, with
    /// an account offered by two sources kept once at its higher confidence.
    /// A description with no similarity key gets no suggestions at all.
    #[must_use]
    pub fn suggest(&self, journal: &Journal, classes: &Classes, index: Tindex) -> Vec<Suggestion> {
        let Some(position) = position_of(journal, index) else {
            return Vec::new();
        };
        let txn = &journal.transactions[position];
        let Some(key) = similarity_key(&txn.description) else {
            return Vec::new();
        };
        let mut found: Vec<Suggestion> = Vec::new();

        // 1. Exact-key precedent; 3. payee-prefix only when that is silent.
        let exact = self.tally(
            journal,
            classes,
            &self.key_pool(journal, &key),
            position,
            Some(&key),
        );
        if exact.total > 0 {
            found.extend(exact.ranked().into_iter().map(|vote| Suggestion {
                account: vote.account.to_string(),
                description: exact.rename_for(vote.account),
                source: SuggestionSource::ExactKey,
                confidence: vote.confidence(exact.total_weight, 1, 1),
                count: vote.count,
            }));
        } else if let Some((shared, prefix)) = self.prefix_tally(journal, classes, &key, position) {
            let tokens = key.split(' ').count();
            found.extend(prefix.ranked().into_iter().map(|vote| Suggestion {
                account: vote.account.to_string(),
                description: None,
                source: SuggestionSource::PayeePrefix,
                confidence: vote.confidence(
                    prefix.total_weight,
                    PREFIX_DISCOUNT_PCT * shared as u64,
                    100 * tokens as u64,
                ),
                count: vote.count,
            }));
        }

        // 4. A known recurring charge this one matches.
        found.extend(self.recurring_matches(journal, classes, txn));

        rank(found)
    }

    /// Every position in `bucket` (a key's transactions), skipping any that
    /// no longer resolve in `journal`.
    fn positions<'a>(
        &'a self,
        journal: &'a Journal,
        bucket: Option<&'a Vec<Tindex>>,
    ) -> impl Iterator<Item = usize> + 'a {
        bucket
            .into_iter()
            .flatten()
            .filter_map(move |index| position_of(journal, *index))
    }

    /// The precedent pool for `key`: the transactions that have it, plus the
    /// renamed ones whose `payee-key:` records they once had it. Deduped.
    fn key_pool(&self, journal: &Journal, key: &str) -> Vec<usize> {
        let mut seen = HashSet::new();
        self.positions(journal, self.by_key.get(key))
            .chain(self.positions(journal, self.by_payee_key.get(key)))
            .filter(|position| seen.insert(*position))
            .collect()
    }

    /// Count the counter-accounts the categorised transactions in `pool` used,
    /// leaving out `exclude` (the transaction being reviewed). `rename_key`,
    /// when given, also collects the renames reviewed transactions with that
    /// original key received.
    fn tally<'j>(
        &self,
        journal: &'j Journal,
        classes: &Classes,
        pool: &[usize],
        exclude: usize,
        rename_key: Option<&str>,
    ) -> Tally<'j> {
        let mut tally = Tally::default();
        for &position in pool {
            if position == exclude || self.unknown.get(position) != Some(&false) {
                continue;
            }
            let txn = &journal.transactions[position];
            let counters = category_accounts(txn, classes);
            if counters.is_empty() {
                continue;
            }
            let reviewed = self
                .state
                .get(position)
                .is_some_and(ReviewState::is_reviewed);
            let weight = if reviewed { 2 } else { 1 };
            tally.total += 1;
            tally.total_weight += weight;
            for account in &counters {
                let vote = tally.votes.entry(account).or_default();
                vote.0 += weight;
                vote.1 += 1;
            }
            let renamed = rename_key.is_some_and(|key| {
                txn.tags
                    .iter()
                    .any(|(name, value)| name == PAYEE_KEY_TAG && value.trim() == key)
                    && similarity_key(&txn.description).as_deref() != Some(key)
            });
            if reviewed && renamed {
                for account in counters {
                    *tally
                        .renames
                        .entry(account)
                        .or_default()
                        .entry(txn.description.as_str())
                        .or_default() += 1;
                }
            }
        }
        tally
    }

    /// D13a source 3: the longest leading run of at least
    /// [`MIN_PREFIX_TOKENS`] tokens `key` shares with OTHER keys that have
    /// categorised history, with how many tokens it is and that history's
    /// tally.
    fn prefix_tally<'j>(
        &self,
        journal: &'j Journal,
        classes: &Classes,
        key: &str,
        exclude: usize,
    ) -> Option<(usize, Tally<'j>)> {
        let prefixes = self.prefixes.get_or_init(|| {
            let mut prefixes: HashMap<String, Vec<String>> = HashMap::new();
            for key in self.by_key.keys() {
                let tokens: Vec<&str> = key.split(' ').collect();
                for shared in MIN_PREFIX_TOKENS..=tokens.len() {
                    prefixes
                        .entry(tokens[..shared].join(" "))
                        .or_default()
                        .push(key.clone());
                }
            }
            prefixes
        });
        let tokens: Vec<&str> = key.split(' ').collect();
        (MIN_PREFIX_TOKENS..=tokens.len()).rev().find_map(|shared| {
            let mut seen = HashSet::new();
            let pool: Vec<usize> = prefixes
                .get(&tokens[..shared].join(" "))
                .into_iter()
                .flatten()
                .filter(|other| other.as_str() != key)
                .flat_map(|other| self.positions(journal, self.by_key.get(other)))
                .filter(|position| seen.insert(*position))
                .collect();
            let tally = self.tally(journal, classes, &pool, exclude, None);
            (tally.total > 0).then_some((shared, tally))
        })
    }

    /// D13a source 4: the categories of the recurring charges `txn` looks
    /// like — the same funding account, its amount within
    /// [`RECURRING_AMOUNT_TOLERANCE_PCT`] of theirs, billed within
    /// [`RECURRING_DAY_TOLERANCE`] days of the same day of month.
    fn recurring_matches(
        &self,
        journal: &Journal,
        classes: &Classes,
        txn: &Transaction,
    ) -> Vec<Suggestion> {
        let recurring = self.recurring.get_or_init(|| {
            // As of the newest charge on file, so the detector's staleness
            // test is measured against the data rather than the calendar.
            let as_of = journal
                .transactions
                .iter()
                .map(|txn| txn.date.as_str())
                .max()
                .unwrap_or_default();
            let opts = SubscriptionOpts {
                as_of,
                ..SubscriptionOpts::default()
            };
            detect_subscriptions(journal, &opts)
                .map(|report| report.monthly.into_iter().chain(report.annual).collect())
                .unwrap_or_default()
        });
        let funding = funding_accounts(txn, classes);
        let Some(amount) = spend(txn, classes) else {
            return Vec::new();
        };
        let day = crate::reports::periods::parts(&txn.date).2;
        let matches: Vec<(&Subscription, Vec<&str>)> = recurring
            .iter()
            .filter(|sub| {
                sub.funding
                    .iter()
                    .any(|account| funding.contains(&account.as_str()))
            })
            .filter(|sub| within_tolerance(amount, sub.typical_amount))
            .filter(|sub| {
                day_distance(day, crate::reports::periods::parts(&sub.last_seen).2)
                    <= RECURRING_DAY_TOLERANCE
            })
            .map(|sub| {
                let accounts = sub
                    .accounts
                    .iter()
                    .map(String::as_str)
                    .filter(|account| !is_uncategorized(account, classes))
                    .collect();
                (sub, accounts)
            })
            .filter(|(_, accounts): &(_, Vec<&str>)| !accounts.is_empty())
            .collect();
        let shares = matches
            .iter()
            .map(|(_, accounts)| accounts.len())
            .sum::<usize>();
        let share = u32::try_from(shares).unwrap_or(u32::MAX).max(1);
        matches
            .into_iter()
            .flat_map(|(sub, accounts)| {
                accounts.into_iter().map(move |account| Suggestion {
                    account: account.to_string(),
                    description: None,
                    source: SuggestionSource::Recurring,
                    confidence: RECURRING_CONFIDENCE / share,
                    count: sub.occurrences,
                })
            })
            .collect()
    }
}

/// The position of the transaction with `index` in `journal.transactions`. A
/// `tindex` is its 1-based file-order position, so that is tried first.
fn position_of(journal: &Journal, index: Tindex) -> Option<usize> {
    let transactions = &journal.transactions;
    (index.0 as usize)
        .checked_sub(1)
        .filter(|&guess| transactions.get(guess).is_some_and(|t| t.index == index))
        .or_else(|| transactions.iter().position(|t| t.index == index))
}

/// A transaction's CATEGORY accounts: its written postings to an expense or
/// revenue account that is not itself uncategorized. Distinct, in order.
///
/// That is what a precedent votes for. A transfer between two asset accounts
/// has none, so it is never precedent — its "category" is the other account,
/// which says nothing about a purchase.
fn category_accounts<'j>(txn: &'j Transaction, classes: &Classes) -> Vec<&'j str> {
    let mut accounts: Vec<&str> = Vec::new();
    for posting in written(txn) {
        let account = posting.account.0.as_str();
        if is_category(account, classes)
            && !is_uncategorized(account, classes)
            && !accounts.contains(&account)
        {
            accounts.push(account);
        }
    }
    accounts
}

/// Whether `account` is on the category side of a transaction.
fn is_category(account: &str, classes: &Classes) -> bool {
    classes.is_type(account, AccountType::Expense) || classes.is_type(account, AccountType::Revenue)
}

/// The accounts `txn` was paid from or into: every written posting that is
/// neither a category nor uncategorized.
fn funding_accounts<'j>(txn: &'j Transaction, classes: &Classes) -> Vec<&'j str> {
    written(txn)
        .map(|posting| posting.account.0.as_str())
        .filter(|account| !is_category(account, classes) && !is_uncategorized(account, classes))
        .collect()
}

/// The positive total `txn` spends on its category and uncategorized side, if
/// that side is in one commodity.
fn spend(txn: &Transaction, classes: &Classes) -> Option<Dec> {
    let mut commodity = None;
    let mut total = Dec::zero();
    for posting in written(txn) {
        let account = posting.account.0.as_str();
        if !is_category(account, classes) && !is_uncategorized(account, classes) {
            continue;
        }
        for amount in &posting.amounts {
            match commodity {
                None => commodity = Some(&amount.commodity),
                Some(seen) if *seen != amount.commodity => return None,
                Some(_) => {}
            }
            total = total.add(amount.quantity).ok()?;
        }
    }
    (total.mantissa > 0).then_some(total)
}

/// Whether `amount` is within [`RECURRING_AMOUNT_TOLERANCE_PCT`] of `typical`,
/// exactly: `|amount − typical| × 100 <= typical × tolerance`.
fn within_tolerance(amount: Dec, typical: Dec) -> bool {
    let check = || -> Result<bool, crate::decimal::DecError> {
        let gap = amount.sub(typical)?.abs()?;
        Ok(gap.mul(Dec::new(100, 0))?
            <= typical.mul(Dec::new(RECURRING_AMOUNT_TOLERANCE_PCT, 0))?)
    };
    check().unwrap_or(false)
}

/// Counter-account votes over a precedent pool.
#[derive(Default)]
struct Tally<'j> {
    /// Account → (weighted votes, transactions).
    votes: HashMap<&'j str, (u64, usize)>,
    /// Sum of every voting transaction's weight.
    total_weight: u64,
    /// How many transactions voted.
    total: usize,
    /// Account → renamed description → how many reviewed renames used it.
    renames: HashMap<&'j str, HashMap<&'j str, usize>>,
}

/// One account's share of a [`Tally`].
struct Vote<'j> {
    account: &'j str,
    weight: u64,
    count: usize,
}

impl Vote<'_> {
    /// `weight / total_weight`, scaled by `numerator / denominator`, in basis
    /// points.
    fn confidence(&self, total_weight: u64, numerator: u64, denominator: u64) -> u32 {
        let scaled = u128::from(self.weight) * u128::from(CERTAIN) * u128::from(numerator);
        let over = u128::from(total_weight.max(1)) * u128::from(denominator.max(1));
        u32::try_from(scaled / over).unwrap_or(CERTAIN).min(CERTAIN)
    }
}

impl<'j> Tally<'j> {
    /// Every account, most votes first; ties by transactions, then by name.
    fn ranked(&self) -> Vec<Vote<'j>> {
        let mut votes: Vec<Vote<'j>> = self
            .votes
            .iter()
            .map(|(account, (weight, count))| Vote {
                account,
                weight: *weight,
                count: *count,
            })
            .collect();
        votes.sort_by(|a, b| {
            b.weight
                .cmp(&a.weight)
                .then(b.count.cmp(&a.count))
                .then(a.account.cmp(b.account))
        });
        votes
    }

    /// The most common rename reviewed transactions posting to `account`
    /// received, ties broken by name.
    fn rename_for(&self, account: &str) -> Option<String> {
        self.renames
            .get(account)?
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(description, _)| (*description).to_string())
    }
}

/// Merge suggestions from every source: one per account at its best
/// confidence, ranked by confidence, then source, then name, and capped.
fn rank(found: Vec<Suggestion>) -> Vec<Suggestion> {
    let mut best: Vec<Suggestion> = Vec::new();
    for suggestion in found {
        match best
            .iter_mut()
            .find(|kept| kept.account == suggestion.account)
        {
            Some(kept)
                if (suggestion.confidence, std::cmp::Reverse(suggestion.source))
                    > (kept.confidence, std::cmp::Reverse(kept.source)) =>
            {
                *kept = suggestion;
            }
            Some(_) => {}
            None => best.push(suggestion),
        }
    }
    best.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then(a.source.cmp(&b.source))
            .then(a.account.cmp(&b.account))
    });
    best.truncate(MAX_SUGGESTIONS);
    best
}

// ---------------------------------------------------------------------------
// Autocomplete vocabularies (D15)
// ---------------------------------------------------------------------------

/// The most values [`tag_uses`] lists per tag name: a completion list, not a
/// census.
pub const MAX_TAG_VALUES: usize = 20;

/// One payee the journal uses, for description completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayeeUse {
    /// The payee: the description before `|`, trimmed.
    pub payee: String,
    /// How many transactions use it.
    pub count: usize,
    /// The newest of their dates.
    pub last: String,
}

/// One transaction tag name the journal uses, with its commonest values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagUse {
    /// The tag name.
    pub name: String,
    /// How many transaction headers carry it.
    pub count: usize,
    /// Its non-empty values, commonest first (then by value), at most
    /// [`MAX_TAG_VALUES`].
    pub values: Vec<(String, usize)>,
}

/// Every payee in `journal`, most used first (then newest, then by name).
/// Forecast-generated transactions are not anyone's payee history.
#[must_use]
pub fn payees(journal: &Journal) -> Vec<PayeeUse> {
    let mut found: HashMap<&str, (usize, &str)> = HashMap::new();
    for txn in journal.transactions.iter().filter(|txn| !is_generated(txn)) {
        let payee = payee_of(&txn.description);
        if payee.is_empty() {
            continue;
        }
        let entry = found.entry(payee).or_insert((0, txn.date.as_str()));
        entry.0 += 1;
        entry.1 = entry.1.max(txn.date.as_str());
    }
    let mut payees: Vec<PayeeUse> = found
        .into_iter()
        .map(|(payee, (count, last))| PayeeUse {
            payee: payee.to_string(),
            count,
            last: last.to_string(),
        })
        .collect();
    payees.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| b.last.cmp(&a.last))
            .then_with(|| a.payee.cmp(&b.payee))
    });
    payees
}

/// Every transaction-header tag name in `journal`, most used first (then by
/// name), each with its commonest values. Hidden tags (`_name`, hledger's
/// machinery) and forecast-generated transactions are left out.
#[must_use]
pub fn tag_uses(journal: &Journal) -> Vec<TagUse> {
    let mut found: HashMap<&str, (usize, HashMap<&str, usize>)> = HashMap::new();
    for txn in journal.transactions.iter().filter(|txn| !is_generated(txn)) {
        for (name, value) in &txn.tags {
            if name.is_empty() || name.starts_with('_') {
                continue;
            }
            let entry = found.entry(name.as_str()).or_default();
            entry.0 += 1;
            let value = value.trim();
            if !value.is_empty() {
                *entry.1.entry(value).or_default() += 1;
            }
        }
    }
    let mut tags: Vec<TagUse> = found
        .into_iter()
        .map(|(name, (count, values))| {
            let mut values: Vec<(String, usize)> = values
                .into_iter()
                .map(|(value, count)| (value.to_string(), count))
                .collect();
            values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            values.truncate(MAX_TAG_VALUES);
            TagUse {
                name: name.to_string(),
                count,
                values,
            }
        })
        .collect();
    tags.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_date_is_judged_by_shape() {
        assert!(is_iso_date("2026-10-30"));
        assert!(!is_iso_date("2026-10-3"));
        assert!(!is_iso_date("2026/10/30"));
        assert!(!is_iso_date("yes"));
    }

    #[test]
    fn fullwidth_forms_fold_to_ascii() {
        assert_eq!(
            similarity_key("ＳＱ ＊ＢＬＵＥ　ＢＯＴＴＬＥ ４４１２"),
            Some("sq blue bottle".to_string())
        );
    }

    #[test]
    fn vote_confidence_is_exact_at_the_threshold() {
        let vote = Vote {
            account: "expenses:x",
            weight: 1,
            count: 1,
        };
        assert_eq!(vote.confidence(2, 1, 1), STRONG);
        assert_eq!(vote.confidence(3, 1, 1), 3_333);
        // A unanimous prefix sharing 3 of 4 tokens: 80% × 3/4.
        let unanimous = Vote { weight: 4, ..vote };
        assert_eq!(unanimous.confidence(4, 80 * 3, 100 * 4), 6_000);
    }

    #[test]
    fn recurring_tolerance_is_one_percent_inclusive() {
        let typical = Dec::new(1_000, 2);
        assert!(within_tolerance(Dec::new(1_010, 2), typical));
        assert!(within_tolerance(Dec::new(990, 2), typical));
        assert!(!within_tolerance(Dec::new(1_011, 2), typical));
    }

    #[test]
    fn rank_keeps_one_suggestion_per_account_at_its_best() {
        let suggestion = |account: &str, source, confidence| Suggestion {
            account: account.to_string(),
            description: None,
            source,
            confidence,
            count: 1,
        };
        let ranked = rank(vec![
            suggestion("a", SuggestionSource::ExactKey, 4_000),
            suggestion("b", SuggestionSource::Recurring, 6_000),
            suggestion("a", SuggestionSource::Recurring, 4_000),
            suggestion("c", SuggestionSource::ExactKey, 1_000),
            suggestion("d", SuggestionSource::ExactKey, 500),
        ]);
        let order: Vec<(&str, SuggestionSource)> = ranked
            .iter()
            .map(|s| (s.account.as_str(), s.source))
            .collect();
        assert_eq!(
            order,
            vec![
                ("b", SuggestionSource::Recurring),
                ("a", SuggestionSource::ExactKey),
                ("c", SuggestionSource::ExactKey),
            ]
        );
    }
}
