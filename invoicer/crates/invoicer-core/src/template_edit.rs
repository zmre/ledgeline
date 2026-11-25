//! Editing templates (invoicer PLAN 4d, ARCHITECTURE §3.5): the books'
//! `.html` invoice templates, one library call each, shared by the API and,
//! later, the CLI.
//!
//! A template's file is its `{# invoicer: name="…" kinds="…" #}` line (the
//! **header**, [`templates::header`]) and the Tera text after it (the
//! **body**). The editor edits the name and kinds as fields and the body as
//! code, so the header is written by this module only, and an unchanged name
//! and kinds keep the line byte for byte.
//!
//! - [`list`] gives every template with its content hash, what it extends
//!   (the shipped base, and which version), the base's blocks it overrides,
//!   and who uses it: the invoices whose `template:` names it and the
//!   customers whose `invoice_template:` does.
//! - [`create`] writes `<templates-dir>/<slug>.html`: a child of the base
//!   (the header, then `{% extends "_invoice_base.v1.html" %}` on line 2,
//!   PLAN Q1h), or a copy of another template's body (a starter, or a
//!   duplicate), never replacing a file. The base is copied in beside it
//!   when it extends the base and the base isn't there (PLAN Q1d's rule).
//! - [`save`] writes the name, kinds and body over the content hash the
//!   caller was shown ([`TemplateError::Changed`] otherwise).
//! - [`delete`] removes a template no invoice or customer names.
//!
//! The shipped base's text is the app's ([`BaseTemplate`]): this crate
//! doesn't embed templates.

use crate::customer_edit::content_hash;
use crate::doc::DocType;
use crate::index::DocumentIndex;
use crate::templates::{self, BASE_PREFIX, TemplateHeader};
use crate::tera_scan;
use ledgeline_core::vault::{Revision, Vault, VaultError, VaultPath};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The largest template accepted (MBR reads no larger page template).
pub const MAX_TEMPLATE_BYTES: usize = 1024 * 1024;

/// The longest display name accepted.
const MAX_NAME: usize = 80;

/// The longest file stem a new template gets (before any `-2`).
const MAX_SLUG: usize = 60;

/// The shipped base a new child extends: its file name in the templates
/// folder (`_invoice_base.v1.html`) and its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseTemplate<'a> {
    /// Its file name.
    pub file: &'a str,
    /// Its Tera source.
    pub source: &'a str,
}

/// One of the base's blocks, for "Override block…".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaseBlock {
    /// The block's name (`theme`).
    pub name: String,
    /// What it is for, from the base's own comment; empty when it says
    /// nothing.
    pub about: String,
}

/// The blocks of a base, in page order, each with the sentence the base's
/// comment header gives it (`- \`theme\`: the CSS for the sheet …`).
#[must_use]
pub fn base_blocks(base: &str) -> Vec<BaseBlock> {
    let Some(found) = tera_scan::blocks(base) else {
        return Vec::new();
    };
    let mut names: Vec<&str> = Vec::new();
    for block in &found {
        if !names.contains(&block.name) {
            names.push(block.name);
        }
    }
    names
        .into_iter()
        .map(|name| BaseBlock {
            name: name.to_string(),
            about: block_about(base, name),
        })
        .collect()
}

/// The text after `- \`name\`:` in the base's comment, joined across its
/// indented continuation lines.
fn block_about(base: &str, name: &str) -> String {
    let marker = format!("- `{name}`:");
    let mut lines = base.lines();
    let Some(first) = lines.by_ref().find(|l| l.trim_start().starts_with(&marker)) else {
        return String::new();
    };
    let indent = first.len() - first.trim_start().len();
    let mut out = first.trim_start()[marker.len()..].trim().to_string();
    for line in lines {
        let trimmed = line.trim_start();
        let deeper = line.len() - trimmed.len() > indent;
        if trimmed.is_empty() || !deeper {
            break;
        }
        out.push(' ');
        out.push_str(trimmed.trim_end());
    }
    out
}

/// The version in a base's file name: `_invoice_base.v1.html` → 1.
#[must_use]
pub fn base_version(file: &str) -> Option<u32> {
    let rest = file.strip_prefix(BASE_PREFIX)?.strip_prefix(".v")?;
    rest.strip_suffix(".html")?.parse().ok()
}

