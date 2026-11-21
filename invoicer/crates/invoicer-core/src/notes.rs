//! Discovery (ARCHITECTURE §3.1): every note in the books folder, found by
//! content, with its frontmatter.
//!
//! This is the one module that knows how notes are found. Its API is shaped
//! like MBR's planned `mbr_core::Site` façade (MBR-CHANGES M2: `open`,
//! `notes`, `notes_of_type`, `note`, `field_values`, `resolve_wikilink`), so
//! when MBR ships that façade on crates.io this module becomes a thin wrapper
//! over it and no caller changes. Until then it walks the vault itself,
//! following MBR's rules where they matter:
//!
//! - **Ignored:** hidden names (`.git`, `.obsidian`), MBR's default ignore
//!   directories (`node_modules`, `target`, `templates`, …), symlinks (as
//!   everywhere in Ledgeline), and the invoicer's own `pdf-dir`.
//! - **Type** matches case-insensitively, trimmed, as MBR's contact model
//!   does.
//! - **Tags** come from `tags:`, a list or a comma-separated string, as
//!   MBR's `extract_tag_values` reads them, and match case-insensitively.
//! - **Wikilinks** resolve as MBR resolves body wikilinks: a note with that
//!   file stem in the linking note's folder first, then by title, then by
//!   alias, then by stem, case-insensitively, the lexicographically smallest
//!   path winning a tie. A target with a `/` or a `.md` is a path.
//!
//! All reads go through ledgeline-core's [`Vault`], and files are read in
//! parallel (§5.6). Every note keeps a [`Stamp`] — mtime, size and content
//! hash — so an index can tell which files changed.

use crate::frontmatter::{self, Fields, Frontmatter, FrontmatterError, KeyLine};
use crate::par::par_map;
use crate::settings::Settings;
use ledgeline_core::vault::{Entry, EntryKind, Fingerprint, Vault, VaultError, VaultPath};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::time::SystemTime;

/// MBR's default ignored directories (`mbr_core::config`), by name.
pub const MBR_IGNORE_DIRS: &[&str] = &[
    "target",
    "result",
    "build",
    "node_modules",
    "ci",
    "templates",
    ".git",
    ".github",
    "dist",
    "out",
    "coverage",
];

/// How to walk a books folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteConfig {
    /// Directory names never entered.
    pub ignore_dirs: Vec<String>,
    /// Vault paths never entered or read (the PDF folder).
    pub ignore_paths: Vec<VaultPath>,
    /// Extensions read as notes (`md`).
    pub markdown_extensions: Vec<String>,
    /// Extensions listed, unread, as other files (`html`, for templates).
    pub other_extensions: Vec<String>,
    /// The most directory entries examined before the walk stops. A books
    /// folder in `$HOME` must not turn into a full-disk walk.
    pub max_entries: usize,
    /// Notes larger than this are skipped with a warning.
    pub max_note_bytes: u64,
}

impl Default for SiteConfig {
    fn default() -> Self {
        Self {
            ignore_dirs: MBR_IGNORE_DIRS.iter().map(|d| (*d).to_string()).collect(),
            ignore_paths: Vec::new(),
            markdown_extensions: vec!["md".to_string()],
            other_extensions: vec!["html".to_string()],
            max_entries: 200_000,
            max_note_bytes: 4 * 1024 * 1024,
        }
    }
}

impl SiteConfig {
    /// The defaults plus the invoicer's `pdf-dir`.
    #[must_use]
    pub fn for_settings(settings: &Settings) -> Self {
        let mut config = Self::default();
        config
            .ignore_paths
            .extend(Settings::vault_path(&settings.pdf_dir).ok());
        config
    }

