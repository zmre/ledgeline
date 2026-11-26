//! The invoice editor's routes (invoicer PLAN 4a, ARCHITECTURE §6.2–6.3).
//! Every write is the CLI's own library call ([`invoicer_core::edit`],
//! [`invoicer_core::editor`], [`invoicer_core::books`]).
//!
//! | Route                                   | Does                                                      |
//! |-----------------------------------------|-----------------------------------------------------------|
//! | `GET /api/editor/options`               | What the editor's pickers offer: customers (with open POs), the catalog, the journal's accounts (and which are revenue, by declared type), custom field keys, the table words, and the default terms and template. |
//! | `GET /api/fields/{key}/values`          | Autocomplete for a custom frontmatter field.              |
//! | `POST /api/invoices`                    | `new`: a draft for a customer (snapshot, terms, snippets). `deposit`: a deposit invoice. |
//! | `GET /api/invoices/{num}/form`          | The document as the editor's form, with its revision and its computed amounts. |
//! | `POST /api/invoices/{num}/compute`      | The text a form makes, its amounts and problems; with `books`, what it would book. Writes nothing. |
//! | `PUT /api/invoices/{num}/form`          | Save a form over the revision read; a sent invoice only with `rebook` (then re-booked). |
//! | `GET /api/invoices/{num}/deposits`      | The customer's open deposits, each with the row that applies it. |
//! | `POST /api/invoices/{num}/void`         | `void` (a reversing entry in accrual books).               |
//! | `POST /api/invoices/{num}/rebook`       | `rebook` (replace its transaction).                        |
//! | `GET /api/invoices/{num}/payments`      | Cash books: the deposits that might pay it (`apply-payment N`). |
//! | `POST /api/invoices/{num}/payments`     | Cash books: Apply payment (`apply-payment N --txn REF`); `dry_run` previews it. |
//! | `POST /api/preview/draft`               | Keep unsaved text for the preview frame; its page URL.     |
//!
//! Errors are the API's one envelope ([`super::api::ApiError`],
//! `{"error": "…", "code": "…"}`): `changed` (409, the file changed since
//! `revision`; the body has the current `revision`), `confirm-rebook`
//! (409), `void` (409), `refused` (409), `invalid` (422), `bad-request`
//! (400), `not-found` (404), `exists` (409), the books bridge's codes
//! ([`invoicer_core::books::BooksError::code`]), and
//! `saved-not-rebooked` (422: a sent invoice's file was written but its
//! re-book failed; `cause`, `written`, `path` and the file's new
//! `revision` say so, [`super::edits::rebook_written`]).

use std::collections::BTreeMap;

use axum::Json;
use axum::Router;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use invoicer_core::books::{self, ApplyPayment, Document, PaymentCandidate, Void, retry_once};
use invoicer_core::customers::ContactKind;
use invoicer_core::doc::{DocType, Invoice, Status};
use invoicer_core::draft::{DraftRow, NewInvoice};
use invoicer_core::edit;
use invoicer_core::editor::{self, EditorError, Form, KNOWN_KEYS};
use invoicer_core::session::Session;
use invoicer_core::table::books::is_iso_date;
use invoicer_core::table::money::MoneyStyle;
use invoicer_core::views;
use ledgeline_core::Dec;
use ledgeline_core::vault::{Revision, VaultPath};
use serde::Deserialize;
use serde_json::{Value, json};

use super::api::{ApiError, ApiResult, body, to_value, today, with_books};
use super::books::{log_mirror, reload_after_write};
use super::edits::{rebook_written, write_error};
use super::preview::{MAX_SOURCE_BYTES, PREVIEW_PAGE, PreviewRequest};
use super::{AppState, prefs};
use crate::{flows, starters};

/// The editor's routes (state still to be supplied).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/editor/options", get(options))
        .route("/api/fields/{key}/values", get(field_values))
        .route("/api/invoices", post(create))
        .route("/api/invoices/{number}/form", get(form).put(save))
        .route("/api/invoices/{number}/compute", post(compute))
        .route("/api/invoices/{number}/deposits", get(deposits))
        .route("/api/invoices/{number}/void", post(void))
        .route("/api/invoices/{number}/rebook", post(rebook))
        .route(
            "/api/invoices/{number}/payments",
            get(payments).post(apply_payment),
        )
        .route("/api/preview/draft", post(preview_draft))
}

