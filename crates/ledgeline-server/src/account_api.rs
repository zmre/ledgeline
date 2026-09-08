//! The HTTP surface for the **account list editor** — `GET /api/accounts`,
//! `PUT /api/accounts/{*journalId}`, and `POST /api/accounts/file`.
//!
//! An `account` declaration is the chart-of-accounts entry this module
//! presents and edits: its `type:`/`bsgroup:`/`holdings:`/`valuation:`/any
//! other tags, and a free-text note. See [`ledgeline_core::accounts`] for why
//! the document model needs no lock the way [`ledgeline_core::aliases`] does.
//!
//! # Why this is its own route family
//!
//! Same reasoning as `alias_api`'s: an `account` declaration lives IN the
//! journal itself, so there is no scan and no discovery set — the only files
//! this module opens are ones the parser already read, named by the same
//! `journalId` handles `alias_api`/`budget_api` use.
//!
//! # The write path
//!
//! Identical five layers to `alias_api`'s: the handle resolves by exact
//! membership in [`journals::targets`], never by path arithmetic; the
//! revision is a [`Fingerprint`] over the file's raw bytes, checked on read
//! and again immediately before the write; the rewrite is
//! [`AccountDoc::apply`], a span splice; [`AccountDoc::verify`] must agree,
//! and then the whole journal is re-parsed with the edited text in memory
//! ([`parse::parse_journal_with_overrides`]); one [`atomic_write`], the last
//! statement that can have an effect.
//!
//! # Creating `accounts.journal`
//!
//! `POST /api/accounts/file` is `budget_api`'s `create_file` for this
//! construct: the new file is written FIRST, the `include` line SECOND (an
//! `include` naming a file that is not there is a journal that does not
//! parse, so a failed second write leaves only an unreferenced file, never a
//! broken journal), and both are proved by a whole-journal re-parse before
//! either lands.
//!
//! # No absolute path is ever echoed
//!
//! Same rule as everywhere else in this crate. Errors quote the caller's own
//! `journalId`; a whole-journal parse failure is reported without hledger's or
//! our own diagnostic text, because `ParseError::Located` names the file it
//! was reading and that name is not the caller's to have.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::accounts::{AccountDoc, AccountEdit, AccountError, AccountLine, AccountPlan};
use ledgeline_core::edit::{Fingerprint, atomic_write};
use ledgeline_core::journals;
use ledgeline_core::model::Journal;
use ledgeline_core::parse;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};

use crate::AppState;
use crate::edit_api::json_body;
use crate::error::{AppError, editing_disabled};
use crate::reports_api::compute;

/// How many changes one `PUT` may name. A bound on the request, not on the
/// user: the same number `alias_api`/`rules_api` use.
const MAX_EDITS: usize = 200;

/// Longest accepted `journalId`, in bytes, and how many components it may
/// have. The same numbers every other API module in this crate uses.
const MAX_ID_BYTES: usize = 1024;
/// See [`MAX_ID_BYTES`].
const MAX_ID_COMPONENTS: usize = 9;

/// The file a first account declaration is offered a home in, when nothing
/// declares one yet. A NAME, chosen by us, for a file that does not exist —
/// the one situation where `journals.rs`'s "no filename is ever inspected"
/// rule does not apply, per `budget_api`'s identical `BUDGET_FILE`.
const ACCOUNTS_FILE: &str = "accounts.journal";

/// The contents a new `accounts.journal` is created with. A comment and
/// nothing else — the same reasoning `budget_api::BUDGET_FILE_HEADER` gives:
/// the file's whole job is to be a place declarations go.
const ACCOUNTS_FILE_HEADER: &str = "\
; Chart of accounts.
;
; Each `account` line below declares one account and its tags (`type:`,
; `bsgroup:`, `holdings:`, `valuation:`, or any of your own). Ledgeline edits
; this file from the Settings \u{2192} Accounts tab.
";

// ===========================================================================
// Wire types
// ===========================================================================

