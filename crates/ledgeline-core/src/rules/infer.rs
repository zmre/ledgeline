//! Which rules file produced a journal transaction (plan 32 D27: "infer the
//! rules file, don't record it"), so a review card in the Journal view can
//! offer **Remember as a rule** without any new metadata in the journal.
//!
//! Three signals already in the ledger, in this order:
//!
//! 1. **The funding account.** A rules file names its bank account
//!    (`account1`, at the top level or in a block every record matches). A
//!    transaction's balance-sheet posting — the one that is neither a
//!    category nor uncategorised — names the files that could have written
//!    it. Only these are candidates at all.
//! 2. **The FITID convention.** A transaction carrying an `id:` tag came from a
//!    file that writes one (`comment id:%fitid`), and one without from a file
//!    that doesn't. This narrows a tie; it never adds a candidate.
//! 3. **`.latest.NAME` markers.** A file whose marker stops before the
//!    transaction's date did not import it. This narrows a tie too.
//!
//! Candidates are then grouped into **families** ([`super::families`]): one
//! family left is a recommendation (its pick for the transaction's date);
//! several are a genuine tie, so none is recommended and the user picks. The
//! family order is the one the import's own ranking uses
//! ([`super::matching::rank_in_families`]).
//!
//! [`RulesIndex::build`] reads every discovered rules file once (and its
//! marker); [`rules_for`] is then pure.

use super::families::{Family, Member, Placement, family_of, order_family, pick_is_decided};
use super::{Discovery, HledgerField, ItemKind, MatchScope, NumberedField, RulesDoc};
use crate::model::{Journal, Transaction};
use crate::projections::is_generated;
use crate::reimport::{ID_TAG, id_of};
use crate::review::{funding_accounts, matcher_regex};
use crate::view::Classes;
use std::collections::BTreeMap;
use std::time::SystemTime;

/// The field a rule built from a journal description matches, when the rules
/// file's `fields` list names it.
pub const DESCRIPTION_FIELD: &str = "description";

/// One rules file, as inference reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedRules {
    /// [`super::DiscoveredRules::id`].
    pub id: String,
    /// [`super::DiscoveredRules::label`].
    pub label: String,
    /// The account the file books the statement's side to ([`effective_account1`]).
    pub account1: Option<String>,
    /// It writes an `id:` tag from a field reference ([`writes_id`]).
    pub writes_id: bool,
    /// Its `fields` list, names as written, or `None`.
    pub fields: Option<Vec<String>>,
    /// Its `fields` list names `description`.
    pub names_description: bool,
    /// Its family, from the id.
    pub family: Family,
    /// The newest date its `.latest.NAME` marker records.
    pub latest: Option<String>,
    /// Its mtime. Ranking only.
    pub modified: Option<SystemTime>,
}

impl IndexedRules {
    /// What inference needs from the rules file `id`, whose parsed text is
    /// `doc`.
    #[must_use]
    pub fn describe(
        id: &str,
        doc: &RulesDoc,
        latest: Option<String>,
        modified: Option<SystemTime>,
    ) -> Self {
        let fields = doc.settings().fields.map(|fields| fields.value);
        Self {
            id: id.to_string(),
            label: super::label_for(id),
            account1: effective_account1(doc),
            writes_id: writes_id(doc),
            names_description: fields.as_ref().is_some_and(|fields| {
                fields
                    .iter()
                    .any(|name| name.trim().eq_ignore_ascii_case(DESCRIPTION_FIELD))
            }),
            fields,
            family: family_of(id),
            latest,
            modified,
        }
    }
}

/// Every rules file in the journal's tree, read once for inference.
#[derive(Debug, Clone, Default)]
pub struct RulesIndex {
    files: Vec<IndexedRules>,
}

impl RulesIndex {
    /// Read every parsed file `discovery` found, and its `.latest` marker.
    #[must_use]
    pub fn build(discovery: &Discovery) -> Self {
        Self::new(
            discovery
                .files
                .iter()
                .filter(|found| found.parsed)
                .filter_map(|found| {
                    let doc = RulesDoc::parse(&found.path().read_text()?);
                    Some(IndexedRules::describe(
                        &found.id,
                        &doc,
                        discovery.latest_date(&found.id),
                        found.modified,
                    ))
                })
                .collect(),
        )
    }

    /// An index over `files` as given.
    #[must_use]
    pub fn new(files: Vec<IndexedRules>) -> Self {
        Self { files }
    }