impl From<EditorError> for ApiError {
    fn from(error: EditorError) -> Self {
        let code = error.code();
        let status = match code {
            "changed" | "confirm-rebook" | "void" | "refused" => StatusCode::CONFLICT,
            "invalid" => StatusCode::UNPROCESSABLE_ENTITY,
            "bad-request" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let failure = Self::new(status, code, error.to_string());
        match error {
            EditorError::Changed { current } => failure.with("revision", Value::String(current)),
            _ => failure,
        }
    }
}

/// The document `number` names (a number or a path), as the session has it.
fn find(session: &Session, number: &str) -> Result<VaultPath, ApiError> {
    session
        .find(number)
        .map(|doc| doc.path.clone())
        .map_err(|e| ApiError::not_found(e.to_string()))
}

/// The document at `path`, read now, with its revision.
fn read(session: &Session, path: &VaultPath) -> Result<(String, Revision), ApiError> {
    let (bytes, revision) = session
        .vault()
        .read_pinned(path, Some(MAX_SOURCE_BYTES as u64))
        .map_err(|e| {
            if e.is_not_found() {
                ApiError::not_found(format!("{path}: no such file"))
            } else {
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "storage", e.to_string())
            }
        })?;
    let text = String::from_utf8(bytes).map_err(|_| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid",
            format!("{path} isn't UTF-8 text"),
        )
    })?;
    Ok((text, revision))
}

fn kind_name(kind: ContactKind) -> &'static str {
    match kind {
        ContactKind::Person => "person",
        ContactKind::Organization => "organization",
    }
}

/// How many notes `link_notes` offers (a suggestion list, not an index).
const LINK_NOTES_LIMIT: usize = 2000;