/// `GET /api/accounts` — every account the open journal declares.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAccounts {
    /// `false` means no journal is bound to an editor, so the screen is
    /// read-only and says why.
    editable: bool,
    /// Whether `POST /api/accounts/file` would succeed. Drives the "create
    /// one" button, so the UI never offers a button that would `409`.
    can_create_file: bool,
    /// The name that button would create, so the UI can say it out loud.
    create_file_name: &'static str,
    /// The journal files that declare an account, plus the root journal —
    /// which is always offered, even with none, because it is where a first
    /// declaration goes.
    files: Vec<WireAccountFile>,
}

/// One journal file's `account` declarations.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAccountFile {
    /// The file's handle: its path relative to the include root, forward
    /// slashes. Never an absolute path.
    journal_id: String,
    /// The file's own name, for display.
    label: String,
    /// A fingerprint of the file's raw bytes. Echo it back in a `PUT` to
    /// prove the edit is against these bytes.
    revision: String,
    /// A regular file inside the include root; `false` means this file can be
    /// listed but not written.
    writable: bool,
    /// This file's account declarations, in file order.
    accounts: Vec<WireAccount>,
}

/// One `account` declaration.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAccount {
    /// The file this line is in.
    journal_id: String,
    /// 0-based position among that file's account lines — the handle a `PUT`
    /// names. A parse index, not a durable id; the revision is what makes
    /// that safe.
    index: usize,
    /// 1-based line number in that file.
    line: u32,
    /// The declared account name.
    name: String,
    /// Its tags, in the order the comment writes them.
    tags: Vec<WireTag>,
    /// The comment's free-text note, or empty.
    note: String,
}

/// One tag.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireTag {
    /// The tag name, e.g. `type`.
    name: String,
    /// The tag value, e.g. `A`. May be empty (a marker tag).
    value: String,
}

/// `PUT /api/accounts/{*journalId}`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSaveAccounts {
    /// The revision this edit was planned against.
    revision: String,
    /// The changes. Unlike a rules-file save, omission is not itself a
    /// delete — removing a declaration takes an explicit `delete` edit
    /// naming the line, so a client that sends one edit changes one line.
    edits: Vec<WireAccountEdit>,
}

/// One change. `kind` is the tag, so an unknown one is a `400` rather than a
/// silently different edit.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub(crate) enum WireAccountEdit {
    /// Rewrite one line's tags and note.
    Replace {
        /// Which line, by [`WireAccount::index`].
        index: usize,
        /// The new tags.
        tags: Vec<WireTag>,
        /// The new note.
        note: String,
    },
    /// Declare a previously-undeclared account.
    Declare {
        /// The account name to declare.
        name: String,
        /// Its initial tags.
        tags: Vec<WireTag>,
        /// Its initial note.
        note: String,
    },
    /// Remove one declaration. Only the chart-of-accounts entry — see
    /// [`ledgeline_core::accounts::AccountEdit::Delete`].
    Delete {
        /// Which line, by [`WireAccount::index`].
        index: usize,
    },
}

/// `POST /api/accounts/file` — what was created.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireCreatedFile {
    /// The new file's handle, ready to be used as a `PUT` target.
    journal_id: String,
    /// The file's own name, for display.
    label: String,
    /// The `include` line that was appended to the main journal, verbatim.
    included_as: String,
    /// The main journal's own handle, so the client can show where the
    /// `include` landed.
    main_journal_id: String,
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `Cache-Control: no-store`, no `ETag` — same posture as every other edit
/// route in this crate: none of this is derived from the journal snapshot's
/// generation counter.
fn no_store<T: Serialize>(body: T) -> Response {
    const NO_STORE: (HeaderName, HeaderValue) =
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    ([NO_STORE], Json(body)).into_response()
}

/// `GET /api/accounts` — every account the open journal declares.
pub(crate) async fn index(State(state): State<AppState>) -> Result<Response, AppError> {
    let editable = state.editing_enabled();
    let Json(body) = compute(move || account_lines(&state, editable)).await?;
    Ok(no_store(body))
}

