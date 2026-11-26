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
//! - **Templates and catalogs too.** The `.html` templates and the
//!   `type: catalog` notes are read with everything else at open, and again
//!   only when [`DocumentIndex::refresh`] names them, so a list, show or
//!   sidebar request reads no file (ARCHITECTURE §5.6; review RA-3).
//! - **Lookups by number and by customer** are built on first use after
//!   each refresh (in memory, no file read), so neither `find` nor a
//!   customer's documents scan every document (review RC-5, RC-15).
//!
//! Lists, numbering and `check` read the index, never the files (contract
//! paths aside, which `check` looks up in the vault).

use crate::catalog::Catalog;
use crate::customers::{self, Customer, CustomerLookup, is_customer};
use crate::doc::{DocType, Header, Invoice, InvoiceNumber, Money, Status, TableSummary};
use crate::notes::{self, Note, OtherFile, ScannedNote, Site, SiteConfig, Stamp};
use crate::numbering::{self, NumberingError, natural_cmp};
use crate::par::par_map;
use crate::problem::{Problem, ProblemCode};
use crate::settings::Settings;
use crate::templates::{self, Template};
use ledgeline_core::vault::{Vault, VaultPath};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

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

/// A catalog note, parsed, with what is wrong with it.
#[derive(Debug, Clone)]
struct ParsedCatalog {
    catalog: Catalog,
    problems: Vec<Problem>,
}

/// Lookups derived from the documents and the site, in memory, built on
/// first use and dropped by every [`DocumentIndex::refresh`] that changed
/// anything.
#[derive(Debug, Clone, Default)]
struct Lookups {
    /// Documents by number (trimmed), each list by path.
    by_number: OnceLock<HashMap<String, Vec<VaultPath>>>,
    /// Documents by the customer note their `customer:` link resolves to,
    /// each list newest first ([`DocumentIndex::list`]'s order).
    by_customer: OnceLock<HashMap<VaultPath, Vec<VaultPath>>>,
}

/// The index.
#[derive(Debug, Clone)]
pub struct DocumentIndex {
    settings: Settings,
    money: Money,
    site: Site,
    docs: BTreeMap<VaultPath, IndexedDoc>,
    /// Every template among the site's `.html` files, by path
    /// ([`templates::discover`], kept).
    templates: Vec<Template>,
    /// Every `type: catalog` note, parsed, by path.
    catalogs: BTreeMap<VaultPath, ParsedCatalog>,
    lookups: Lookups,
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
        let catalogs = site
            .notes()
            .zip(sources.iter())
            .filter(|(note, _)| note.is_type(CATALOG_TYPE))
            .map(|(note, source)| {
                (
                    note.path.clone(),
                    parse_catalog(&note.path, source, &settings, &money),
                )
            })
            .collect();
        let candidates: Vec<&VaultPath> = site
            .others()
            .iter()
            .map(|o| &o.path)
            .filter(|p| templates::is_candidate(p))
            .collect();
        let templates = par_map(&candidates, |path| templates::read(vault, path))
            .into_iter()
            .flatten()
            .collect();
        Self {
            settings,
            money,
            site,
            docs,
            templates,
            catalogs,
            lookups: Lookups::default(),
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

    /// Documents numbered `number`, by path.
    #[must_use]
    pub fn by_number(&self, number: &str) -> Vec<&IndexedDoc> {
        let map = self.lookups.by_number.get_or_init(|| {
            let mut map: HashMap<String, Vec<VaultPath>> = HashMap::new();
            for doc in self.docs.values() {
                if let Some(n) = doc.number() {
                    map.entry(n.to_string()).or_default().push(doc.path.clone());
                }
            }
            map
        });
        self.docs_at(map.get(number.trim()))
    }

    /// The documents whose `customer:` link resolves to the note at
    /// `customer`, newest first ([`Self::list`]'s order).
    #[must_use]
    pub fn billed_to(&self, customer: &VaultPath) -> Vec<&IndexedDoc> {
        let map = self.lookups.by_customer.get_or_init(|| {
            let mut map: HashMap<VaultPath, Vec<VaultPath>> = HashMap::new();
            for doc in self.list(&ListFilter::default()) {
                let found = doc
                    .header
                    .invoicee
                    .link
                    .as_deref()
                    .and_then(|link| self.site.resolve_wikilink(&doc.path, link));
                if let Some(note) = found {
                    map.entry(note.path.clone())
                        .or_default()
                        .push(doc.path.clone());
                }
            }
            map
        });
        self.docs_at(map.get(customer))
    }

    fn docs_at(&self, paths: Option<&Vec<VaultPath>>) -> Vec<&IndexedDoc> {
        paths
            .into_iter()
            .flatten()
            .filter_map(|p| self.docs.get(p))
            .collect()
    }

    /// Every template in the books folder, by path (what
    /// [`templates::discover`] finds, as of the last refresh).
    #[must_use]
    pub fn templates(&self) -> &[Template] {
        &self.templates
    }

    /// Every catalog in the books folder, by path, as of the last refresh.
    pub fn catalogs(&self) -> impl Iterator<Item = &Catalog> {
        self.catalogs.values().map(|c| &c.catalog)
    }

    /// What is wrong with the catalogs (rows that don't read, duplicate
    /// codes), by catalog path.
    pub fn catalog_problems(&self) -> impl Iterator<Item = &Problem> {
        self.catalogs.values().flat_map(|c| c.problems.iter())
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

    /// [`Self::next_number`] with where it comes from (Phase 4 feedback H19).
    ///
    /// # Errors
    /// [`NumberingError`].
    pub fn explain_next_number<'a>(
        &'a self,
        books: impl IntoIterator<Item = &'a str>,
    ) -> Result<numbering::NextNumber, NumberingError> {
        numbering::explain_next_number(self.numbers(), books, &self.settings.first_number())
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
                self.catalogs.remove(path);
                self.set_template(path, None);
                continue;
            };
            if config.is_other(path) {
                self.site.apply_other(OtherFile {
                    path: path.clone(),
                    size: entry.size,
                    modified: entry.modified_time(),
                });
                // Read again whatever the stamp says: a template's name and
                // kinds are in its text.
                let template = templates::is_candidate(path)
                    .then(|| templates::read(vault, path))
                    .flatten();
                self.set_template(path, template);
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
                self.catalogs.remove(path);
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
            if note.is_type(CATALOG_TYPE) {
                let parsed = parse_catalog(path, &source, &self.settings, &self.money);
                self.catalogs.insert(path.clone(), parsed);
            } else {
                self.catalogs.remove(path);
            }
            done.updated.push(path.clone());
            notes_in.push(note);
        }
        if !notes_in.is_empty() || !done.removed.is_empty() {
            self.site.apply(notes_in, &done.removed);
        }
        if !done.updated.is_empty() || !done.removed.is_empty() {
            // A new or renamed note can change what any link resolves to.
            self.lookups = Lookups::default();
        }
        done
    }

