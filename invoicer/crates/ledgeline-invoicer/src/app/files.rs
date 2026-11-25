//! Files in the books folder, for the customer page's contracts (Phase 4
//! feedback J5, J6): what to pick from, the system's open dialog, and
//! opening a contract in its viewer.
//!
//! | Route                   | Does                                                          |
//! |-------------------------|---------------------------------------------------------------|
//! | `GET /api/files[?list=false]` | `{pick, open, files}`: the notes and documents a contract may name (`ContractCandidate`), and whether this process can show the open dialog (`pick`) and open a file (`open`). |
//! | `POST /api/files/pick`  | The system's open dialog at the books folder (the window, macOS): `{cancelled: true}`, or `{cancelled: false, file: ContractCandidate}`; `422` for a file outside the books folder. |
//! | `POST /api/files/open`  | `{path}`: open a document in the books folder in its default viewer, or show any other file in Finder (the window, macOS). |
//!
//! A contract is stored relative to the books folder (`/customers/contracts/SOW.pdf`)
//! or as a `[[Title]]` link for a note, never as a path outside it: the
//! books folder is what moves between machines.

use std::path::{Path as FsPath, PathBuf};

use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use invoicer_core::customer_edit::{self, ContractCandidate};
use ledgeline_core::vault::VaultPath;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use super::api::{ApiError, ApiResult, blocking, no_books, to_value, with_books};
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
    with_books(&state, move |books| {
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
    books.vault().root().canonicalize().map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the books folder: {e}"),
        )
    })
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
    let outside = || {
        format!(
            "{shown} isn't in the books folder. Contracts are kept with the books: \
             move or copy it into the books folder (customers/contracts/, say), then choose it again."
        )
    };
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
    let books = state.books().ok_or_else(no_books)?;
    blocking(move || {
        let root = books_root(&books)?;
        let Some(chosen) = native
            .pick_file(&root, "Choose a contract in the books folder")
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?
        else {
            return Ok(json!({ "cancelled": true }));
        };
        let path =
            in_books(&root, &chosen).map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, e))?;
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
    let books = state.books().ok_or_else(no_books)?;
    blocking(move || {
        let path = VaultPath::new(body.path.trim().trim_start_matches('/'))
            .map_err(|e| ApiError::bad_request(format!("{}: {e}", body.path)))?;
        if !books.vault().is_file(&path) {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                format!("{path} isn't a file in the books folder"),
            ));
        }
        let root = books_root(&books)?;
        let file = root.join(path.as_str());
        // A link out of the folder is refused like a path out of it.
        in_books(&root, &file).map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, e))?;
        // SEC-5: only a document opens; anything else (`.command`,
        // `.terminal`, `.app`, a script) is shown in Finder, never run.
        let revealed = !is_document(&file);
        if revealed {
            native.reveal_file(&file)
        } else {
            native.open_file(&file)
        }
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok(json!({ "done": true, "revealed": revealed }))
    })
    .await
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
