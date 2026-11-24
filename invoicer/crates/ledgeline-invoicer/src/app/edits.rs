//! The invoice page's two writes (invoicer PLAN, Phase 3 feedback F9–F10),
//! each the CLI's own library call:
//!
//! | Route                                | Does                                                          |
//! |--------------------------------------|---------------------------------------------------------------|
//! | `POST /api/invoices/{num}/template`  | `{template}`: write the invoice's `template:` (`set`), copying a starter in on first use. |
//! | `POST /api/invoices/{num}/recalc`    | `{rebook?, today?}`: rewrite its amounts (`recalc`); a sent invoice is re-booked too (`rebook`), only with `rebook: true`. |
//!
//! **Template.** Any template `GET /api/templates` lists (one in the books,
//! or a starter, copied into `templates-dir` once the invoice names it; never
//! over a file already there) and that is for the document's type. `set`'s
//! rules apply, so a sent or void invoice may change its template too (it is
//! styling; the books never see it). The `template:` line is part of what
//! `sent_hash` covers, so a sent invoice whose template changed warns at its
//! next export that the PDF differs from what the customer has (P4, P5).
//!
//! **Recalculate.** A draft's amounts are only arithmetic: rewritten. A sent
//! invoice's amount is booked, so recalculating it means re-booking it, by
//! the CLI's rules: refused for a date on or before `closed-through` (I8)
//! before anything is written, and asked for explicitly (`rebook: true`, the
//! SPA's confirmation); without it the answer is `409` saying so. A void
//! invoice is history: `409`.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use invoicer_core::books::{Rebook, retry_once};
use invoicer_core::doc::Status;
use invoicer_core::edit::{self, WriteError};
use invoicer_core::frontmatter::FieldEdit;
use ledgeline_core::vault::VaultError;
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use super::api::{ApiError, ApiResult, blocking, no_books, to_value, today};
use super::books::{reconcile_in, reload_after_write};
use super::pdf::books_error;
use super::preview;
use crate::starters;

/// A write the core refused, as an API error: `409` when the file changed
/// under it (read it again), `400` for a refused key, else `422` (a fixable
/// problem in the file).
pub(crate) fn write_error(error: &WriteError) -> ApiError {
    let changed = matches!(
        error,
        WriteError::Vault(VaultError::Changed { .. })
            | WriteError::Draft(invoicer_core::draft::DraftError::Vault(
                VaultError::Changed { .. }
            ))
    );
    let status = if changed {
        StatusCode::CONFLICT
    } else if matches!(error, WriteError::Protected { .. }) {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    ApiError(status, error.to_string())
}

/// `POST /api/invoices/{num}/template`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TemplateBody {
    /// The `template:` value: a path `GET /api/templates` lists.
    pub template: String,
}

/// `POST /api/invoices/{num}/template`: `{template, changed, copied,
/// status}`. `copied` is the starter's file when it was copied in now.
pub(crate) async fn use_template(
    State(state): State<AppState>,
    Path(number): Path<String>,
    body: Result<Json<TemplateBody>, JsonRejection>,
) -> ApiResult {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let books = state.books().ok_or_else(no_books)?;
    blocking(move || {
        let mut session = books.session();
        let doc = session
            .find(&number)
            .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
        let (path, doc_type, status) = (doc.path.clone(), doc.header.doc_type, doc.header.status);
        let wanted = format!("/{}", body.template.trim().trim_start_matches('/'));
        let entry = preview::templates_in(&session)
            .into_iter()
            .find(|t| t.path == wanted)
            .ok_or_else(|| {
                ApiError(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!(
                        "{wanted} isn't a template: name one in the books (an .html file \
                         mentioning invoice_num) or a starter"
                    ),
                )
            })?;
        if let Some(kind) = doc_type
            && !entry.kinds.iter().any(|k| k == kind.as_str())
        {
            return Err(ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "the {} template isn't for {} documents",
                    entry.name,
                    kind.as_str()
                ),
            ));
        }
        let (index, vault) = session.parts_mut();
        let outcome = edit::set(
            index,
            vault,
            &path,
            &[FieldEdit::Set(
                "template".to_string(),
                Value::String(wanted.clone()),
            )],
        )
        .map_err(|e| write_error(&e))?;
        // A starter is copied in once the invoice names it (B1), and indexed
        // so the invoice's `unknown-template` warning goes at once.
        let dir = session.settings().templates_dir.clone();
        let copied = match starters::install_named(session.vault(), &dir, &wanted) {
            Ok(Some(done)) if done.written => {
                let (index, vault) = session.parts_mut();
                index.refresh(vault, &done.written_files());
                Some(done.path.to_string())
            }
            Ok(_) => None,
            Err(error) => {
                return Err(ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "the invoice names {wanted} now, but the starter wasn't copied in: \
                         {error}"
                    ),
                ));
            }
        };
        Ok(json!({
            "template": wanted,
            "changed": outcome.written,
            "copied": copied,
            "status": status.map(Status::as_str),
        }))
    })
    .await
}