/// `GET /api/editor/options`.
async fn options(State(state): State<AppState>) -> ApiResult {
    with_books(&state, StatusCode::OK, |books| {
        let session = books.session();
        let index = session.index();
        let settings = session.settings();
        let words = &settings.words;
        let customers: Vec<Value> = index
            .customers()
            .into_iter()
            .map(|c| {
                json!({
                    "title": c.contact.title,
                    "link": c.wikilink(),
                    "path": c.contact.path.to_string(),
                    "kind": kind_name(c.contact.kind),
                    "terms": c.terms,
                    "invoice_template": c.invoice_template,
                    "open_pos": c.open_pos.iter().map(|po| json!({
                        "reference": po.reference,
                        "amount": po.amount,
                        "expires": po.expires,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let catalog: Vec<Value> = edit::catalogs(index)
            .iter()
            .flat_map(|c| &c.items)
            .map(|item| {
                json!({
                    "code": item.code,
                    "description": item.description,
                    "unit": item.unit,
                    "rate": item.rate_text,
                    "books": item.books_text,
                })
            })
            .collect();
        // Custom keys, most used first, over the documents.
        let mut keys: BTreeMap<String, usize> = BTreeMap::new();
        for note in index.site().notes() {
            if !DocType::ALL.iter().any(|t| note.is_type(t.as_str())) {
                continue;
            }
            for key in &note.order {
                if !KNOWN_KEYS.contains(&key.as_str()) {
                    *keys.entry(key.clone()).or_default() += 1;
                }
            }
        }
        let mut fields: Vec<(String, usize)> = keys.into_iter().collect();
        fields.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        // What `deposit_for` may link to: estimates first, then the other
        // notes that aren't documents, the catalog or snippets.
        let not_linkable = ["invoice", "credit-note", "catalog", "snippet"];
        let site = index.site();
        let estimates = site.notes_of_type(DocType::Estimate.as_str());
        let others = site.notes().filter(|n| {
            !n.is_type(DocType::Estimate.as_str()) && !not_linkable.iter().any(|t| n.is_type(t))
        });
        let link_notes: Vec<Value> = estimates
            .chain(others)
            .take(LINK_NOTES_LIMIT)
            .map(|n| json!({ "title": n.title, "link": format!("[[{}]]", n.title), "kind": n.note_type }))
            .collect();
        let journal = session.journal();
        let accounts = ledgeline_core::wire::journal_to_accountnames(journal);
        // The revenue accounts (and gains), by each account's declared type,
        // else its nearest declared ancestor's, else its name: what a
        // customer's revenue account field offers first.
        let classes = ledgeline_core::view::Classes::of(journal);
        let revenue_accounts: Vec<&String> = accounts
            .iter()
            .filter(|a| classes.is_type(a, ledgeline_core::reports::AccountType::Revenue))
            .collect();
        let next = session.explain_next_number().ok();
        Ok(json!({
            "customers": customers,
            "catalog": catalog,
            "accounts": accounts,
            "revenue_accounts": revenue_accounts,
            "deposit_account": invoicer_core::books::spec::deposit_account(settings, journal),
            "revenue_account": settings.revenue_account,
            "fields": fields.into_iter().map(|(k, _)| k).collect::<Vec<_>>(),
            "words": {
                "description": words.description.first(),
                "quantity": words.quantity.first(),
                "rate": words.rate.first(),
                "amount": words.amount.first(),
                "unit": words.unit.first(),
                "item": words.item.first(),
                "subtotal": words.subtotal.first(),
                "total": words.total.first(),
                "deposit": words.deposit.first(),
                "tax": words.tax.first(),
            },
            "total_words": words.total.words(),
            "subtotal_words": words.subtotal.words(),
            "link_notes": link_notes,
            "next_number": next.as_ref().map(|n| n.number.to_string()),
            "next_number_after": next.as_ref().and_then(|n| n.after.clone()),
            "next_number_source": next.as_ref().map(|n| n.source.as_str()),
            "customers_dir": settings.customers_dir,
            // A new invoice's terms and template when neither it nor its
            // customer names any (`[invoicer] default-terms`,
            // `default-template`).
            "default_terms": settings.default_terms,
            "default_template": settings.default_template,
        }))
    })
    .await
}

/// `GET /api/fields/{key}/values`: `[{value, count}]`, most used first.
async fn field_values(State(state): State<AppState>, Path(key): Path<String>) -> ApiResult {
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let values: Vec<Value> = session
            .index()
            .site()
            .field_values(&key)
            .into_iter()
            .take(100)
            .map(|(value, count)| {
                let text = match value {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                json!({ "value": text, "count": count })
            })
            .collect();
        Ok(Value::Array(values))
    })
    .await
}

/// `POST /api/invoices`'s body: `new`'s options.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    customer: String,
    #[serde(default)]
    template: Option<String>,
    date: String,
    #[serde(default)]
    number: Option<String>,
    #[serde(default)]
    terms: Option<u32>,
    #[serde(default)]
    reference: Option<String>,
    /// A deposit invoice (ARCHITECTURE §4.4): one row, booked to the deposit
    /// account, at zero for the author to fill in.
    #[serde(default)]
    deposit: bool,
}

fn text(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A deposit invoice's one row: "Deposit", booked to the deposit account,
/// at zero for the author to fill in (a row needs an amount). The `deposit`
/// word ("Less deposit") is the other side: a final invoice's row applying it.
fn deposit_row(session: &Session) -> DraftRow {
    DraftRow {
        description: "Deposit".to_string(),
        amount: Some(session.index().money().style(None).format(Dec::zero())),
        books: Some(invoicer_core::books::spec::deposit_account(
            session.settings(),
            session.journal(),
        )),
        ..DraftRow::default()
    }
}

/// `POST /api/invoices`: `new` (a draft), `{path, number, total, written,
/// copied, warnings}`; with `deposit`, a deposit invoice's draft.
async fn create(
    State(state): State<AppState>,
    payload: Result<Json<CreateBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    if !is_iso_date(request.date.trim()) {
        return Err(ApiError::bad_request("date: not a date (YYYY-MM-DD)"));
    }
    with_books(&state, StatusCode::OK, move |books| {
        let mut session = books.session();
        let template = text(request.template);
        let (starter_template, starter) =
            crate::starter_for_new(&session, &request.customer, template.as_deref());
        let new = NewInvoice {
            template: template.or(starter_template),
            number: text(request.number),
            terms: request.terms,
            reference: text(request.reference),
            rows: if request.deposit {
                vec![deposit_row(&session)]
            } else {
                Vec::new()
            },
            ..NewInvoice::new(request.customer.trim(), request.date.trim())
        };
        let booked: Vec<String> = session.booked_numbers().to_vec();
        let booked: Vec<&str> = booked.iter().map(String::as_str).collect();
        let (index, vault) = session.parts_mut();
        let created =
            edit::create(index, vault, &new, &booked, false).map_err(|e| write_error(&e))?;
        // The starter it uses is copied in now it is written (B1). The draft
        // exists (and has its number) either way, so a failed copy is a
        // warning, not an error a retry would answer with a second draft
        // (RA-7).
        let mut warnings: Vec<String> = created.warnings.clone();
        let copied = match starter {
            Some(starter) => {
                let dir = session.settings().templates_dir.clone();
                match starters::install(session.vault(), &dir, starter) {
                    Ok(installed) => {
                        let (index, vault) = session.parts_mut();
                        index.refresh(vault, &installed.written_files());
                        installed.written.then(|| installed.path.to_string())
                    }
                    Err(e) => {
                        warnings.push(format!(
                            "the draft is written, but the {} template wasn't copied in: {e}",
                            starter.name
                        ));
                        None
                    }
                }
            }
            None => None,
        };
        let mut out = to_value(&created)?;
        out["copied"] = json!(copied);
        out["warnings"] = json!(warnings);
        Ok(out)
    })
    .await
}

/// What the editor shows for `text` (the file at `path` as `form` makes it):
/// the table computed, its problems, and with `books` what it would book.
fn computed(
    session: &Session,
    path: &VaultPath,
    saved: &str,
    text: &str,
    books: bool,
) -> Result<Value, ApiError> {
    let settings = session.settings();
    let money = session.index().money();
    let invoice = Invoice::parse(text, &settings.words);
    let (table, table_error) = match invoice.bill_table() {
        Ok(Some(t)) => match views::table_view(t, &settings.words, money) {
            Ok(view) => (Some(view), None),
            Err(e) => (None, Some(e.to_string())),
        },
        Ok(None) => (None, None),
        Err(e) => (None, Some(e.to_string())),
    };
    let problems: Vec<Value> = invoice
        .problems(&settings.words, money)
        .into_iter()
        .map(|p| {
            let mut v = serde_json::to_value(&p).unwrap_or(Value::Null);
            if let Some(map) = v.as_object_mut() {
                map.remove("file");
                map.entry("line").or_insert(Value::Null);
            }
            v
        })
        .collect();
    let books_view = books.then(|| {
        let document = Document {
            path: path.clone(),
            invoice: Invoice::parse(text, &settings.words),
            revision: Revision::of_bytes(text.as_bytes()),
        };
        match session
            .invoice_books()
            .preview_of(session.index(), &document)
        {
            Ok(p) => json!({
                "basis": p.basis,
                "would_book": p.would_book,
                "booked": p.booked,
                "in_step": p.in_step,
                "receivable": p.receivable,
                "error": Value::Null,
            }),
            Err(e) => json!({
                "basis": session.basis(),
                "would_book": Value::Null,
                "booked": Value::Null,
                "in_step": Value::Null,
                "receivable": Value::Null,
                "error": e.to_string(),
            }),
        }
    });
    Ok(json!({
        "text": text,
        "changed": text != saved,
        "total": table.as_ref().map(|t| t.total.clone()),
        "commodity": table.as_ref().map(|t| t.commodity.clone()),
        "table": table,
        "table_error": table_error,
        "problems": problems,
        "books": books_view,
    }))
}

/// `GET /api/invoices/{num}/form`: `{path, title, number, status, revision,
/// form, computed}`.
async fn form(State(state): State<AppState>, Path(number): Path<String>) -> ApiResult {
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let path = find(&session, &number)?;
        let (text, revision) = read(&session, &path)?;
        let invoice = Invoice::parse(text.as_str(), &session.settings().words);
        let form = editor::form_of(&invoice);
        let header = invoice.header();
        Ok(json!({
            "path": path.to_string(),
            "title": invoice.frontmatter().str("title").unwrap_or_default(),
            "number": header.number.as_ref().map(ToString::to_string),
            "status": header.status.map(Status::as_str),
            "revision": revision.token(),
            "form": to_value(&form)?,
            "computed": computed(&session, &path, &text, &text, false)?,
        }))
    })
    .await
}

/// `POST /api/invoices/{num}/compute`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComputeBody {
    form: Form,
    #[serde(default)]
    books: bool,
}

/// `POST /api/invoices/{num}/compute`: the text the form makes of the file
/// as it is now, computed. Writes nothing.
async fn compute(
    State(state): State<AppState>,
    Path(number): Path<String>,
    payload: Result<Json<ComputeBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let path = find(&session, &number)?;
        let (saved, _) = read(&session, &path)?;
        let invoice = Invoice::parse(saved.as_str(), &session.settings().words);
        let text = editor::apply(session.index(), &path, &invoice, &request.form)?;
        computed(&session, &path, &saved, &text, request.books)
    })
    .await
}

/// `PUT /api/invoices/{num}/form`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveBody {
    form: Form,
    /// The revision the editor read (`GET …/form`).
    revision: String,
    /// A sent invoice: save and re-book it (the confirmation).
    #[serde(default)]
    rebook: bool,
    today: Option<String>,
}

