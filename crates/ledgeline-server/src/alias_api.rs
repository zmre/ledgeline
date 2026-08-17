//! The HTTP surface for **account aliases** — `GET /api/aliases` and
//! `PUT /api/aliases/{*journalId}`.
//!
//! An `alias` directive is the mapping table the import pipeline forwards to
//! `hledger --alias` (see [`ledgeline_core::aliases`] for why that forwarding
//! has to exist at all). This module presents those lines and edits them.
//!
//! # Why this is its own route family and not part of `rules_api`
//!
//! A rules file is a file *beside* the journal, discovered by a scan, and
//! `rules_api`'s five security layers are built around a client-supplied path
//! that must be proved to name one of them. An alias lives **in the journal
//! itself**, so there is no scan and no discovery set: the only files this
//! module will open are the ones the parser already read, named by the same
//! `journalId` handles `/api/import/capabilities` hands out. Reusing
//! `rules_api`'s machinery would mean reusing a resolution model that does not
//! describe this problem.
//!
//! # The write path, which is the risky half
//!
//! A journal is the most valuable file this application touches, so the write is
//! the narrowest one that still does the job:
//!
//! 1. **The handle resolves by set membership**, never by path arithmetic — by
//!    exact string equality against [`journals::targets`] over the journal this
//!    server has open, exactly as `import_api::resolve_journal` does. `--` and
//!    `..` never get a chance to mean anything because a handle that is not in
//!    the set is a `404`.
//! 2. **The revision is a [`Fingerprint`] over the file's raw bytes**, checked
//!    when the file is read and again immediately before the write, so a journal
//!    edited in vim underneath you is a `409` rather than a silent clobber. The
//!    same model, and the same single shared message, as `rules_api`.
//! 3. **The rewrite is a span splice** ([`AliasDoc::apply`]) that touches only
//!    the pattern and replacement of the line being changed.
//! 4. **[`AliasDoc::verify`] must agree**, and then the *whole journal* is
//!    re-parsed with the edited text in memory
//!    ([`parse_journal_with_overrides`]). Only a file that still reads as part
//!    of its journal is written. That last check is here rather than in the
//!    engine because only the server knows which journal a file belongs to.
//! 5. **One [`atomic_write`]**, and it is the last statement that can have an
//!    effect. Every `?` above it is a decision to write nothing.
//!
//! # No absolute path is ever echoed
//!
//! Same rule as everywhere else in this crate. Errors quote the caller's own
//! `journalId`; a whole-journal parse failure is reported *without* hledger's or
//! our own diagnostic text, because `ParseError::Located` names the file it was
//! reading and that name is not the caller's to have.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::aliases::{
    AliasDoc, AliasEdit, AliasError, AliasLine, AliasLock, AliasPlan, AliasRefusal, Forwarded,
};
use ledgeline_core::edit::{Fingerprint, atomic_write};
use ledgeline_core::journals;
use ledgeline_core::model::Journal;
use ledgeline_core::{aliases, parse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};

use crate::AppState;
use crate::edit_api::json_body;
use crate::error::{AppError, editing_disabled};
use crate::reports_api::compute;

/// How many changes one `PUT` may name.
///
/// A bound on the request, not on the user: nobody edits fifty aliases in one
/// gesture, and a plan larger than the file it edits is a client bug.
const MAX_EDITS: usize = 200;

/// Longest accepted `journalId`, in bytes, and how many components it may have.
/// The same numbers `import_api` and `rules_api` use, for the same reason: a
/// handle longer than the platform's own limit cannot name a file that exists.
const MAX_ID_BYTES: usize = 1024;
/// See [`MAX_ID_BYTES`].
const MAX_ID_COMPONENTS: usize = 9;

// ===========================================================================
// Wire types
// ===========================================================================

/// `GET /api/aliases` — every alias the open journal declares.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAliases {
    /// `false` means no journal is bound to an editor, so the screen is
    /// read-only and says why.
    editable: bool,
    /// The journal files that declare an alias, plus the root journal — which is
    /// always offered, even with none, because it is where a first alias goes.
    files: Vec<WireAliasFile>,
}