/// `POST /api/invoices/{num}/recalc`'s body.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecalcBody {
    /// A sent invoice: recalculate and re-book it (the confirmation).
    #[serde(default)]
    pub rebook: bool,
    /// Today (YYYY-MM-DD): a re-booking's restatement date.
    pub today: Option<String>,
}

/// `POST /api/invoices/{num}/recalc`: `{recalculated, rebooked}` (the
/// CLI's `recalc` and, for a sent invoice, `rebook` shapes; `rebooked` is
/// `null` for a draft).
pub(crate) async fn recalc(
    State(state): State<AppState>,
    Path(number): Path<String>,
    body: Result<Json<RecalcBody>, JsonRejection>,
) -> ApiResult {
    let body = match body {
        Ok(Json(body)) => body,
        Err(JsonRejection::MissingJsonContentType(_)) => RecalcBody::default(),
        Err(e) => return Err(ApiError::bad_request(e.body_text())),
    };
    let today = today(body.today.as_deref())?;
    let books = state.books().ok_or_else(no_books)?;
    blocking(move || {
        let mut session = books.session();
        let doc = session
            .find(&number)
            .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
        let (path, status) = (doc.path.clone(), doc.header.status);
        let doc_number = doc.number().map(str::to_string);
        match status {
            Some(Status::Void) => Err(ApiError(
                StatusCode::CONFLICT,
                format!("invoice {number} is void: its amounts stay as they were booked"),
            )),
            Some(Status::Sent) => {
                let doc_number = doc_number.ok_or_else(|| {
                    ApiError(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "it has no `invoice_num`".to_string(),
                    )
                })?;
                if !body.rebook {
                    return Err(ApiError(
                        StatusCode::CONFLICT,
                        format!(
                            "invoice {doc_number} is sent, so its amount is booked: \
                             recalculating it re-books it. Confirm to recalculate and re-book."
                        ),
                    ));
                }
                let invoice_books = session.invoice_books();
                // Refuse before the file changes, not after (closed period).
                invoice_books
                    .check_rebook(session.index(), &doc_number)
                    .map_err(|e| books_error(&e))?;
                let (index, vault) = session.parts_mut();
                let recalculated =
                    edit::recalc_one(index, vault, &path, false).map_err(|e| write_error(&e))?;
                let request = Rebook {
                    number: doc_number.clone(),
                    today,
                    expected: None,
                };
                let (result, _) = retry_once(|| invoice_books.rebook(index, &request));
                reload_after_write(&mut session);
                let rebooked = result.map_err(|e| {
                    let failed = books_error(&e);
                    ApiError(
                        failed.0,
                        format!(
                            "the amounts were rewritten, but re-booking invoice {doc_number} \
                             failed: {}; fix it, then run `ledgeline-invoicer rebook \
                             {doc_number}`",
                            failed.1
                        ),
                    )
                })?;
                // A new total can move the payment state: refresh `paid:`.
                if rebooked.changed {
                    reconcile_in(&mut session, Some(vec![doc_number]));
                }
                Ok(json!({
                    "recalculated": to_value(&recalculated)?,
                    "rebooked": to_value(&rebooked)?,
                }))
            }
            _ => {
                let (index, vault) = session.parts_mut();
                let recalculated =
                    edit::recalc_one(index, vault, &path, false).map_err(|e| write_error(&e))?;
                Ok(json!({
                    "recalculated": to_value(&recalculated)?,
                    "rebooked": Value::Null,
                }))
            }
        }
    })
    .await
}
