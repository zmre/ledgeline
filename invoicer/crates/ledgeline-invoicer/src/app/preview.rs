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
//! used.
//!
//! **The preview page** (`GET /.ledgeline-invoicer/preview?number=&template=`,
//! [`page`]) is the same render as a page of its own, for the SPA's frame.
//! A preview can't be shown through `srcdoc`: an `about:srcdoc` document
//! inherits its parent's CSP, and the SPA's (the shell's) has
//! `frame-ancestors 'none'`, which WebKit applies to the inherited policy, so
//! the window showed a blank frame (2026-11-23). As a page of its own it
//! gets MBR's policy ([`super::mbr::MBR_CSP`]: `frame-ancestors 'self'`,
//! remote images), passes the same gate as MBR's pages (the session cookie),
//! and its relative links resolve from the note's own page (`<base>`).

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

/// The request body.
#[derive(Debug, Default, Deserialize)]
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
    let override_template = request
        .template
        .as_deref()
        .map(|t| template_text(&**vault, &templates_dir, t))
        .transpose()?;

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
                        starter.source,
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

/// The preview page's query: the saved invoice, through `template`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageParams {
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
    let request = PreviewRequest {
        number: params.number,
        path: params.path,
        source: None,
        template: params.template,
    };
    let rendered = tokio::task::spawn_blocking(move || render(&books, request)).await;
    let preview = match rendered {
        Ok(Ok(preview)) => preview,
        Ok(Err(error)) => {
            let (status, message) = match error {
                PreviewError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
                PreviewError::NotFound(m) => (StatusCode::NOT_FOUND, m),
                PreviewError::Template(m) => (StatusCode::UNPROCESSABLE_ENTITY, m),
                PreviewError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
            };
            return problem_page(status, &message);
        }
        Err(error) => return problem_page(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
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
                HeaderValue::from_static(mbr::MBR_CSP),
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
                HeaderValue::from_static(mbr::MBR_CSP),
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

/// The template a request names, and its text: the file in the books, else
/// a starter not copied in yet.
fn template_text(
    vault: &dyn Vault,
    templates_dir: &str,
    template: &str,
) -> Result<(VaultPath, String), PreviewError> {
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
    match vault.read_pinned(&path, Some(MAX_TEMPLATE_BYTES)) {
        Ok((bytes, _)) => String::from_utf8(bytes)
            .map(|text| (path, text))
            .map_err(|_| bad("not UTF-8")),
        Err(e) if e.is_not_found() => starters::named(templates_dir, template)
            .map(|s| (path, s.source.to_string()))
            .ok_or_else(|| PreviewError::NotFound(format!("template {template}: no such file"))),
        Err(e) => Err(PreviewError::Internal(e.to_string())),
    }
}

/// `GET /api/templates`: every template in the books, then each starter not
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
}

/// The templates list.
pub(crate) fn templates(books: &OpenBooks) -> Vec<TemplateEntry> {
    templates_in(&books.session())
}

/// The templates list, from a session already locked.
pub(crate) fn templates_in(session: &invoicer_core::session::Session) -> Vec<TemplateEntry> {
    let dir = session.settings().templates_dir.clone();
    let found = invoicer_core::templates::discover(session.vault(), session.index().site());
    let mut out: Vec<TemplateEntry> = found
        .iter()
        .map(|t| {
            let path = t.template_path();
            TemplateEntry {
                starter: starters::named(&dir, &path).is_some(),
                path,
                name: t.name.clone(),
                kinds: t.kinds.clone(),
                installed: true,
            }
        })
        .collect();
    for starter in &starters::STARTERS {
        let Some(path) = starters::template_value(&dir, starter) else {
            continue;
        };
        if out.iter().any(|t| t.path == path) {
            continue;
        }
        let kinds = invoicer_core::templates::header(starter.source)
            .map(|h| h.kinds)
            .unwrap_or_default();
        out.push(TemplateEntry {
            path,
            name: starter.name.to_string(),
            kinds,
            starter: true,
            installed: false,
        });
    }
    out
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
