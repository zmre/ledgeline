//! The document index (ARCHITECTURE §5.6): every document's frontmatter and
//! bill-table summary, in memory, keyed by path, with each file's
//! `(mtime, size, content hash)`.
//!
//! - **Built at open** from one parallel scan: each note is read once, its
//!   frontmatter parsed, and documents (`type: invoice | estimate |
//!   credit-note`) get their bill table computed in the same pass. Prose is
//!   never parsed here.
//! - **A cache, nothing else.** It is never persisted; delete it and
//!   [`DocumentIndex::open`] rebuilds it from the files (principle 1).
//! - **Per-file invalidation.** [`DocumentIndex::refresh`] re-reads only the
//!   paths a watcher names; the content hash decides whether a file changed.
//! - **`check --full`.** [`DocumentIndex::rebuild_and_diff`] rebuilds from
//!   scratch and reports every entry the live index has wrong.
//!
//! Lists, numbering and `check` read the index, never the files (templates
//! and contract paths aside, which `check` looks up in the vault).

use crate::customers::{self, Customer, CustomerLookup, is_customer};
use crate::doc::{DocType, Header, Invoice, InvoiceNumber, Money, Status, TableSummary};
use crate::notes::{self, Note, OtherFile, ScannedNote, Site, SiteConfig, Stamp};
use crate::numbering::{self, NumberingError, natural_cmp};
use crate::par::par_map;
use crate::problem::{Problem, ProblemCode};
use crate::settings::Settings;
use crate::templates;
use ledgeline_core::vault::{Vault, VaultPath};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// One document in the index.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IndexedDoc {
    /// The file.
    pub path: VaultPath,
    /// For change detection.
    #[serde(skip)]
    pub stamp: Stamp,
    /// Its title.
    pub title: String,
    /// The fields the invoicer reads.
    pub header: Header,
    /// The bill table's summary, when it reads.
    pub summary: Option<TableSummary>,
    /// What is wrong with the file on its own.
    pub problems: Vec<Problem>,
}

fn is_document(note: &Note) -> bool {
    DocType::ALL.iter().any(|t| note.is_type(t.as_str()))
}

impl IndexedDoc {
    fn build(note: &Note, source: &str, settings: &Settings, money: &Money) -> Self {
        let invoice = Invoice::parse(source, &settings.words);
        let problems = invoice
            .problems(&settings.words, money)
            .into_iter()
            .map(|p| p.in_file(&note.path))
            .collect();
        Self {
            path: note.path.clone(),
            stamp: note.stamp.clone(),
            title: note.title.clone(),
            summary: invoice.summary(&settings.words, money),
            header: invoice.header().clone(),
            problems,
        }
    }

    /// Its number, as text.
    #[must_use]
    pub fn number(&self) -> Option<&str> {
        self.header.number.as_ref().map(InvoiceNumber::as_str)
    }
}

/// What [`DocumentIndex::refresh`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refreshed {
    /// Notes re-read whose content changed (or that are new).
    pub updated: Vec<VaultPath>,
    /// Notes and files gone.
    pub removed: Vec<VaultPath>,
    /// Paths re-read whose content hash hadn't changed.
    pub unchanged: Vec<VaultPath>,
}

/// One way the live index differs from a fresh build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum IndexDrift {
    /// A note on disk the index lacks.
    Missing {
        /// The note.
        path: VaultPath,
    },
    /// A note in the index that is gone from disk.
    Stale {
        /// The note.
        path: VaultPath,
    },
    /// A note whose indexed content differs from the file's.
    Changed {
        /// The note.
        path: VaultPath,
    },
}

/// Filters for [`DocumentIndex::list`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFilter {
    /// Only this type.
    pub doc_type: Option<DocType>,
    /// Only this status.
    pub status: Option<Status>,
    /// Only this customer: a link, title or snapshot name, matched
    /// case-insensitively.
    pub customer: Option<String>,
}