    fn extension_in(path: &VaultPath, list: &[String]) -> bool {
        path.extension()
            .is_some_and(|ext| list.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    }

    /// Whether `path` is a markdown note's name.
    #[must_use]
    pub fn is_markdown(&self, path: &VaultPath) -> bool {
        Self::extension_in(path, &self.markdown_extensions)
    }

    /// Whether `path` is an other file the site lists.
    #[must_use]
    pub fn is_other(&self, path: &VaultPath) -> bool {
        Self::extension_in(path, &self.other_extensions)
    }

    /// Whether discovery skips `path` (any segment hidden or ignored, or
    /// under an ignored path).
    #[must_use]
    pub fn is_ignored(&self, path: &VaultPath) -> bool {
        let segments: Vec<&str> = path.segments().collect();
        let last = segments.len().saturating_sub(1);
        segments.iter().enumerate().any(|(i, segment)| {
            segment.starts_with('.') || (i < last && self.ignore_dirs.iter().any(|d| d == segment))
        }) || self.ignore_paths.iter().any(|p| path.starts_with(p))
    }
}

/// What an index needs to know whether a file changed: the content hash
/// decides; mtime and size are the cheap first look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// Last modification time, when the vault reports one.
    pub modified: Option<SystemTime>,
    /// Size in bytes.
    pub size: u64,
    /// The content fingerprint (hash and length).
    pub fingerprint: Fingerprint,
}

/// One note: its path, title, type, tags and frontmatter.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// Vault-relative path.
    pub path: VaultPath,
    /// `title:`, or the file stem.
    pub title: String,
    /// `type:`, trimmed.
    pub note_type: Option<String>,
    /// `tags:`.
    pub tags: Vec<String>,
    /// `aliases:` (strings, or the values of single-key maps).
    pub aliases: Vec<String>,
    /// Every field, lossless.
    pub frontmatter: Fields,
    /// Top-level keys in document order.
    pub order: Vec<String>,
    /// Why the frontmatter didn't read (the note then has no fields).
    pub error: Option<FrontmatterError>,
    /// Each top-level key's line and raw text, for problems (empty when the
    /// layout isn't one the editor understands).
    pub keys: Vec<KeyLine>,
    /// For change detection.
    pub stamp: Stamp,
}

