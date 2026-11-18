//! Everything the journal and the recognition file say about schedules, in
//! one pass (plan 45 "Performance": `RecognitionIndex` build).
//!
//! [`RecognitionIndex::build`] walks the transactions and their postings' own
//! tags once. It collects, per schedule id (interned to a `u32`):
//!
//! - the **source postings** (deferred/prepaid lines tagged `recognition:`);
//! - the **rules** in the recognition file, and their occurrences as day
//!   numbers;
//! - the **posted entries**, by `(id, recognized)`;
//! - the restatement **adjustments**.
//!
//! Plus schedules by customer/vendor (D-renewal), the payee → customer map,
//! the deferrals that have no schedule yet ("Needs a schedule"), and the
//! integrity issues behind the Problems findings. Immutable once built, and
//! `Sync`, so a snapshot can hold one in a `OnceLock`.

use super::book::RecognitionBook;
use super::schedule::{
    Party, PartyKind, Proration, ScheduleLine, ScheduleSpec, Term, occurrence_days,
};
use super::{
    AccrualRole, AccrualRoles, CALENDAR_VALUE, CONTRACT_TAG, CUSTOMER_TAG, Contract, NONE_VALUE,
    PRORATION_TAG, RECOGNITION_TAG, RECOGNIZED_TAG, RENEWAL_DATE_TAG, RESTATED_TAG,
    RESTATED_THROUGH_TAG, SERVICE_END_TAG, SERVICE_START_TAG, TERM_MONTHS_TAG, VENDOR_TAG, day_of,
    is_iso_date, iso_of, tag_value,
};
use crate::decimal::Dec;
use crate::matching::PostingRef;
use crate::model::{Amount, Journal, Posting, PostingType, Tindex, Transaction};
use crate::projections::is_generated;
use crate::review::similarity_key;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A deferred or prepaid posting tagged with a schedule id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLine {
    /// Where it is.
    pub at: PostingRef,
    /// Its account.
    pub account: String,
    /// Its one amount (the first, for a posting with several).
    pub amount: Amount,
    /// The source transaction's date.
    pub date: String,
    /// `service-start:`, else the transaction date.
    pub service_start: String,
    /// `term-months:` or `service-end:`, when either is readable.
    pub term: Option<Term>,
    /// `proration:`.
    pub proration: Proration,
    /// `contract:`, when readable.
    pub contract: Option<Contract>,
    /// The posting's `customer:`/`vendor:`, else the transaction's.
    pub party: Option<Party>,
}

/// A posted recognition entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedEntry {
    /// Its position in `journal.transactions`.
    pub position: usize,
    /// Its `tindex`.
    pub tindex: Tindex,
    /// The scheduled date it fills (`recognized:`).
    pub recognized: String,
    /// Its transaction date (which the user may have moved).
    pub date: String,
}

/// A restatement's catch-up adjustment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adjustment {
    /// Its position in `journal.transactions`.
    pub position: usize,
    /// Its `tindex`.
    pub tindex: Tindex,
    /// Its date.
    pub date: String,
    /// `restated:`.
    pub restated: String,
    /// `restated-through:`: no occurrence on or before it is due again.
    pub through: Option<String>,
}

/// One schedule as the journal and the recognition file describe it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScheduleInfo {
    /// The id.
    pub id: String,
    /// Its source postings, in file order.
    pub sources: Vec<SourceLine>,
    /// Indices into the book's rules, in file order.
    pub rules: Vec<usize>,
    /// Every occurrence of its rules, as `(day, rule index into the book)`,
    /// in date order. `None` when a rule has no bounded set of dates.
    pub occurrences: Option<Vec<(i32, usize)>>,
    /// Its posted entries, in file order.
    pub posted: Vec<PostedEntry>,
    /// Its adjustments, in file order.
    pub adjustments: Vec<Adjustment>,
}