/// `PUT /api/invoices/{num}/form`: `{written, path, number, status,
/// revision, rebooked, copied, warnings}`.
///
/// A sent invoice's amount is booked, so saving it re-books it: only with
/// `rebook: true`. The text the form makes is re-booked in memory before
/// anything is written ([`invoicer_core::books::InvoiceBooks::check_rebook`],
/// RC-7), so a refusal (a date on or before `closed-through`, I8; a
/// dropped schedule; a row with no account) leaves the file as it was.
/// Saved, it is re-booked by `rebook`'s rules; a re-book that still fails
/// answers `saved-not-rebooked` (TW-4, [`super::edits::rebook_written`]).
async fn save(
    State(state): State<AppState>,
    Path(number): Path<String>,
    payload: Result<Json<SaveBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    let today = today(request.today.as_deref())?;
    with_books(&state, StatusCode::OK, move |books| {
        let mut session = books.session();
        let path = find(&session, &number)?;
        // Read it, and the books, afresh: sent outside the app (the CLI, a
        // hand edit) since the watcher's last refresh, it must still be
        // checked and re-booked as sent (RA-6).
        reload_after_write(&mut session);
        let (index, vault) = session.parts_mut();
        index.refresh(vault, std::slice::from_ref(&path));
        let doc = session.index().doc(&path);
        let sent = doc.is_some_and(|d| d.header.status == Some(Status::Sent));
        let doc_number = doc.and_then(|d| d.number()).map(str::to_string);
        if sent && request.rebook {
            // Refuse before the file changes, not after: the text this form
            // makes of the file as the editor read it, re-booked in memory.
            // (Read at another revision, the save refuses it as `changed`.)
            let (source, revision) = read(&session, &path)?;
            if revision.token() == request.revision {
                let invoice = Invoice::parse(source.as_str(), &session.settings().words);
                let text = editor::apply(session.index(), &path, &invoice, &request.form)?;
                session
                    .invoice_books()
                    .check_rebook(
                        session.index(),
                        doc_number.as_deref().unwrap_or_default(),
                        &text,
                        &today,
                    )
                    .map_err(|e| ApiError::from(&e))?;
            }
        }
        let (index, vault) = session.parts_mut();
        let saved = editor::save(
            index,
            vault,
            &path,
            &request.form,
            &request.revision,
            request.rebook,
        )?;
        // A starter the form now names is copied in (B1, F9); the invoice
        // is saved either way, so a failed copy is a warning.
        let mut warnings: Vec<String> = Vec::new();
        let copied = match request.form.template.as_deref().map(str::trim) {
            Some(template) if saved.written && !template.is_empty() => {
                starters::copy_in_named(&mut session, template).unwrap_or_else(|warning| {
                    warnings.push(warning);
                    None
                })
            }
            _ => None,
        };
        let rebooked = if sent && saved.written {
            let number = saved.number.clone().or(doc_number).unwrap_or_default();
            let rebooked = rebook_written(&mut session, &number, &today, &saved.path, true)?;
            Some(to_value(&rebooked)?)
        } else {
            None
        };
        // The paid mirror may have rewritten the file: answer its revision.
        let revision = session
            .vault()
            .read_pinned(&saved.path, None)
            .map(|(_, r)| r.token())
            .unwrap_or(saved.revision.clone());
        Ok(json!({
            "written": saved.written,
            "path": saved.path.to_string(),
            "number": saved.number,
            "status": saved.status.map(Status::as_str),
            "revision": revision,
            "rebooked": rebooked,
            "copied": copied.map(|p| p.to_string()),
            "warnings": warnings,
        }))
    })
    .await
}

