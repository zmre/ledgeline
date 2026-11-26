//! `POST /api/preview` (invoicer PLAN 3b, ARCHITECTURE §6.1–6.2): an
//! invoice rendered to the whole page its template gives, from text that
//! need not be saved, through MBR's standalone renderer (MBR-CHANGES M3,
//! and M10 for a template other than the invoice's own). It reads files and
//! writes none.
//!
//! The request names the note by `number` or `path` (vault-relative,
//! `.md`): its folder is where links resolve and where a relative
//! `template:` is looked up. `source` is the unsaved text; without it the
//! saved file is rendered. `template` renders through another template (a
//! `/`-rooted `.html` path) instead of the one the note names: a file in
//! the books, or a starter that isn't copied in yet, whose shipped text is
//! used (folded into the shipped base when the books don't have the base
//! either: [`starters::preview_source`]).
//!
//! **The preview page** (`GET /.ledgeline-invoicer/preview?number=&template=`,
//! [`page`]) is the same render as a page of its own, for the SPA's frame.
//! A preview can't be shown through `srcdoc`: an `about:srcdoc` document
//! inherits its parent's CSP, and the SPA's (the shell's) has
//! `frame-ancestors 'none'`, which WebKit applies to the inherited policy, so
//! the window showed a blank frame (2026-11-23). As a page of its own it
//! gets the policy of MBR's pages ([`super::mbr::SANDBOXED_CSP`]: no script
//! runs, nothing loads from another host, `frame-ancestors 'self'`), passes
//! the same gate as MBR's pages (the session cookie), and its relative links
//! resolve from the note's own page (`<base>`).
//!
//! **The preview flag.** Every preview renders the note with
//! `invoicer_preview: true` added to its frontmatter ([`PREVIEW_FLAG`],
//! [`flagged`]; nothing is written). It is how a template tells the app's
//! preview from real output: the "with logo" starters draw a "Your logo"
//! box where an invoice has no logo only `{% if fm.invoicer_preview %}`.
//! The PDF export prints MBR's page for the saved file, and MBR's browser,
//! its static build and QuickLook render the saved file too, so none of
//! them ever has the flag: the box's markup is never in what a customer
//! gets, CSS or no CSS. A frontmatter that can't be edited safely is
//! rendered as it is (no box, never a failed preview).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use axum::Extension;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};

use ledgeline_core::vault::{Vault, VaultPath};
use mbr_core::errors::MbrError;
use mbr_core::page_template::{MAX_TEMPLATE_BYTES, PageTemplateError};
use serde::{Deserialize, Serialize};

use super::AppState;
use super::books::OpenBooks;
use super::mbr::{self, MbrGate};
use crate::starters;

/// Where the preview page is served: a hidden folder, which MBR never serves.
pub(crate) const PREVIEW_PAGE: &str = "/.ledgeline-invoicer/preview";

/// The largest unsaved source accepted.
pub(crate) const MAX_SOURCE_BYTES: usize = 1024 * 1024;

/// The frontmatter key only the app's preview sets (see the module docs).
pub(crate) const PREVIEW_FLAG: &str = "invoicer_preview";

/// `source` with [`PREVIEW_FLAG`] set to `true` in its frontmatter, for the
/// render alone; as it is when its frontmatter can't be edited safely.
fn flagged(source: String) -> String {
    invoicer_core::frontmatter::set(&source, PREVIEW_FLAG, serde_json::Value::Bool(true))
        .unwrap_or(source)
}

/// The request body.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewRequest {
    /// An invoice number (or a path, as `show` takes it).
    pub number: Option<String>,
    /// The note's path in the books folder.
    pub path: Option<String>,
    /// The unsaved text. Default: the saved file.
    pub source: Option<String>,
    /// Render through this template instead of the note's own.
    pub template: Option<String>,
    /// The unsaved text of `template` (the template editor, PLAN 4d): used
    /// instead of its file, which need not exist yet.
    #[serde(default)]
    pub template_source: Option<String>,
}