impl ScheduleInfo {
    /// The latest `restated-through` of its adjustments: nothing on or before
    /// it is ever due again.
    #[must_use]
    pub fn restated_through(&self) -> Option<&str> {
        self.adjustments
            .iter()
            .filter_map(|adjustment| adjustment.through.as_deref())
            .max()
    }

    /// The service start of its first source line, else its first occurrence.
    #[must_use]
    pub fn start(&self) -> Option<String> {
        self.sources
            .first()
            .map(|source| source.service_start.clone())
            .or_else(|| {
                self.occurrences
                    .as_ref()
                    .and_then(|o| o.first())
                    .map(|(day, _)| iso_of(*day))
            })
    }

    /// Its customer or vendor, from its first attributed source line.
    #[must_use]
    pub fn party(&self) -> Option<&Party> {
        self.sources.iter().find_map(|source| source.party.as_ref())
    }
}

/// What is wrong with a schedule (the `recognition-*` findings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// Two or more transactions carry the same `(recognition, recognized)`.
    Duplicate {
        /// The id.
        id: String,
        /// The period.
        recognized: String,
        /// Every transaction carrying it.
        positions: Vec<usize>,
    },
    /// What the schedule will have recognized once fully posted (its posted
    /// entries, its adjustments, and its periods still to post after its
    /// restated cut) doesn't add up to its source line(s).
    SumMismatch {
        /// The id.
        id: String,
        /// The source postings' position, to anchor the finding.
        position: Option<usize>,
        /// What differs.
        detail: String,
    },
    /// A posted entry's amount differs from its rule's, or it fills a period
    /// the rules don't have.
    EntryMismatch {
        /// The id.
        id: String,
        /// The entry.
        position: usize,
        /// What differs.
        detail: String,
    },
    /// A posted entry or adjustment names an id with no rule.
    OrphanEntry {
        /// The id.
        id: String,
        /// The entry.
        position: usize,
    },
    /// A source posting names an id with no rule.
    OrphanSource {
        /// The id.
        id: String,
        /// The posting.
        at: PostingRef,
    },
    /// A rule names an id no source posting carries.
    OrphanRule {
        /// The id.
        id: String,
    },
    /// A rule posts to an account the journal doesn't declare.
    RuleAccount {
        /// The id.
        id: String,
        /// The undeclared account.
        account: String,
    },
}

/// The index (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct RecognitionIndex {
    schedules: Vec<ScheduleInfo>,
    by_id: HashMap<String, u32>,
    posted: HashMap<(u32, i32), Vec<usize>>,
    by_party: BTreeMap<(PartyKindKey, String), Vec<u32>>,
    payee_parties: HashMap<String, Party>,
    needs_schedule: Vec<PostingRef>,
    issues: Vec<Issue>,
    roles: AccrualRoles,
}

/// [`PartyKind`] as an ordered key.
type PartyKindKey = u8;

fn kind_key(kind: PartyKind) -> PartyKindKey {
    match kind {
        PartyKind::Customer => 0,
        PartyKind::Vendor => 1,
    }
}

/// The posting's `customer:`/`vendor:` (its own tag first), else the
/// transaction's.
fn party_of(txn: &Transaction, posting: &Posting) -> Option<Party> {
    let find = |tags: &[(String, String)]| {
        [
            (CUSTOMER_TAG, PartyKind::Customer),
            (VENDOR_TAG, PartyKind::Vendor),
        ]
        .into_iter()
        .find_map(|(tag, kind)| {
            tag_value(tags, tag)
                .filter(|name| !name.is_empty())
                .map(|name| Party {
                    kind,
                    name: name.to_string(),
                })
        })
    };
    find(&posting.tags).or_else(|| find(&txn.tags))
}

