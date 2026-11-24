//! PDF export and the Send flow over the API (invoicer PLAN 3c), and the
//! CLI's `pdf` in a desktop build.
//!
//! | Route                                 | Does                                                        |
//! |---------------------------------------|-------------------------------------------------------------|
//! | `GET /api/invoices/{num}/pdf`         | [`Delivery`]: where Export puts it, the kept PDFs, `sent_hash`, the portal, the copy fields, what this process can do |
//! | `POST /api/invoices/{num}/pdf`        | Export (`{"temp": true}` for a temporary file): [`crate::pdf::Exported`] |
//! | `POST /api/invoices/{num}/send`       | Send (book it, `status: sent`), `{"via": "portal"}`          |
//! | `POST /api/invoices/{num}/portal`     | Open the customer's portal in the browser (window only); answers its URL |
//! | `POST /api/pdf/share`, `/api/pdf/copy`| Share… or Copy file `{destination, file}`: a PDF this app exported |
//! | `POST /api/pdf/open`                  | Open `{destination, file}` in the default PDF viewer (window, macOS) |
//!
//! **How a PDF is made** ([`export_invoice`], shared by the API and the
//! CLI): plan on the blocking pool ([`crate::pdf::plan`]); mint a one-time
//! export link for the invoice's MBR page ([`MbrGate::export_link`]); ask
//! the main thread to print it ([`Native`]); file it
//! ([`crate::pdf::complete`]). The session is never locked while printing.
//!
//! Errors: `404` unknown invoice, `409` no books, `422` refused (a portal
//! that needs a `reference`, a closed period), `502` the print failed,
//! `503` this process can't print (a server-only build, or no display).

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::{Extension, Json};
use invoicer_core::books::{self, BooksError, Send};
use invoicer_core::doc::Invoice;
use invoicer_core::sent;
use ledgeline_core::vault::VaultPath;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::AppState;
use super::api::{ApiError, ApiResult, blocking, no_books, to_value, today};
use super::books::OpenBooks;
use super::mbr::MbrGate;
use super::native::{Anchor, Capabilities, Native, NativeRequest, PrintJob, answer};
use crate::pdf::{self as filing, Destination, Exported, PdfError};

/// What one export asks for.
#[derive(Debug, Clone)]
pub(crate) struct ExportRequest {
    /// A number or a path in the books folder.
    pub target: String,
    /// `-o`.
    pub output: Option<PathBuf>,
    /// A temporary file even for a sent invoice.
    pub temp: bool,
    /// Today (`YYYY-MM-DD`): the date an earlier export is kept under when
    /// its own can't be read.
    pub today: String,
}

/// Export one invoice's PDF (see the module docs). `origin` is this
/// process's server (`http://127.0.0.1:5199`).
///
/// # Errors
/// [`PdfError`].
pub(crate) async fn export_invoice(
    books: Arc<OpenBooks>,
    gate: &MbrGate,
    native: &dyn Native,
    origin: &str,
    request: ExportRequest,
) -> Result<Exported, PdfError> {
    if !native.capabilities().pdf {
        return Err(PdfError::Refused(crate::pdf::NEEDS_GUI.to_string()));
    }
    let ExportRequest {
        target,
        output,
        temp,
        today,
    } = request;
    let planning = Arc::clone(&books);
    let (plan, pdf) = run_blocking(move || {
        let plan = {
            let session = planning.session();
            filing::plan(&session, &target, output.as_deref(), temp)?
        };
        let pdf = planning.pdf_config().map_err(PdfError::Failed)?;
        Ok((plan, pdf))
    })
    .await?;
    let link = gate.export_link(&plan.page).ok_or_else(|| {
        PdfError::Failed("MBR's pages are off in this process, so there is nothing to print".into())
    })?;
    let (reply, printed) = oneshot::channel();
    native.submit(NativeRequest::Print(
        PrintJob {
            url: format!("{}{link}", origin.trim_end_matches('/')),
            output: plan.staging.clone(),
            pdf,
            ready_on_load: plan.ready_on_load,
        },
        reply,
    ));
    let printed = answer(printed)
        .await
        .map_err(|e| PdfError::Failed(format!("printing invoice {}: {e}", plan.number)))?;
    run_blocking(move || {
        let mut session = books.session();
        filing::complete(&mut session, &plan, printed.bytes, printed.warnings, &today)
    })
    .await
}

