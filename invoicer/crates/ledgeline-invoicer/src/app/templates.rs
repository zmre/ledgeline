//! The templates list and the template editor (invoicer PLAN 3b, 4d). The
//! writes are the core's ([`invoicer_core::template_edit`]), so the CLI can
//! share them; the starters and the shipped base are this crate's
//! ([`crate::starters`]).
//!
//! | Route                               | Does                                                                     |
//! |-------------------------------------|--------------------------------------------------------------------------|
//! | `GET /api/templates`                | The books' templates by name, then the starters not copied in ([`TemplateEntry`]). |
//! | `POST /api/templates`               | `{name, kinds?, from?}`: a new child of the base, or a copy of `from` → `201` [`TemplateSaved`]. |
//! | `POST /api/templates/customize`     | `{template}`: copy a starter into the books (with the base) → `{path, written, base}`. |
//! | `POST /api/templates/preview`       | `{number, template, body}`: render unsaved text → `{url, problem}`.     |
//! | `GET /api/templates/{*path}`        | One template, its `body` (the text after the name line) and the base's blocks. |
//! | `PUT /api/templates/{*path}`        | `{hash, fields: {name?, kinds?, body?}}` → [`TemplateSaved`].             |
//! | `DELETE /api/templates/{*path}`     | `{hash}` → `{deleted}`; `409` `in-use` while invoices or customers name it. |
//!
//! `path` is the template's path in the books folder (`invoices/tpl/x.html`);
//! a starter not copied in is read at the path it would have, read-only.
//! Errors are `{"error": "…", "code": "…"}` as in [`super::library`]:
//! `changed` (409, with `revision`), `invalid` (422), `name-taken` (409),
//! `in-use` (409, with `invoices` and `customers`), `not-a-starter` (422),
//! `not-found` (404), `bad-request` (400), `storage` (500).
//!
//! **The preview** renders the unsaved body as the template at `template`
//! for the invoice `number` (MBR 0.7.4's `render_source_with_template`, M10:
//! its siblings, the base among them, come from disk), keeps the page under
//! a random id ([`super::preview::DraftPreviews::put_page`]) and answers the
//! preview page's URL; the frame loads it behind MBR's gate (no `srcdoc`,
//! F1). A template that doesn't render answers `200` with `url: null` and
//! `problem: {message, line, column}`: Tera's error in plain words
//! ([`plain_problem`]), its line in the body when Tera or the text gives one.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use invoicer_core::session::Session;
use invoicer_core::template_edit::{
    self, BaseBlock, BaseTemplate, NewTemplate, TemplateError, TemplateFields, TemplateUse,
};
use ledgeline_core::vault::{Vault, VaultPath};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::AppState;
use super::api::{ApiError, ApiResult, body, to_value, with_books};
use super::books::OpenBooks;
use super::preview::{self, PREVIEW_PAGE, PreviewError, PreviewRequest};
use crate::starters;

/// The template routes (state still to be supplied).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/templates", get(list).post(create))
        .route("/api/templates/customize", post(customize))
        .route("/api/templates/preview", post(preview_template))
        .route("/api/templates/{*path}", get(one).put(save).delete(remove))
}

/// The shipped base, as the core takes it.
const BASE: BaseTemplate<'static> = BaseTemplate {
    file: starters::BASE.file,
    source: starters::BASE.source,
};

/// One template in `GET /api/templates`: in the books, or a starter not
/// copied in yet (which a preview can use, and an invoice copies in).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TemplateEntry {
    /// The `template:` value naming it.
    pub path: String,
    /// Its display name.
    pub name: String,
    /// The document types it is for.
    pub kinds: Vec<String>,
    /// One of the shipped starters (copied in or not).
    pub starter: bool,
    /// A file in the books (`false`: a starter not copied in yet).
    pub installed: bool,
    /// The file's content hash (`None`: not in the books).
    pub hash: Option<String>,
    /// The file it extends (`_invoice_base.v1.html`), if any.
    pub extends: Option<String>,
    /// The base version it extends, when that is an invoice base.
    pub base_version: Option<u32>,
    /// The blocks it overrides.
    pub overrides: Vec<String>,
    /// The invoices naming it, then the customers.
    pub used_by: Vec<TemplateUse>,
}