fn source_line(at: PostingRef, txn: &Transaction, posting: &Posting) -> Option<SourceLine> {
    let amount = posting.amounts.first()?.clone();
    let tags = &posting.tags;
    let service_start = tag_value(tags, SERVICE_START_TAG)
        .filter(|date| is_iso_date(date))
        .map_or_else(|| txn.date.clone(), str::to_string);
    let term = tag_value(tags, SERVICE_END_TAG)
        .filter(|date| is_iso_date(date))
        .map(|date| Term::Through(date.to_string()))
        .or_else(|| {
            tag_value(tags, TERM_MONTHS_TAG)
                .and_then(|months| months.parse::<u32>().ok())
                .filter(|months| *months >= 1)
                .map(Term::Months)
        });
    let proration = match tag_value(tags, PRORATION_TAG) {
        Some(value) if value.eq_ignore_ascii_case(CALENDAR_VALUE) => Proration::Calendar,
        _ => Proration::Anniversary,
    };
    Some(SourceLine {
        at,
        account: posting.account.0.clone(),
        amount,
        date: txn.date.clone(),
        service_start,
        term,
        proration,
        contract: tag_value(tags, CONTRACT_TAG).and_then(Contract::parse),
        party: party_of(txn, posting),
    })
}

/// Per source account, the quantity (one commodity each) `postings` move it
/// by, negated: what a rule or entry recognizes out of it.
fn recognized_by(postings: &[Posting], accounts: &[&str]) -> Vec<BTreeMap<String, Dec>> {
    accounts
        .iter()
        .map(|account| {
            postings
                .iter()
                .filter(|posting| posting.account.0 == *account)
                .flat_map(|posting| &posting.amounts)
                .fold(
                    BTreeMap::new(),
                    |mut sums: BTreeMap<String, Dec>, amount| {
                        let sum = sums
                            .entry(amount.commodity.0.clone())
                            .or_insert(Dec::zero());
                        *sum = sum.sub(amount.quantity).unwrap_or(*sum);
                        sums
                    },
                )
        })
        .collect()
}

fn normalized(sums: Vec<BTreeMap<String, Dec>>) -> Vec<BTreeMap<String, Dec>> {
    sums.into_iter()
        .map(|sums| {
            sums.into_iter()
                .filter(|(_, value)| !value.is_zero())
                .map(|(commodity, value)| (commodity, value.normalized()))
                .collect()
        })
        .collect()
}

fn add_into(total: &mut [BTreeMap<String, Dec>], more: &[BTreeMap<String, Dec>]) {
    for (into, from) in total.iter_mut().zip(more) {
        for (commodity, value) in from {
            let sum = into.entry(commodity.clone()).or_insert(Dec::zero());
            *sum = sum.add(*value).unwrap_or(*sum);
        }
    }
}

impl RecognitionIndex {
    /// Index `journal` and `book`, resolving accrual roles itself.
    #[must_use]
    pub fn build(journal: &Journal, book: &RecognitionBook) -> Self {
        Self::build_with(journal, book, AccrualRoles::of(journal))
    }

