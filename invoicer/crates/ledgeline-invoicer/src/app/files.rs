//! Files for the customer page's contracts and PO attachments (Phase 4
//! feedback J5, J6): what to pick from, the system's open dialog, and
//! opening one in its viewer.
//!
//! | Route                   | Does                                                          |
//! |-------------------------|---------------------------------------------------------------|
//! | `GET /api/files[?list=false]` | `{pick, open, files}`: the notes and documents a contract may name (`ContractCandidate`), and whether this process can show the open dialog (`pick`) and open a file (`open`). |
//! | `POST /api/files/pick`  | The system's open dialog at the books folder (the window, macOS): `{cancelled: true}`, or `{cancelled: false, file: ContractCandidate}`; a file outside the books folder answers `{path, name, kind: null, entry, outside: true}`, `path` being where it is (`~/Documents/MSA.pdf`). `422` for a file in a hidden folder of the books. |
//! | `POST /api/files/open`  | `{path}`: a path in the books folder, or an outside entry as written (`~/…`, `file:///…`). A document opens in its default viewer; any other file is shown in Finder (the window, macOS). `ledgeline.toml` works even when it keeps the books from opening (the Settings page's "Open the settings file"). |
//!
//! An entry is written by `invoicer_core::attachment`: relative to the books
//! folder (`/customers/contracts/SOW.pdf`), a `[[Title]]` link for a note,
//! or, for a file outside the books folder, `~/…` under the home folder,
//! else a `file:///` URL.
//!
//! Outside files are opened only when some customer note names them
//! (a contract or a PO attachment), compared after resolving links, so the
//! API can't be used to open any file on the machine.

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use invoicer_core::attachment::{self, Attachment, OutsidePath};
use invoicer_core::customer_edit::{self, ContractCandidate};
use ledgeline_core::ledger_file;
use ledgeline_core::vault::{Vault, VaultPath};
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use super::api::{ApiError, ApiResult, blocking, to_value, with_books};
use super::books::OpenBooks;
use super::pdf::native_for;

/// The file routes (state still to be supplied).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/files", get(list))
        .route("/api/files/pick", post(pick))
        .route("/api/files/open", post(open))
}

/// `GET /api/files`'s query.
#[derive(Debug, Deserialize)]
struct ListParams {
    /// `false`: only what this process can do (`pick`, `open`), no walk.
    #[serde(default = "yes")]
    list: bool,
}

const fn yes() -> bool {
    true
}

/// `GET /api/files` (`?list=false`: the capabilities alone).
async fn list(State(state): State<AppState>, Query(params): Query<ListParams>) -> ApiResult {
    let caps = state.native().map(|n| n.capabilities()).unwrap_or_default();
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let files = if params.list {
            customer_edit::contract_candidates(session.index(), session.vault())
        } else {
            Vec::new()
        };
        Ok(json!({
            "pick": caps.pick_file,
            "open": caps.open_file,
            "files": to_value(files)?,
        }))
    })
    .await
}

/// The books folder, canonical.
fn books_root(books: &OpenBooks) -> Result<PathBuf, ApiError> {
    root_of(&**books.vault())
}

/// A vault's folder, canonical.
fn root_of(vault: &dyn Vault) -> Result<PathBuf, ApiError> {
    vault.root().canonicalize().map_err(|e| {
        ApiError::status(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the books folder: {e}"),
        )
    })
}

/// Where a chosen file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Chosen {
    /// In the books folder.
    Books(VaultPath),
    /// Outside it: the file, links resolved.
    Outside(PathBuf),
}

/// Where the chosen `file` (absolute) is, links resolved: in the books
/// folder, or outside it. A file in a hidden folder of the books is refused
/// (the books don't read those); one outside them is fine.
pub(crate) fn locate(root: &FsPath, file: &FsPath) -> Result<Chosen, String> {
    match in_books(root, file) {
        Ok(path) => Ok(Chosen::Books(path)),
        Err(e) => {
            let canonical = file.canonicalize().map_err(|_| e.clone())?;
            if canonical.starts_with(root) {
                Err(e)
            } else {
                Ok(Chosen::Outside(canonical))
            }
        }
    }
}

/// `file` (absolute) as a path in the books folder, or why it can't be one.
pub(crate) fn in_books(root: &FsPath, file: &FsPath) -> Result<VaultPath, String> {
    let shown = file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let canonical = file
        .canonicalize()
        .map_err(|e| format!("{shown} can't be read: {e}"))?;
    let outside = || format!("{shown} isn't in the books folder");
    let relative = canonical.strip_prefix(root).map_err(|_| outside())?;
    let text = relative
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| format!("{shown}: the path isn't text"))?
        .join("/");
    if text.split('/').any(|s| s.starts_with('.')) {
        return Err(format!(
            "{shown} is in a hidden folder, which the books don't read"
        ));
    }
    VaultPath::new(&text).map_err(|e| format!("{shown}: {e}"))
}