/// `PUT /api/accounts/{*journalId}` — rewrite one file's account lines.
pub(crate) async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<WireSaveAccounts>, JsonRejection>,
) -> Result<Response, AppError> {
    // Shape first, before any filesystem call, so the route is decided on
    // syntax and is never an existence oracle.
    validate_journal_id(&id)?;
    let request = json_body(payload)?;
    if request.edits.len() > MAX_EDITS {
        return Err(AppError::BadRequest(format!(
            "a save may name at most {MAX_EDITS} changes; this one named {}",
            request.edits.len()
        )));
    }
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    // The journal write mutex, shared with imports, aliases and budget: any of
    // them can name the same file. Held across the `.await`, which is why it
    // is a tokio mutex.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || save_accounts(&state, &id, &request)).await?;
    Ok(no_store(body))
}

/// `POST /api/accounts/file` — create `accounts.journal` and `include` it.
pub(crate) async fn create_file(State(state): State<AppState>) -> Result<Response, AppError> {
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || create_accounts_file(&state)).await?;
    Ok(no_store(body))
}

// ===========================================================================
// Reading
// ===========================================================================

/// The whole of `GET`.
fn account_lines(state: &AppState, editable: bool) -> Result<WireAccounts, AppError> {
    let journal = state.snapshot().journal.clone();
    Ok(WireAccounts {
        editable,
        can_create_file: editable && can_create(&journal).is_ok(),
        create_file_name: ACCOUNTS_FILE,
        files: account_files(state)?,
    })
}

/// Every journal file worth listing, with its accounts.
///
/// "Worth listing" mirrors `budget_api::budget_lines`'s `listed`, not
/// `alias_api::alias_files`'s narrower "declaring, or root" rule — because
/// (like budget, and unlike aliases) this module has a create-file flow, and
/// a freshly created `accounts.journal` declares nothing yet. If it were
/// excluded until its first declaration there would be nowhere to add one:
///
/// - When some file already declares an account, list exactly those files —
///   "detect where they live", with no filename ever inspected.
/// - When none do, list every writable file the parse read that holds no
///   transactions (a pure directive file — a freshly created
///   `accounts.journal` is exactly this shape). This is what makes the file
///   `POST /api/accounts/file` just created immediately usable.
/// - When there is not even one of those, fall back to the root journal, so a
///   first declaration always has somewhere to go before any button is
///   pressed.
fn account_files(state: &AppState) -> Result<Vec<WireAccountFile>, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let targets = journals::targets(journal);

    let listed: Vec<&PathBuf> = if journal.accounts.is_empty() {
        let empty: Vec<&PathBuf> = journal
            .source_files
            .iter()
            .filter(|path| holds_no_transactions(&targets, journal, path))
            .collect();
        if empty.is_empty() {
            journal.source_files.first().into_iter().collect()
        } else {
            empty
        }
    } else {
        journal
            .source_files
            .iter()
            .filter(|path| {
                journal
                    .accounts
                    .iter()
                    .any(|decl| &&decl.source_file == path)
            })
            .collect()
    };

    Ok(listed
        .into_iter()
        .filter_map(|path| {
            let target = targets
                .iter()
                .find(|target| journal_path(journal, &target.id).as_ref() == Some(path))?;
            let text = std::fs::read_to_string(path).ok()?;
            let doc = AccountDoc::parse(&text);
            Some(WireAccountFile {
                accounts: doc
                    .lines()
                    .iter()
                    .map(|line| wire_account(&target.id, line))
                    .collect(),
                journal_id: target.id.clone(),
                label: target.label.clone(),
                revision: Fingerprint::of_bytes(text.as_bytes()).token(),
                writable: target.writable,
            })
        })
        .collect())
}

/// Whether `path` is a writable file the parse read that holds no
/// transactions. `journals::targets` already tallies both facts from
/// content, so this is a lookup rather than a second scan. Identical to
/// `budget_api::holds_no_transactions`.
fn holds_no_transactions(
    targets: &[journals::JournalTarget],
    journal: &Journal,
    path: &PathBuf,
) -> bool {
    targets.iter().any(|target| {
        target.txn_count == 0
            && target.writable
            && journal_path(journal, &target.id).as_ref() == Some(path)
    })
}