async fn run_blocking<T: std::marker::Send + 'static>(
    f: impl FnOnce() -> Result<T, PdfError> + std::marker::Send + 'static,
) -> Result<T, PdfError> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(PdfError::Failed(format!("the export failed: {e}"))))
}

impl From<PdfError> for ApiError {
    fn from(error: PdfError) -> Self {
        match error {
            PdfError::NotFound(m) => Self(StatusCode::NOT_FOUND, m),
            PdfError::Refused(m) => Self(StatusCode::UNPROCESSABLE_ENTITY, m),
            PdfError::Failed(m) => Self(StatusCode::BAD_GATEWAY, m),
        }
    }
}

pub(crate) fn books_error(error: &BooksError) -> ApiError {
    let status = match error.code() {
        "not-found" => StatusCode::NOT_FOUND,
        "external-change" | "stale" => StatusCode::CONFLICT,
        "storage" | "books" | "markdown-not-written" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    };
    ApiError(status, error.to_string())
}

/// The process's native side, or `503` with why not.
pub(crate) fn native_for(
    state: &AppState,
    wants: fn(&Capabilities) -> bool,
    what: &str,
) -> Result<Arc<dyn Native>, ApiError> {
    state
        .native()
        .filter(|n| wants(&n.capabilities()))
        .ok_or_else(|| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("{what} isn't available here: it needs the desktop app"),
            )
        })
}

/// This server's origin, as the request reached it (the Host guard has
/// already accepted the header).
fn origin(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|h| !h.is_empty() && !h.contains(['/', '@', ' ']))
        .map(|host| format!("http://{host}"))
        .ok_or_else(|| ApiError::bad_request("the request has no usable Host"))
}

/// The body of `POST /api/invoices/{num}/pdf` (optional).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportBody {
    #[serde(default)]
    temp: bool,
    today: Option<String>,
}

/// `POST /api/invoices/{num}/pdf`.
pub(crate) async fn export(
    State(state): State<AppState>,
    Extension(gate): Extension<Arc<MbrGate>>,
    headers: HeaderMap,
    Path(number): Path<String>,
    body: Result<Json<ExportBody>, JsonRejection>,
) -> ApiResult {
    // An empty body is the defaults.
    let body = match body {
        Ok(Json(body)) => body,
        Err(JsonRejection::MissingJsonContentType(_)) => ExportBody::default(),
        Err(e) => return Err(ApiError::bad_request(e.body_text())),
    };
    let native = native_for(&state, |c| c.pdf, "PDF export")?;
    let books = state.books().ok_or_else(no_books)?;
    let origin = origin(&headers)?;
    let mut exported = export_invoice(
        books,
        &gate,
        native.as_ref(),
        &origin,
        ExportRequest {
            target: number,
            output: None,
            temp: body.temp,
            today: today(body.today.as_deref())?,
        },
    )
    .await?;
    // The API names files by the books folder, never by full path.
    exported.path = None;
    Ok(ledgeline_shell::security::no_store_json(to_value(
        &exported,
    )?))
}

/// A kept PDF of the invoice, newest first.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct KeptPdf {
    pub file: String,
    /// In the books folder.
    pub kept_at: Option<VaultPath>,
    /// The day it was written.
    pub date: Option<String>,
    pub bytes: u64,
    /// The current one (`<name>.pdf`), not an earlier one kept under its date.
    pub latest: bool,
}

/// The customer's portal, as the Send step shows it.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PortalView {
    pub name: Option<String>,
    /// Only a plain `https:` URL is offered.
    pub url: Option<String>,
    pub login: Option<String>,
    pub supplier_id: Option<String>,
    pub requires_reference: bool,
    pub instructions: Option<String>,
}

/// The values the Send step offers to copy.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CopyFields {
    pub number: String,
    pub reference: Option<String>,
    pub total: Option<String>,
}

/// `GET /api/invoices/{num}/pdf`: everything the Send and PDF buttons need.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Delivery {
    pub number: String,
    pub document: VaultPath,
    pub status: Option<&'static str>,
    pub sent_date: Option<String>,
    pub sent_via: Option<String>,
    /// Where Export puts the PDF now: `kept` (sent) or `temp`.
    pub destination: Destination,
    /// The PDF's file name.
    pub file: String,
    /// `[invoicer] pdf-dir`.
    pub pdf_dir: String,
    pub sent_hash: Option<String>,
    /// Whether the invoice changed since its PDF was last kept (`None`:
    /// never kept): the "differs from what was sent" warning.
    pub changed_since_sent: Option<bool>,
    pub exports: Vec<KeptPdf>,
    pub portal: Option<PortalView>,
    pub copy: CopyFields,
    pub capabilities: Capabilities,
}