/// A starter not in the books, as listed.
fn starter_entry(
    session: &Session,
    starter: &starters::Starter,
    path: &VaultPath,
) -> TemplateEntry {
    let shape = template_edit::shape_of(path, starter.source);
    TemplateEntry {
        path: format!("/{path}"),
        name: starter.name.to_string(),
        kinds: shape.kinds,
        starter: true,
        installed: false,
        hash: None,
        extends: shape.extends,
        base_version: shape.base_version,
        overrides: shape.overrides,
        used_by: template_edit::uses(session.index(), path.as_str()),
    }
}

/// The templates list, from a session already locked: the books' by name,
/// then each starter not copied in.
pub(crate) fn templates_in(session: &Session) -> Vec<TemplateEntry> {
    let dir = session.settings().templates_dir.clone();
    let mut out: Vec<TemplateEntry> = template_edit::list(session.index(), session.vault())
        .into_iter()
        .map(|t| TemplateEntry {
            starter: starters::named(&dir, &t.template).is_some(),
            path: t.template,
            name: t.shape.name,
            kinds: t.shape.kinds,
            installed: true,
            hash: Some(t.hash),
            extends: t.shape.extends,
            base_version: t.shape.base_version,
            overrides: t.shape.overrides,
            used_by: t.used_by,
        })
        .collect();
    for starter in &starters::STARTERS {
        let Ok(path) = starters::path_in(&dir, starter) else {
            continue;
        };
        let value = format!("/{path}");
        if out.iter().any(|t| t.path == value) {
            continue;
        }
        out.push(starter_entry(session, starter, &path));
    }
    out
}

/// How many templates [`templates_in`] lists, without reading who uses
/// them (the sidebar's count, `GET /api/books`).
pub(crate) fn count_in(session: &Session) -> usize {
    let dir = session.settings().templates_dir.clone();
    let found = session.index().templates();
    let starters = starters::STARTERS
        .iter()
        .filter_map(|s| starters::path_in(&dir, s).ok())
        .filter(|p| found.iter().all(|t| &t.path != p))
        .count();
    found.len() + starters
}

impl From<TemplateError> for ApiError {
    fn from(error: TemplateError) -> Self {
        let message = error.to_string();
        match error {
            TemplateError::Invalid(_) | TemplateError::NotText(_) => {
                Self::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid", message)
            }
            TemplateError::NotFound(_) | TemplateError::NotATemplate(_) => {
                Self::new(StatusCode::NOT_FOUND, "not-found", message)
            }
            TemplateError::NameTaken { .. } => {
                Self::new(StatusCode::CONFLICT, "name-taken", message)
            }
            TemplateError::Exists(_) => Self::new(StatusCode::CONFLICT, "exists", message),
            TemplateError::Changed { current, .. } => {
                Self::new(StatusCode::CONFLICT, "changed", message)
                    .with("revision", Value::String(current))
            }
            TemplateError::InUse {
                invoices,
                customers,
            } => Self::new(StatusCode::CONFLICT, "in-use", message)
                .with("invoices", json!(invoices))
                .with("customers", json!(customers)),
            TemplateError::Vault(_) => {
                Self::new(StatusCode::INTERNAL_SERVER_ERROR, "storage", message)
            }
        }
    }
}

/// The template `key` names: a path in the books folder, `.html`.
fn template_path(key: &str) -> Result<VaultPath, ApiError> {
    preview::template_path(key).map_err(ApiError::from)
}

/// `GET /api/templates`.
async fn list(State(state): State<AppState>) -> ApiResult {
    with_books(&state, StatusCode::OK, |books| {
        to_value(templates_in(&books.session()))
    })
    .await
}

/// One template, for the editor.
#[derive(Debug, Serialize)]
struct TemplateDetail {
    #[serde(flatten)]
    entry: TemplateEntry,
    /// The text after the name line.
    body: String,
    /// A starter not copied in: Customize copies it first.
    read_only: bool,
    /// The blocks of the base it extends, each with what it is for.
    base_blocks: Vec<BaseBlock>,
}

