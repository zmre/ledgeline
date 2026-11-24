//! The invoice editor's routes (invoicer PLAN 4a, ARCHITECTURE §6.2–6.3).
//! Every write is the CLI's own library call ([`invoicer_core::edit`],
//! [`invoicer_core::editor`], [`invoicer_core::books`]).
//!
//! | Route                                   | Does                                                      |
//! |-----------------------------------------|-----------------------------------------------------------|
//! | `GET /api/editor/options`               | What the editor's pickers offer: customers (with open POs), the catalog, the journal's accounts, custom field keys, the table words. |
//! | `GET /api/fields/{key}/values`          | Autocomplete for a custom frontmatter field.              |
//! | `POST /api/invoices`                    | `new`: a draft for a customer (snapshot, terms, snippets). |
//! | `GET /api/invoices/{num}/form`          | The document as the editor's form, with its revision and its computed amounts. |
//! | `POST /api/invoices/{num}/compute`      | The text a form makes, its amounts and problems; with `books`, what it would book. Writes nothing. |
//! | `PUT /api/invoices/{num}/form`          | Save a form over the revision read; a sent invoice only with `rebook` (then re-booked). |
//! | `GET /api/invoices/{num}/deposits`      | The customer's open deposits, each with the row that applies it. |
//! | `POST /api/invoices/{num}/void`         | `void` (a reversing entry in accrual books).               |
//! | `POST /api/invoices/{num}/rebook`       | `rebook` (replace its transaction).                        |
//! | `POST /api/preview/draft`               | Keep unsaved text for the preview frame; its page URL.     |
//!
//! Errors are `{"error": "…", "code": "…"}`: `changed` (409, the file
//! changed since `revision`; the body has the current `revision`),
//! `confirm-rebook` (409), `void` (409), `refused` (409), `invalid` (422),
//! `bad-request` (400), `not-found` (404), `exists` (409), and the books
//! bridge's codes ([`invoicer_core::books::BooksError::code`]).

use std::collections::BTreeMap;

use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use invoicer_core::books::{BooksError, Document, Rebook, Void, retry_once};
use invoicer_core::customers::ContactKind;
use invoicer_core::doc::{DocType, Invoice, Status};
use invoicer_core::draft::NewInvoice;
use invoicer_core::edit;
use invoicer_core::editor::{self, EditorError, Form, KNOWN_KEYS};
use invoicer_core::session::Session;
use invoicer_core::table::books::is_iso_date;
use invoicer_core::views;
use ledgeline_core::vault::{Revision, VaultPath};
use ledgeline_shell::security::no_store_json;
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use super::api::{ApiError, today};
use super::books::{OpenBooks, reconcile_in, reload_after_write};
use super::edits::write_error;
use super::pdf::books_error;
use super::preview::{MAX_SOURCE_BYTES, PREVIEW_PAGE, PreviewRequest};
use crate::starters;

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
        .route("/api/preview/draft", post(preview_draft))
}

/// An editor failure: a status, a stable code, a sentence, and for
/// `changed` the file's current revision.
#[derive(Debug)]
pub(crate) struct Failure {
    status: StatusCode,
    code: &'static str,
    message: String,
    revision: Option<String>,
}

impl Failure {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            revision: None,
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad-request", message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not-found", message)
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message, "code": self.code });
        if let Some(revision) = self.revision {
            body["revision"] = Value::String(revision);
        }
        let mut response = no_store_json(body);
        *response.status_mut() = self.status;
        response
    }
}

impl From<ApiError> for Failure {
    fn from(ApiError(status, message): ApiError) -> Self {
        let code = match status {
            StatusCode::BAD_REQUEST => "bad-request",
            StatusCode::NOT_FOUND => "not-found",
            StatusCode::CONFLICT => "conflict",
            StatusCode::UNPROCESSABLE_ENTITY => "invalid",
            _ => "internal",
        };
        Self::new(status, code, message)
    }
}

impl From<&BooksError> for Failure {
    fn from(error: &BooksError) -> Self {
        let ApiError(status, message) = books_error(error);
        Self::new(status, error.code(), message)
    }
}

impl From<EditorError> for Failure {
    fn from(error: EditorError) -> Self {
        let code = error.code();
        let status = match code {
            "changed" | "confirm-rebook" | "void" | "refused" => StatusCode::CONFLICT,
            "invalid" => StatusCode::UNPROCESSABLE_ENTITY,
            "bad-request" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let revision = match &error {
            EditorError::Changed { current } => Some(current.clone()),
            _ => None,
        };
        Self {
            status,
            code,
            message: error.to_string(),
            revision,
        }
    }
}

type Answer = Result<Response, Failure>;

/// Run `f` over the open books on the blocking pool, answering its JSON.
async fn with_books<F>(state: &AppState, f: F) -> Answer
where
    F: FnOnce(&OpenBooks) -> Result<Value, Failure> + Send + 'static,
{
    let books = state
        .books()
        .ok_or_else(|| Failure::from(super::api::no_books()))?;
    match tokio::task::spawn_blocking(move || f(&books)).await {
        Ok(result) => result.map(no_store_json),
        Err(error) => Err(Failure::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            format!("the request failed: {error}"),
        )),
    }
}