fn plain_https(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let plain = !host.is_empty()
        && !host.contains(['@', ':'])
        && !url.chars().any(|c| c.is_whitespace() || c.is_control());
    plain.then(|| url.to_string())
}

fn delivery_of(books: &OpenBooks, number: &str, caps: Capabilities) -> Result<Delivery, ApiError> {
    let session = books.session();
    let doc = session
        .find(number)
        .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    let number = doc
        .number()
        .ok_or_else(|| {
            ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("{} has no `invoice_num`", doc.path),
            )
        })?
        .to_string();
    let source = session
        .vault()
        .read(&doc.path)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(|| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{} can't be read", doc.path),
            )
        })?;
    let invoice = Invoice::parse(source.as_str(), &session.settings().words);
    let header = invoice.header();
    let file = filing::pdf_name(&doc.path).map_err(ApiError::from)?;
    let pdf_dir = session.settings().pdf_dir.clone();
    let stem = file.trim_end_matches(".pdf").to_string();
    let local_dir = invoicer_core::settings::Settings::vault_path(&pdf_dir)
        .ok()
        .map(|p| session.local_path(&p));
    let mut exports: Vec<KeptPdf> = local_dir
        .as_ref()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !filing::is_export_of(&name, &stem) {
                return None;
            }
            let meta = entry.metadata().ok().filter(std::fs::Metadata::is_file)?;
            let date = meta.modified().ok().map(|m| {
                chrono::DateTime::<chrono::Local>::from(m)
                    .format("%Y-%m-%d")
                    .to_string()
            });
            Some(KeptPdf {
                latest: name == file,
                kept_at: session.vault().vault_path(&entry.path()).ok(),
                file: name,
                date,
                bytes: meta.len(),
            })
        })
        .collect();
    exports.sort_by(|a, b| b.latest.cmp(&a.latest).then_with(|| b.file.cmp(&a.file)));
    let invoice_books = session.invoice_books();
    let portal = invoice_books
        .document(session.index(), &number)
        .ok()
        .and_then(|document| invoice_books.customer(session.index(), &document).ok())
        .and_then(|customer| customer.billing_portal)
        .map(|p| PortalView {
            name: p.name,
            url: p.url.as_deref().and_then(plain_https),
            login: p.login,
            supplier_id: p.supplier_id,
            requires_reference: p.requires_reference,
            instructions: p.instructions,
        });
    let sent = header.status == Some(invoicer_core::doc::Status::Sent);
    Ok(Delivery {
        status: header.status.map(invoicer_core::doc::Status::as_str),
        sent_date: header.sent_date.clone(),
        sent_via: header.sent_via.clone(),
        destination: if sent {
            Destination::Kept
        } else {
            Destination::Temp
        },
        file,
        pdf_dir,
        sent_hash: sent::recorded(&invoice).map(str::to_string),
        changed_since_sent: sent::changed_since_sent(&invoice),
        exports,
        portal,
        copy: CopyFields {
            number: number.clone(),
            reference: header.reference.clone(),
            total: doc.summary.as_ref().map(|s| s.total_text.clone()),
        },
        capabilities: caps,
        document: doc.path.clone(),
        number,
    })
}

/// `GET /api/invoices/{num}/pdf`.
pub(crate) async fn delivery(
    State(state): State<AppState>,
    Path(number): Path<String>,
) -> ApiResult {
    let caps = state.native().map(|n| n.capabilities()).unwrap_or_default();
    let books = state.books().ok_or_else(no_books)?;
    blocking(move || to_value(delivery_of(&books, &number, caps)?)).await
}

/// The body of `POST /api/invoices/{num}/send`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendBody {
    /// `sent_via`: `email`, `portal`, `mail`, …
    via: Option<String>,
    today: Option<String>,
}

