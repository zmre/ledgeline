//! The invoice page's writes (invoicer PLAN, Phase 3 feedback F9–F10),
//! each the CLI's own library call:
//!
//! | Route                                | Does                                                          |
//! |--------------------------------------|---------------------------------------------------------------|
//! | `POST /api/invoices/{num}/template`  | `{template}`: write the invoice's `template:` (`set`), copying a starter in on first use. |
//! | `POST /api/invoices/{num}/recalc`    | `{rebook?, today?}`: rewrite its amounts (`recalc`); a sent invoice is re-booked too (`rebook`), only with `rebook: true`. |
//! | `POST /api/invoices/{num}/company`   | `{today?}`: Update from company (`update-company`): its `invoicer_*` keys from the company note; a sent invoice is re-booked too. |
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
//! the CLI's rules, and asked for explicitly (`rebook: true`, the SPA's
//! confirmation); without it the answer is `409` saying so. Before anything
//! is written, the text it would write is re-booked in memory
//! ([`invoicer_core::books::InvoiceBooks::check_rebook`]): a refusal (a date
//! on or before `closed-through`, I8; a dropped schedule; a row with no
//! account) is answered then, with the books' code. A void invoice is
//! history: `409`.
//!
//! **Written, not re-booked** (TW-4). What the check can't foresee (the
//! journal changing twice under the write, a file that can't be written)
//! can still stop the re-book after the markdown is written. That answers
//! `422` with code `saved-not-rebooked` (never `changed`), and the body
//! says what happened: `written: true`, the file's new `revision` and
//! `path`, `cause` (the books' code, [`BooksError::code`]) and an `error`
//! saying how to finish (Re-book, or `ledgeline-invoicer rebook N`). The
//! editor's save answers the same ([`super::editor`]).
//!
//! **Update from company.** A draft follows the company note wherever it is
//! shown ([`invoicer_core::company`]); a sent invoice keeps what was sent
//! until this is asked for. It is a sent-invoice edit like any other: checked
//! as re-booking would book it, written (only the company keys), re-booked.
//! `sent_hash` stays as it was, so the next export says the PDF differs from
//! what the customer has.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use invoicer_core::books::Rebooked;
use invoicer_core::doc::Status;
use invoicer_core::edit::{self, WriteError};
use invoicer_core::frontmatter::FieldEdit;
use invoicer_core::session::Session;
use ledgeline_core::vault::{VaultError, VaultPath};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use super::api::{ApiError, ApiResult, blocking, to_value, today};
use super::books::log_mirror;
use super::pdf::books_error;
use crate::{flows, starters};

/// A write the core refused, as an API error: `409` `changed` when the
/// file changed under it (read it again), `400` `bad-request` for a
/// refused key, else `422` `invalid` (a fixable problem in the file).
pub(crate) fn write_error(error: &WriteError) -> ApiError {
    let changed = matches!(
        error,
        WriteError::Vault(VaultError::Changed { .. })
            | WriteError::Draft(invoicer_core::draft::DraftError::Vault(
                VaultError::Changed { .. }
            ))
    );
    if changed {
        ApiError::new(StatusCode::CONFLICT, "changed", error.to_string())
    } else if matches!(error, WriteError::Protected { .. }) {
        ApiError::bad_request(error.to_string())
    } else {
        ApiError::status(StatusCode::UNPROCESSABLE_ENTITY, error.to_string())
    }
}

/// Re-book sent invoice `number` after this request rewrote its file at
/// `path` (`written`; else after it found nothing to rewrite)
/// ([`flows::rebook`], its mirror logged). A failure after a write leaves
/// the file written and the books as they were: answered as `422`
/// `saved-not-rebooked` with the books' `cause`, `written: true` and the
/// file's new `revision` and `path` (TW-4), so the SPA can take its own
/// save as saved and offer Re-book, rather than read the write as someone
/// else's change.
pub(crate) fn rebook_written(
    session: &mut Session,
    number: &str,
    today: &str,
    path: &VaultPath,
    written: bool,
) -> Result<Rebooked, ApiError> {
    let (result, _) = flows::rebook(session, number, today);
    match result {
        Ok(rebooked) => {
            log_mirror(rebooked.mirror.as_ref());
            Ok(rebooked.done)
        }
        Err(error) if !written => Err(books_error(&error)),
        Err(error) => {
            let revision = session
                .vault()
                .read_pinned(path, None)
                .map(|(_, r)| Value::String(r.token()))
                .unwrap_or(Value::Null);
            Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                SAVED_NOT_REBOOKED,
                format!(
                    "invoice {number}'s file was saved, but its books weren't updated: {error}. \
                     The file and the books now disagree: fix that, then Re-book (or run \
                     `ledgeline-invoicer rebook {number}`)"
                ),
            )
            .with("cause", Value::String(error.code().to_string()))
            .with("written", Value::Bool(true))
            .with("path", Value::String(path.to_string()))
            .with("revision", revision))
        }
    }
}