/// The blocks of `parent`, the file the template at `path` extends: the one
/// beside it, else the shipped base when it names that.
fn parent_blocks(vault: &dyn Vault, path: &VaultPath, parent: Option<&str>) -> Vec<BaseBlock> {
    let Some(parent) = parent else {
        return Vec::new();
    };
    let beside = path
        .parent()
        .unwrap_or_else(VaultPath::root)
        .child(parent)
        .ok()
        .and_then(|p| vault.read_to_string(&p).ok());
    match beside {
        Some(text) => template_edit::base_blocks(&text),
        None if parent == BASE.file => template_edit::base_blocks(BASE.source),
        None => Vec::new(),
    }
}

/// `GET /api/templates/{path}`.
async fn one(State(state): State<AppState>, Path(key): Path<String>) -> ApiResult {
    with_books(&state, StatusCode::OK, move |books| {
        let path = template_path(&key)?;
        let session = books.session();
        let vault = session.vault();
        let dir = session.settings().templates_dir.clone();
        let on_disk = vault.stat(&path).map_err(TemplateError::from)?.is_some();
        let starter = starters::named(&dir, path.as_str());
        let detail = match (on_disk, starter) {
            (false, Some(starter)) => {
                let entry = starter_entry(&session, starter, &path);
                TemplateDetail {
                    base_blocks: parent_blocks(vault, &path, entry.extends.as_deref()),
                    body: template_edit::body_of(starter.source).to_string(),
                    read_only: true,
                    entry,
                }
            }
            _ => {
                let file = template_edit::read(session.index(), vault, &path)?;
                let entry = TemplateEntry {
                    starter: starter.is_some(),
                    path: file.entry.template,
                    name: file.entry.shape.name,
                    kinds: file.entry.shape.kinds,
                    installed: true,
                    hash: Some(file.entry.hash),
                    extends: file.entry.shape.extends,
                    base_version: file.entry.shape.base_version,
                    overrides: file.entry.shape.overrides,
                    used_by: file.entry.used_by,
                };
                TemplateDetail {
                    base_blocks: parent_blocks(vault, &path, entry.extends.as_deref()),
                    body: file.body,
                    read_only: false,
                    entry,
                }
            }
        };
        to_value(detail)
    })
    .await
}

/// `POST /api/templates`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    /// The display name.
    name: String,
    /// The document types it is for (default: those of `from`, else invoice).
    #[serde(default)]
    kinds: Option<Vec<String>>,
    /// A template to start from (a `template:` value: one in the books, or
    /// a starter); none: a blank child of the base.
    #[serde(default)]
    from: Option<String>,
}

/// The text of the template `from` names: the file in the books, else a
/// starter's shipped text.
fn source_of(session: &Session, from: &str) -> Result<String, ApiError> {
    let path = template_path(from)?;
    match template_edit::read(session.index(), session.vault(), &path) {
        Ok(_) => session
            .vault()
            .read_to_string(&path)
            .map_err(|e| ApiError::from(TemplateError::from(e))),
        Err(TemplateError::NotFound(_)) => starters::named(&session.settings().templates_dir, from)
            .map(|s| s.source.to_string())
            .ok_or_else(|| ApiError::from(TemplateError::NotFound(path))),
        Err(other) => Err(other.into()),
    }
}

/// `POST /api/templates`.
async fn create(
    State(state): State<AppState>,
    payload: Result<Json<CreateBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::CREATED, move |books| {
        let mut session = books.session();
        let from = request
            .from
            .as_deref()
            .map(|f| source_of(&session, f))
            .transpose()?;
        let (index, vault) = session.parts_mut();
        to_value(template_edit::create(
            index,
            vault,
            &NewTemplate {
                name: &request.name,
                kinds: request.kinds,
                from: from.as_deref(),
            },
            &BASE,
        )?)
    })
    .await
}

/// `POST /api/templates/customize`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomizeBody {
    /// The starter's `template:` value.
    template: String,
}