/// The rendered page.
#[derive(Debug, Serialize)]
pub(crate) struct Preview {
    /// The whole page.
    pub html: String,
    /// The note it was rendered as.
    pub path: String,
    /// The template given in the request, as `template:` writes it; `None`
    /// when the note's own was used.
    pub template: Option<String>,
    /// How long rendering took, in milliseconds (also sent as
    /// `Server-Timing`).
    #[serde(skip)]
    pub render_ms: f64,
}

/// Why there is no preview.
#[derive(Debug)]
pub(crate) enum PreviewError {
    /// The request is malformed: no note, a bad path, too large.
    BadRequest(String),
    /// The note or the template isn't there.
    NotFound(String),
    /// The page template couldn't be used: a fixable problem in the files.
    Template(String),
    /// Anything else.
    Internal(String),
}

impl PreviewError {
    /// The HTTP status it answers with, and its sentence.
    pub(crate) fn status_and_message(self) -> (StatusCode, String) {
        match self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m),
            Self::Template(m) => (StatusCode::UNPROCESSABLE_ENTITY, m),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        }
    }
}

/// Render `request` over `books`. Runs on the blocking pool.
///
/// # Errors
/// [`PreviewError`].
pub(crate) fn render(books: &OpenBooks, request: PreviewRequest) -> Result<Preview, PreviewError> {
    let (note, templates_dir) = {
        let session = books.session();
        let note = match (&request.number, &request.path) {
            (Some(number), _) => session
                .find(number)
                .map(|doc| doc.path.clone())
                .map_err(|e| PreviewError::NotFound(e.to_string()))?,
            (None, Some(path)) => note_path(path)?,
            (None, None) => {
                return Err(PreviewError::BadRequest(
                    "name the invoice: `number` or `path`".to_string(),
                ));
            }
        };
        (note, session.settings().templates_dir.clone())
    };
    let vault = books.vault();
    let source = match request.source {
        Some(source) if source.len() > MAX_SOURCE_BYTES => {
            return Err(PreviewError::BadRequest(format!(
                "source: more than {MAX_SOURCE_BYTES} bytes"
            )));
        }
        Some(source) => source,
        None => vault.read_to_string(&note).map_err(|e| {
            if e.is_not_found() {
                PreviewError::NotFound(format!("{note}: no such file"))
            } else {
                PreviewError::Internal(e.to_string())
            }
        })?,
    };
    let override_template = match (request.template.as_deref(), request.template_source) {
        (Some(template), Some(text)) => Some(unsaved_template(&**vault, template, text)?),
        (Some(template), None) => Some(template_text(&**vault, &templates_dir, template)?),
        (None, Some(_)) => {
            return Err(PreviewError::BadRequest(
                "template_source: name its `template` too".to_string(),
            ));
        }
        (None, None) => None,
    };

    let source = flagged(source);
    let renderer = books.renderer().map_err(PreviewError::Internal)?;
    let started = Instant::now();
    let note_rel = Path::new(note.as_str());
    let rendered = match &override_template {
        Some((path, text)) => {
            renderer.render_source_with_template(note_rel, &source, Path::new(path.as_str()), text)
        }
        None => match renderer.render_source(note_rel, &source) {
            // The note names a starter that isn't copied in yet: preview it
            // with the shipped text (it is copied when an invoice is
            // written with it).
            Err(MbrError::PageTemplate(PageTemplateError::NotFound { template })) => {
                match starters::named(&templates_dir, &template)
                    .and_then(|s| starters::path_in(&templates_dir, s).ok().map(|p| (s, p)))
                {
                    Some((starter, path)) => renderer.render_source_with_template(
                        note_rel,
                        &source,
                        Path::new(path.as_str()),
                        &starters::preview_source(&**vault, &templates_dir, starter),
                    ),
                    None => Err(MbrError::PageTemplate(PageTemplateError::NotFound {
                        template,
                    })),
                }
            }
            other => other,
        },
    };
    let render_ms = started.elapsed().as_secs_f64() * 1000.0;
    let html = rendered.map_err(|e| match e {
        MbrError::PageTemplate(e) => PreviewError::Template(e.to_string()),
        other => PreviewError::Internal(other.to_string()),
    })?;
    Ok(Preview {
        html,
        path: note.to_string(),
        template: override_template.map(|(p, _)| format!("/{p}")),
        render_ms,
    })
}