/// One account line as the wire carries it.
fn wire_account(journal_id: &str, line: &AccountLine) -> WireAccount {
    WireAccount {
        journal_id: journal_id.to_string(),
        index: line.index,
        line: line.line,
        name: line.name.clone(),
        tags: line
            .tags
            .iter()
            .map(|(name, value)| WireTag {
                name: name.clone(),
                value: value.clone(),
            })
            .collect(),
        note: line.note.clone(),
    }
}

// ===========================================================================
// Writing
// ===========================================================================

/// The whole of `PUT`, synchronously. Every `?` is a decision not to write.
fn save_accounts(
    state: &AppState,
    id: &str,
    request: &WireSaveAccounts,
) -> Result<WireAccountFile, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let target = journals::targets(journal)
        .into_iter()
        .find(|target| target.id == id)
        .ok_or_else(|| unresolved(id))?;
    if !target.writable {
        return Err(AppError::BadRequest(format!(
            "{} cannot be edited: an account can only be written to a regular file inside the \
             journal's own directory, not a symlink or a directory",
            quoted(id)
        )));
    }
    let path = journal_path(journal, id).ok_or_else(|| unresolved(id))?;

    let text = read_journal(&path, id)?;
    let fingerprint = Fingerprint::of_bytes(text.as_bytes());
    // Checked BEFORE any index is resolved, so a client editing an older parse
    // is told the file moved rather than "there is no account number 3".
    if fingerprint.token() != request.revision {
        return Err(stale(id));
    }

    let doc = AccountDoc::parse(&text);
    let plan = AccountPlan {
        edits: request.edits.iter().map(edit_from_wire).collect(),
    };
    let new_text = doc.apply(&plan)?;
    doc.verify(&plan, &new_text)?;

    // The engine proved the account lines are what was asked for and that
    // nothing else moved. This proves the result is still a journal — only
    // this crate knows which journal a file belongs to.
    if new_text != text {
        let overrides = HashMap::from([(path.clone(), new_text.clone())]);
        if parse::parse_journal_with_overrides(&journal.source_name, &overrides).is_err() {
            // Deliberately no detail: `ParseError::Located` names the file it
            // was reading, and that name is not the caller's to have.
            return Err(AppError::BadRequest(format!(
                "this change would make {} unreadable as part of your journal, so nothing was \
                 written",
                quoted(id)
            )));
        }
    }

    if new_text == text {
        // A no-op writes NOTHING. Writing byte-identical content still bumps
        // mtime, and a user's own watch loop would see a spurious change.
        return Ok(file_response(&target, &doc, fingerprint.token()));
    }

    // Narrow the TOCTOU window from "the whole request" to "hash → rename".
    let before_write = Fingerprint::of_bytes(read_journal(&path, id)?.as_bytes());
    if !before_write.content_matches(&fingerprint) {
        return Err(stale(id));
    }

    atomic_write(&path, new_text.as_bytes()).map_err(|error| {
        AppError::Internal(format!(
            "{} could not be written: {}. Nothing else was changed.",
            quoted(id),
            error.kind()
        ))
    })?;

    // The journal changed underneath the editor, so re-open it — otherwise
    // the next edit sees a stale fingerprint and reports a conflict the user
    // did not cause. Logged rather than returned: the write landed, and the
    // file watcher will retry.
    if let Some(Err(error)) = state.reopen_editor() {
        eprintln!("ledgeline: the journal could not be re-read after an account edit: {error}");
    }

    // The new revision comes from what we WROTE, never from a re-read — a
    // re-read could pick up somebody else's write and hand this client a
    // token for bytes it has never seen.
    let revision = Fingerprint::of_bytes(new_text.as_bytes()).token();
    Ok(file_response(
        &target,
        &AccountDoc::parse(&new_text),
        revision,
    ))
}

/// The file listing a save answers with. Note what it re-reads: nothing. The
/// document is the text we just wrote and the revision is its fingerprint, so
/// the client's next save is against bytes this response describes exactly.
fn file_response(
    target: &journals::JournalTarget,
    doc: &AccountDoc,
    revision: String,
) -> WireAccountFile {
    WireAccountFile {
        accounts: doc
            .lines()
            .iter()
            .map(|line| wire_account(&target.id, line))
            .collect(),
        journal_id: target.id.clone(),
        label: target.label.clone(),
        revision,
        writable: target.writable,
    }
}