    /// The file with this id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&IndexedRules> {
        self.files.iter().find(|file| file.id == id)
    }

    /// Every file, in discovery order.
    #[must_use]
    pub fn files(&self) -> &[IndexedRules] {
        &self.files
    }
}

/// One rules file that could have produced a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesCandidate {
    /// The rules id — what the rules PUT and the rule preview take.
    pub id: String,
    /// [`IndexedRules::label`].
    pub label: String,
    /// The family's display name.
    pub family: String,
    /// How many candidates share the family.
    pub family_size: usize,
    /// The year the file is named for.
    pub year: Option<u16>,
    /// The account that made it a candidate.
    pub account1: String,
    /// Which signals matched, in plain words for the builder.
    pub reasons: Vec<String>,
    /// The one to save to unless the user picks another. At most one per
    /// list; none when two families tie.
    pub recommended: bool,
    /// The field a matcher built from the description should scope to:
    /// `description` when the file's `fields` names it, else `None` (the
    /// whole record).
    pub match_field: Option<String>,
    /// The file's `fields` names, for the builder's field select, or `None`.
    pub fields: Option<Vec<String>>,
}

/// What [`rules_for`] found for one transaction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RulesInference {
    /// Best first. Empty: no rules file books to the transaction's account,
    /// and "Remember as a rule" is not offered.
    pub candidates: Vec<RulesCandidate>,
    /// Candidates exist but the D27 rules leave more than one possible file,
    /// so none is recommended: the user picks, with nothing preselected, and
    /// no rule is saved until they do.
    pub needs_choice: bool,
}

/// One family's candidates, ordered, with the family-level signals.
struct Group<'a> {
    label: &'a str,
    key: &'a str,
    order: Vec<(&'a IndexedRules, Placement)>,
    /// The family's pick was decided by a rule, not by the id alone.
    decided: bool,
    /// The FITID convention agrees with the transaction for some member.
    fitid: bool,
    /// Some member's marker reaches the transaction's date, or has none.
    window: bool,
}