/// `GET /api/invoices/{num}/deposits`: the customer's open deposits, each
/// with the row that applies all that is left of it.
async fn deposits(State(state): State<AppState>, Path(number): Path<String>) -> ApiResult {
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let path = find(&session, &number)?;
        let doc_number = session
            .index()
            .doc(&path)
            .and_then(|d| d.number())
            .map(str::to_string)
            .ok_or_else(|| ApiError::bad_request("it has no `invoice_num`"))?;
        let open = session
            .invoice_books()
            .open_deposits(session.index(), &doc_number)
            .map_err(|e| ApiError::from(&e))?;
        let style = session.index().money().style(Some(&open.commodity));
        let word = session.settings().words.deposit.first().to_string();
        let deposits: Vec<Value> = open
            .deposits
            .iter()
            .map(|d| {
                let amount = d
                    .remaining
                    .neg()
                    .map(|n| style.format(n))
                    .unwrap_or_default();
                json!({
                    "number": d.number,
                    "path": d.path.as_ref().map(ToString::to_string),
                    "date": d.date,
                    "description": d.description,
                    "credited": style.format(d.credited),
                    "remaining": style.format(d.remaining),
                    "row": {
                        "description": match &d.number {
                            Some(n) => format!("{word} (Invoice {n})"),
                            None => format!("{word} ({})", d.date),
                        },
                        "amount": amount,
                        "books": open.account,
                    },
                })
            })
            .collect();
        Ok(json!({
            "customer": open.customer,
            "account": open.account,
            "commodity": open.commodity,
            "balance": style.format(open.balance),
            "deposits": deposits,
        }))
    })
    .await
}

