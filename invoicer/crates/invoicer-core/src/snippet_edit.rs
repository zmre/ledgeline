//! Editing snippets (invoicer PLAN 4c, ARCHITECTURE §3.7): `type: snippet`
//! notes whose markdown body an invoice copies. One library call each,
//! shared by the API and, later, the CLI.
//!
//! - [`list`] gives every snippet with its body, its content hash and where
//!   it is used: the customers whose `snippets:` name it (a new draft copies
//!   it), and the documents that hold a copy of its body as it is now.
//! - [`create`] writes a new note in `<templates-dir>/snippets/`, named after
//!   its title, never replacing a file, and refusing a title another note
//!   already answers to.
//! - [`update`] edits the title and `snippet_kind` **by key**
//!   ([`frontmatter::edit`]) and replaces the body; every other key, comment
//!   and the frontmatter's layout stay byte for byte. A rename keeps the old
//!   title in `aliases:`, so the customers' `[[Old title]]` still resolve
//!   (the file keeps its name).
//! - [`delete`] removes the note; refused while a customer names it unless
//!   forced (their drafts would silently lose it). Invoices only ever hold
//!   copies, so they never change.
//!
//! Every write names the content hash the caller was shown and is refused
//! ([`SnippetError::Changed`]) when the file is no longer those bytes, as
//! customers (K2).

use crate::customer_edit::{content_hash, file_stem, link_safe_title};
use crate::frontmatter::{self, EditError, FieldEdit, locate};
use crate::index::DocumentIndex;
use crate::notes::{Note, Stamp};
use crate::snippets::{self, Snippet, body_of};
use ledgeline_core::vault::{Fingerprint, Revision, Vault, VaultError, VaultPath};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

/// The longest `snippet_kind` accepted.
const MAX_KIND: usize = 40;

/// The snippet form: every field optional on an update (absent leaves it);
/// a create needs `title`. `kind` `""` removes `snippet_kind`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SnippetFields {
    /// `title:`, what `[[Net 30 ACH]]` names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `snippet_kind:` (`payment`, `terms`, `note`, or another word).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The markdown body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// How a snippet is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UseKind {
    /// A customer's `snippets:` names it: its new drafts copy it.
    Default,
    /// A document holds a copy of its body as it is now.
    Copy,
}

/// One note that uses a snippet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnippetUse {
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// How.
    pub how: UseKind,
    /// The document's number (a copy).
    pub number: Option<String>,
}

/// One snippet, as the editor lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnippetEntry {
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// `snippet_kind`, as written.
    pub kind: Option<String>,
    /// `aliases:` (older titles, after a rename).
    pub aliases: Vec<String>,
    /// The body, as [`body_of`] gives it.
    pub body: String,
    /// The file's content hash: what a save names.
    pub hash: String,
    /// Customers naming it, then documents holding a copy.
    pub used_by: Vec<SnippetUse>,
}

/// What a create or an update wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnippetSaved {
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// `[[Title]]`, what a customer's `snippets:` writes.
    pub link: String,
    /// `snippet_kind`.
    pub kind: Option<String>,
    /// The content hash now: what the next save names.
    pub hash: String,
    /// Whether the file was written.
    pub written: bool,
    /// What changed (`title`, `aliases`, `snippet_kind`, `body`), in order.
    pub changed: Vec<String>,
}