/// The index.
#[derive(Debug, Clone)]
pub struct DocumentIndex {
    settings: Settings,
    money: Money,
    site: Site,
    docs: BTreeMap<VaultPath, IndexedDoc>,
}

impl DocumentIndex {
    /// Scan the books folder and build the index.
    #[must_use]
    pub fn open(vault: &dyn Vault, settings: Settings, money: Money) -> Self {
        let config = SiteConfig::for_settings(&settings);
        let scan = notes::scan(vault, &config);
        let (site, sources) = Site::from_scan(scan, config);
        let pairs: Vec<(&Note, &String)> = site
            .notes()
            .zip(sources.iter())
            .filter(|(note, _)| is_document(note))
            .collect();
        let built = par_map(&pairs, |(note, source)| {
            IndexedDoc::build(note, source, &settings, &money)
        });
        let docs = built
            .into_iter()
            .map(|doc| (doc.path.clone(), doc))
            .collect();
        Self {
            settings,
            money,
            site,
            docs,
        }
    }

    /// The settings it was built with.
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The money styles it was built with.
    #[must_use]
    pub fn money(&self) -> &Money {
        &self.money
    }

    /// Every note.
    #[must_use]
    pub fn site(&self) -> &Site {
        &self.site
    }

    /// Every document, sorted by path.
    pub fn docs(&self) -> impl Iterator<Item = &IndexedDoc> {
        self.docs.values()
    }

    /// The document at `path`.
    #[must_use]
    pub fn doc(&self, path: &VaultPath) -> Option<&IndexedDoc> {
        self.docs.get(path)
    }

    /// Documents numbered `number`.
    #[must_use]
    pub fn by_number(&self, number: &str) -> Vec<&IndexedDoc> {
        self.docs
            .values()
            .filter(|d| d.number() == Some(number.trim()))
            .collect()
    }

    /// Every number in use in the files.
    pub fn numbers(&self) -> impl Iterator<Item = &str> {
        self.docs.values().filter_map(IndexedDoc::number)
    }