    /// Put `template` in at `path`, or take out whatever is there.
    fn set_template(&mut self, path: &VaultPath, template: Option<Template>) {
        let at = self.templates.binary_search_by(|t| t.path.cmp(path));
        match (at, template) {
            (Ok(i), Some(t)) => self.templates[i] = t,
            (Err(i), Some(t)) => self.templates.insert(i, t),
            (Ok(i), None) => {
                self.templates.remove(i);
            }
            (Err(_), None) => {}
        }
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
        // Templates the index has wrong (an `.html` file isn't a note).
        let kept: HashMap<&VaultPath, &Template> =
            self.templates.iter().map(|t| (&t.path, t)).collect();
        let read: HashMap<&VaultPath, &Template> =
            fresh.templates.iter().map(|t| (&t.path, t)).collect();
        for (path, template) in &read {
            match kept.get(path) {
                None => drift.push(IndexDrift::Missing {
                    path: (*path).clone(),
                }),
                Some(old) if old != template => drift.push(IndexDrift::Changed {
                    path: (*path).clone(),
                }),
                Some(_) => {}
            }
        }
        drift.extend(
            kept.keys()
                .filter(|p| !read.contains_key(*p))
                .map(|p| IndexDrift::Stale { path: (*p).clone() }),
        );
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
        let templates = &self.templates;
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
                && templates::find(templates, template).is_none()
            {
                let is_base = VaultPath::new(template.trim().trim_start_matches('/'))
                    .is_ok_and(|p| templates::is_partial(&p));
                let message = if is_base {
                    format!(
                        "`template: {template}` is a base for templates to extend (its name \
                         starts with `_`), not a template: name one that extends it"
                    )
                } else {
                    format!(
                        "`template: {template}` isn't a template (an .html file mentioning \
                         invoice_num, or extending an invoice base)"
                    )
                };
                out.push(at(
                    Problem::warning(ProblemCode::UnknownTemplate, message),
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

/// The `type:` of a catalog note.
const CATALOG_TYPE: &str = "catalog";

fn parse_catalog(
    path: &VaultPath,
    source: &str,
    settings: &Settings,
    money: &Money,
) -> ParsedCatalog {
    let (catalog, problems) = Catalog::parse(path.clone(), source, &settings.words, money);
    ParsedCatalog { catalog, problems }
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