/// One journal file's alias lines.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAliasFile {
    /// The file's handle: its path relative to the include root, forward
    /// slashes. Never an absolute path.
    journal_id: String,
    /// The file's own name, for display.
    label: String,
    /// A fingerprint of the file's raw bytes. Echo it back in a `PUT` to prove
    /// the edit is against these bytes.
    revision: String,
    /// A regular file inside the include root; `false` means this file can be
    /// listed but not written.
    writable: bool,
    /// This file's alias lines, in file order.
    aliases: Vec<WireAlias>,
}

/// One `alias` line: what it says, whether an import will use it, and whether
/// the GUI will rewrite it.
///
/// The two questions are genuinely independent, which is why there are two pairs
/// of fields. An alias can be forwarded but not editable (a `;` in the
/// replacement — hledger uses it, we will not rewrite it) and editable but not
/// forwarded (an `end aliases` closed its scope).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAlias {
    /// The file this line is in.
    journal_id: String,
    /// 0-based position among that file's alias lines — the handle a `PUT`
    /// names. A parse index, not a durable id; the revision is what makes that
    /// safe.
    index: usize,
    /// 1-based line number in that file.
    line: u32,
    /// The pattern as written, **without** a regex's slashes.
    pattern: String,
    /// The replacement as written.
    replacement: String,
    /// Whether the pattern is the `/REGEX/` form.
    regex: bool,
    /// Will an import hand this to hledger as `--alias`?
    forwarded: bool,
    /// A closed set the UI switches on, present exactly when `forwarded` is
    /// false.
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal: Option<&'static str>,
    /// The sentence to show for `refusal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal_message: Option<&'static str>,
    /// May this line be rewritten here?
    editable: bool,
    /// A closed set the UI switches on, present exactly when `editable` is
    /// false.
    #[serde(skip_serializing_if = "Option::is_none")]
    lock: Option<&'static str>,
    /// The sentence to show for `lock`.
    #[serde(skip_serializing_if = "Option::is_none")]
    lock_message: Option<&'static str>,
}

/// `PUT /api/aliases/{*journalId}`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSaveAliases {
    /// The revision this edit was planned against.
    revision: String,
    /// The changes. Unlike a rules-file save, omission is **not** a delete: this
    /// plan cannot reorder and cannot delete by omission, so naming one line
    /// changes one line.
    edits: Vec<WireAliasEdit>,
}

/// One change. `kind` is the tag, so an unknown one is a `400` rather than a
/// silently different edit.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub(crate) enum WireAliasEdit {
    /// Rewrite one line's pattern and replacement.
    Replace {
        /// Which line, by [`WireAlias::index`].
        index: usize,
        /// The new pattern, without a regex's slashes.
        pattern: String,
        /// The new replacement.
        replacement: String,
        /// Write it as `/REGEX/`.
        regex: bool,
    },
    /// Remove one line.
    Delete {
        /// Which line, by [`WireAlias::index`].
        index: usize,
    },
    /// Add an alias at the end of the file. See `ledgeline_core::aliases` for
    /// why that is the only insertion point.
    Append {
        /// The pattern, without a regex's slashes.
        pattern: String,
        /// The replacement.
        replacement: String,
        /// Write it as `/REGEX/`.
        regex: bool,
    },
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `Cache-Control: no-store`, no `ETag` — the same posture, and the same
/// reasoning, as the rules and import routes: none of this is derived from the
/// journal snapshot's generation counter.
fn no_store<T: Serialize>(body: T) -> Response {
    const NO_STORE: (HeaderName, HeaderValue) =
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    ([NO_STORE], Json(body)).into_response()
}

/// `GET /api/aliases` — every alias the open journal declares.
pub(crate) async fn index(State(state): State<AppState>) -> Result<Response, AppError> {
    let editable = state.editing_enabled();
    let Json(body) = compute(move || {
        Ok(WireAliases {
            editable,
            files: alias_files(&state)?,
        })
    })
    .await?;
    Ok(no_store(body))
}

/// `PUT /api/aliases/{*journalId}` — rewrite one file's alias lines.
pub(crate) async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<WireSaveAliases>, JsonRejection>,
) -> Result<Response, AppError> {
    // Shape first, before any filesystem call, so the route is decided on syntax
    // and is never an existence oracle.
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
    // The journal write mutex, shared with imports: an import appends to a
    // journal file and this rewrites a line of one, and they can name the same
    // file. Held across the `.await`, which is why it is a tokio mutex.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || save_aliases(&state, &id, &request)).await?;
    Ok(no_store(body))
}