    /// The next number (§5.1): over the files and `books` (the `invoice:`
    /// tag values, from Phase 2).
    ///
    /// # Errors
    /// [`NumberingError`].
    pub fn next_number<'a>(
        &'a self,
        books: impl IntoIterator<Item = &'a str>,
    ) -> Result<InvoiceNumber, NumberingError> {
        numbering::next_number(self.numbers(), books, &self.settings.first_number())
    }

    /// Documents matching `filter`, newest first (date, then number).
    #[must_use]
    pub fn list(&self, filter: &ListFilter) -> Vec<&IndexedDoc> {
        let wanted = filter.customer.as_deref().map(notes::normalize_name);
        let mut found: Vec<&IndexedDoc> = self
            .docs
            .values()
            .filter(|d| filter.doc_type.is_none_or(|t| d.header.doc_type == Some(t)))
            .filter(|d| filter.status.is_none_or(|s| d.header.status == Some(s)))
            .filter(|d| {
                wanted.as_deref().is_none_or(|w| {
                    let party = &d.header.invoicee;
                    let link = party
                        .link
                        .as_deref()
                        .map(|l| notes::wikilink_target(l).unwrap_or(l));
                    [link, party.name.as_deref()]
                        .into_iter()
                        .flatten()
                        .any(|name| notes::normalize_name(name) == w)
                })
            })
            .collect();
        found.sort_by(|a, b| {
            b.header
                .date
                .cmp(&a.header.date)
                .then_with(|| natural_cmp(b.number().unwrap_or(""), a.number().unwrap_or("")))
                .then_with(|| a.path.cmp(&b.path))
        });
        found
    }

    /// Every customer.
    #[must_use]
    pub fn customers(&self) -> Vec<Customer> {
        customers::customers(&self.site, &self.settings.words)
    }

    /// Re-read `changed` paths (from a watcher), updating only what changed.
    pub fn refresh(&mut self, vault: &dyn Vault, changed: &[VaultPath]) -> Refreshed {
        let config = self.site.config().clone();
        let mut done = Refreshed::default();
        let mut notes_in: Vec<Note> = Vec::new();
        let mut sorted: Vec<&VaultPath> =
            changed.iter().filter(|p| !config.is_ignored(p)).collect();
        sorted.sort();
        sorted.dedup();
        for path in sorted {
            let entry = vault.stat(path).ok().flatten().filter(|e| e.is_file());
            let Some(entry) = entry else {
                if self.site.note(path).is_some()
                    || self.site.others().iter().any(|o| &o.path == path)
                {
                    done.removed.push(path.clone());
                }
                self.docs.remove(path);
                continue;
            };
            if config.is_other(path) {
                self.site.apply_other(OtherFile {
                    path: path.clone(),
                    size: entry.size,
                    modified: entry.modified_time(),
                });
                done.updated.push(path.clone());
                continue;
            }
            if !config.is_markdown(path) {
                continue;
            }
            let Ok(ScannedNote { note, source }) =
                notes::read_note(vault, path, entry.modified_time(), &config)
            else {
                done.removed.push(path.clone());
                self.docs.remove(path);
                continue;
            };
            let same = self
                .site
                .note(path)
                .is_some_and(|old| old.stamp.fingerprint == note.stamp.fingerprint);
            if same {
                done.unchanged.push(path.clone());
                continue;
            }
            if is_document(&note) {
                let doc = IndexedDoc::build(&note, &source, &self.settings, &self.money);
                self.docs.insert(path.clone(), doc);
            } else {
                self.docs.remove(path);
            }
            done.updated.push(path.clone());
            notes_in.push(note);
        }
        if !notes_in.is_empty() || !done.removed.is_empty() {
            self.site.apply(notes_in, &done.removed);
        }
        done
    }

    /// Rebuild from scratch and report every way the live index differs
    /// (`check --full`). Empty means the live index is right.
    #[must_use]
    pub fn rebuild_and_diff(&self, vault: &dyn Vault) -> Vec<IndexDrift> {
        let fresh = Self::open(vault, self.settings.clone(), self.money.clone());
        let live: HashMap<&VaultPath, &Note> = self.site.notes().map(|n| (&n.path, n)).collect();
        let new: HashMap<&VaultPath, &Note> = fresh.site.notes().map(|n| (&n.path, n)).collect();
        let mut drift: Vec<IndexDrift> = new
            .iter()
            .filter_map(|(path, note)| match live.get(path) {
                None => Some(IndexDrift::Missing {
                    path: (*path).clone(),
                }),
                Some(old)
                    if old.stamp.fingerprint != note.stamp.fingerprint
                        || old.frontmatter != note.frontmatter
                        || !same_doc(self.docs.get(*path), fresh.docs.get(*path)) =>
                {
                    Some(IndexDrift::Changed {
                        path: (*path).clone(),
                    })
                }
                Some(_) => None,
            })
            .chain(
                live.keys()
                    .filter(|p| !new.contains_key(*p))
                    .map(|p| IndexDrift::Stale { path: (*p).clone() }),
            )
            .collect();
        drift.sort_by(|a, b| drift_path(a).cmp(drift_path(b)));
        drift
    }

    /// `check` on the documents (books checks come in Phase 2): every
    /// document's own problems, plus customers that don't resolve, templates
    /// that don't exist, portals that need a reference, duplicate numbers,
    /// and the customers' own problems. Sorted by file and line.
    #[must_use]
    pub fn check(&self, vault: &dyn Vault) -> Vec<Problem> {
        let words = &self.settings.words;
        let templates = templates::discover(vault, &self.site);
        let mut customers: HashMap<VaultPath, Result<Customer, CustomerLookup>> = HashMap::new();
        let mut out: Vec<Problem> = Vec::new();
        for doc in self.docs.values() {
            out.extend(doc.problems.iter().cloned());
            let line = |key: &str| self.site.note(&doc.path).and_then(|n| n.line_of(key));
            let at = |p: Problem, key: &str| p.in_file(&doc.path).at_line(line(key));
            if let Some(link) = &doc.header.invoicee.link {
                let found = self
                    .site
                    .resolve_wikilink(&doc.path, link)
                    .map(|n| n.path.clone());
                let customer = match &found {
                    None => Err(CustomerLookup::NotFound),
                    Some(path) => customers
                        .entry(path.clone())
                        .or_insert_with(|| {
                            customers::find_customer(&self.site, &doc.path, link, words)
                        })
                        .clone(),
                };
                match customer {
                    Err(CustomerLookup::NotFound) => out.push(at(
                        Problem::error(ProblemCode::UnknownCustomer, format!("`customer: {link}` names no note")),
                        "customer",
                    )),
                    Err(CustomerLookup::NotACustomer(path)) => out.push(at(
                        Problem::warning(
                            ProblemCode::UnknownCustomer,
                            format!("`customer: {link}` is {path}, which isn't a person or organization tagged `{}`", words.customer_tag),
                        )
                        .with_fix(format!("add `{}` to its tags", words.customer_tag), None),
                        "customer",
                    )),
                    Ok(customer) => {
                        let needs_reference = customer
                            .billing_portal
                            .as_ref()
                            .is_some_and(|p| p.requires_reference);
                        let live = doc.header.status != Some(Status::Void);
                        if needs_reference && live && doc.header.reference.is_none() {
                            out.push(at(
                                Problem::error(
                                    ProblemCode::ReferenceRequired,
                                    format!("{}'s portal rejects invoices without a `reference`", customer.contact.title),
                                )
                                .with_fix(
                                    customer.open_pos.first().map_or_else(
                                        || "add the customer's PO as `reference`".to_string(),
                                        |po| format!("add `reference: {}` (an open PO)", po.reference),
                                    ),
                                    None,
                                ),
                                "status",
                            ));
                        }
                    }
                }
            }
            if let Some(template) = &doc.header.template
                && templates::find(&templates, template).is_none()
            {
                out.push(at(
                    Problem::warning(ProblemCode::UnknownTemplate, format!("`template: {template}` isn't a template (an .html file mentioning invoice_num)")),
                    "template",
                ));
            }
        }
        let mut by_number: BTreeMap<&str, Vec<&IndexedDoc>> = BTreeMap::new();
        for doc in self.docs.values() {
            if let Some(n) = doc.number() {
                by_number.entry(n).or_default().push(doc);
            }
        }
        for (number, docs) in by_number.iter().filter(|(_, d)| d.len() > 1) {
            for doc in docs {
                let others: Vec<String> = docs
                    .iter()
                    .filter(|d| d.path != doc.path)
                    .map(|d| d.path.to_string())
                    .collect();
                out.push(
                    Problem::error(
                        ProblemCode::DuplicateNumber,
                        format!("invoice {number} is also {}", others.join(", ")),
                    )
                    .in_file(&doc.path)
                    .at_line(
                        self.site
                            .note(&doc.path)
                            .and_then(|n| n.line_of("invoice_num")),
                    ),
                );
            }
        }
        for note in self.site.notes().filter(|n| is_customer(n, words)) {
            if let Some(customer) = Customer::from_note(note) {
                out.extend(customer.problems(note, &self.site, vault));
            }
        }
        out.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then_with(|| a.line.cmp(&b.line))
                .then_with(|| a.message.cmp(&b.message))
        });
        out
    }
}

/// Two index entries with the same content (an mtime alone is no change).
fn same_doc(a: Option<&IndexedDoc>, b: Option<&IndexedDoc>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.stamp.fingerprint == b.stamp.fingerprint
                && a.stamp.size == b.stamp.size
                && a.title == b.title
                && a.header == b.header
                && a.summary == b.summary
                && a.problems == b.problems
        }
        _ => false,
    }
}

fn drift_path(drift: &IndexDrift) -> &VaultPath {
    match drift {
        IndexDrift::Missing { path }
        | IndexDrift::Stale { path }
        | IndexDrift::Changed { path } => path,
    }
}

#[cfg(test)]
mod tests;