/// How many unsaved previews are kept for the frame (the editor posts one
/// per pause in typing, and frames the newest).
const MAX_DRAFTS: usize = 16;

/// How long an unsaved preview is kept.
const DRAFT_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// Unsaved previews (invoicer PLAN 4a): the editor posts its unsaved text
/// (`POST /api/preview/draft`, token-gated) and frames
/// `/.ledgeline-invoicer/preview?draft=<id>`, behind MBR's gate, since a
/// preview can't be a `srcdoc` (see the module docs). Each id is random, one
/// per post; the oldest are dropped past [`MAX_DRAFTS`] or [`DRAFT_TTL`].
/// Held by the open books, so File → Open forgets them.
///
/// The template editor (4d) renders when it posts, to answer with Tera's
/// error at once, and keeps the page it rendered ([`Kept::Page`]), so the
/// frame's load doesn't render it again.
#[derive(Debug, Default)]
pub(crate) struct DraftPreviews {
    drafts: std::sync::Mutex<std::collections::HashMap<String, (Kept, Instant)>>,
}

/// What an unsaved preview's id names.
#[derive(Debug, Clone)]
pub(crate) enum Kept {
    /// Rendered when the frame loads it.
    Request(PreviewRequest),
    /// Rendered already: the page (without its `<base>`) and the note's path.
    Page {
        /// The whole page.
        html: Arc<str>,
        /// The note it was rendered as.
        path: String,
    },
}

impl DraftPreviews {
    /// Keep `request` (a path and its unsaved source); its id.
    pub(crate) fn put(&self, request: PreviewRequest) -> Option<String> {
        self.keep(Kept::Request(request))
    }

    /// Keep a page rendered already (the template editor's); its id.
    pub(crate) fn put_page(&self, preview: &Preview) -> Option<String> {
        self.keep(Kept::Page {
            html: Arc::from(preview.html.as_str()),
            path: preview.path.clone(),
        })
    }

    fn keep(&self, kept: Kept) -> Option<String> {
        let id = ledgeline_shell::security::AccessToken::generate()
            .ok()?
            .as_str()
            .to_string();
        let mut drafts = self
            .drafts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        drafts.retain(|_, (_, at)| now.duration_since(*at) < DRAFT_TTL);
        while drafts.len() >= MAX_DRAFTS {
            let Some(oldest) = drafts
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            drafts.remove(&oldest);
        }
        drafts.insert(id.clone(), (kept, now));
        Some(id)
    }

    /// The preview `id` names (kept: the frame may load it again).
    fn get(&self, id: &str) -> Option<Kept> {
        let drafts = self
            .drafts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drafts
            .get(id)
            .filter(|(_, at)| at.elapsed() < DRAFT_TTL)
            .map(|(kept, _)| kept.clone())
    }
}

/// The preview page's query: the saved invoice, through `template`; or an
/// unsaved preview (`draft`).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageParams {
    /// An unsaved preview's id ([`DraftPreviews`]).
    pub draft: Option<String>,
    /// An invoice number (or a path, as `show` takes it).
    pub number: Option<String>,
    /// The note's path in the books folder.
    pub path: Option<String>,
    /// Render through this template instead of the note's own.
    pub template: Option<String>,
}