/// `POST /api/invoices/{num}/send`: what `send NUM --via HOW` does.
pub(crate) async fn send(
    State(state): State<AppState>,
    Path(number): Path<String>,
    body: Result<Json<SendBody>, JsonRejection>,
) -> ApiResult {
    let body = match body {
        Ok(Json(body)) => body,
        Err(JsonRejection::MissingJsonContentType(_)) => SendBody::default(),
        Err(e) => return Err(ApiError::bad_request(e.body_text())),
    };
    let via = body
        .via
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if via
        .as_deref()
        .is_some_and(|v| v.len() > 64 || v.contains(['\n', '\r']))
    {
        return Err(ApiError::bad_request(
            "via: one short line (email, portal, mail…)",
        ));
    }
    let today = today(body.today.as_deref())?;
    let books = state.books().ok_or_else(no_books)?;
    blocking(move || {
        let mut session = books.session();
        let doc_number = session
            .find(&number)
            .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?
            .number()
            .map(str::to_string)
            .ok_or_else(|| {
                ApiError(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "it has no `invoice_num`".to_string(),
                )
            })?;
        let invoice_books = session.invoice_books();
        let request = Send {
            number: doc_number,
            today,
            sent_via: via,
            expected: None,
        };
        let (index, _) = session.parts_mut();
        let (result, _) = books::retry_once(|| invoice_books.send(index, &request));
        // Booked (or not): read the journal as it now is, before answering.
        super::books::reload_after_write(&mut session);
        to_value(result.map_err(|e| books_error(&e))?)
    })
    .await
}

/// `POST /api/invoices/{num}/portal`: `{url, opened}`. The window opens the
/// customer's portal in the browser; elsewhere the SPA opens `url` itself.
/// Only the portal the customer note names is ever opened.
pub(crate) async fn open_portal(
    State(state): State<AppState>,
    Path(number): Path<String>,
) -> ApiResult {
    let books = state.books().ok_or_else(no_books)?;
    let delivery =
        tokio::task::spawn_blocking(move || delivery_of(&books, &number, Capabilities::default()))
            .await
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    let url = delivery
        .portal
        .and_then(|portal| portal.url)
        .ok_or_else(|| {
            ApiError(
                StatusCode::NOT_FOUND,
                "the customer has no billing portal with an https URL".to_string(),
            )
        })?;
    let opened = match state.native().filter(|n| n.capabilities().open_url) {
        Some(native) => {
            native
                .open_url(&url)
                .map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, e))?;
            true
        }
        None => false,
    };
    Ok(ledgeline_shell::security::no_store_json(
        serde_json::json!({ "url": url, "opened": opened }),
    ))
}

/// The body of Share… and Copy file: a PDF this app exported.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileBody {
    destination: Destination,
    file: String,
    anchor: Option<Anchor>,
}

/// The exported PDF a [`FileBody`] names, on disk: a plain `.pdf` name in
/// `pdf-dir` or the temporary folder, and nothing else.
fn exported_file(books: &OpenBooks, body: &FileBody) -> Result<PathBuf, ApiError> {
    let name = body.file.as_str();
    let plain = name.ends_with(".pdf")
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && name.len() <= 255;
    if !plain {
        return Err(ApiError::bad_request(format!(
            "{name:?} isn't an exported PDF's name"
        )));
    }
    let dir = match body.destination {
        Destination::Kept => {
            let session = books.session();
            let dir = invoicer_core::settings::Settings::vault_path(&session.settings().pdf_dir)
                .map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
            session.local_path(&dir)
        }
        Destination::Temp => filing::temp_dir().map_err(ApiError::from)?,
        Destination::Copy => return Err(ApiError::bad_request("destination: kept or temp")),
    };
    let path = dir.join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("no exported PDF is called {name:?}"),
        ))
    }
}

async fn file_request(
    state: AppState,
    body: Result<Json<FileBody>, JsonRejection>,
    wants: fn(&Capabilities) -> bool,
    what: &str,
    request: fn(PathBuf, Option<Anchor>, super::native::Reply<()>) -> NativeRequest,
) -> ApiResult {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let native = native_for(&state, wants, what)?;
    let books = state.books().ok_or_else(no_books)?;
    let anchor = body.anchor;
    let file = tokio::task::spawn_blocking(move || exported_file(&books, &body))
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    let (reply, done) = oneshot::channel();
    native.submit(request(file, anchor, reply));
    answer(done)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(ledgeline_shell::security::no_store_json(
        serde_json::json!({ "done": true }),
    ))
}

/// `POST /api/pdf/share`: Share… (macOS, the window).
pub(crate) async fn share(
    State(state): State<AppState>,
    body: Result<Json<FileBody>, JsonRejection>,
) -> ApiResult {
    file_request(
        state,
        body,
        |c| c.share,
        "Share…",
        |file, anchor, reply| NativeRequest::Share {
            file,
            anchor,
            reply,
        },
    )
    .await
}