// ===========================================================================
// Reading
// ===========================================================================

/// Every journal file worth listing, with its aliases.
///
/// "Worth listing" is: it declares at least one alias, **or** it is the root
/// journal. The root is always offered because a journal-wide mapping table's
/// natural home is the file that includes everything else, and because a user
/// with no aliases yet needs somewhere to put the first one.
///
/// Only those files are read. A journal that splits into forty year-files does
/// not cost forty `read`s per page load.
fn alias_files(state: &AppState) -> Result<Vec<WireAliasFile>, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let targets = journals::targets(journal);
    let forwarded = aliases::forward(journal);

    // Which files to open: every one that declares an alias, plus the root, in
    // the order the parse read them, deduplicated.
    let declaring: Vec<&PathBuf> = journal
        .source_files
        .iter()
        .filter(|path| {
            journal
                .aliases
                .iter()
                .any(|alias| &&alias.source_file == path)
                || Some(*path) == journal.source_files.first()
        })
        .collect();

    Ok(declaring
        .into_iter()
        .filter_map(|path| {
            let target = targets
                .iter()
                .find(|target| journal_path(journal, &target.id).as_ref() == Some(path))?;
            let text = std::fs::read_to_string(path).ok()?;
            let doc = AliasDoc::parse(&text);
            Some(WireAliasFile {
                aliases: doc
                    .lines()
                    .iter()
                    .map(|line| wire_alias(&target.id, line, journal, path, &forwarded))
                    .collect(),
                journal_id: target.id.clone(),
                label: target.label.clone(),
                revision: Fingerprint::of_bytes(text.as_bytes()).token(),
                writable: target.writable,
            })
        })
        .collect())
}

/// The flat alias list `/api/import/capabilities` carries.
///
/// The same builder, so the import screen and the alias editor cannot disagree
/// about what is in force — which would be the one bug that makes the whole
/// visibility requirement pointless.
pub(crate) fn capability_aliases(state: &AppState) -> Vec<WireAlias> {
    alias_files(state)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|file| file.aliases)
        .collect()
}

/// One alias line as the wire carries it.
///
/// The forwarding half is joined on `(file, line)` against the parser's own
/// list. It is a join rather than an ordinal lookup because the scan and the
/// parse are two readings of the file, and a line number is the one thing that
/// cannot drift between them; a miss means the file changed since this
/// snapshot, which is reported as "not forwarded, reload" rather than guessed.
fn wire_alias(
    journal_id: &str,
    line: &AliasLine,
    journal: &Journal,
    path: &FsPath,
    forwarded: &[Forwarded],
) -> WireAlias {
    let matched = journal
        .aliases
        .iter()
        .position(|alias| alias.source_file == path && alias.position.line == line.line)
        .and_then(|at| forwarded.get(at));
    let (forwards, refusal, refusal_message) = match matched.map(|entry| &entry.argument) {
        Some(Ok(_)) => (true, None, None),
        Some(Err(refusal)) => (false, Some(refusal_code(*refusal)), Some(refusal.message())),
        None => (
            false,
            Some("stale"),
            Some(
                "this line is not in the journal as this server last read it; reload the page \
                 and try again",
            ),
        ),
    };
    WireAlias {
        journal_id: journal_id.to_string(),
        index: line.index,
        line: line.line,
        pattern: line.pattern.clone(),
        replacement: line.replacement.clone(),
        regex: line.regex,
        forwarded: forwards,
        refusal,
        refusal_message,
        editable: line.editable(),
        lock: line.lock.map(lock_code),
        lock_message: line.lock.map(AliasLock::message),
    }
}

/// The machine-readable half of a refusal.
///
/// Spelled out rather than derived from `Debug`, so a rename in the engine
/// cannot silently change a wire value — the same rule `hledger_reason` follows.
const fn refusal_code(refusal: AliasRefusal) -> &'static str {
    match refusal {
        AliasRefusal::Scoped => "scoped",
        AliasRefusal::Empty => "empty",
        AliasRefusal::Control => "control",
        AliasRefusal::TooLong => "tooLong",
        AliasRefusal::Limit => "limit",
    }
}