/// The code of a write that changed a sent invoice's file but couldn't
/// re-book it ([`rebook_written`]).
pub(crate) const SAVED_NOT_REBOOKED: &str = "saved-not-rebooked";

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
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let mut session = books.session();
        let doc = session
            .find(&number)
            .map_err(|e| ApiError::status(StatusCode::NOT_FOUND, e.to_string()))?;
        let (path, doc_type, status) = (doc.path.clone(), doc.header.doc_type, doc.header.status);
        let wanted = format!("/{}", body.template.trim().trim_start_matches('/'));
        let entry = super::templates::templates_in(&session)
            .into_iter()
            .find(|t| t.path == wanted)
            .ok_or_else(|| {
                ApiError::status(
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
            return Err(ApiError::status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "the {} template isn't for {} documents",
                    entry.name,
                    kind.as_str()
                ),
            ));
        }
        // A starter is copied in once the invoice names it (B1), and indexed
        // so the invoice's `unknown-template` warning goes at once. The
        // invoice names it either way, so a failed copy is a warning.
        let done = starters::set(
            &mut session,
            &path,
            &[FieldEdit::Set(
                "template".to_string(),
                Value::String(wanted.clone()),
            )],
        )
        .map_err(|e| write_error(&e))?;
        Ok(json!({
            "template": wanted,
            "changed": done.outcome.written,
            "copied": done.copied.map(|p| p.to_string()),
            "status": status.map(Status::as_str),
            "warnings": done.warnings,
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
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let mut session = books.session();
        let doc = session
            .find(&number)
            .map_err(|e| ApiError::status(StatusCode::NOT_FOUND, e.to_string()))?;
        let (path, status) = (doc.path.clone(), doc.header.status);
        let doc_number = doc.number().map(str::to_string);
        match status {
            Some(Status::Void) => Err(ApiError::status(
                StatusCode::CONFLICT,
                format!("invoice {number} is void: its amounts stay as they were booked"),
            )),
            Some(Status::Sent) => {
                let doc_number = doc_number.ok_or_else(|| {
                    ApiError::status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "it has no `invoice_num`".to_string(),
                    )
                })?;
                if !body.rebook {
                    return Err(ApiError::status(
                        StatusCode::CONFLICT,
                        format!(
                            "invoice {doc_number} is sent, so its amount is booked: \
                             recalculating it re-books it. Confirm to recalculate and re-book."
                        ),
                    ));
                }
                // The text it will write is checked first, as rebook would
                // book it: a refusal (a closed period, a dropped schedule, …)
                // comes before the file changes, not after (RC-7).
                let (source, revision) =
                    edit::read_text(session.vault(), &path).map_err(|e| write_error(&e))?;
                let text =
                    edit::recalc_text(session.index(), &source).map_err(|e| write_error(&e))?;
                session
                    .invoice_books()
                    .check_rebook(
                        session.index(),
                        &doc_number,
                        text.as_deref().unwrap_or(&source),
                        &today,
                    )
                    .map_err(|e| books_error(&e))?;
                let (index, vault) = session.parts_mut();
                let recalculated = edit::recalc_at(index, vault, &path, &source, &revision)
                    .map_err(|e| write_error(&e))?;
                let rebooked = rebook_written(
                    &mut session,
                    &doc_number,
                    &today,
                    &path,
                    recalculated.written,
                )
                .map_err(|e| e.with("recalculated", to_value(&recalculated).unwrap_or_default()))?;
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

/// `POST /api/invoices/{num}/company`'s body.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompanyBody {
    /// Today (YYYY-MM-DD): a re-booking's restatement date.
    pub today: Option<String>,
}

/// `POST /api/invoices/{num}/company`: Update from company
/// ([`flows::update_company`]), `{changed, written, rebooked}`. A sent
/// invoice is checked as re-booking would book it before its file changes,
/// then re-booked ([`rebook_written`]); `rebooked` is `null` otherwise. A
/// void invoice, or no company, is `422`.
pub(crate) async fn update_company(
    State(state): State<AppState>,
    Path(number): Path<String>,
    body: Result<Json<CompanyBody>, JsonRejection>,
) -> ApiResult {
    let body = match body {
        Ok(Json(body)) => body,
        Err(JsonRejection::MissingJsonContentType(_)) => CompanyBody::default(),
        Err(e) => return Err(ApiError::bad_request(e.body_text())),
    };
    let today = today(body.today.as_deref())?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let mut session = books.session();
        let path = session
            .find(&number)
            .map_err(|e| ApiError::status(StatusCode::NOT_FOUND, e.to_string()))?
            .path
            .clone();
        let (outcome, rebook) = flows::update_company(&mut session, &path, &today).map_err(
            |refused| match refused {
                flows::UpdateRefused::Refused(why) => {
                    ApiError::status(StatusCode::UNPROCESSABLE_ENTITY, why)
                }
                flows::UpdateRefused::Write(error) => write_error(&error),
                flows::UpdateRefused::Books(error) => books_error(&error),
            },
        )?;
        let rebooked = match rebook {
            Some(number) => Some(to_value(&rebook_written(
                &mut session,
                &number,
                &today,
                &path,
                true,
            )?)?),
            None => None,
        };
        Ok(json!({
            "changed": outcome.changed,
            "written": outcome.written,
            "rebooked": rebooked,
        }))
    })
    .await
}