/// MBR's tag reading: a list of strings, or one comma-separated string.
#[must_use]
pub fn tag_values(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn alias_values(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .flat_map(|item| match item {
                Value::String(s) => vec![s.clone()],
                Value::Object(map) => map
                    .values()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The file stem of a path (`Customer X` for `customers/Customer X.md`).
#[must_use]
pub fn stem(path: &VaultPath) -> &str {
    let name = path.file_name().unwrap_or("");
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// MBR's name normalization: trimmed and lowercased.
#[must_use]
pub fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

impl Note {
    /// A note from its source text.
    #[must_use]
    pub fn from_source(path: VaultPath, source: &str, stamp: Stamp) -> Self {
        let (frontmatter, error) = match Frontmatter::parse(source) {
            Ok(fm) => (fm, None),
            Err(e) => (Frontmatter::default(), Some(e)),
        };
        let fields = frontmatter.fields;
        let title = fields
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map_or_else(|| stem(&path).to_string(), str::to_string);
        let note_type = fields
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        let tags = fields.get("tags").map(tag_values).unwrap_or_default();
        let aliases = fields.get("aliases").map(alias_values).unwrap_or_default();
        let keys = frontmatter::key_lines(source).unwrap_or_default();
        Self {
            path,
            title,
            note_type,
            tags,
            aliases,
            frontmatter: fields,
            order: frontmatter.order,
            error,
            keys,
            stamp,
        }
    }

    /// A top-level key's 1-based line.
    #[must_use]
    pub fn line_of(&self, key: &str) -> Option<usize> {
        self.keys.iter().find(|k| k.key == key).map(|k| k.line)
    }

    /// Whether its `type` is `ty`, case-insensitively.
    #[must_use]
    pub fn is_type(&self, ty: &str) -> bool {
        self.note_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case(ty.trim()))
    }

    /// Whether it carries `tag`, case-insensitively.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags
            .iter()
            .any(|t| t.trim().eq_ignore_ascii_case(tag.trim()))
    }

    /// A text field.
    #[must_use]
    pub fn str(&self, key: &str) -> Option<&str> {
        self.frontmatter.get(key).and_then(Value::as_str)
    }
}

/// A non-note file the site lists (a template, a contract PDF is not listed:
/// only [`SiteConfig::other_extensions`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherFile {
    /// Vault-relative path.
    pub path: VaultPath,
    /// Size in bytes.
    pub size: u64,
    /// Last modification time.
    pub modified: Option<SystemTime>,
}

/// The files a walk found, before any is read.
#[derive(Debug, Clone, Default)]
pub struct Walk {
    /// Markdown notes, sorted by path.
    pub markdown: Vec<Entry>,
    /// Other listed files, sorted by path.
    pub others: Vec<OtherFile>,
    /// Things skipped that a user should hear about.
    pub warnings: Vec<String>,
    /// The entry budget ran out.
    pub truncated: bool,
}

/// List the books folder: every markdown note and other listed file not
/// ignored. Directories are listed sequentially (cheap); reading is
/// [`scan`]'s job.
#[must_use]
pub fn walk(vault: &dyn Vault, config: &SiteConfig) -> Walk {
    let mut found = Walk::default();
    let mut examined = 0usize;
    let mut stack = vec![VaultPath::root()];
    while let Some(dir) = stack.pop() {
        let cap = config.max_entries.saturating_sub(examined) + 1;
        let children = match vault.list_dir(&dir, Some(cap)) {
            Ok(children) => children,
            Err(e) => {
                found
                    .warnings
                    .push(format!("{dir}: can't be listed ({e}); skipped"));
                continue;
            }
        };
        for entry in children {
            if examined >= config.max_entries {
                found.truncated = true;
                break;
            }
            examined += 1;
            let name = entry.name();
            if name.starts_with('.')
                || config
                    .ignore_paths
                    .iter()
                    .any(|p| entry.path.starts_with(p))
            {
                continue;
            }
            match entry.kind {
                EntryKind::Dir if !config.ignore_dirs.iter().any(|d| d == name) => {
                    stack.push(entry.path);
                }
                EntryKind::File if config.is_markdown(&entry.path) => found.markdown.push(entry),
                EntryKind::File if config.is_other(&entry.path) => found.others.push(OtherFile {
                    modified: entry.modified_time(),
                    size: entry.size,
                    path: entry.path,
                }),
                EntryKind::Symlink if config.is_markdown(&entry.path) => found
                    .warnings
                    .push(format!("{}: a symbolic link; skipped", entry.path)),
                _ => {}
            }
        }
    }
    found.markdown.sort_by(|a, b| a.path.cmp(&b.path));
    found.others.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

/// A note with the source it was read from.
#[derive(Debug, Clone)]
pub struct ScannedNote {
    /// The note.
    pub note: Note,
    /// Its text.
    pub source: String,
}

/// Read one note.
///
/// # Errors
/// [`VaultError`] when it can't be read, is too large, or isn't UTF-8.
pub fn read_note(
    vault: &dyn Vault,
    path: &VaultPath,
    modified: Option<SystemTime>,
    config: &SiteConfig,
) -> Result<ScannedNote, VaultError> {
    let (bytes, revision) = vault.read_pinned(path, Some(config.max_note_bytes))?;
    let size = bytes.len() as u64;
    let source = String::from_utf8(bytes).map_err(|e| VaultError::Io {
        path: path.to_string(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
    })?;
    let stamp = Stamp {
        modified,
        size,
        fingerprint: revision.fingerprint,
    };
    Ok(ScannedNote {
        note: Note::from_source(path.clone(), &source, stamp),
        source,
    })
}

/// A walk with every note read, in parallel.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    /// Every note, sorted by path, with its source.
    pub notes: Vec<ScannedNote>,
    /// Other listed files.
    pub others: Vec<OtherFile>,
    /// Things skipped.
    pub warnings: Vec<String>,
    /// The walk stopped early.
    pub truncated: bool,
}

/// Walk and read every note, in parallel.
#[must_use]
pub fn scan(vault: &dyn Vault, config: &SiteConfig) -> Scan {
    let walked = walk(vault, config);
    let read = par_map(&walked.markdown, |entry| {
        read_note(vault, &entry.path, entry.modified_time(), config)
            .map_err(|e| format!("{}: can't be read ({e}); skipped", entry.path))
    });
    let (notes, failures): (Vec<_>, Vec<_>) = read.into_iter().partition(Result::is_ok);
    let mut warnings = walked.warnings;
    warnings.extend(failures.into_iter().filter_map(Result::err));
    Scan {
        notes: notes.into_iter().filter_map(Result::ok).collect(),
        others: walked.others,
        warnings,
        truncated: walked.truncated,
    }
}

/// Names a wikilink can resolve by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Names {
    by_title: HashMap<String, Vec<usize>>,
    by_alias: HashMap<String, Vec<usize>>,
    by_stem: HashMap<String, Vec<usize>>,
}

impl Names {
    fn build(notes: &[Note]) -> Self {
        let mut names = Self::default();
        for (i, note) in notes.iter().enumerate() {
            names
                .by_title
                .entry(normalize_name(&note.title))
                .or_default()
                .push(i);
            for alias in &note.aliases {
                names
                    .by_alias
                    .entry(normalize_name(alias))
                    .or_default()
                    .push(i);
            }
            names
                .by_stem
                .entry(normalize_name(stem(&note.path)))
                .or_default()
                .push(i);
        }
        names
    }
}

/// The target inside `[[Target|Shown]]` or `[[Target#Heading]]`, or `None`
/// when `text` isn't a wikilink.
#[must_use]
pub fn wikilink_target(text: &str) -> Option<&str> {
    let inner = text.trim().strip_prefix("[[")?.strip_suffix("]]")?;
    let target = inner.split('|').next().unwrap_or(inner);
    let target = target.split('#').next().unwrap_or(target).trim();
    (!target.is_empty()).then_some(target)
}

/// The books folder's notes, indexed for lookup.
#[derive(Debug, Clone, Default)]
pub struct Site {
    notes: Vec<Note>,
    by_path: HashMap<VaultPath, usize>,
    names: Names,
    others: Vec<OtherFile>,
    warnings: Vec<String>,
    truncated: bool,
    config: SiteConfig,
}

impl Site {
    /// Walk and read the books folder.
    #[must_use]
    pub fn open(vault: &dyn Vault, config: SiteConfig) -> Self {
        let scanned = scan(vault, &config);
        Self::from_scan(scanned, config).0
    }

    /// A site from a [`scan`], handing the sources back (an index parses
    /// the invoices out of them without a second read).
    #[must_use]
    pub fn from_scan(scan: Scan, config: SiteConfig) -> (Self, Vec<String>) {
        let (notes, sources): (Vec<Note>, Vec<String>) = scan
            .notes
            .into_iter()
            .map(|scanned| (scanned.note, scanned.source))
            .unzip();
        let mut site = Self {
            notes,
            others: scan.others,
            warnings: scan.warnings,
            truncated: scan.truncated,
            config,
            ..Self::default()
        };
        site.reindex();
        (site, sources)
    }

    fn reindex(&mut self) {
        self.by_path = self
            .notes
            .iter()
            .enumerate()
            .map(|(i, note)| (note.path.clone(), i))
            .collect();
        self.names = Names::build(&self.notes);
    }

    /// How it walks.
    #[must_use]
    pub fn config(&self) -> &SiteConfig {
        &self.config
    }

    /// Every note, sorted by path.
    pub fn notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.iter()
    }

    /// How many notes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Notes whose `type` is `ty`, case-insensitively.
    pub fn notes_of_type<'a>(&'a self, ty: &'a str) -> impl Iterator<Item = &'a Note> + 'a {
        self.notes.iter().filter(move |note| note.is_type(ty))
    }

    /// Notes tagged `tag`, case-insensitively.
    pub fn tagged<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Note> + 'a {
        self.notes.iter().filter(move |note| note.has_tag(tag))
    }

    /// The note at `path`.
    #[must_use]
    pub fn note(&self, path: &VaultPath) -> Option<&Note> {
        self.by_path.get(path).map(|&i| &self.notes[i])
    }

    /// Other listed files (templates), sorted by path.
    #[must_use]
    pub fn others(&self) -> &[OtherFile] {
        &self.others
    }

    /// What the walk skipped.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Whether the walk stopped at its entry budget.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Distinct values of `key` across notes with their counts, most used
    /// first (for autocomplete). A list counts each of its scalars.
    #[must_use]
    pub fn field_values(&self, key: &str) -> Vec<(Value, usize)> {
        let mut counts: BTreeMap<String, (Value, usize)> = BTreeMap::new();
        let mut count = |value: &Value| {
            if value.is_object() || value.is_array() || value.is_null() {
                return;
            }
            counts
                .entry(value.to_string())
                .or_insert_with(|| (value.clone(), 0))
                .1 += 1;
        };
        for note in &self.notes {
            match note.frontmatter.get(key) {
                Some(Value::Array(items)) => items.iter().for_each(&mut count),
                Some(value) => count(value),
                None => {}
            }
        }
        let mut values: Vec<(Value, usize)> = counts.into_values().collect();
        values.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.to_string().cmp(&b.0.to_string()))
        });
        values
    }

    /// The note a wikilink (`[[Customer X]]`, or a bare target) names, seen
    /// from the note at `from`. See the module docs for the rules.
    #[must_use]
    pub fn resolve_wikilink(&self, from: &VaultPath, link: &str) -> Option<&Note> {
        let target = wikilink_target(link).unwrap_or_else(|| link.trim());
        if target.is_empty() {
            return None;
        }
        if target.contains('/') || VaultPath::new(target).is_ok_and(|p| self.config.is_markdown(&p))
        {
            return self.resolve_path(from, target);
        }
        let name = normalize_name(target);
        let dir = from.parent().unwrap_or_default();
        let same_folder = self.names.by_stem.get(&name).and_then(|hits| {
            hits.iter()
                .find(|&&i| self.notes[i].path.parent().unwrap_or_default() == dir)
        });
        let first = |map: &'_ HashMap<String, Vec<usize>>| -> Option<usize> {
            map.get(&name).and_then(|h| h.first().copied())
        };
        same_folder
            .copied()
            .or_else(|| first(&self.names.by_title))
            .or_else(|| first(&self.names.by_alias))
            .or_else(|| first(&self.names.by_stem))
            .map(|i| &self.notes[i])
    }

    /// A path link: `/`-rooted from the books folder or relative to `from`'s
    /// folder, `.md` optional.
    fn resolve_path(&self, from: &VaultPath, target: &str) -> Option<&Note> {
        let path = resolve_relative(from, target)?;
        self.note(&path).or_else(|| {
            let with_ext = VaultPath::new(&format!("{}.md", path.as_str())).ok()?;
            self.note(&with_ext)
        })
    }

    /// Put `notes` in (replacing any at the same path) and take `removed`
    /// out, then rebuild the lookups once.
    pub fn apply(&mut self, notes: Vec<Note>, removed: &[VaultPath]) {
        self.notes.retain(|note| !removed.contains(&note.path));
        self.others.retain(|other| !removed.contains(&other.path));
        for note in notes {
            match self.notes.binary_search_by(|n| n.path.cmp(&note.path)) {
                Ok(at) => self.notes[at] = note,
                Err(at) => self.notes.insert(at, note),
            }
        }
        self.reindex();
    }

    /// Add or replace an other file.
    pub fn apply_other(&mut self, other: OtherFile) {
        match self.others.binary_search_by(|o| o.path.cmp(&other.path)) {
            Ok(at) => self.others[at] = other,
            Err(at) => self.others.insert(at, other),
        }
    }
}