/// The machine-readable half of a lock. See [`refusal_code`].
const fn lock_code(lock: AliasLock) -> &'static str {
    match lock {
        AliasLock::CommentLike => "commentLike",
        AliasLock::Empty => "empty",
        AliasLock::Delimiter => "delimiter",
        AliasLock::Control => "control",
        AliasLock::TooLong => "tooLong",
    }
}

// ===========================================================================
// Writing
// ===========================================================================

/// The whole of `PUT`, synchronously. Every `?` is a decision not to write.
fn save_aliases(
    state: &AppState,
    id: &str,
    request: &WireSaveAliases,
) -> Result<WireAliasFile, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let target = journals::targets(journal)
        .into_iter()
        .find(|target| target.id == id)
        .ok_or_else(|| unresolved(id))?;
    if !target.writable {
        return Err(AppError::BadRequest(format!(
            "{} cannot be edited: an alias can only be written to a regular file inside the \
             journal's own directory, not a symlink or a directory",
            quoted(id)
        )));
    }
    let path = journal_path(journal, id).ok_or_else(|| unresolved(id))?;

    let text = read_journal(&path, id)?;
    let fingerprint = Fingerprint::of_bytes(text.as_bytes());
    // Checked BEFORE any index is resolved, so a client editing an older parse
    // is told the file moved rather than "there is no alias number 3" — which
    // describes the wrong problem and suggests the wrong fix.
    if fingerprint.token() != request.revision {
        return Err(stale(id));
    }

    let doc = AliasDoc::parse(&text);
    let plan = AliasPlan {
        edits: request.edits.iter().map(edit_from_wire).collect(),
    };
    let new_text = doc.apply(&plan)?;
    doc.verify(&plan, &new_text)?;

    // The engine proved the alias lines are what was asked for and that nothing
    // else moved. This proves the result is still a journal — the check `edit.rs`
    // makes for every transaction edit, and the reason it is here rather than in
    // the engine is that only this crate knows which journal this file is part
    // of. Overrides key on the resolved path, which is what `source_files`
    // already holds.
    if new_text != text {
        let overrides = HashMap::from([(path.clone(), new_text.clone())]);
        if parse::parse_journal_with_overrides(&journal.source_name, &overrides).is_err() {
            // Deliberately no detail: `ParseError::Located` names the file it was
            // reading, and that name is not the caller's to have.
            return Err(AppError::BadRequest(format!(
                "this change would make {} unreadable as part of your journal, so nothing was \
                 written",
                quoted(id)
            )));
        }
    }

    if new_text == text {
        // A no-op writes NOTHING. Writing byte-identical content still bumps
        // mtime, and a user's own watch loop would see a spurious change — the
        // same lesson `rules_api` records.
        return Ok(file_response(
            &target,
            &doc,
            fingerprint.token(),
            journal,
            &path,
        ));
    }

    // Narrow the TOCTOU window from "the whole request" to "hash → rename".
    let before_write = Fingerprint::of_bytes(read_journal(&path, id)?.as_bytes());
    if !before_write.content_matches(&fingerprint) {
        return Err(stale(id));
    }

    atomic_write(&path, new_text.as_bytes()).map_err(|error| {
        // Only the `ErrorKind`: `atomic_write` builds a temp path from the
        // target, so its io errors can carry one.
        AppError::Internal(format!(
            "{} could not be written: {}. Nothing else was changed.",
            quoted(id),
            error.kind()
        ))
    })?;

    // The journal changed underneath the editor, so re-open it — otherwise the
    // next transaction edit sees a stale fingerprint and reports a conflict the
    // user did not cause. Logged rather than returned: the write landed, and the
    // file watcher will retry.
    //
    // Done BEFORE the response is built, not after, and that ordering is the
    // whole reason this line is here rather than at the end. `reopen_editor`
    // republishes the snapshot, so the forwarding verdicts below are computed
    // against the journal that now exists. Built from the pre-write snapshot, a
    // freshly appended alias is a line the parse has never seen and honestly
    // reports itself `stale` — telling a user who just added an alias to reload
    // the page, immediately after a save that worked.
    if let Some(Err(error)) = state.reopen_editor() {
        eprintln!("ledgeline: the journal could not be re-read after an alias edit: {error}");
    }

    // The new revision comes from what we WROTE, never from a re-read: a re-read
    // could pick up somebody else's write and hand this client a token for bytes
    // it has never seen, which is exactly how the next save clobbers that person
    // silently.
    let revision = Fingerprint::of_bytes(new_text.as_bytes()).token();
    Ok(file_response(
        &target,
        &AliasDoc::parse(&new_text),
        revision,
        &state.snapshot().journal,
        &path,
    ))
}