/// One wire edit as the engine's.
fn edit_from_wire(edit: &WireAccountEdit) -> AccountEdit {
    let tags_of = |tags: &[WireTag]| {
        tags.iter()
            .map(|tag| (tag.name.clone(), tag.value.clone()))
            .collect()
    };
    match edit {
        WireAccountEdit::Replace { index, tags, note } => AccountEdit::Replace {
            index: *index,
            tags: tags_of(tags),
            note: note.clone(),
        },
        WireAccountEdit::Declare { name, tags, note } => AccountEdit::Declare {
            name: name.clone(),
            tags: tags_of(tags),
            note: note.clone(),
        },
        WireAccountEdit::Delete { index } => AccountEdit::Delete { index: *index },
    }
}

// ===========================================================================
// Creating an accounts file
// ===========================================================================

/// Whether `accounts.journal` may be created, and why not when it may not.
///
/// Two refusals, both about not surprising anyone, mirroring
/// `budget_api::can_create`'s:
///
/// - **The journal already declares an account.** Then it already has a home
///   for declarations, and adding a second one would split them across files
///   for no reason the user asked for.
/// - **Something already sits at `accounts.journal`.** Never overwritten,
///   never appended to, not even when it is empty.
fn can_create(journal: &Journal) -> Result<(PathBuf, PathBuf), AppError> {
    let main = journal
        .source_files
        .first()
        .ok_or_else(|| AppError::BadRequest("no journal is open".to_string()))?;
    let root = main.parent().ok_or_else(|| {
        AppError::Internal("the open journal has no containing directory".to_string())
    })?;
    if !journal.accounts.is_empty() {
        return Err(AppError::Conflict(
            "this journal already declares accounts, so a new accounts file was not created; add \
             your declaration to the file that holds them"
                .to_string(),
        ));
    }
    let accounts = root.join(ACCOUNTS_FILE);
    if accounts.symlink_metadata().is_ok() {
        return Err(AppError::Conflict(format!(
            "a file called {ACCOUNTS_FILE} already sits beside your journal, and Ledgeline will \
             not write over it. Include it from your main journal yourself, or move it aside."
        )));
    }
    Ok((main.clone(), accounts))
}

/// Create `accounts.journal` and `include` it from the main journal.
///
/// Same order, and the same safety argument, as `budget_api::create_budget_file`:
/// the new file is written FIRST, the `include` line SECOND (at EOF), and
/// both are proved by a whole-journal re-parse before either lands.
fn create_accounts_file(state: &AppState) -> Result<WireCreatedFile, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let (main, accounts) = can_create(journal)?;

    let main_text = std::fs::read_to_string(&main).map_err(|error| {
        AppError::Internal(format!("your journal could not be read: {}", error.kind()))
    })?;
    let newline = if main_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let include_line = format!("include {ACCOUNTS_FILE}");
    let lead = if main_text.is_empty() || main_text.ends_with('\n') {
        String::new()
    } else {
        newline.to_string()
    };
    let new_main = format!("{main_text}{lead}{include_line}{newline}");

    // Prove BOTH files before writing EITHER.
    let overrides = HashMap::from([
        (main.clone(), new_main.clone()),
        (accounts.clone(), ACCOUNTS_FILE_HEADER.to_string()),
    ]);
    if parse::parse_journal_with_overrides(&journal.source_name, &overrides).is_err() {
        return Err(AppError::BadRequest(
            "including an accounts file would make your journal unreadable, so nothing was \
             written"
                .to_string(),
        ));
    }

    atomic_write(&accounts, ACCOUNTS_FILE_HEADER.as_bytes()).map_err(|error| {
        AppError::Internal(format!(
            "{ACCOUNTS_FILE} could not be created: {}. Nothing else was changed.",
            error.kind()
        ))
    })?;
    atomic_write(&main, new_main.as_bytes()).map_err(|error| {
        AppError::Internal(format!(
            "{ACCOUNTS_FILE} was created, but your main journal could not be updated to include \
             it: {}. Add `{include_line}` to it yourself, or delete {ACCOUNTS_FILE}.",
            error.kind()
        ))
    })?;

    if let Some(Err(error)) = state.reopen_editor() {
        eprintln!(
            "ledgeline: the journal could not be re-read after creating an accounts file: {error}"
        );
    }

    let journal = state.snapshot().journal.clone();
    let targets = journals::targets(&journal);
    let main_journal_id = targets
        .iter()
        .find(|target| target.is_root)
        .map_or_else(String::new, |target| target.id.clone());
    Ok(WireCreatedFile {
        journal_id: ACCOUNTS_FILE.to_string(),
        label: ACCOUNTS_FILE.to_string(),
        included_as: include_line,
        main_journal_id,
    })
}