/// `POST /api/pdf/copy`: Copy file (macOS, the window).
pub(crate) async fn copy_file(
    State(state): State<AppState>,
    body: Result<Json<FileBody>, JsonRejection>,
) -> ApiResult {
    file_request(
        state,
        body,
        |c| c.copy_file,
        "Copy file",
        |file, _, reply| NativeRequest::CopyFile { file, reply },
    )
    .await
}

/// `POST /api/pdf/open`: open an exported PDF in the default viewer (the
/// window, macOS). Only the `{destination, file}` handles Share… and Copy
/// file take: a plain `.pdf` name in `pdf-dir` or the temporary folder,
/// never a path.
pub(crate) async fn open_pdf(
    State(state): State<AppState>,
    body: Result<Json<FileBody>, JsonRejection>,
) -> ApiResult {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let native = native_for(&state, |c| c.open_file, "Opening a PDF")?;
    let books = state.books().ok_or_else(no_books)?;
    let file = tokio::task::spawn_blocking(move || exported_file(&books, &body))
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    native
        .open_file(&file)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(ledgeline_shell::security::no_store_json(
        serde_json::json!({ "done": true }),
    ))
}

/// The CLI's `pdf` in a desktop build: the books and MBR on a private
/// server (`127.0.0.1`, an ephemeral port, the token never printed), a tao
/// event loop on this (the main) thread with no Dock icon, one export, and
/// out. Needs the main thread: call it from `main`.
///
/// # Errors
/// A message for `main`.
#[cfg(feature = "gui")]
pub(crate) fn run_cli(
    journal: &std::path::Path,
    request: ExportRequest,
) -> Result<Exported, String> {
    use ::mbr::pdf_export;
    use ledgeline_shell::security::{AccessToken, Security};
    use tao::event::Event;
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::platform::run_return::EventLoopExtRunReturn as _;

    use super::desktop::{self, DesktopHost, LoopNative, NativeEvent};

    enum CliEvent {
        Native(NativeEvent),
        Done(Result<Exported, String>),
    }

    // No backend or no display is a clear refusal before anything starts.
    pdf_export::preflight().map_err(|e| desktop::report(&e))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("building the async runtime: {e}"))?;
    let books = {
        let _entered = runtime.enter();
        Arc::new(OpenBooks::open(journal, false)?)
    };
    let token = AccessToken::generate().map_err(|e| e.to_string())?;
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0)))
        .map_err(|e| format!("binding the export's server: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let gate = Arc::new(MbrGate::new(token.clone(), port).map_err(|e| e.to_string())?);

    let mut event_loop = EventLoopBuilder::<CliEvent>::with_user_event().build();
    pdf_export::configure_cli_event_loop(&mut event_loop);
    let native = Arc::new(LoopNative::new(
        event_loop.create_proxy(),
        CliEvent::Native,
        desktop::capabilities(false),
    ));
    let mut host = DesktopHost::new(desktop::export_sink(
        event_loop.create_proxy(),
        CliEvent::Native,
    ));

    let state = AppState::with_native(
        Some(Arc::clone(&books)),
        ledgeline_shell::AppDirs::from_env(),
        Some(Arc::clone(&native) as Arc<dyn Native>),
    );
    let router = super::router_with_gate(state, Security::local(token, port), Arc::clone(&gate));
    runtime.spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let done = event_loop.create_proxy();
    let origin = format!("http://127.0.0.1:{port}");
    runtime.spawn(async move {
        let result = export_invoice(books, &gate, native.as_ref(), &origin, request)
            .await
            .map_err(|e| e.to_string());
        let _ = done.send_event(CliEvent::Done(result));
    });

    let mut outcome = None;
    event_loop.run_return(|event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(CliEvent::Native(event)) => host.handle(target, event, None),
            Event::UserEvent(CliEvent::Done(result)) => {
                outcome = Some(result);
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
    drop(host);
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    outcome.unwrap_or_else(|| Err("the export ended before it finished".to_string()))
}

#[cfg(test)]
mod tests {
    use super::plain_https;

    #[test]
    fn only_plain_https_portals_are_offered() {
        assert_eq!(
            plain_https(" https://customerx.coupahost.com/ ").as_deref(),
            Some("https://customerx.coupahost.com/")
        );
        for refused in [
            "http://customerx.coupahost.com/",
            "https://user@evil.example/",
            "https://host:8443/",
            "javascript:alert(1)",
            "https:///nohost",
            "https://a b/",
        ] {
            assert_eq!(plain_https(refused), None, "{refused}");
        }
    }
}