/// The file listing a save answers with.
///
/// Note what it re-reads: nothing. The document is the text we just wrote and
/// the revision is its fingerprint, so the client's next save is against bytes
/// this response describes exactly.
///
/// The `journal` it is handed is the one the caller wants the forwarding
/// verdicts computed against — the freshly re-parsed one after a write, and the
/// current snapshot for a no-op. A verdict derived from a journal that no longer
/// exists would be confidently wrong about the thing this screen is for.
fn file_response(
    target: &journals::JournalTarget,
    doc: &AliasDoc,
    revision: String,
    journal: &Journal,
    path: &FsPath,
) -> WireAliasFile {
    let forwarded = aliases::forward(journal);
    WireAliasFile {
        aliases: doc
            .lines()
            .iter()
            .map(|line| wire_alias(&target.id, line, journal, path, &forwarded))
            .collect(),
        journal_id: target.id.clone(),
        label: target.label.clone(),
        revision,
        writable: target.writable,
    }
}

/// One wire edit as the engine's.
fn edit_from_wire(edit: &WireAliasEdit) -> AliasEdit {
    match edit {
        WireAliasEdit::Replace {
            index,
            pattern,
            replacement,
            regex,
        } => AliasEdit::Replace {
            index: *index,
            pattern: pattern.clone(),
            replacement: replacement.clone(),
            regex: *regex,
        },
        WireAliasEdit::Delete { index } => AliasEdit::Delete { index: *index },
        WireAliasEdit::Append {
            pattern,
            replacement,
            regex,
        } => AliasEdit::Append {
            pattern: pattern.clone(),
            replacement: replacement.clone(),
            regex: *regex,
        },
    }
}

// ===========================================================================
// Handles and errors
// ===========================================================================

/// The path a `journalId` names, taken from the files the parse actually read.
///
/// Security layer 2. `root.join(id)` appears here, and the very next thing that
/// happens to the result is a membership test against
/// [`Journal::source_files`] — the set of files the include guard already
/// admitted. A handle that does not name one of them resolves to nothing, so no
/// path this function returns was invented by string arithmetic.
fn journal_path(journal: &Journal, id: &str) -> Option<PathBuf> {
    let root = journal.source_files.first()?.parent()?;
    let candidate = root.join(id);
    journal
        .source_files
        .iter()
        .find(|source| *source == &candidate)
        .cloned()
}

/// Layer 1: shape, before any filesystem call.
///
/// The same rules `rules_api::validate_id` applies, minus the suffix: no `..`,
/// no leading `/`, no `\`, no `:`, no control character. A hostile handle never
/// reaches the filesystem, and 400-vs-404 is decided on syntax rather than on
/// existence.
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

/// A `404` that quotes only the caller's own handle. Every resolution failure
/// returns the same string, so the route is not an existence oracle.
fn unresolved(id: &str) -> AppError {
    AppError::NotFound(format!("no journal file called {}", quoted(id)))
}

/// The one `409`. One message for every staleness check, deliberately: which
/// check tripped is a function of *when* somebody else wrote, and that is not
/// something to leak.
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

impl From<AliasError> for AppError {
    /// Every alias error is the caller's: a stale index, a duplicate, a locked
    /// line, or a value this module will not write. The one exception is
    /// [`AliasError::RoundTripMismatch`], which is **ours** — given `apply`'s own
    /// output the only way to reach it is a bug in the engine — so it is a `500`.
    fn from(error: AliasError) -> Self {
        match error {
            AliasError::RoundTripMismatch => Self::Internal(error.to_string()),
            other => Self::BadRequest(other.to_string()),
        }
    }
}