/// How a note uses a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UseKind {
    /// A document's `template:` names it.
    Invoice,
    /// A customer's `invoice_template:` names it: its new invoices use it.
    Customer,
}

/// One note that uses a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TemplateUse {
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// How.
    pub how: UseKind,
    /// The document's number.
    pub number: Option<String>,
}

/// Whether a `template:` value names `path` (`/`-rooted or not).
fn names(value: &str, path: &str) -> bool {
    value.trim().trim_start_matches('/') == path
}

/// Who uses the template at `path` (a vault path; `template:` values are
/// compared without their leading `/`): documents by number, then customers,
/// each by title.
#[must_use]
pub fn uses(index: &DocumentIndex, path: &str) -> Vec<TemplateUse> {
    let path = path.trim_start_matches('/');
    let mut docs: Vec<TemplateUse> = index
        .docs()
        .filter(|d| d.header.template.as_deref().is_some_and(|t| names(t, path)))
        .map(|d| TemplateUse {
            path: d.path.clone(),
            title: d.title.clone(),
            how: UseKind::Invoice,
            number: d.number().map(str::to_string),
        })
        .collect();
    docs.sort_by(|a, b| a.number.cmp(&b.number).then_with(|| a.path.cmp(&b.path)));
    let mut customers: Vec<TemplateUse> = index
        .customers()
        .into_iter()
        .filter(|c| {
            c.invoice_template
                .as_deref()
                .is_some_and(|t| names(t, path))
        })
        .map(|c| TemplateUse {
            path: c.contact.path,
            title: c.contact.title,
            how: UseKind::Customer,
            number: None,
        })
        .collect();
    customers.sort_by_key(|c| c.title.to_lowercase());
    docs.extend(customers);
    docs
}

/// What a template's text says about it: its name and kinds, what it
/// extends, and the blocks it defines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Shape {
    /// The display name (the header's, else the file stem).
    pub name: String,
    /// The document types it is for (`invoice` when unsaid).
    pub kinds: Vec<String>,
    /// The file it extends (`_invoice_base.v1.html`), if any.
    pub extends: Option<String>,
    /// The base version it extends, when that is an invoice base.
    pub base_version: Option<u32>,
    /// The blocks it defines, in order (for a child: what it overrides).
    pub overrides: Vec<String>,
}

/// The [`Shape`] of `source`, the template at `path`.
#[must_use]
pub fn shape_of(path: &VaultPath, source: &str) -> Shape {
    let head = templates::header(source).unwrap_or_default();
    let extends = tera_scan::extends(source).map(str::to_string);
    let mut overrides: Vec<String> = Vec::new();
    if let Some(found) = tera_scan::blocks(source) {
        for block in tera_scan::outermost(found) {
            if !overrides.iter().any(|o| o == block.name) {
                overrides.push(block.name.to_string());
            }
        }
    }
    Shape {
        name: head
            .name
            .unwrap_or_else(|| crate::notes::stem(path).to_string()),
        kinds: if head.kinds.is_empty() {
            vec![DocType::Invoice.as_str().to_string()]
        } else {
            head.kinds
        },
        base_version: extends.as_deref().and_then(base_version),
        extends,
        overrides,
    }
}

/// One template in the books, as the editor lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TemplateEntry {
    /// The file.
    pub path: VaultPath,
    /// The `template:` value naming it (`/invoices/tpl/x.html`).
    pub template: String,
    /// What its text says.
    #[serde(flatten)]
    pub shape: Shape,
    /// The file's content hash: what a save names.
    pub hash: String,
    /// Invoices, then customers.
    pub used_by: Vec<TemplateUse>,
}