    /// Index `journal` and `book` with roles already resolved (a snapshot
    /// caches them).
    #[must_use]
    pub fn build_with(journal: &Journal, book: &RecognitionBook, roles: AccrualRoles) -> Self {
        let mut index = Self {
            roles,
            ..Self::default()
        };
        let intern = |index: &mut Self, id: &str| -> u32 {
            if let Some(&n) = index.by_id.get(id) {
                return n;
            }
            let n = u32::try_from(index.schedules.len()).unwrap_or(u32::MAX);
            index.by_id.insert(id.to_string(), n);
            index.schedules.push(ScheduleInfo {
                id: id.to_string(),
                ..ScheduleInfo::default()
            });
            n
        };
        let mut payee_counts: HashMap<String, BTreeMap<(PartyKindKey, String), usize>> =
            HashMap::new();
        for (position, txn) in journal.transactions.iter().enumerate() {
            if is_generated(txn) {
                continue;
            }
            if let Some(id) = tag_value(&txn.tags, RECOGNITION_TAG) {
                let recognized = tag_value(&txn.tags, RECOGNIZED_TAG);
                let restated = tag_value(&txn.tags, RESTATED_TAG);
                if recognized.is_some() || restated.is_some() {
                    let n = intern(&mut index, id);
                    let schedule = &mut index.schedules[n as usize];
                    if let Some(recognized) = recognized {
                        schedule.posted.push(PostedEntry {
                            position,
                            tindex: txn.index,
                            recognized: recognized.to_string(),
                            date: txn.date.clone(),
                        });
                        index
                            .posted
                            .entry((n, day_of(recognized)))
                            .or_default()
                            .push(position);
                    } else if let Some(restated) = restated {
                        schedule.adjustments.push(Adjustment {
                            position,
                            tindex: txn.index,
                            date: txn.date.clone(),
                            restated: restated.to_string(),
                            through: tag_value(&txn.tags, RESTATED_THROUGH_TAG)
                                .filter(|date| is_iso_date(date))
                                .map(str::to_string),
                        });
                    }
                    continue;
                }
            }
            let mut party_here: Option<Party> = None;
            for (at, posting) in txn.postings.iter().enumerate() {
                if posting.generated.is_some() || posting.ptype != PostingType::Regular {
                    continue;
                }
                let at = PostingRef {
                    position,
                    txn: txn.index,
                    posting: at,
                };
                match tag_value(&posting.tags, RECOGNITION_TAG) {
                    Some(NONE_VALUE) => {}
                    Some(id) if !id.is_empty() => {
                        if let Some(line) = source_line(at, txn, posting) {
                            let n = intern(&mut index, id);
                            index.schedules[n as usize].sources.push(line);
                        }
                    }
                    _ => {
                        let deferral = match index.roles.role(&posting.account.0) {
                            Some(AccrualRole::DeferredRevenue) => {
                                posting.amounts.iter().any(|a| a.quantity.mantissa < 0)
                            }
                            Some(AccrualRole::Prepaid) => {
                                posting.amounts.iter().any(|a| a.quantity.mantissa > 0)
                            }
                            _ => false,
                        };
                        if deferral {
                            index.needs_schedule.push(at);
                        }
                    }
                }
                if party_here.is_none() {
                    party_here = party_of(txn, posting);
                }
            }
            if let Some(party) = party_here
                && let Some(key) = similarity_key(&txn.description)
            {
                *payee_counts
                    .entry(key)
                    .or_default()
                    .entry((kind_key(party.kind), party.name))
                    .or_default() += 1;
            }
        }
        // The rules.
        for (at, rule) in book.rules.iter().enumerate() {
            if let Some(id) = tag_value(&rule.tags, RECOGNITION_TAG) {
                let n = intern(&mut index, id);
                index.schedules[n as usize].rules.push(at);
            }
        }
        for schedule in &mut index.schedules {
            schedule.occurrences = schedule
                .rules
                .iter()
                .map(|&rule| {
                    occurrence_days(&book.rules[rule].period, None).map(|days| {
                        days.into_iter()
                            .map(move |day| (day, rule))
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Option<Vec<Vec<_>>>>()
                .map(|all| {
                    let mut flat: Vec<(i32, usize)> = all.into_iter().flatten().collect();
                    flat.sort_unstable();
                    flat
                });
        }
        // Parties, latest service start last.
        for (n, schedule) in index.schedules.iter().enumerate() {
            if let Some(party) = schedule.party() {
                index
                    .by_party
                    .entry((kind_key(party.kind), party.name.clone()))
                    .or_default()
                    .push(u32::try_from(n).unwrap_or(u32::MAX));
            }
        }
        let starts: Vec<Option<String>> = index.schedules.iter().map(ScheduleInfo::start).collect();
        for ids in index.by_party.values_mut() {
            ids.sort_by(|a, b| starts[*a as usize].cmp(&starts[*b as usize]));
        }
        index.payee_parties = payee_counts
            .into_iter()
            .filter_map(|(key, counts)| {
                let best = counts.iter().max_by_key(|(_, count)| **count)?;
                let ((kind, name), _) = best;
                Some((
                    key,
                    Party {
                        kind: if *kind == 0 {
                            PartyKind::Customer
                        } else {
                            PartyKind::Vendor
                        },
                        name: name.clone(),
                    },
                ))
            })
            .collect();
        index.issues = index.integrity(journal, book);
        index
    }

    fn integrity(&self, journal: &Journal, book: &RecognitionBook) -> Vec<Issue> {
        let declared: BTreeSet<&str> = journal
            .accounts
            .iter()
            .map(|decl| decl.name.0.as_str())
            .collect();
        let mut issues = Vec::new();
        let mut duplicates: Vec<(&(u32, i32), &Vec<usize>)> = self
            .posted
            .iter()
            .filter(|(_, positions)| positions.len() > 1)
            .collect();
        duplicates.sort();
        issues.extend(
            duplicates
                .into_iter()
                .map(|(&(n, day), positions)| Issue::Duplicate {
                    id: self.schedules[n as usize].id.clone(),
                    recognized: iso_of(day),
                    positions: positions.clone(),
                }),
        );
        for schedule in &self.schedules {
            let id = &schedule.id;
            if schedule.rules.is_empty() {
                issues.extend(
                    schedule
                        .posted
                        .iter()
                        .map(|entry| entry.position)
                        .chain(schedule.adjustments.iter().map(|a| a.position))
                        .map(|position| Issue::OrphanEntry {
                            id: id.clone(),
                            position,
                        }),
                );
                issues.extend(schedule.sources.iter().map(|source| Issue::OrphanSource {
                    id: id.clone(),
                    at: source.at,
                }));
                continue;
            }
            if schedule.sources.is_empty() {
                issues.push(Issue::OrphanRule { id: id.clone() });
            }
            if !declared.is_empty() {
                let undeclared: BTreeSet<&str> = schedule
                    .rules
                    .iter()
                    .flat_map(|&rule| &book.rules[rule].postings)
                    .map(|posting| posting.account.0.as_str())
                    .filter(|account| !declared.contains(account))
                    .collect();
                issues.extend(undeclared.into_iter().map(|account| Issue::RuleAccount {
                    id: id.clone(),
                    account: account.to_string(),
                }));
            }
            issues.extend(self.sum_issues(journal, book, schedule));
        }
        issues
    }

    fn sum_issues(
        &self,
        journal: &Journal,
        book: &RecognitionBook,
        schedule: &ScheduleInfo,
    ) -> Vec<Issue> {
        let Some(occurrences) = &schedule.occurrences else {
            return Vec::new();
        };
        let accounts: Vec<&str> = {
            let mut seen = BTreeSet::new();
            schedule
                .sources
                .iter()
                .map(|s| s.account.as_str())
                .filter(|a| seen.insert(*a))
                .collect()
        };
        let mut issues = Vec::new();
        let per_rule: HashMap<usize, Vec<BTreeMap<String, Dec>>> = schedule
            .rules
            .iter()
            .map(|&rule| (rule, recognized_by(&book.rules[rule].postings, &accounts)))
            .collect();
        // What the schedule will have recognized once it is fully posted:
        // its posted entries (one per period), its adjustments, and every
        // occurrence still to post after its restated cut. Against the
        // sources.
        let n = self.number(&schedule.id).unwrap_or(usize::MAX);
        let cut = schedule.restated_through().map(day_of);
        let mut scheduled = vec![BTreeMap::new(); accounts.len()];
        let mut filled = BTreeSet::new();
        for entry in &schedule.posted {
            if filled.insert(entry.recognized.as_str()) {
                add_into(
                    &mut scheduled,
                    &recognized_by(&journal.transactions[entry.position].postings, &accounts),
                );
            }
        }
        for adjustment in &schedule.adjustments {
            add_into(
                &mut scheduled,
                &recognized_by(
                    &journal.transactions[adjustment.position].postings,
                    &accounts,
                ),
            );
        }
        for (day, rule) in occurrences {
            if !self.is_posted_day(n, *day) && cut.is_none_or(|cut| *day > cut) {
                add_into(&mut scheduled, &per_rule[rule]);
            }
        }
        let mut sources = vec![BTreeMap::new(); accounts.len()];
        for source in &schedule.sources {
            let at = accounts
                .iter()
                .position(|a| *a == source.account)
                .unwrap_or(0);
            let sum = sources[at]
                .entry(source.amount.commodity.0.clone())
                .or_insert(Dec::zero());
            *sum = sum.sub(source.amount.quantity).unwrap_or(*sum);
        }
        // `sources` holds the negation; the rules recognize the negation of
        // the rule posting, which has the source's sign: compare negated.
        let negate = |sums: Vec<BTreeMap<String, Dec>>| {
            sums.into_iter()
                .map(|m| {
                    m.into_iter()
                        .map(|(c, v)| (c, v.neg().unwrap_or(v)))
                        .collect::<BTreeMap<_, _>>()
                })
                .collect::<Vec<_>>()
        };
        if !schedule.sources.is_empty()
            && normalized(scheduled.clone()) != normalized(negate(sources))
        {
            issues.push(Issue::SumMismatch {
                id: schedule.id.clone(),
                position: schedule.sources.first().map(|s| s.at.position),
                detail: "its posted entries, adjustments and remaining periods don't add up to its source line".to_string(),
            });
        }
        // Each posted entry against its rule.
        let by_day: HashMap<i32, Vec<usize>> =
            occurrences
                .iter()
                .fold(HashMap::new(), |mut map, (day, rule)| {
                    map.entry(*day).or_insert_with(Vec::new).push(*rule);
                    map
                });
        for entry in &schedule.posted {
            // A closed entry a restatement kept is reconciled by its
            // adjustment, whatever the rules now say about its period.
            if cut.is_some_and(|cut| day_of(&entry.date).min(day_of(&entry.recognized)) <= cut) {
                continue;
            }
            let Some(rules) = by_day.get(&day_of(&entry.recognized)) else {
                issues.push(Issue::EntryMismatch {
                    id: schedule.id.clone(),
                    position: entry.position,
                    detail: format!(
                        "it fills {}, a period the schedule doesn't have",
                        entry.recognized
                    ),
                });
                continue;
            };
            let mut expected = vec![BTreeMap::new(); accounts.len()];
            for rule in rules {
                add_into(&mut expected, &per_rule[rule]);
            }
            let posted = recognized_by(&journal.transactions[entry.position].postings, &accounts);
            if normalized(expected) != normalized(posted) {
                issues.push(Issue::EntryMismatch {
                    id: schedule.id.clone(),
                    position: entry.position,
                    detail: format!(
                        "its amount differs from the schedule's for {}",
                        entry.recognized
                    ),
                });
            }
        }
        issues
    }

    /// Every schedule, in order of first appearance (journal first, then the
    /// recognition file).
    #[must_use]
    pub fn schedules(&self) -> &[ScheduleInfo] {
        &self.schedules
    }

    /// The schedule with `id`.
    #[must_use]
    pub fn schedule(&self, id: &str) -> Option<&ScheduleInfo> {
        self.by_id.get(id).map(|&n| &self.schedules[n as usize])
    }

    /// The transactions posted for `(id, recognized)`.
    #[must_use]
    pub fn posted(&self, id: &str, recognized: &str) -> &[usize] {
        self.by_id
            .get(id)
            .and_then(|&n| self.posted.get(&(n, day_of(recognized))))
            .map_or(&[], Vec::as_slice)
    }

    /// Whether `(id, recognized)` has been posted.
    #[must_use]
    pub fn is_posted(&self, id: &str, recognized: &str) -> bool {
        !self.posted(id, recognized).is_empty()
    }

    /// Whether the occurrence `day` of the schedule interned as `n` is posted.
    pub(crate) fn is_posted_day(&self, n: usize, day: i32) -> bool {
        u32::try_from(n).is_ok_and(|n| self.posted.contains_key(&(n, day)))
    }

    /// A party's schedules, by service start, latest last (D-renewal).
    #[must_use]
    pub fn schedules_of(&self, party: &Party) -> Vec<&ScheduleInfo> {
        self.by_party
            .get(&(kind_key(party.kind), party.name.clone()))
            .map(|ids| ids.iter().map(|&n| &self.schedules[n as usize]).collect())
            .unwrap_or_default()
    }

    /// A party's latest schedule (by service start), whose terms prefill a
    /// renewal.
    #[must_use]
    pub fn latest_schedule_of(&self, party: &Party) -> Option<&ScheduleInfo> {
        self.schedules_of(party).last().copied()
    }

    /// The customer or vendor most often seen with this payee similarity key
    /// in tagged history (recognition entries aside): a suggestion, never
    /// applied silently.
    #[must_use]
    pub fn party_for_payee(&self, description: &str) -> Option<&Party> {
        similarity_key(description).and_then(|key| self.payee_parties.get(&key))
    }

    /// Deferred-revenue credits and prepaid debits with no `recognition:` tag,
    /// in file order ("Needs a schedule"). The caller applies its lookback.
    #[must_use]
    pub fn needs_schedule(&self) -> &[PostingRef] {
        &self.needs_schedule
    }

    /// The integrity issues, in a stable order.
    #[must_use]
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    /// The accrual roles the index was built with.
    #[must_use]
    pub fn roles(&self) -> &AccrualRoles {
        &self.roles
    }

    /// The spec `id`'s schedule was built from, read back from its source
    /// postings and its rules: what a restatement compares against and what a
    /// renewal prefills from (D-renewal). `None` when the schedule has no
    /// source posting, no rule, or no readable term.
    #[must_use]
    pub fn spec_of(&self, id: &str, book: &RecognitionBook) -> Option<ScheduleSpec> {
        let info = self.schedule(id)?;
        let first = info.sources.first()?;
        let rules: Vec<&crate::model::PeriodicTransaction> = info
            .rules
            .iter()
            .filter_map(|&at| book.rules.get(at))
            .collect();
        let main = rules
            .iter()
            .find(|rule| tag_value(&rule.tags, RENEWAL_DATE_TAG).is_some())
            .or_else(|| rules.first())?;
        let sources: BTreeSet<&str> = info.sources.iter().map(|s| s.account.as_str()).collect();
        let targets: Vec<&str> = main
            .postings
            .iter()
            .map(|p| p.account.0.as_str())
            .filter(|account| !sources.contains(account))
            .collect();
        Some(ScheduleSpec {
            id: id.to_string(),
            lines: info
                .sources
                .iter()
                .enumerate()
                .map(|(i, source)| {
                    Some(ScheduleLine {
                        source_account: source.account.clone(),
                        target_account: targets.get(i).or_else(|| targets.first())?.to_string(),
                        total: source.amount.clone(),
                        party: source.party.clone(),
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            start: first.service_start.clone(),
            term: first.term.clone()?,
            proration: first.proration,
            description: main.description.clone(),
            header_tags: main
                .tags
                .iter()
                .filter(|(name, _)| name != RECOGNITION_TAG && name != RENEWAL_DATE_TAG)
                .cloned()
                .collect(),
        })
    }

    /// The interned number of `id`.
    pub(crate) fn number(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).map(|&n| n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;
    use std::path::Path;

    const BOOKS: &str = "\
account liabilities:deferred-revenue  ; type: L, kpi: deferred-revenue
account revenue:sales  ; type: R
account assets:bank  ; type: C

2026-01-05 Customer X Payment
    liabilities:deferred-revenue  $-300.00  ; customer: Customer X, recognition: aaa111, term-months: 3
    assets:bank

2026-01-05 Customer X Recognized  ; recognition: aaa111, recognized: 2026-01-05
    liabilities:deferred-revenue  $100.00  ; customer: Customer X
    revenue:sales  $-100.00  ; customer: Customer X

2026-02-04 Customer X Recognized  ; recognition: aaa111, recognized: 2026-02-05
    liabilities:deferred-revenue  $100.00  ; customer: Customer X
    revenue:sales  $-100.00  ; customer: Customer X

2026-02-05 Copy  ; recognition: aaa111, recognized: 2026-02-05
    liabilities:deferred-revenue  $100.00
    revenue:sales  $-100.00

2026-02-10 Customer Y Payment
    liabilities:deferred-revenue  $-50.00  ; customer: Customer Y
    assets:bank
";

    const RULES: &str = "\
~ monthly from 2026-01-05 to 2026-04-05  Customer X Recognized  ; recognition: aaa111
    liabilities:deferred-revenue  $100.00  ; customer: Customer X
    revenue:sales  $-100.00  ; customer: Customer X
~ 2026-05-01  Orphan  ; recognition: zzz999
    liabilities:deferred-revenue  $1.00
    revenue:other  $-1.00
";

    fn index() -> (Journal, RecognitionIndex) {
        let journal = parse_journal(BOOKS, "books.journal").expect("parses");
        let book = RecognitionBook::parse(
            RULES,
            Path::new("/nonexistent/r.journal"),
            &journal.commodity_styles,
        )
        .expect("parses");
        let index = RecognitionIndex::build(&journal, &book);
        (journal, index)
    }

    #[test]
    fn posted_entries_are_keyed_by_the_recognized_date_not_the_transaction_date() {
        let (_, index) = index();
        assert!(index.is_posted("aaa111", "2026-02-05"));
        assert!(!index.is_posted("aaa111", "2026-02-04"));
        assert!(!index.is_posted("aaa111", "2026-03-05"));
        let schedule = index.schedule("aaa111").expect("indexed");
        assert_eq!(schedule.sources.len(), 1);
        assert_eq!(schedule.sources[0].term, Some(Term::Months(3)));
        assert_eq!(schedule.occurrences.as_ref().map(Vec::len), Some(3));
    }

    #[test]
    fn duplicates_orphans_and_undeclared_accounts_are_issues() {
        let (_, index) = index();
        let issues = index.issues();
        assert!(issues.iter().any(|issue| matches!(
            issue,
            Issue::Duplicate { id, recognized, positions } if id == "aaa111" && recognized == "2026-02-05" && positions == &vec![2, 3]
        )));
        assert!(issues.contains(&Issue::OrphanRule {
            id: "zzz999".into()
        }));
        assert!(issues.contains(&Issue::RuleAccount {
            id: "zzz999".into(),
            account: "revenue:other".into()
        }));
        // The copy has no customer tag but the same amounts: no mismatch.
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i, Issue::SumMismatch { id, .. } if id == "aaa111"))
        );
    }

    #[test]
    fn untagged_deferrals_need_a_schedule_and_parties_are_indexed() {
        let (journal, index) = index();
        let needs: Vec<&str> = index
            .needs_schedule()
            .iter()
            .map(|at| journal.transactions[at.position].description.as_str())
            .collect();
        assert_eq!(needs, vec!["Customer Y Payment"]);
        let x = Party {
            kind: PartyKind::Customer,
            name: "Customer X".into(),
        };
        assert_eq!(
            index.latest_schedule_of(&x).map(|s| s.id.as_str()),
            Some("aaa111")
        );
        assert_eq!(
            index
                .party_for_payee("Customer Y Payment")
                .map(|p| p.name.as_str()),
            Some("Customer Y")
        );
    }
}