/// `POST /api/templates/customize`: copy a starter into the books (the
/// base first when it isn't there, [`starters::install`]), so it can be
/// edited. A starter already there is left as it is.
async fn customize(
    State(state): State<AppState>,
    payload: Result<Json<CustomizeBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::OK, move |books| {
        let mut session = books.session();
        let dir = session.settings().templates_dir.clone();
        let starter = starters::named(&dir, &request.template).ok_or_else(|| {
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "not-a-starter",
                format!("{} isn't one of the starters", request.template.trim()),
            )
        })?;
        let installed = starters::install(session.vault(), &dir, starter)
            .map_err(|e| ApiError::from(TemplateError::from(e)))?;
        let written = installed.written_files();
        if !written.is_empty() {
            let (index, vault) = session.parts_mut();
            index.refresh(vault, &written);
        }
        Ok(json!({
            "path": format!("/{}", installed.path),
            "written": installed.written,
            "base": installed.base.map(|b| b.to_string()),
        }))
    })
    .await
}

/// `PUT /api/templates/{path}`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveBody {
    /// The content hash the editor read.
    hash: String,
    /// What to change; absent fields are left.
    fields: TemplateFields,
}

/// `PUT /api/templates/{path}`.
async fn save(
    State(state): State<AppState>,
    Path(key): Path<String>,
    payload: Result<Json<SaveBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::OK, move |books| {
        let path = template_path(&key)?;
        let mut session = books.session();
        let (index, vault) = session.parts_mut();
        to_value(template_edit::save(
            index,
            vault,
            &path,
            &request.hash,
            &request.fields,
            Some(&BASE),
        )?)
    })
    .await
}

/// `DELETE /api/templates/{path}`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteBody {
    /// The content hash the page shows.
    hash: String,
}

/// `DELETE /api/templates/{path}`.
async fn remove(
    State(state): State<AppState>,
    Path(key): Path<String>,
    payload: Result<Json<DeleteBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::OK, move |books| {
        let path = template_path(&key)?;
        let mut session = books.session();
        let (index, vault) = session.parts_mut();
        template_edit::delete(index, vault, &path, &request.hash)?;
        Ok(json!({ "deleted": path.to_string() }))
    })
    .await
}

/// `POST /api/templates/preview`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewBody {
    /// The invoice to show (a number, or a path as `show` takes it).
    number: String,
    /// The template's `template:` value (its file need not exist yet).
    template: String,
    /// The unsaved text after the name line.
    body: String,
}

/// Why a template didn't render, in words, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TemplateProblem {
    /// One or two plain sentences.
    pub message: String,
    /// The line in the body (1-based), when known.
    pub line: Option<u32>,
    /// The column, when Tera gives one.
    pub column: Option<u32>,
}

/// `POST /api/templates/preview`.
async fn preview_template(
    State(state): State<AppState>,
    payload: Result<Json<PreviewBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::OK, move |books| {
        render_body(books, request)
    })
    .await
}

fn render_body(books: &OpenBooks, request: PreviewBody) -> Result<Value, ApiError> {
    let rendered = preview::render(
        books,
        PreviewRequest {
            number: Some(request.number),
            path: None,
            source: None,
            template: Some(request.template),
            template_source: Some(request.body.clone()),
        },
    );
    match rendered {
        Ok(page) => {
            let id = books.drafts().put_page(&page).ok_or_else(|| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "no preview id",
                )
            })?;
            Ok(json!({
                "url": format!("{PREVIEW_PAGE}?draft={id}"),
                "problem": null,
                "render_ms": page.render_ms,
            }))
        }
        Err(PreviewError::Template(message)) => Ok(json!({
            "url": null,
            "problem": plain_problem(&message, &request.body),
        })),
        Err(other) => Err(other.into()),
    }
}

/// Tera's error (MBR's chain: "template `/x.html` failed to render: Failed
/// to parse …\n --> 3:5 …") in plain words, with its line and column in
/// the body when Tera gives them, else the first line holding the name the
/// error is about (an unknown variable or filter, a missing include).
pub(crate) fn plain_problem(message: &str, body: &str) -> TemplateProblem {
    let (line, column) = position(message).unzip();
    let (text, needle) = reason_of(message);
    let line = line.or_else(|| {
        needle.as_deref().and_then(|needle| {
            body.lines()
                .position(|l| l.contains(needle))
                .and_then(|i| u32::try_from(i + 1).ok())
        })
    });
    TemplateProblem {
        message: text,
        line,
        column,
    }
}