/// `GET /.ledgeline-invoicer/preview`: the saved invoice rendered through
/// another template, as a page for the SPA's frame (see the module docs).
/// Behind MBR's gate; writes nothing. A failure is a short page saying why,
/// since a frame shows it as it is.
pub(crate) async fn page(
    State(state): State<AppState>,
    Extension(gate): Extension<Arc<MbrGate>>,
    headers: HeaderMap,
    params: Result<Query<PageParams>, axum::extract::rejection::QueryRejection>,
) -> Response {
    if !gate.allows(&headers) {
        return mbr::refused();
    }
    let Some(books) = state.books() else {
        return mbr::no_books();
    };
    let Query(params) = match params {
        Ok(params) => params,
        Err(error) => return problem_page(StatusCode::BAD_REQUEST, &error.body_text()),
    };
    let kept = match params.draft {
        Some(id) => match books.drafts().get(&id) {
            Some(kept) => kept,
            None => {
                return problem_page(
                    StatusCode::NOT_FOUND,
                    "this preview has expired; edit the invoice to preview it again",
                );
            }
        },
        None => Kept::Request(PreviewRequest {
            number: params.number,
            path: params.path,
            source: None,
            template: params.template,
            template_source: None,
        }),
    };
    let preview = match kept {
        Kept::Page { html, path } => Preview {
            html: html.to_string(),
            path,
            template: None,
            render_ms: 0.0,
        },
        Kept::Request(request) => {
            match tokio::task::spawn_blocking(move || render(&books, request)).await {
                Ok(Ok(preview)) => preview,
                Ok(Err(error)) => {
                    let (status, message) = error.status_and_message();
                    return problem_page(status, &message);
                }
                Err(error) => {
                    return problem_page(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
                }
            }
        }
    };
    let base = VaultPath::new(&preview.path)
        .map(|note| crate::pdf::page_url(&note))
        .ok();
    let html = match base {
        Some(base) => with_base(&preview.html, &base),
        None => preview.html,
    };
    let mut response = (
        [
            (header::CACHE_CONTROL, ledgeline_shell::security::no_store()),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(mbr::SANDBOXED_CSP),
            ),
        ],
        Html(html),
    )
        .into_response();
    if let Ok(timing) = HeaderValue::from_str(&format!("render;dur={:.2}", preview.render_ms)) {
        response.headers_mut().insert("server-timing", timing);
    }
    response
}

/// `html` with `<base href="{base}">` first in its `<head>`, so its
/// relative links resolve as on the note's own page. Unchanged when it has
/// no `<head>` or already sets a base.
fn with_base(html: &str, base: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if lower.contains("<base") {
        return html.to_string();
    }
    let Some(open) = lower.find("<head") else {
        return html.to_string();
    };
    // `<head>` or `<head …>`, not `<header>`.
    let after = &lower[open + 5..];
    if !after.starts_with(['>', ' ', '\t', '\n', '\r']) {
        return html.to_string();
    }
    let Some(close) = after.find('>') else {
        return html.to_string();
    };
    let at = open + 5 + close + 1;
    let href = base.replace('&', "&amp;").replace('"', "&quot;");
    format!("{}<base href=\"{href}\">{}", &html[..at], &html[at..])
}

/// A small page saying why there is no preview.
fn problem_page(status: StatusCode, message: &str) -> Response {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    (
        status,
        [
            (header::CACHE_CONTROL, ledgeline_shell::security::no_store()),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(mbr::SANDBOXED_CSP),
            ),
        ],
        Html(format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>No preview</title></head>\
             <body style=\"font-family: system-ui, sans-serif; padding: 1.5rem\">\
             <p><strong>Couldn't preview it.</strong></p><p>{escaped}</p></body></html>"
        )),
    )
        .into_response()
}

/// A note's path from the request: inside the books folder, not hidden,
/// markdown.
fn note_path(path: &str) -> Result<VaultPath, PreviewError> {
    let bad = |why: &str| PreviewError::BadRequest(format!("path {path:?}: {why}"));
    let note =
        VaultPath::new(path.trim().trim_start_matches('/')).map_err(|e| bad(&e.to_string()))?;
    if note.is_root() {
        return Err(bad("not a file"));
    }
    if note.segments().any(|s| s.starts_with('.')) {
        return Err(bad("in a hidden folder"));
    }
    if !note
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
    {
        return Err(bad("not a markdown note"));
    }
    Ok(note)
}

/// A template's path from a request: inside the books folder, not hidden,
/// `.html`.
pub(crate) fn template_path(template: &str) -> Result<VaultPath, PreviewError> {
    let bad = |why: &str| PreviewError::BadRequest(format!("template {template:?}: {why}"));
    let path =
        VaultPath::new(template.trim().trim_start_matches('/')).map_err(|e| bad(&e.to_string()))?;
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("html"))
    {
        return Err(bad("not an .html file"));
    }
    if path.segments().any(|s| s.starts_with('.')) {
        return Err(bad("in a hidden folder"));
    }
    Ok(path)
}