/// The number of the document `target` names.
fn number_of(session: &Session, target: &str) -> Result<String, ApiError> {
    let path = find(session, target)?;
    session
        .index()
        .doc(&path)
        .and_then(|d| d.number())
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid",
                "it has no `invoice_num`",
            )
        })
}

/// `POST /api/invoices/{num}/void`'s body.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct VoidBody {
    date: Option<String>,
    today: Option<String>,
}

/// `POST /api/invoices/{num}/void`: the CLI's `void` (default date today).
async fn void(
    State(state): State<AppState>,
    Path(number): Path<String>,
    payload: Result<Json<VoidBody>, JsonRejection>,
) -> ApiResult {
    let request = match payload {
        Ok(Json(body)) => body,
        Err(JsonRejection::MissingJsonContentType(_)) => VoidBody::default(),
        Err(e) => return Err(ApiError::bad_request(e.body_text())),
    };
    let today = today(request.today.as_deref())?;
    with_books(&state, StatusCode::OK, move |books| {
        let mut session = books.session();
        let number = number_of(&session, &number)?;
        let void = Void {
            number,
            date: text(request.date).unwrap_or_else(|| today.clone()),
            today,
            expected: None,
        };
        let (result, _) = flows::void(&mut session, &void);
        let written = result.map_err(|e| ApiError::from(&e))?;
        log_mirror(written.mirror.as_ref());
        to_value(&written.done)
    })
    .await
}