/// The rules files that could have produced `txn`, best first.
///
/// No candidates when no file books to `txn`'s funding account — the toggle
/// is then hidden. One is `recommended` only when the D27 rules leave exactly
/// one winner (one family left after the FITID and `.latest` signals, whose
/// year rule picks one file); otherwise [`RulesInference::needs_choice`]. See
/// the module docs.
#[must_use]
pub fn rules_for(txn: &Transaction, classes: &Classes, index: &RulesIndex) -> RulesInference {
    let funding = funding_accounts(txn, classes);
    let mut families: BTreeMap<&str, Vec<&IndexedRules>> = BTreeMap::new();
    for file in index.files() {
        if file
            .account1
            .as_deref()
            .is_some_and(|account| funding.contains(&account))
        {
            families
                .entry(file.family.key.as_str())
                .or_default()
                .push(file);
        }
    }
    let has_id = id_of(txn, ID_TAG).is_some();
    let date = txn.date.as_str();
    let mut groups: Vec<Group<'_>> = families
        .into_iter()
        .map(|(key, files)| {
            let members: Vec<Member<'_>> = files
                .iter()
                .map(|file| Member {
                    id: &file.id,
                    stamp: file.family.stamp,
                    latest: file.latest.as_deref(),
                    modified: file.modified,
                })
                .collect();
            let placed = order_family(&members, Some(date));
            let decided = pick_is_decided(&members, &placed);
            let order = placed
                .into_iter()
                .map(|(at, why)| (files[at], why))
                .collect();
            Group {
                label: &files[0].family.label,
                key,
                order,
                decided,
                fitid: files.iter().any(|file| file.writes_id == has_id),
                window: files
                    .iter()
                    .any(|file| file.latest.as_deref().is_none_or(|latest| latest >= date)),
            }
        })
        .collect();
    groups.sort_by(|a, b| a.label.cmp(b.label).then_with(|| a.key.cmp(b.key)));

    // Narrow by each signal in turn, but never to nothing.
    let mut live: Vec<usize> = (0..groups.len()).collect();
    let signals: [fn(&Group<'_>) -> bool; 2] = [|group| group.fitid, |group| group.window];
    for signal in signals {
        let kept: Vec<usize> = live
            .iter()
            .copied()
            .filter(|&at| signal(&groups[at]))
            .collect();
        if !kept.is_empty() {
            live = kept;
        }
    }
    let recommended = (live.len() == 1 && groups[live[0]].decided).then(|| live[0]);
    let tied = recommended.is_none();

    let mut order: Vec<usize> = live.clone();
    order.extend((0..groups.len()).filter(|at| !live.contains(at)));
    let candidates: Vec<RulesCandidate> = order
        .into_iter()
        .flat_map(|at| {
            let group = &mut groups[at];
            let size = group.order.len();
            // With no recommendation, each live family's pick says it is tied.
            let in_tie = tied && live.contains(&at);
            std::mem::take(&mut group.order)
                .into_iter()
                .enumerate()
                .map(move |(rank, (file, why))| {
                    let first = rank == 0;
                    candidate(
                        file,
                        why,
                        size,
                        has_id,
                        date,
                        first && Some(at) == recommended,
                        first && in_tie,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    RulesInference {
        needs_choice: !candidates.is_empty() && recommended.is_none(),
        candidates,
    }
}

/// One candidate, with its reasons.
fn candidate(
    file: &IndexedRules,
    why: Placement,
    family_size: usize,
    has_id: bool,
    date: &str,
    recommended: bool,
    tied: bool,
) -> RulesCandidate {
    let account1 = file.account1.clone().unwrap_or_default();
    let mut reasons = vec![format!(
        "Imports into {account1}, this transaction's account"
    )];
    match (has_id, file.writes_id) {
        (true, true) => reasons.push("Writes an id: tag, as this transaction has".to_string()),
        (false, true) => {
            reasons.push("Writes an id: tag, which this transaction lacks".to_string())
        }
        (true, false) => {
            reasons.push("Writes no id: tag, but this transaction has one".to_string())
        }
        (false, false) => {}
    }
    match file.latest.as_deref() {
        Some(latest) if latest >= date => {
            reasons.push(format!(
                "Has imported through {latest}, so it could have written this"
            ));
        }
        Some(latest) => reasons.push(format!(
            "Has imported only through {latest}, before this transaction"
        )),
        None => {}
    }
    if family_size > 1 {
        let of = format!("of {family_size} {} files", file.family.label);
        match why {
            Placement::ForYear(year) => {
                reasons.push(format!("Named for {year}, this transaction's year"))
            }
            Placement::NearestBefore(year) => reasons.push(format!(
                "The newest year ({year}) {of} at or before this transaction's"
            )),
            Placement::NearestAfter(year) => {
                reasons.push(format!("The earliest year ({year}) {of}; none is older"))
            }
            Placement::Newest => reasons.push(format!("The most recently used {of}")),
            Placement::Ahead(year) => reasons.push(format!(
                "The newest {of} ({year}), in case you are importing ahead"
            )),
            Placement::Rest => {}
        }
    }
    if tied {
        reasons.push("Another rules file for this account fits as well".to_string());
    }
    RulesCandidate {
        id: file.id.clone(),
        label: file.label.clone(),
        family: file.family.label.clone(),
        family_size,
        year: file.family.stamp.map(|stamp| stamp.year),
        account1,
        reasons,
        recommended,
        match_field: file
            .names_description
            .then(|| DESCRIPTION_FIELD.to_string()),
        fields: file.fields.clone(),
    }
}

/// The account a rules file books the statement's own side to: the
/// `account1` of the last block that every record matches, else the
/// top-level `account1` (hledger applies a matching block's assignment over
/// the top-level one). A value with a `%` field reference names no one
/// account and counts as none.
#[must_use]
pub fn effective_account1(doc: &RulesDoc) -> Option<String> {
    let text = doc.text();
    let value = |span: &super::Span| text.get(span.clone()).unwrap_or_default().trim();
    let is_account1 = |field: HledgerField| {
        matches!(
            field,
            HledgerField::Numbered {
                base: NumberedField::Account,
                n: 1
            }
        )
    };
    let from_block = doc
        .items()
        .iter()
        .filter_map(super::Item::if_block)
        .filter(|block| block.control.is_none() && matches_every_record(block))
        .filter_map(|block| {
            block
                .assignments
                .iter()
                .rfind(|assignment| is_account1(assignment.field))
                .map(|assignment| value(&assignment.value_span))
        })
        .next_back();
    let top = doc.settings().account1.map(|setting| setting.value);
    from_block
        .map(str::to_string)
        .or(top)
        .map(|account| account.trim().to_string())
        .filter(|account| !account.is_empty() && !account.contains('%'))
}

/// Whether some OR-group of `block` matches any record at all: every one of
/// its matchers is a pattern like `.` or `.*` that matches every probe.
fn matches_every_record(block: &super::IfBlock) -> bool {
    /// A record is never empty (it has at least a delimiter or a value); a
    /// single field can be.
    const RECORD_PROBES: [&str; 4] = ["x", "0", ",", " "];
    const FIELD_PROBES: [&str; 5] = ["", "x", "0", ",", " "];
    block.groups.iter().any(|group| {
        group.matchers.iter().all(|matcher| {
            let probes: &[&str] = match matcher.scope {
                MatchScope::WholeRecord => &RECORD_PROBES,
                MatchScope::Field(_) => &FIELD_PROBES,
            };
            matcher_regex(&matcher.pattern)
                .is_ok_and(|regex| probes.iter().all(|probe| regex.is_match(probe)))
        })
    })
}

/// Whether the file assigns the transaction `comment` an `id:` tag from a
/// field reference — `comment id:%fitid`, `comment note, id:%3` — at the top
/// level or in any block.
#[must_use]
pub fn writes_id(doc: &RulesDoc) -> bool {
    let text = doc.text();
    doc.items()
        .iter()
        .flat_map(|item| match &item.kind {
            ItemKind::Assignment(assignment) => std::slice::from_ref(assignment),
            ItemKind::IfBlock(block) => block.assignments.as_slice(),
            _ => &[],
        })
        .filter(|assignment| assignment.field == HledgerField::Comment)
        .filter_map(|assignment| text.get(assignment.value_span.clone()))
        .any(tags_id_from_field)
}

/// `value` holds a tag named [`ID_TAG`] whose value starts with a `%`.
fn tags_id_from_field(value: &str) -> bool {
    let tag = format!("{ID_TAG}:");
    value.match_indices(&tag).any(|(at, _)| {
        let starts_word = value[..at]
            .chars()
            .next_back()
            .is_none_or(|before| before.is_whitespace() || before == ',');
        starts_word && value[at + tag.len()..].trim_start().starts_with('%')
    })
}

/// D27's fallback live check, when no CSV is at hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalMatch {
    /// Of `total`, how many have a description some pattern matches.
    pub matched: usize,
    /// The journal's real transactions funded from the account.
    pub total: usize,
}

/// How many of the journal's transactions funded from `account1` have a
/// description one of `patterns` matches, read as hledger reads a matcher (a
/// case-insensitive regex). Forecast transactions are not considered.
///
/// # Errors
/// The first pattern that does not compile, with the regex engine's reason.
pub fn journal_matches(
    journal: &Journal,
    classes: &Classes,
    account1: &str,
    patterns: &[&str],
) -> Result<JournalMatch, regex::Error> {
    let regexes = patterns
        .iter()
        .map(|pattern| matcher_regex(pattern))
        .collect::<Result<Vec<_>, _>>()?;
    let considered: Vec<&Transaction> = journal
        .transactions
        .iter()
        .filter(|txn| !is_generated(txn) && funding_accounts(txn, classes).contains(&account1))
        .collect();
    Ok(JournalMatch {
        matched: considered
            .iter()
            .filter(|txn| regexes.iter().any(|regex| regex.is_match(&txn.description)))
            .count(),
        total: considered.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account1_comes_from_the_top_or_a_block_every_record_matches() {
        let top = RulesDoc::parse("fields date, description, amount\naccount1 assets:bank:citi\n");
        assert_eq!(
            effective_account1(&top).as_deref(),
            Some("assets:bank:citi")
        );

        let block = RulesDoc::parse(
            "fields date, description, amount\naccount1 assets:old\n\nif .\n    account1 assets:bank:chase\n\nif COFFEE\n    account1 assets:cash\n",
        );
        assert_eq!(
            effective_account1(&block).as_deref(),
            Some("assets:bank:chase")
        );

        let reference = RulesDoc::parse("fields date, account, amount\naccount1 %account\n");
        assert_eq!(effective_account1(&reference), None);
        assert_eq!(effective_account1(&RulesDoc::parse("skip 1\n")), None);
    }

    #[test]
    fn the_fitid_convention_is_an_id_tag_from_a_field() {
        assert!(writes_id(&RulesDoc::parse(
            "fields date, fitid\ncomment id:%fitid\n"
        )));
        assert!(writes_id(&RulesDoc::parse(
            "fields a, b\ncomment note, id: %2\n"
        )));
        assert!(writes_id(&RulesDoc::parse(
            "fields date, fitid\n\nif .\n    comment id:%fitid\n"
        )));
        assert!(!writes_id(&RulesDoc::parse(
            "fields date, fitid\ncomment id:fixed\n"
        )));
        assert!(!writes_id(&RulesDoc::parse(
            "fields date, fitid\ncomment paid:%fitid\n"
        )));
        assert!(!writes_id(&RulesDoc::parse(
            "fields date, fitid\ncomment1 id:%fitid\n"
        )));
    }
}