/// Unsaved template text at `template`'s path (PLAN 4d): as it is when the
/// base it extends is beside it on disk (MBR reads it from there), else
/// folded into the shipped base ([`starters::fold_into_base`]) when it
/// extends that; a text that can't be folded (it calls `super()`) says the
/// base is missing.
fn unsaved_template(
    vault: &dyn Vault,
    template: &str,
    text: String,
) -> Result<(VaultPath, String), PreviewError> {
    let path = template_path(template)?;
    if text.len() > MAX_SOURCE_BYTES {
        return Err(PreviewError::BadRequest(format!(
            "template_source: more than {MAX_SOURCE_BYTES} bytes"
        )));
    }
    let Some(parent) = crate::tera_scan::extends(&text).map(str::to_string) else {
        return Ok((path, text));
    };
    let beside = path
        .parent()
        .unwrap_or_else(VaultPath::root)
        .child(&parent)
        .ok()
        .and_then(|p| vault.stat(&p).ok().flatten())
        .is_some();
    if beside || parent != starters::BASE.file {
        return Ok((path, text));
    }
    match starters::fold_into_base(&text, starters::BASE.source) {
        Some(folded) => Ok((path, folded)),
        None => Err(PreviewError::Template(format!(
            "this template extends {parent}, which isn't in the templates folder yet; \
             saving the template copies it in"
        ))),
    }
}

/// The template a request names, and its text: the file in the books, else
/// a starter not copied in yet.
fn template_text(
    vault: &dyn Vault,
    templates_dir: &str,
    template: &str,
) -> Result<(VaultPath, String), PreviewError> {
    let path = template_path(template)?;
    let bad = |why: &str| PreviewError::BadRequest(format!("template {template:?}: {why}"));
    match vault.read_pinned(&path, Some(MAX_TEMPLATE_BYTES)) {
        Ok((bytes, _)) => String::from_utf8(bytes)
            .map(|text| (path, text))
            .map_err(|_| bad("not UTF-8")),
        Err(e) if e.is_not_found() => starters::named(templates_dir, template)
            .map(|s| (path, starters::preview_source(vault, templates_dir, s)))
            .ok_or_else(|| PreviewError::NotFound(format!("template {template}: no such file"))),
        Err(e) => Err(PreviewError::Internal(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_goes_first_in_the_head() {
        assert_eq!(
            with_base(
                "<!doctype html><html><head><meta charset=\"utf-8\"></head>",
                "/a%20b/"
            ),
            "<!doctype html><html><head><base href=\"/a%20b/\"><meta charset=\"utf-8\"></head>"
        );
        assert_eq!(
            with_base("<HEAD lang=x><title>t</title>", "/x/"),
            "<HEAD lang=x><base href=\"/x/\"><title>t</title>"
        );
        for kept in [
            "<html><header>no head</header></html>",
            "<head><base href=\"/y/\"></head>",
            "no html at all",
        ] {
            assert_eq!(with_base(kept, "/x/"), kept);
        }
    }

    #[test]
    fn a_preview_flags_the_note_it_renders_and_nothing_else() {
        let note = "---\ntitle: Invoice 101\ninvoice_num: 101\n---\n\nBody text.\n";
        assert_eq!(
            flagged(note.to_string()),
            "---\ntitle: Invoice 101\ninvoice_num: 101\ninvoicer_preview: true\n---\n\nBody text.\n"
        );
        // A note's own value is the preview's while previewing.
        assert_eq!(
            flagged("---\ninvoicer_preview: false\nx: 1\n---\nB\n".to_string()),
            "---\ninvoicer_preview: true\nx: 1\n---\nB\n"
        );
        // A frontmatter that can't be edited safely renders as it is.
        let odd = "---\n[unclosed\n---\nB\n";
        assert_eq!(flagged(odd.to_string()), odd);
    }

    #[test]
    fn a_note_path_stays_inside_the_books_and_is_markdown() {
        assert_eq!(
            note_path("/invoices/x.md").unwrap().as_str(),
            "invoices/x.md"
        );
        for bad in ["../x.md", "invoices/x.html", ".hidden/x.md", "", "/"] {
            assert!(
                matches!(note_path(bad), Err(PreviewError::BadRequest(_))),
                "{bad}"
            );
        }
    }
}