/// Why a snippet edit refused. Nothing is written when one is returned.
#[derive(Debug, Error)]
pub enum SnippetError {
    /// A field doesn't read.
    #[error("{0}")]
    Invalid(String),
    /// No note there.
    #[error("{0}: no such note")]
    NotFound(VaultPath),
    /// The note isn't `type: snippet`.
    #[error("{0} isn't a snippet (`type: snippet`)")]
    NotASnippet(VaultPath),
    /// A note already answers to the title.
    #[error("a note named {title:?} already exists ({path}); choose another name")]
    NameTaken {
        /// The title asked for.
        title: String,
        /// The note that has it.
        path: VaultPath,
    },
    /// The file is already there.
    #[error("{0} already exists")]
    Exists(VaultPath),
    /// The file isn't what the caller was shown.
    #[error("{path} changed on disk since it was opened; reload it to see the change")]
    Changed {
        /// The note.
        path: VaultPath,
        /// Its hash now.
        current: String,
    },
    /// Customers name it as a default snippet (delete without `force`).
    #[error("{} still name it as a default snippet; remove it from their `snippets:` first, or delete it anyway", .customers.join(", "))]
    InUse {
        /// Their titles.
        customers: Vec<String>,
    },
    /// The frontmatter can't be edited safely (edit it by hand).
    #[error(transparent)]
    Frontmatter(#[from] EditError),
    /// The file isn't UTF-8 text.
    #[error("{0} isn't UTF-8 text")]
    NotText(VaultPath),
    /// The file can't be read or written.
    #[error(transparent)]
    Vault(#[from] VaultError),
}

fn note_of(path: &VaultPath, source: &str) -> Note {
    Note::from_source(
        path.clone(),
        source,
        Stamp {
            modified: None,
            size: source.len() as u64,
            fingerprint: Fingerprint::of_bytes(source.as_bytes()),
        },
    )
}

/// The customers naming `path` as a default snippet: `(title, note)`.
fn customers_naming(index: &DocumentIndex, path: &VaultPath) -> Vec<(String, VaultPath)> {
    let site = index.site();
    index
        .customers()
        .into_iter()
        .filter(|c| {
            c.snippets.iter().any(|link| {
                snippets::find(site, &c.contact.path, link).is_some_and(|s| &s.path == path)
            })
        })
        .map(|c| (c.contact.title, c.contact.path))
        .collect()
}

/// Every snippet, by path, each with its body, hash and uses. Read now, not
/// as the index last saw them; a document is read once for all of them.
#[must_use]
pub fn list(index: &DocumentIndex, vault: &dyn Vault) -> Vec<SnippetEntry> {
    let mut entries: Vec<SnippetEntry> = snippets::snippets(index.site())
        .into_iter()
        .filter_map(|snippet: Snippet| {
            let source = vault.read_to_string(&snippet.path).ok()?;
            let note = note_of(&snippet.path, &source);
            let used_by = customers_naming(index, &snippet.path)
                .into_iter()
                .map(|(title, path)| SnippetUse {
                    path,
                    title,
                    how: UseKind::Default,
                    number: None,
                })
                .collect();
            Some(SnippetEntry {
                title: note.title.clone(),
                kind: note.str("snippet_kind").map(|k| k.trim().to_string()),
                aliases: note.aliases.clone(),
                body: body_of(&source),
                hash: content_hash(source.as_bytes()),
                path: snippet.path,
                used_by,
            })
        })
        .collect();
    // Copies: each document's text, read once.
    let mut docs: Vec<(&VaultPath, &str, Option<&str>)> = index
        .docs()
        .map(|d| (&d.path, d.title.as_str(), d.number()))
        .collect();
    docs.sort_by(|a, b| a.0.cmp(b.0));
    let mut sources: HashMap<&VaultPath, String> = HashMap::new();
    for (path, _, _) in &docs {
        if let Ok(text) = vault.read_to_string(path) {
            sources.insert(path, text.replace("\r\n", "\n"));
        }
    }
    for entry in &mut entries {
        let body = entry.body.trim();
        if body.is_empty() {
            continue;
        }
        for (path, title, number) in &docs {
            if sources.get(path).is_some_and(|text| text.contains(body)) {
                entry.used_by.push(SnippetUse {
                    path: (*path).clone(),
                    title: (*title).to_string(),
                    how: UseKind::Copy,
                    number: number.map(str::to_string),
                });
            }
        }
    }
    entries
}

fn invalid(message: impl Into<String>) -> SnippetError {
    SnippetError::Invalid(message.into())
}

fn clean_kind(kind: &str) -> Result<Option<String>, SnippetError> {
    let kind = kind.trim();
    if kind.chars().any(char::is_control) {
        return Err(invalid("the kind: one line of text, please"));
    }
    if kind.chars().count() > MAX_KIND {
        return Err(invalid(format!(
            "the kind is longer than {MAX_KIND} characters"
        )));
    }
    Ok((!kind.is_empty()).then(|| kind.to_string()))
}

/// A body as written: `\n` line ends, no blank lines at either end, one
/// final newline (empty when there is nothing).
fn clean_body(body: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let body = body.trim_matches('\n');
    if body.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n", body.trim_end())
    }
}

/// `source` with everything after its frontmatter replaced by `body`, one
/// blank line between them.
fn with_body(source: &str, body: &str) -> String {
    let start = locate(source).map_or(0, |b| b.body_start);
    let mut out = source[..start].to_string();
    if start > 0 && !out.ends_with('\n') {
        out.push('\n');
    }
    if !body.is_empty() {
        if start > 0 {
            out.push('\n');
        }
        out.push_str(body);
    }
    out
}

fn saved(path: VaultPath, text: &str, written: bool, changed: Vec<String>) -> SnippetSaved {
    let note = note_of(&path, text);
    SnippetSaved {
        link: format!("[[{}]]", note.title),
        title: note.title.clone(),
        kind: note.str("snippet_kind").map(|k| k.trim().to_string()),
        hash: content_hash(text.as_bytes()),
        written,
        changed,
        path,
    }
}

/// The title `title` would take, or why not; refused when another note
/// (not `except`) answers to it.
fn free_title(
    index: &DocumentIndex,
    title: &str,
    except: Option<&VaultPath>,
) -> Result<String, SnippetError> {
    let title = link_safe_title(title, "snippet").map_err(SnippetError::Invalid)?;
    if let Some(note) = index
        .site()
        .resolve_wikilink(&VaultPath::root(), &title)
        .filter(|n| Some(&n.path) != except)
    {
        return Err(SnippetError::NameTaken {
            title,
            path: note.path.clone(),
        });
    }
    Ok(title)
}

/// Create a snippet: `<templates-dir>/snippets/<Title>.md` with `title`,
/// `type: snippet`, `snippet_kind` when given, and the body. Indexed at once.
///
/// # Errors
/// [`SnippetError::Invalid`], [`SnippetError::NameTaken`],
/// [`SnippetError::Exists`], or a vault error.
pub fn create(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    fields: &SnippetFields,
) -> Result<SnippetSaved, SnippetError> {
    let title = free_title(index, fields.title.as_deref().unwrap_or_default(), None)?;
    let kind = match &fields.kind {
        Some(kind) => clean_kind(kind)?,
        None => None,
    };
    let dir = format!(
        "{}/snippets",
        index.settings().templates_dir.trim_matches('/')
    );
    let dir = dir.trim_start_matches('/');
    let name = format!("{dir}/{}.md", file_stem(&title));
    let path = VaultPath::new(&name)
        .map_err(|e| invalid(format!("{name:?} can't be a file in the books: {e}")))?;
    let mut block: Vec<(&str, Value)> = vec![
        ("title", Value::String(title.clone())),
        ("type", Value::String("snippet".into())),
    ];
    if let Some(kind) = kind {
        block.push(("snippet_kind", Value::String(kind)));
    }
    let head = frontmatter::render_block(&block)?;
    let text = with_body(
        &head,
        &clean_body(fields.body.as_deref().unwrap_or_default()),
    );
    if let Some(parent) = path.parent() {
        vault.create_dir_all(&parent)?;
    }
    vault
        .create_new_shared(&path, text.as_bytes())
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                SnippetError::Exists(path.clone())
            } else {
                SnippetError::Vault(e)
            }
        })?;
    index.refresh(vault, std::slice::from_ref(&path));
    let changed = block.iter().map(|(k, _)| (*k).to_string()).collect();
    Ok(saved(path, &text, true, changed))
}