fn body<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, Failure> {
    payload
        .map(|Json(body)| body)
        .map_err(|e| Failure::bad_request(e.body_text()))
}

fn value(value: impl serde::Serialize) -> Result<Value, Failure> {
    serde_json::to_value(value)
        .map_err(|e| Failure::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))
}

/// The document `number` names (a number or a path), as the session has it.
fn find(session: &Session, number: &str) -> Result<VaultPath, Failure> {
    session
        .find(number)
        .map(|doc| doc.path.clone())
        .map_err(|e| Failure::not_found(e.to_string()))
}

/// The document at `path`, read now, with its revision.
fn read(session: &Session, path: &VaultPath) -> Result<(String, Revision), Failure> {
    let (bytes, revision) = session
        .vault()
        .read_pinned(path, Some(MAX_SOURCE_BYTES as u64))
        .map_err(|e| {
            if e.is_not_found() {
                Failure::not_found(format!("{path}: no such file"))
            } else {
                Failure::new(StatusCode::INTERNAL_SERVER_ERROR, "storage", e.to_string())
            }
        })?;
    let text = String::from_utf8(bytes).map_err(|_| {
        Failure::new(
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

/// `GET /api/editor/options`.
async fn options(State(state): State<AppState>) -> Answer {
    with_books(&state, |books| {
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
        let catalog: Vec<Value> = edit::catalogs(index, session.vault())
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
        let journal = session.journal();
        let next = session.next_number().map(|n| n.to_string()).ok();
        Ok(json!({
            "customers": customers,
            "catalog": catalog,
            "accounts": ledgeline_core::wire::journal_to_accountnames(journal),
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
            "next_number": next,
            "customers_dir": settings.customers_dir,
        }))
    })
    .await
}

/// `GET /api/fields/{key}/values`: `[{value, count}]`, most used first.
async fn field_values(State(state): State<AppState>, Path(key): Path<String>) -> Answer {
    with_books(&state, move |books| {
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
}

fn text(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `POST /api/invoices`: `new` (a draft), `{path, number, total, written,
/// copied}`.
async fn create(
    State(state): State<AppState>,
    payload: Result<Json<CreateBody>, JsonRejection>,
) -> Answer {
    let request = body(payload)?;
    if !is_iso_date(request.date.trim()) {
        return Err(Failure::bad_request("date: not a date (YYYY-MM-DD)"));
    }
    with_books(&state, move |books| {
        let mut session = books.session();
        let template = text(request.template);
        let (starter_template, starter) =
            crate::starter_for_new(&session, &request.customer, template.as_deref());
        let new = NewInvoice {
            template: template.or(starter_template),
            number: text(request.number),
            terms: request.terms,
            reference: text(request.reference),
            ..NewInvoice::new(request.customer.trim(), request.date.trim())
        };
        let booked: Vec<String> = session.booked_numbers().to_vec();
        let booked: Vec<&str> = booked.iter().map(String::as_str).collect();
        let (index, vault) = session.parts_mut();
        let created = edit::create(index, vault, &new, &booked, false).map_err(|e| {
            let ApiError(status, message) = write_error(&e);
            Failure::new(status, "invalid", message)
        })?;
        // The starter it uses is copied in now it is written (B1).
        let copied = match starter {
            Some(starter) => {
                let dir = session.settings().templates_dir.clone();
                let installed = starters::install(session.vault(), &dir, starter).map_err(|e| {
                    Failure::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "storage",
                        format!(
                            "the draft is written, but the {} template wasn't copied in: {e}",
                            starter.name
                        ),
                    )
                })?;
                let (index, vault) = session.parts_mut();
                index.refresh(vault, &installed.written_files());
                installed.written.then(|| installed.path.to_string())
            }
            None => None,
        };
        let mut out = value(&created)?;
        out["copied"] = json!(copied);
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
) -> Result<Value, Failure> {
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
async fn form(State(state): State<AppState>, Path(number): Path<String>) -> Answer {
    with_books(&state, move |books| {
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
            "form": value(&form)?,
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
) -> Answer {
    let request = body(payload)?;
    with_books(&state, move |books| {
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
/// revision, rebooked, copied}`.
///
/// A sent invoice's amount is booked, so saving it re-books it: only with
/// `rebook: true`, refused before anything is written when it is dated on
/// or before `closed-through` (I8), then re-booked by `rebook`'s rules.
async fn save(
    State(state): State<AppState>,
    Path(number): Path<String>,
    payload: Result<Json<SaveBody>, JsonRejection>,
) -> Answer {
    let request = body(payload)?;
    let today = today(request.today.as_deref()).map_err(Failure::from)?;
    with_books(&state, move |books| {
        let mut session = books.session();
        let path = find(&session, &number)?;
        let doc = session.index().doc(&path);
        let sent = doc.is_some_and(|d| d.header.status == Some(Status::Sent));
        let doc_number = doc.and_then(|d| d.number()).map(str::to_string);
        if sent && request.rebook {
            // Refuse before the file changes, not after (closed period).
            let doc_number = doc_number.clone().unwrap_or_default();
            session
                .invoice_books()
                .check_rebook(session.index(), &doc_number)
                .map_err(|e| Failure::from(&e))?;
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
        // A starter the form now names is copied in (B1, F9).
        let copied = match request.form.template.as_deref().map(str::trim) {
            Some(template) if saved.written && !template.is_empty() => {
                let dir = session.settings().templates_dir.clone();
                match starters::install_named(session.vault(), &dir, template) {
                    Ok(Some(done)) if done.written => {
                        let (index, vault) = session.parts_mut();
                        index.refresh(vault, &done.written_files());
                        Some(done.path.to_string())
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let rebooked = if sent && saved.written {
            let number = saved.number.clone().or(doc_number).unwrap_or_default();
            let invoice_books = session.invoice_books();
            let request = Rebook {
                number: number.clone(),
                today,
                expected: None,
            };
            let (index, _) = session.parts_mut();
            let (result, _) = retry_once(|| invoice_books.rebook(index, &request));
            reload_after_write(&mut session);
            let rebooked = result.map_err(|e| {
                let failed = Failure::from(&e);
                Failure {
                    message: format!(
                        "the invoice was saved, but re-booking it failed: {}; fix it, then \
                         Re-book (or `ledgeline-invoicer rebook {number}`)",
                        failed.message
                    ),
                    ..failed
                }
            })?;
            if rebooked.changed {
                reconcile_in(&mut session, Some(vec![number]));
            }
            Some(value(&rebooked)?)
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
            "copied": copied,
        }))
    })
    .await
}

/// `GET /api/invoices/{num}/deposits`: the customer's open deposits, each
/// with the row that applies all that is left of it.
async fn deposits(State(state): State<AppState>, Path(number): Path<String>) -> Answer {
    with_books(&state, move |books| {
        let session = books.session();
        let path = find(&session, &number)?;
        let doc_number = session
            .index()
            .doc(&path)
            .and_then(|d| d.number())
            .map(str::to_string)
            .ok_or_else(|| Failure::bad_request("it has no `invoice_num`"))?;
        let open = session
            .invoice_books()
            .open_deposits(session.index(), &doc_number)
            .map_err(|e| Failure::from(&e))?;
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
fn number_of(session: &Session, target: &str) -> Result<String, Failure> {
    let path = find(session, target)?;
    session
        .index()
        .doc(&path)
        .and_then(|d| d.number())
        .map(str::to_string)
        .ok_or_else(|| {
            Failure::new(
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
) -> Answer {
    let request = match payload {
        Ok(Json(body)) => body,
        Err(JsonRejection::MissingJsonContentType(_)) => VoidBody::default(),
        Err(e) => return Err(Failure::bad_request(e.body_text())),
    };
    let today = today(request.today.as_deref()).map_err(Failure::from)?;
    with_books(&state, move |books| {
        let mut session = books.session();
        let number = number_of(&session, &number)?;
        let invoice_books = session.invoice_books();
        let void = Void {
            number: number.clone(),
            date: text(request.date).unwrap_or_else(|| today.clone()),
            today,
            expected: None,
        };
        let (index, _) = session.parts_mut();
        let (result, _) = retry_once(|| invoice_books.void(index, &void));
        reload_after_write(&mut session);
        let voided = result.map_err(|e| Failure::from(&e))?;
        reconcile_in(&mut session, Some(vec![number]));
        value(&voided)
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
) -> Answer {
    let request = match payload {
        Ok(Json(body)) => body,
        Err(JsonRejection::MissingJsonContentType(_)) => RebookBody::default(),
        Err(e) => return Err(Failure::bad_request(e.body_text())),
    };
    let today = today(request.today.as_deref()).map_err(Failure::from)?;
    with_books(&state, move |books| {
        let mut session = books.session();
        let number = number_of(&session, &number)?;
        let invoice_books = session.invoice_books();
        let request = Rebook {
            number: number.clone(),
            today,
            expected: None,
        };
        let (index, _) = session.parts_mut();
        let (result, _) = retry_once(|| invoice_books.rebook(index, &request));
        reload_after_write(&mut session);
        let rebooked = result.map_err(|e| Failure::from(&e))?;
        if rebooked.changed {
            reconcile_in(&mut session, Some(vec![number]));
        }
        value(&rebooked)
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
) -> Answer {
    let request = body(payload)?;
    if request.source.len() > MAX_SOURCE_BYTES {
        return Err(Failure::bad_request(format!(
            "source: more than {MAX_SOURCE_BYTES} bytes"
        )));
    }
    with_books(&state, move |books| {
        let path = find(&books.session(), &request.number)?;
        let id = books
            .drafts()
            .put(PreviewRequest {
                number: None,
                path: Some(path.to_string()),
                source: Some(request.source),
                template: text(request.template),
            })
            .ok_or_else(|| {
                Failure::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "no preview id",
                )
            })?;
        Ok(json!({ "url": format!("{PREVIEW_PAGE}?draft={id}") }))
    })
    .await
}