/// A file reference from a note: `/`-rooted from the books folder, or
/// relative to the note's folder. `None` when it climbs out of the vault.
#[must_use]
pub fn resolve_relative(from: &VaultPath, target: &str) -> Option<VaultPath> {
    match target.strip_prefix('/') {
        Some(rooted) => VaultPath::new(rooted).ok(),
        None => {
            let dir = from.parent().unwrap_or_default();
            VaultPath::new(&format!("{}/{target}", dir.as_str())).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::vault::MemVault;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).expect("path")
    }

    fn vault() -> MemVault {
        MemVault::new()
            .with_file("customers/Customer X.md", "---\ntitle: Customer X\ntype: Organization\ntags: [customer]\naliases: [CX]\n---\n")
            .with_file("customers/Fred.md", "---\ntype: person\ntags: customer, vip\n---\n")
            .with_file("invoices/105.md", "---\ntype: invoice\ninvoice_num: 105\n---\n")
            .with_file("invoices/tpl/x.html", "{{ frontmatter.invoice_num }}")
            .with_file("invoices/pdf/105.md", "---\ntype: invoice\n---\n")
            .with_file(".obsidian/a.md", "---\ntype: invoice\n---\n")
            .with_file("node_modules/b.md", "---\ntype: invoice\n---\n")
            .with_file("templates/c.md", "---\ntype: invoice\n---\n")
            .with_file("notes/bad.md", "---\na: [\n---\n")
            .with_file("notes/Customer X.md", "# a same-named note elsewhere\n")
            .with_file("ledgeline.toml", "")
    }

    fn site() -> Site {
        Site::open(&vault(), SiteConfig::for_settings(&Settings::default()))
    }

    #[test]
    fn walks_by_mbrs_ignore_rules() {
        let site = site();
        let paths: Vec<&str> = site.notes().map(|n| n.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "customers/Customer X.md",
                "customers/Fred.md",
                "invoices/105.md",
                "notes/Customer X.md",
                "notes/bad.md"
            ]
        );
        assert_eq!(site.others().len(), 1);
        assert!(site.warnings().is_empty());
    }

    #[test]
    fn type_and_tags_match_case_insensitively() {
        let site = site();
        assert_eq!(site.notes_of_type("organization").count(), 1);
        assert_eq!(site.notes_of_type(" INVOICE ").count(), 1);
        let tagged: Vec<&str> = site.tagged("Customer").map(|n| n.title.as_str()).collect();
        assert_eq!(tagged, ["Customer X", "Fred"]);
        let fred = site.note(&vp("customers/Fred.md")).expect("fred");
        assert_eq!(fred.tags, ["customer", "vip"]);
        assert_eq!(fred.title, "Fred");
    }

    #[test]
    fn a_bad_note_keeps_its_error_and_no_fields() {
        let site = site();
        let bad = site.note(&vp("notes/bad.md")).expect("listed");
        assert!(bad.error.is_some());
        assert!(bad.frontmatter.is_empty());
    }

    #[test]
    fn wikilinks_resolve_like_mbr() {
        let site = site();
        let from_invoices = vp("invoices/105.md");
        let hit = |link: &str, from: &VaultPath| -> Option<String> {
            site.resolve_wikilink(from, link)
                .map(|n| n.path.as_str().to_string())
        };
        // Title beats the stem of a note elsewhere.
        assert_eq!(
            hit("[[Customer X]]", &from_invoices).as_deref(),
            Some("customers/Customer X.md")
        );
        // Same folder wins first.
        assert_eq!(
            hit("[[Customer X]]", &vp("notes/other.md")).as_deref(),
            Some("notes/Customer X.md")
        );
        assert_eq!(
            hit("[[cx]]", &from_invoices).as_deref(),
            Some("customers/Customer X.md")
        );
        assert_eq!(
            hit("[[customer x|Shown]]", &from_invoices).as_deref(),
            Some("customers/Customer X.md")
        );
        assert_eq!(
            hit("[[/customers/Fred]]", &from_invoices).as_deref(),
            Some("customers/Fred.md")
        );
        assert_eq!(
            hit("[[../customers/Fred.md]]", &from_invoices).as_deref(),
            Some("customers/Fred.md")
        );
        assert_eq!(hit("[[Nobody]]", &from_invoices), None);
        assert_eq!(hit("[[]]", &from_invoices), None);
    }

    #[test]
    fn field_values_count_scalars_and_list_items() {
        let site = site();
        let tags = site.field_values("tags");
        assert_eq!(tags[0], (Value::String("customer".into()), 1));
        let types = site.field_values("type");
        assert_eq!(types.iter().map(|(_, n)| n).sum::<usize>(), 3);
    }

    #[test]
    fn apply_replaces_and_removes() {
        let mut site = site();
        let source = "---\ntitle: Renamed\ntype: organization\n---\n";
        let stamp = Stamp {
            modified: None,
            size: source.len() as u64,
            fingerprint: Fingerprint::of_bytes(source.as_bytes()),
        };
        let note = Note::from_source(vp("customers/Customer X.md"), source, stamp);
        site.apply(vec![note], &[vp("customers/Fred.md")]);
        assert_eq!(site.len(), 4);
        assert!(site.note(&vp("customers/Fred.md")).is_none());
        assert!(site.resolve_wikilink(&vp("a.md"), "[[Renamed]]").is_some());
    }

    #[test]
    fn ignore_rules_apply_to_single_paths() {
        let config = SiteConfig::for_settings(&Settings::default());
        assert!(config.is_ignored(&vp(".git/x.md")));
        assert!(config.is_ignored(&vp("a/node_modules/x.md")));
        assert!(config.is_ignored(&vp("invoices/pdf/x.md")));
        assert!(!config.is_ignored(&vp("invoices/x.md")));
        assert!(!config.is_ignored(&vp("target.md")));
    }

    #[test]
    fn relative_references() {
        let from = vp("customers/Customer X.md");
        assert_eq!(
            resolve_relative(&from, "contracts/MSA.pdf"),
            Some(vp("customers/contracts/MSA.pdf"))
        );
        assert_eq!(resolve_relative(&from, "/a/b.pdf"), Some(vp("a/b.pdf")));
        assert_eq!(resolve_relative(&from, "../../escape.pdf"), None);
    }
}