/// The snippet note at `path`, if the file is still the bytes `expected`
/// names.
fn read_expected(
    vault: &dyn Vault,
    path: &VaultPath,
    expected: &str,
) -> Result<(String, Revision, Note), SnippetError> {
    let (bytes, revision) = vault.read_pinned(path, None).map_err(|e| {
        if e.is_not_found() {
            SnippetError::NotFound(path.clone())
        } else {
            SnippetError::Vault(e)
        }
    })?;
    let source = String::from_utf8(bytes).map_err(|_| SnippetError::NotText(path.clone()))?;
    let note = note_of(path, &source);
    if !note.is_type("snippet") {
        return Err(SnippetError::NotASnippet(path.clone()));
    }
    let current = revision.fingerprint.token();
    if current != expected {
        return Err(SnippetError::Changed {
            path: path.clone(),
            current,
        });
    }
    Ok((source, revision, note))
}

/// Edit the snippet at `path`: the fields `fields` names, when they change.
/// `expected` is the hash the caller was shown.
///
/// # Errors
/// [`SnippetError::Changed`], [`SnippetError::Invalid`],
/// [`SnippetError::NameTaken`], [`SnippetError::NotFound`],
/// [`SnippetError::NotASnippet`], [`SnippetError::Frontmatter`], or a vault
/// error.
pub fn update(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    expected: &str,
    fields: &SnippetFields,
) -> Result<SnippetSaved, SnippetError> {
    let (source, revision, note) = read_expected(vault, path, expected)?;
    let mut edits: Vec<FieldEdit> = Vec::new();
    let mut changed: Vec<String> = Vec::new();
    if let Some(title) = &fields.title
        && title.trim() != note.title
    {
        let title = free_title(index, title, Some(path))?;
        // The old title stays a name for it, so `[[Old title]]` still
        // resolves; a title taken back leaves the aliases.
        let mut aliases: Vec<String> = note
            .aliases
            .iter()
            .filter(|a| !a.eq_ignore_ascii_case(&title))
            .cloned()
            .collect();
        if !note.title.trim().is_empty()
            && !aliases.iter().any(|a| a.eq_ignore_ascii_case(&note.title))
        {
            aliases.push(note.title.clone());
        }
        edits.push(FieldEdit::Set("title".into(), Value::String(title)));
        changed.push("title".into());
        if aliases != note.aliases {
            edits.push(if aliases.is_empty() {
                FieldEdit::Remove("aliases".into())
            } else {
                FieldEdit::Set(
                    "aliases".into(),
                    Value::Array(aliases.into_iter().map(Value::String).collect()),
                )
            });
            changed.push("aliases".into());
        }
    }
    if let Some(kind) = &fields.kind {
        let kind = clean_kind(kind)?;
        let current = note.str("snippet_kind").map(str::trim);
        if kind.as_deref() != current {
            edits.push(match kind {
                Some(kind) => FieldEdit::Set("snippet_kind".into(), Value::String(kind)),
                None => FieldEdit::Remove("snippet_kind".into()),
            });
            changed.push("snippet_kind".into());
        }
    }
    let mut text = frontmatter::edit(&source, &edits)?;
    if let Some(body) = &fields.body {
        let body = clean_body(body);
        if body.trim() != body_of(&source).trim() {
            text = with_body(&text, &body);
            changed.push("body".into());
        }
    }
    let written = text != source;
    if written {
        vault
            .replace(path, text.as_bytes(), &revision)
            .map_err(|e| match e {
                VaultError::Changed { .. } => SnippetError::Changed {
                    path: path.clone(),
                    current: vault
                        .read(path)
                        .map(|b| content_hash(&b))
                        .unwrap_or_default(),
                },
                other => SnippetError::Vault(other),
            })?;
        index.refresh(vault, std::slice::from_ref(path));
    }
    Ok(saved(path.clone(), &text, written, changed))
}

/// Delete the snippet at `path`, if it is still the bytes `expected` names.
/// Refused while customers name it as a default, unless `force`.
///
/// # Errors
/// [`SnippetError::InUse`], [`SnippetError::Changed`],
/// [`SnippetError::NotFound`], [`SnippetError::NotASnippet`], or a vault
/// error.
pub fn delete(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    expected: &str,
    force: bool,
) -> Result<(), SnippetError> {
    read_expected(vault, path, expected)?;
    let customers: Vec<String> = customers_naming(index, path)
        .into_iter()
        .map(|(title, _)| title)
        .collect();
    if !customers.is_empty() && !force {
        return Err(SnippetError::InUse { customers });
    }
    vault.remove_file(path)?;
    index.refresh(vault, std::slice::from_ref(path));
    Ok(())
}

#[cfg(test)]
mod tests;