/// `POST /api/invoices/{num}/rebook`'s body.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RebookBody {
    today: Option<String>,
}

/// `POST /api/invoices/{num}/rebook`: the CLI's `rebook`.
async fn rebook(
    State(state): State<AppState>,
    Path(number): Path<String>,
    payload: Result<Json<RebookBody>, JsonRejection>,
) -> ApiResult {
    let request = match payload {
        Ok(Json(body)) => body,
        Err(JsonRejection::MissingJsonContentType(_)) => RebookBody::default(),
        Err(e) => return Err(ApiError::bad_request(e.body_text())),
    };
    let today = today(request.today.as_deref())?;
    with_books(&state, StatusCode::OK, move |books| {
        let mut session = books.session();
        let number = number_of(&session, &number)?;
        let (result, _) = flows::rebook(&mut session, &number, &today);
        let written = result.map_err(|e| ApiError::from(&e))?;
        log_mirror(written.mirror.as_ref());
        to_value(&written.done)
    })
    .await
}

/// `GET /api/invoices/{num}/payments`'s query.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PaymentsQuery {
    today: Option<String>,
}

/// A deposit that might pay the invoice, its amount in the books' style.
fn candidate_view(candidate: &PaymentCandidate, style: &MoneyStyle) -> Value {
    json!({
        "reference": candidate.reference,
        "date": candidate.date,
        "description": candidate.description,
        "received": style.format(candidate.received),
        "exact": candidate.exact,
        "by_number": candidate.by_number,
        "by_customer": candidate.by_customer,
        "gap_days": candidate.gap_days,
    })
}

/// Row shares (`render_dec` text, as the CLI's JSON has them) in the books'
/// style: `[{account, amount}]`.
fn shares_view(shares: &[(String, String)], style: &MoneyStyle) -> Value {
    shares
        .iter()
        .map(|(account, amount)| {
            let shown =
                Dec::parse(amount, '.').map_or_else(|_| amount.clone(), |d| style.format(d));
            json!({ "account": account, "amount": shown })
        })
        .collect()
}

/// The commodity of the invoice at `path` (its table's), for formatting.
fn commodity_of(session: &Session, path: &VaultPath) -> Option<String> {
    session
        .index()
        .doc(path)
        .and_then(|d| d.summary.as_ref())
        .map(|s| s.commodity.clone())
}

/// `GET /api/invoices/{num}/payments`: Apply payment's choices (cash books,
/// I17) — the CLI's `apply-payment N`: `{number, commodity, total, paid,
/// remaining, candidates: [{reference, date, description, received, exact,
/// by_number, by_customer, gap_days}]}`, amounts in the books' style, best
/// candidate first. `refused` in accrual books; a `status` code for an
/// invoice not sent.
async fn payments(
    State(state): State<AppState>,
    Path(number): Path<String>,
    query: Result<Query<PaymentsQuery>, QueryRejection>,
) -> ApiResult {
    let Query(query) = query.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let today = today(query.today.as_deref())?;
    let prefs_state = state.clone();
    with_books(&state, StatusCode::OK, move |books| {
        // The matcher's settings (RC-10), read before the session is locked.
        let settle = prefs::settle_settings(&prefs_state);
        let session = books.session();
        let number = number_of(&session, &number)?;
        let options = session
            .invoice_books()
            .payment_options(session.index(), &number, &settle, &today)
            .map_err(|e| ApiError::from(&e))?;
        let style = session.index().money().style(Some(&options.commodity));
        Ok(json!({
            "number": options.number,
            "commodity": options.commodity,
            "total": style.format(options.total),
            "paid": style.format(options.paid),
            "remaining": style.format(options.remaining),
            "candidates": options
                .candidates
                .iter()
                .map(|c| candidate_view(c, &style))
                .collect::<Vec<_>>(),
        }))
    })
    .await
}