/// `POST /api/files/pick`.
async fn pick(State(state): State<AppState>) -> ApiResult {
    let native = native_for(&state, |c| c.pick_file, "Choosing a file")?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let root = books_root(&books)?;
        let Some(chosen) = native
            .pick_file(&root, "Choose a file to attach")
            .map_err(|e| ApiError::status(StatusCode::INTERNAL_SERVER_ERROR, e))?
        else {
            return Ok(json!({ "cancelled": true }));
        };
        let path = match locate(&root, &chosen)
            .map_err(|e| ApiError::status(StatusCode::UNPROCESSABLE_ENTITY, e))?
        {
            Chosen::Books(path) => path,
            Chosen::Outside(file) => {
                let home = home();
                let entry = Attachment::outside(&file, home.as_deref());
                let Attachment::Outside(outside) = &entry else {
                    return Err(ApiError::internal("an outside file read as another kind"));
                };
                return Ok(json!({
                    "cancelled": false,
                    "file": {
                        "path": outside.location(),
                        "name": entry.name(),
                        "kind": null,
                        "entry": entry.entry(),
                        "outside": true,
                    },
                }));
            }
        };
        let session = books.session();
        let index = session.index();
        let note = index.site().note(&path);
        let file = ContractCandidate {
            name: note.map_or_else(
                || path.file_name().unwrap_or_default().to_string(),
                |n| n.title.clone(),
            ),
            kind: note.and_then(|n| n.note_type.clone()),
            entry: customer_edit::contract_entry(index, &path),
            path,
        };
        Ok(json!({ "cancelled": false, "file": to_value(file)? }))
    })
    .await
}

/// `POST /api/files/open`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenBody {
    /// In the books folder (`customers/contracts/SOW.pdf`; a leading `/`
    /// is allowed).
    path: String,
}

/// `POST /api/files/open`: a document in the books folder, in its viewer;
/// any other file shown in Finder (`revealed`).
async fn open(
    State(state): State<AppState>,
    body: Result<Json<OpenBody>, JsonRejection>,
) -> ApiResult {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let native = native_for(&state, |c| c.open_file, "Opening a file")?;
    if let Ok(Attachment::Outside(outside)) = Attachment::parse(&body.path) {
        let books = state.open_books()?;
        return blocking(StatusCode::OK, move || {
            let named = {
                let session = books.session();
                let index = session.index();
                invoicer_core::customers::outside_attachments(index.site(), &index.settings().words)
            };
            let file = named_outside_file(&outside, &named, home().as_deref())?;
            show(&*native, &file)
        })
        .await;
    }
    let path = VaultPath::new(body.path.trim().trim_start_matches('/'))
        .map_err(|e| ApiError::bad_request(format!("{}: {e}", body.path)))?;
    // The books' settings file opens even while it keeps them from opening
    // (an invalid `[invoicer]`): the Settings page's way to fix it.
    let vault: Arc<dyn Vault> = match state.books() {
        Some(books) => Arc::clone(books.vault()),
        None if path.as_str() == ledger_file::LEDGER_FILE => super::settings::place(&state)?.vault,
        None => {
            return Err(state
                .open_books()
                .err()
                .unwrap_or_else(|| ApiError::internal("no books")));
        }
    };
    blocking(StatusCode::OK, move || {
        if !vault.is_file(&path) {
            return Err(ApiError::status(
                StatusCode::NOT_FOUND,
                format!("{path} isn't a file in the books folder"),
            ));
        }
        let root = root_of(&*vault)?;
        let file = root.join(path.as_str());
        // A link out of the folder is refused like a path out of it.
        in_books(&root, &file)
            .map_err(|e| ApiError::status(StatusCode::UNPROCESSABLE_ENTITY, e))?;
        show(&*native, &file)
    })
    .await
}

/// The user's home folder, links resolved, where `~/` entries start.
fn home() -> Option<PathBuf> {
    attachment::home_dir().map(|h| h.canonicalize().unwrap_or(h))
}

/// The file `outside` names, links resolved, when some customer names the
/// same file (`named`: every outside contract and PO attachment); `~/`
/// starts at `home`. Anything else is refused: the API opens only what the
/// notes name.
pub(crate) fn named_outside_file(
    outside: &OutsidePath,
    named: &[OutsidePath],
    home: Option<&FsPath>,
) -> Result<PathBuf, ApiError> {
    let shown = outside.location();
    let resolved = |o: &OutsidePath| o.expand(home).and_then(|p| p.canonicalize().ok());
    let file = resolved(outside).filter(|p| p.is_file()).ok_or_else(|| {
        ApiError::status(StatusCode::NOT_FOUND, format!("{shown} can't be found"))
    })?;
    if named.iter().filter_map(resolved).any(|n| n == file) {
        Ok(file)
    } else {
        Err(ApiError::status(
            StatusCode::FORBIDDEN,
            format!("{shown} isn't attached to a customer"),
        ))
    }
}

/// Show `file` (absolute): a document in its viewer; anything else in
/// Finder. SEC-5: only a document opens, judged by the file a link leads
/// to; anything else (`.command`, `.terminal`, `.app`, a script) is shown
/// in Finder, never run.
fn show(native: &dyn super::native::Native, file: &FsPath) -> Result<serde_json::Value, ApiError> {
    let target = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let revealed = !is_document(&target);
    if revealed {
        native.reveal_file(file)
    } else {
        native.open_file(file)
    }
    .map_err(|e| ApiError::status(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(json!({ "done": true, "revealed": revealed }))
}

/// Extensions `POST /api/files/open` opens in their viewer: documents,
/// images and office or iWork files. Not `.html` or `.svg`, which a
/// browser would run scripts in.
const DOCUMENT_EXTENSIONS: [&str; 31] = [
    "pdf", "txt", "md", "markdown", "csv", "tsv", "rtf", "png", "jpg", "jpeg", "gif", "heic",
    "heif", "tif", "tiff", "webp", "bmp", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt",
    "ods", "odp", "pages", "numbers", "key", "eml", "json",
];

/// Whether `file` is a document by its extension (any case).
fn is_document(file: &std::path::Path) -> bool {
    file.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        DOCUMENT_EXTENSIONS
            .iter()
            .any(|d| d.eq_ignore_ascii_case(e))
    })
}