// ===========================================================================
// Handles, parsing and errors
// ===========================================================================

/// The path a `journalId` names, taken from the files the parse actually
/// read. Security layer 2, identical to `alias_api::journal_path`.
fn journal_path(journal: &Journal, id: &str) -> Option<PathBuf> {
    let root = journal.source_files.first()?.parent()?;
    let candidate = root.join(id);
    journal
        .source_files
        .iter()
        .find(|source| *source == &candidate)
        .cloned()
}

/// Layer 1: shape, before any filesystem call. Identical to
/// `alias_api::validate_journal_id`.
fn validate_journal_id(id: &str) -> Result<(), AppError> {
    let refuse = |why: &str| {
        Err(AppError::BadRequest(format!(
            "{} is not a journal id: {why}",
            quoted(id)
        )))
    };
    if id.is_empty() {
        return refuse("it is empty");
    }
    if id.len() > MAX_ID_BYTES {
        return refuse("it is longer than any path this system can hold");
    }
    if id.starts_with('/') || id.contains('\\') || id.contains(':') {
        return refuse("it must be a relative path with forward slashes");
    }
    if id.chars().any(char::is_control) {
        return refuse("it contains a control character");
    }
    let components: Vec<&str> = id.split('/').collect();
    if components.len() > MAX_ID_COMPONENTS {
        return refuse("it has more path components than the journal scan produces");
    }
    if components
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return refuse("every component must be a plain file or directory name");
    }
    Ok(())
}

/// Read one journal file, reporting only what the caller already knows.
fn read_journal(path: &FsPath, id: &str) -> Result<String, AppError> {
    std::fs::read_to_string(path).map_err(|error| {
        AppError::Internal(format!(
            "{} could not be read: {}",
            quoted(id),
            error.kind()
        ))
    })
}

/// A `404` that quotes only the caller's own handle.
fn unresolved(id: &str) -> AppError {
    AppError::NotFound(format!("no journal file called {}", quoted(id)))
}

/// The one `409`. One message for every staleness check, deliberately.
fn stale(id: &str) -> AppError {
    AppError::Conflict(format!(
        "{} changed on disk since you opened it, so nothing was written. Reload it and re-apply \
         your edit.",
        quoted(id)
    ))
}

/// A caller-supplied handle, escaped and clipped for an error body.
fn quoted(value: &str) -> String {
    /// Long enough to recognise your own id, short enough that a hostile one
    /// cannot make a large response.
    const MAX_CHARS: usize = 120;
    let clipped: String = value.chars().take(MAX_CHARS).collect();
    let ellipsis = if clipped.chars().count() < value.chars().count() {
        "…"
    } else {
        ""
    };
    format!("{clipped:?}{ellipsis}")
}

impl From<AccountError> for AppError {
    /// Every account error is the caller's: a stale index, a duplicate, or a
    /// value this module will not write. The one exception is
    /// [`AccountError::RoundTripMismatch`], which is **ours** — given `apply`'s
    /// own output the only way to reach it is a bug in the engine — so it is a
    /// `500`.
    fn from(error: AccountError) -> Self {
        match error {
            AccountError::RoundTripMismatch => Self::Internal(error.to_string()),
            other => Self::BadRequest(other.to_string()),
        }
    }
}