/// `POST /api/invoices/{num}/payments`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PaymentBody {
    /// A candidate's `reference`.
    reference: String,
    /// Show what it would write; write nothing.
    #[serde(default)]
    dry_run: bool,
    today: Option<String>,
}

/// `POST /api/invoices/{num}/payments`: **Apply payment** (cash books,
/// I17), the CLI's `apply-payment N --txn REF` — the same
/// [`InvoiceBooks::apply_payment`](invoicer_core::books::InvoiceBooks::apply_payment),
/// its undo record, and the `paid:` mirror. With `dry_run`,
/// [`preview_payment`](invoicer_core::books::InvoiceBooks::preview_payment):
/// the same plan, nothing written. Either way `{written, number, date,
/// description, received, applied: [{account, amount}], partial, remaining,
/// warnings}`, amounts in the books' style. `refused` for a reference that
/// is no longer a candidate, and in accrual books.
async fn apply_payment(
    State(state): State<AppState>,
    Path(number): Path<String>,
    payload: Result<Json<PaymentBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    let today = today(request.today.as_deref())?;
    let prefs_state = state.clone();
    with_books(&state, StatusCode::OK, move |books| {
        let settle = prefs::settle_settings(&prefs_state);
        let mut session = books.session();
        let path = find(&session, &number)?;
        let number = number_of(&session, &number)?;
        let style = session
            .index()
            .money()
            .style(commodity_of(&session, &path).as_deref());
        let invoice_books = session.invoice_books();
        let apply = ApplyPayment {
            number: number.clone(),
            reference: request.reference,
            today: today.clone(),
            settle,
            expected: None,
        };
        if request.dry_run {
            let preview = invoice_books
                .preview_payment(session.index(), &apply)
                .map_err(|e| ApiError::from(&e))?;
            let style = session.index().money().style(Some(&preview.commodity));
            return Ok(json!({
                "written": false,
                "number": preview.number,
                "date": preview.deposit.date,
                "description": preview.deposit.description,
                "received": style.format(preview.received),
                "applied": shares_view(&preview.applied, &style),
                "partial": preview.partial,
                "remaining": style.format(preview.remaining),
                "warnings": [],
            }));
        }
        let (index, _) = session.parts_mut();
        let (result, _) = retry_once(|| invoice_books.apply_payment(index, &apply));
        reload_after_write(&mut session);
        let applied = result.map_err(|e| ApiError::from(&e))?;
        let (undo_file, warning) = books::undo::keep(session.vault(), &applied, &today);
        log_mirror(flows::settle(&mut session, &number).as_ref());
        Ok(json!({
            "written": true,
            "number": applied.number,
            "date": applied.transaction.date,
            "description": applied.transaction.description,
            "received": style.format(applied.received),
            "applied": shares_view(&applied.applied, &style),
            "partial": applied.remaining.mantissa > 0,
            "remaining": style.format(applied.remaining),
            "undo_file": undo_file.map(|p| p.to_string()),
            "warnings": warning.into_iter().collect::<Vec<_>>(),
        }))
    })
    .await
}

/// `POST /api/preview/draft`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftBody {
    number: String,
    source: String,
    #[serde(default)]
    template: Option<String>,
}

/// `POST /api/preview/draft`: `{url}`, the preview page of the unsaved
/// `source` (as the note `number` names), for the editor's frame.
async fn preview_draft(
    State(state): State<AppState>,
    payload: Result<Json<DraftBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    if request.source.len() > MAX_SOURCE_BYTES {
        return Err(ApiError::bad_request(format!(
            "source: more than {MAX_SOURCE_BYTES} bytes"
        )));
    }
    with_books(&state, StatusCode::OK, move |books| {
        let path = find(&books.session(), &request.number)?;
        let id = books
            .drafts()
            .put(PreviewRequest {
                number: None,
                path: Some(path.to_string()),
                source: Some(request.source),
                template: text(request.template),
                template_source: None,
            })
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "no preview id",
                )
            })?;
        Ok(json!({ "url": format!("{PREVIEW_PAGE}?draft={id}") }))
    })
    .await
}