/// The ` --> L:C` Tera puts under a syntax error.
fn position(message: &str) -> Option<(u32, u32)> {
    let at = message.find("--> ")?;
    let rest = &message[at + 4..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != ':')
        .unwrap_or(rest.len());
    let (line, column) = rest[..end].split_once(':')?;
    Some((line.parse().ok()?, column.parse().ok()?))
}

/// The text between the first `open` after `after` and the next `close`.
fn between<'a>(text: &'a str, after: &str, open: char, close: char) -> Option<&'a str> {
    let rest = &text[text.find(after)? + after.len()..];
    let rest = &rest[rest.find(open)? + open.len_utf8()..];
    Some(&rest[..rest.find(close)?])
}

/// The error's own sentence (without the template's name or the caret
/// drawing) and the name it is about, to find its line.
fn reason_of(message: &str) -> (String, Option<String>) {
    // "template `/…` failed to render: …": drop the subject.
    let rest = message
        .split_once("failed to render: ")
        .map_or(message, |(_, r)| r);
    if let Some(expected) = rest
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("= "))
    {
        return (format!("The template doesn't read here: {expected}."), None);
    }
    // A chain "A: B: C" ends with its root cause; Tera's outer layers name
    // the template ("Failed to render '/x.html'").
    let cause = rest
        .split(": ")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter(|p| !p.starts_with("Failed to render") && !p.starts_with("Failed to parse"))
        .collect::<Vec<_>>()
        .join(": ");
    let cause = if cause.is_empty() {
        rest.trim()
    } else {
        &cause
    };
    if let Some(name) = between(cause, "Variable", '`', '`') {
        return (
            format!(
                "Nothing called {name} is available to the template: check the spelling, \
                 or give a field some invoices lack a default (| default(value=\"\"))."
            ),
            Some(name.to_string()),
        );
    }
    if let Some(name) = between(cause, "Filter", '\'', '\'') {
        return (
            format!("There's no filter called {name}."),
            Some(name.to_string()),
        );
    }
    if let Some(name) = between(cause, "Function", '\'', '\'') {
        return (
            format!("There's no function called {name}."),
            Some(name.to_string()),
        );
    }
    if let Some(parent) = between(cause, "is inheriting from", '\'', '\'') {
        return (
            format!("It extends {parent}, which isn't in the templates folder."),
            Some(parent.to_string()),
        );
    }
    if let Some(name) = between(cause, "Template", '[', ']') {
        return (
            format!("There's no file {name} beside the template to include."),
            Some(name.to_string()),
        );
    }
    // Anything else: Tera's sentence, without the template's own name.
    let mut text = cause
        .replace("in context while rendering", "")
        .split(" '/")
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches('.')
        .to_string();
    if let Some(first) = text.chars().next() {
        text.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
    }
    (format!("{text}."), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tera_errors_in_plain_words_with_their_line() {
        assert_eq!(position("x\n --> 3:15\n  |"), Some((3, 15)));
        assert_eq!(position("no arrow"), None);
        let body = "{% extends \"_invoice_base.v1.html\" %}\n{% block footer %}\n{{ nope.nope }}\n{% endblock footer %}\n";
        let problem = plain_problem(
            "template `/invoices/tpl/x.html` failed to render: Failed to render '/invoices/tpl/x.html': \
             Variable `nope.nope` not found in context while rendering '/invoices/tpl/x.html'",
            body,
        );
        assert_eq!(problem.line, Some(3));
        assert!(
            problem.message.starts_with("Nothing called nope.nope"),
            "{problem:?}"
        );
        assert!(!problem.message.contains("/invoices"), "{problem:?}");
        let problem = plain_problem(
            "template `/invoices/tpl/x.html` failed to render: Failed to parse '/invoices/tpl/x.html'\n \
             --> 2:26\n  |\n2 | {{ 1 + }}\n  |  ^---\n  |\n  = expected an integer or an expression",
            body,
        );
        assert_eq!((problem.line, problem.column), (Some(2), Some(26)));
        assert_eq!(
            problem.message,
            "The template doesn't read here: expected an integer or an expression."
        );
        let problem = plain_problem(
            "x failed to render: Template '[missing.html]' not found",
            "a\n{% include \"missing.html\" %}",
        );
        assert_eq!(
            problem.message,
            "There's no file missing.html beside the template to include."
        );
        assert_eq!(problem.line, Some(2));
    }
}