/// Every template in the books, by name (case-insensitively), each read
/// now.
#[must_use]
pub fn list(index: &DocumentIndex, vault: &dyn Vault) -> Vec<TemplateEntry> {
    let mut out: Vec<TemplateEntry> = templates::discover(vault, index.site())
        .into_iter()
        .filter_map(|t| {
            let source = vault.read_to_string(&t.path).ok()?;
            Some(entry_of(index, t.path, &source))
        })
        .collect();
    out.sort_by(|a, b| {
        a.shape
            .name
            .to_lowercase()
            .cmp(&b.shape.name.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

fn entry_of(index: &DocumentIndex, path: VaultPath, source: &str) -> TemplateEntry {
    TemplateEntry {
        template: format!("/{path}"),
        shape: shape_of(&path, source),
        hash: content_hash(source.as_bytes()),
        used_by: uses(index, path.as_str()),
        path,
    }
}

/// `source` split into its header line (with its line end) and the body
/// after it. No header: an empty first part and the whole text.
#[must_use]
pub fn split_header(source: &str) -> (&str, &str) {
    if templates::header(source).is_none() {
        return ("", source);
    }
    match source.find('\n') {
        Some(at) => source.split_at(at + 1),
        None => (source, ""),
    }
}

/// The body of `source`: the text after its header line ([`split_header`]).
#[must_use]
pub fn body_of(source: &str) -> &str {
    split_header(source).1
}

/// One template, for the editor: as listed, plus its body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TemplateFile {
    /// As [`list`] gives it.
    #[serde(flatten)]
    pub entry: TemplateEntry,
    /// The text after the header line.
    pub body: String,
}

/// The template at `path`, read now.
///
/// # Errors
/// [`TemplateError::NotFound`], [`TemplateError::NotATemplate`],
/// [`TemplateError::NotText`], or a vault error.
pub fn read(
    index: &DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
) -> Result<TemplateFile, TemplateError> {
    let (source, _) = read_template(vault, path)?;
    Ok(TemplateFile {
        body: body_of(&source).to_string(),
        entry: entry_of(index, path.clone(), &source),
    })
}

/// The template form: every field optional on a save (absent leaves it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TemplateFields {
    /// The display name, written in the header line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The document types it is for (`invoice`, `estimate`, `credit-note`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
    /// The Tera text after the header line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// What a create or a save wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TemplateSaved {
    /// The file.
    pub path: VaultPath,
    /// The `template:` value naming it.
    pub template: String,
    /// Its name.
    pub name: String,
    /// The content hash now: what the next save names.
    pub hash: String,
    /// Whether the file was written.
    pub written: bool,
    /// What changed (`name`, `kinds`, `body`), in order.
    pub changed: Vec<String>,
    /// The base, when it was copied in now beside the template.
    pub base: Option<VaultPath>,
}

/// Why a template edit refused. Nothing is written when one is returned.
#[derive(Debug, Error)]
pub enum TemplateError {
    /// A field doesn't read.
    #[error("{0}")]
    Invalid(String),
    /// No file there.
    #[error("{0}: no such template")]
    NotFound(VaultPath),
    /// The file isn't a template (a base, a partial, other HTML).
    #[error("{0} isn't an invoice template")]
    NotATemplate(VaultPath),
    /// Another template has the name.
    #[error("another template is already called {name:?}; choose another name")]
    NameTaken {
        /// The name asked for.
        name: String,
        /// The template that has it.
        path: VaultPath,
    },
    /// The file is already there.
    #[error("{0} already exists")]
    Exists(VaultPath),
    /// The file isn't what the caller was shown.
    #[error("{path} changed on disk since it was opened; reload it to see the change")]
    Changed {
        /// The file.
        path: VaultPath,
        /// Its hash now.
        current: String,
    },
    /// Invoices or customers name it (delete).
    #[error("{}", in_use_message(.invoices, .customers))]
    InUse {
        /// The documents' numbers (or titles).
        invoices: Vec<String>,
        /// The customers' titles.
        customers: Vec<String>,
    },
    /// The file isn't UTF-8 text.
    #[error("{0} isn't UTF-8 text")]
    NotText(VaultPath),
    /// The file can't be read or written.
    #[error(transparent)]
    Vault(#[from] VaultError),
}

fn in_use_message(invoices: &[String], customers: &[String]) -> String {
    let mut parts = Vec::new();
    match invoices {
        [] => {}
        [one] => parts.push(format!("Invoice {one} uses it")),
        many => parts.push(format!("Invoices {} use it", many.join(", "))),
    }
    match customers {
        [] => {}
        [one] => parts.push(format!("{one} starts new invoices with it")),
        many => parts.push(format!("{} start new invoices with it", many.join(", "))),
    }
    format!(
        "{}; choose another template for them first",
        parts.join(" and ")
    )
}

fn invalid(message: impl Into<String>) -> TemplateError {
    TemplateError::Invalid(message.into())
}

/// A display name as written in the header, or why not: one line, no `"`
/// (the header's quote) and no `#}` (the comment's end), at most
/// [`MAX_NAME`] characters.
///
/// # Errors
/// [`TemplateError::Invalid`].
pub fn clean_name(name: &str) -> Result<String, TemplateError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("a template needs a name"));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid("the name: one line of text, please"));
    }
    if name.contains('"') || name.contains("#}") {
        return Err(invalid("a template's name can't hold \" or #}"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(invalid(format!(
            "the name is longer than {MAX_NAME} characters"
        )));
    }
    Ok(name.to_string())
}

/// Kinds as written: known document types, each once, in [`DocType::ALL`]'s
/// order; at least one.
///
/// # Errors
/// [`TemplateError::Invalid`].
pub fn clean_kinds(kinds: &[String]) -> Result<Vec<String>, TemplateError> {
    let mut wanted: Vec<DocType> = Vec::new();
    for kind in kinds {
        let parsed = DocType::parse(kind.trim()).ok_or_else(|| {
            invalid(format!(
                "{:?} isn't a kind of document (invoice, estimate or credit-note)",
                kind.trim()
            ))
        })?;
        if !wanted.contains(&parsed) {
            wanted.push(parsed);
        }
    }
    if wanted.is_empty() {
        return Err(invalid("a template is for at least one kind of document"));
    }
    Ok(DocType::ALL
        .into_iter()
        .filter(|t| wanted.contains(t))
        .map(|t| t.as_str().to_string())
        .collect())
}

/// The header line for `name` and `kinds`, with its line end.
#[must_use]
pub fn header_line(name: &str, kinds: &[String]) -> String {
    format!(
        "{{# invoicer: name=\"{name}\" kinds=\"{}\" #}}\n",
        kinds.join(",")
    )
}

/// The body of a new blank child: `extends` the base, nothing overridden
/// (so it looks as the base does). With the header, `extends` is line 2.
#[must_use]
pub fn blank_body(base: &BaseTemplate<'_>) -> String {
    format!("{{% extends \"{}\" %}}\n", base.file)
}

/// The file stem a name gets: lower case, letters and digits, `-` between
/// words, at most [`MAX_SLUG`] characters; `template` when nothing is left.
/// Never starts with `_` (a base's or partial's mark).
#[must_use]
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed: String = out.trim_matches('-').chars().take(MAX_SLUG).collect();
    let trimmed = trimmed.trim_end_matches('-');
    if trimmed.is_empty() {
        "template".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The templates folder (`templates-dir`) as a path.
fn templates_dir(index: &DocumentIndex) -> Result<VaultPath, TemplateError> {
    let dir = index.settings().templates_dir.trim().trim_matches('/');
    VaultPath::new(dir).map_err(|e| {
        invalid(format!(
            "templates-dir {dir:?} isn't a folder in the books: {e}"
        ))
    })
}

/// Refused when another template than `except` is called `name`.
fn free_name(
    index: &DocumentIndex,
    vault: &dyn Vault,
    name: &str,
    except: Option<&VaultPath>,
) -> Result<(), TemplateError> {
    let taken = templates::discover(vault, index.site())
        .into_iter()
        .find(|t| Some(&t.path) != except && t.name.trim().eq_ignore_ascii_case(name));
    match taken {
        Some(t) => Err(TemplateError::NameTaken {
            name: name.to_string(),
            path: t.path,
        }),
        None => Ok(()),
    }
}

/// Refused when `text` isn't a template the invoicer would find.
fn still_a_template(text: &str) -> Result<(), TemplateError> {
    if text.len() > MAX_TEMPLATE_BYTES {
        return Err(invalid(format!(
            "a template is at most {MAX_TEMPLATE_BYTES} bytes"
        )));
    }
    if templates::is_template(text) {
        Ok(())
    } else {
        Err(invalid(format!(
            "the invoicer wouldn't find this template: it neither extends the invoice base \
             ({{% extends \"{BASE_PREFIX}….html\" %}}) nor shows the invoice number \
             (frontmatter.invoice_num)"
        )))
    }
}

/// Copy `base` in beside `path` when `text` extends it and it isn't there:
/// a create-new, so a base already there (the user's) stays as it is.
fn ensure_base(
    vault: &dyn Vault,
    path: &VaultPath,
    text: &str,
    base: Option<&BaseTemplate<'_>>,
) -> Result<Option<VaultPath>, TemplateError> {
    let Some(base) = base else {
        return Ok(None);
    };
    if tera_scan::extends(text) != Some(base.file) {
        return Ok(None);
    }
    let folder = path.parent().unwrap_or_else(VaultPath::root);
    let target = folder
        .child(base.file)
        .map_err(|e| invalid(e.to_string()))?;
    if vault.stat(&target)?.is_some() {
        return Ok(None);
    }
    match vault.create_new_shared(&target, base.source.as_bytes()) {
        Ok(_) => Ok(Some(target)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// A new template: its name, its kinds (default: those of what it starts
/// from, else `invoice`), and what it starts from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTemplate<'a> {
    /// The display name.
    pub name: &'a str,
    /// The document types it is for.
    pub kinds: Option<Vec<String>>,
    /// The whole text of a template to start from (a starter's, or one to
    /// duplicate): its body is kept and its header replaced. `None`: a
    /// blank child of `base`.
    pub from: Option<&'a str>,
}

/// Create a template: `<templates-dir>/<slug>.html` (`-2`, `-3`… when the
/// name is taken by a file), the header line, then the body. The base is
/// copied in beside it when the body extends it and it isn't there.
/// Indexed at once.
///
/// # Errors
/// [`TemplateError::Invalid`], [`TemplateError::NameTaken`], or a vault
/// error.
pub fn create(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    new: &NewTemplate<'_>,
    base: &BaseTemplate<'_>,
) -> Result<TemplateSaved, TemplateError> {
    let name = clean_name(new.name)?;
    free_name(index, vault, &name, None)?;
    let (body, start_kinds) = match new.from {
        Some(text) => (
            body_of(text).to_string(),
            templates::header(text).map(|h| h.kinds).unwrap_or_default(),
        ),
        None => (blank_body(base), Vec::new()),
    };
    let kinds = match &new.kinds {
        Some(kinds) => clean_kinds(kinds)?,
        None if !start_kinds.is_empty() => clean_kinds(&start_kinds)?,
        None => vec![DocType::Invoice.as_str().to_string()],
    };
    let text = format!("{}{body}", header_line(&name, &kinds));
    still_a_template(&text)?;
    let dir = templates_dir(index)?;
    vault.create_dir_all(&dir)?;
    let stem = slug(&name);
    let mut written = None;
    for n in 1..=99 {
        let file = if n == 1 {
            format!("{stem}.html")
        } else {
            format!("{stem}-{n}.html")
        };
        let path = dir.child(&file).map_err(|e| invalid(e.to_string()))?;
        match vault.create_new_shared(&path, text.as_bytes()) {
            Ok(_) => {
                written = Some(path);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    let path = written.ok_or_else(|| TemplateError::Exists(dir.clone()))?;
    let base_written = ensure_base(vault, &path, &text, Some(base))?;
    let mut refreshed = vec![path.clone()];
    refreshed.extend(base_written.iter().cloned());
    index.refresh(vault, &refreshed);
    Ok(TemplateSaved {
        template: format!("/{path}"),
        name,
        hash: content_hash(text.as_bytes()),
        written: true,
        changed: vec!["name".into(), "kinds".into(), "body".into()],
        base: base_written,
        path,
    })
}

/// The template at `path` and its revision; refused when it isn't one.
fn read_template(vault: &dyn Vault, path: &VaultPath) -> Result<(String, Revision), TemplateError> {
    let (bytes, revision) = vault
        .read_pinned(path, Some(MAX_TEMPLATE_BYTES as u64))
        .map_err(|e| {
            if e.is_not_found() {
                TemplateError::NotFound(path.clone())
            } else {
                TemplateError::Vault(e)
            }
        })?;
    let source = String::from_utf8(bytes).map_err(|_| TemplateError::NotText(path.clone()))?;
    if templates::Template::from_source(path.clone(), &source).is_none() {
        return Err(TemplateError::NotATemplate(path.clone()));
    }
    Ok((source, revision))
}

/// [`read_template`], if the file is still the bytes `expected` names.
fn read_expected(
    vault: &dyn Vault,
    path: &VaultPath,
    expected: &str,
) -> Result<(String, Revision), TemplateError> {
    let (source, revision) = read_template(vault, path)?;
    let current = revision.fingerprint.token();
    if current != expected {
        return Err(TemplateError::Changed {
            path: path.clone(),
            current,
        });
    }
    Ok((source, revision))
}

/// Edit the template at `path`: the fields `fields` names, when they
/// change. `expected` is the hash the caller was shown. A name and kinds
/// that read the same keep the header line byte for byte; a template
/// without one gets one only when they change. When the text extends
/// `base` and it isn't beside the template, it is copied in.
///
/// # Errors
/// [`TemplateError::Changed`], [`TemplateError::Invalid`],
/// [`TemplateError::NameTaken`], [`TemplateError::NotFound`],
/// [`TemplateError::NotATemplate`], or a vault error.
pub fn save(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    expected: &str,
    fields: &TemplateFields,
    base: Option<&BaseTemplate<'_>>,
) -> Result<TemplateSaved, TemplateError> {
    let (source, revision) = read_expected(vault, path, expected)?;
    let (head_line, old_body) = split_header(&source);
    let shape = shape_of(path, &source);
    let mut changed: Vec<String> = Vec::new();
    let mut name = shape.name.clone();
    if let Some(wanted) = &fields.name {
        let wanted = clean_name(wanted)?;
        if wanted != shape.name {
            free_name(index, vault, &wanted, Some(path))?;
            name = wanted;
            changed.push("name".into());
        }
    }
    let mut kinds = shape.kinds.clone();
    if let Some(wanted) = &fields.kinds {
        let wanted = clean_kinds(wanted)?;
        if wanted != clean_kinds(&shape.kinds).unwrap_or_default() {
            kinds = wanted;
            changed.push("kinds".into());
        }
    }
    let head = if changed.is_empty() {
        head_line.to_string()
    } else {
        header_line(&name, &kinds)
    };
    let body = match &fields.body {
        Some(body) if body != old_body => {
            changed.push("body".into());
            body.as_str()
        }
        _ => old_body,
    };
    let text = format!("{head}{body}");
    let written = text != source;
    if written {
        still_a_template(&text)?;
        vault
            .replace(path, text.as_bytes(), &revision)
            .map_err(|e| match e {
                VaultError::Changed { .. } => TemplateError::Changed {
                    path: path.clone(),
                    current: vault
                        .read(path)
                        .map(|b| content_hash(&b))
                        .unwrap_or_default(),
                },
                other => TemplateError::Vault(other),
            })?;
    }
    let base_written = ensure_base(vault, path, &text, base)?;
    if written || base_written.is_some() {
        let mut refreshed = vec![path.clone()];
        refreshed.extend(base_written.iter().cloned());
        index.refresh(vault, &refreshed);
    }
    Ok(TemplateSaved {
        template: format!("/{path}"),
        name,
        hash: content_hash(text.as_bytes()),
        written,
        changed,
        base: base_written,
        path: path.clone(),
    })
}

/// Delete the template at `path`, if it is still the bytes `expected`
/// names. Refused while any invoice or customer names it: their pages would
/// lose their look (MBR says "template not found"), so they move to another
/// template first. The base stays: other templates may extend it.
///
/// # Errors
/// [`TemplateError::InUse`], [`TemplateError::Changed`],
/// [`TemplateError::NotFound`], [`TemplateError::NotATemplate`], or a vault
/// error.
pub fn delete(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    expected: &str,
) -> Result<(), TemplateError> {
    read_expected(vault, path, expected)?;
    let used = uses(index, path.as_str());
    if !used.is_empty() {
        let pick = |how: UseKind| -> Vec<String> {
            used.iter()
                .filter(|u| u.how == how)
                .map(|u| u.number.clone().unwrap_or_else(|| u.title.clone()))
                .collect()
        };
        return Err(TemplateError::InUse {
            invoices: pick(UseKind::Invoice),
            customers: pick(UseKind::Customer),
        });
    }
    vault.remove_file(path)?;
    index.refresh(vault, std::slice::from_ref(path));
    Ok(())
}

/// The header a template's text gives, for callers that have only text
/// (a starter): its name and kinds.
#[must_use]
pub fn header_of(source: &str) -> TemplateHeader {
    templates::header(source).unwrap_or_default()
}

#[cfg(test)]
mod tests;
