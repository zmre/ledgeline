//! The Settings page's API (settings plan §5): `ledgeline.toml [invoicer]`
//! read and saved. The shapes are [`super::settings_wire`]'s.
//!
//! | Route                      | Does                                                                 |
//! |----------------------------|----------------------------------------------------------------------|
//! | `GET /api/settings`        | The page: values, defaults, what the file sets, choices. Reads only. |
//! | `POST /api/settings/check` | What Save would do: problems, effects, a refusal. Writes nothing.    |
//! | `PUT /api/settings`        | Save: writes only the settings that changed.                         |
//!
//! `GET` reads the file itself, so it answers even when `[invoicer]` is
//! invalid and the books couldn't open ([`super::Unopened`]); the page is
//! then read-only and says why. Check and Save need the books open.
//!
//! **Save** (option C, plan D1) answers, in this order: `400 bad-request`
//! (no such setting, a value of no shape), `409 unusable` (a symbolic link,
//! over 64 KiB, not TOML, or an invalid `[invoicer]`: never written), `409
//! changed` (the revision isn't the file's; `revision` is), `422 invalid`
//! (`problems`, by setting), `422 refused` (plan D5: `refused`), `409
//! confirm` (an effect not in `confirmed`; `effects`), `409 no-file` (no
//! `ledgeline.toml` and no `createFile`). Then it writes through Ledgeline's
//! writer ([`super::prefs::edit_ledger_text`]: pinned, capped, over the
//! revision read, under the prefs lock), with the revision checked again
//! inside the edit, so a retry after a race re-checks it. The session is then
//! opened afresh with the new settings (the index and its caches with it),
//! starters copied in and every invoice reconciled, as an outside edit
//! would; the watcher's echo of the write is then no change.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::routing::{get, post};
use invoicer_core::books::Basis;
use invoicer_core::books::spec::{
    deposit_account_in, inferred_revenue_account, receivable_account_in,
};
use invoicer_core::session::Session;
use invoicer_core::settings::Settings;
use invoicer_core::settings_edit::{
    self, AFFECTS_BOOKINGS, Change, FILE_HEADER, PatchError, SettingProblem,
};
use invoicer_core::templates::Template;
use ledgeline_core::accrual::AccrualRoles;
use ledgeline_core::ledger_file::{self, LedgerFile};
use ledgeline_core::vault::{LocalVault, Vault, VaultPath};
use serde_json::{Map, Value};

use super::api::{ApiError, ApiResult, blocking, body, no_books, to_value, with_books};
use super::books::{OpenBooks, reopen};
use super::settings_wire::{
    BooksFacts, COMING, Choices, CompanyChoice, FileState, Inferred, NextNumber, SettingsChange,
    SettingsCheck, SettingsPage, SettingsSaved, TemplateChoice,
};
use super::{AppState, prefs};
use crate::starters;

/// The settings routes (state still to be supplied).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/settings", get(page).put(save))
        .route("/api/settings/check", post(check))
}

/// Where the settings file is: the open books', or those a problem in it
/// kept shut.
pub(crate) struct Place {
    pub vault: Arc<dyn Vault>,
    pub main: VaultPath,
    pub books: Option<Arc<OpenBooks>>,
    /// Why the books couldn't open, when they couldn't.
    pub books_error: Option<String>,
}

/// The books' folder for the Settings page: open, or asked for and shut by
/// a problem in `ledgeline.toml`.
///
/// # Errors
/// `409` with no books asked for.
pub(crate) fn place(state: &AppState) -> Result<Place, ApiError> {
    if let Some(books) = state.books() {
        return Ok(Place {
            vault: Arc::clone(books.vault()),
            main: books.main().clone(),
            books: Some(books),
            books_error: None,
        });
    }
    let unopened = state.unopened().ok_or_else(|| no_books(None))?;
    let (vault, main) = LocalVault::for_journal(&unopened.journal)
        .map_err(|e| ApiError::internal(format!("the books folder: {e}")))?;
    Ok(Place {
        vault: Arc::new(vault),
        main,
        books: None,
        books_error: Some(unopened.problem.clone()),
    })
}

/// Why the page is read-only when the file is unusable.
fn unusable_message(reason: &str) -> String {
    format!(
        "ledgeline.toml {reason}, so the app won't write to it. Open the settings file to see \
         why."
    )
}

/// Why the page is read-only when `[invoicer]` doesn't read.
const INVALID_MESSAGE: &str = "The invoicer settings in ledgeline.toml have a problem, so they \
                               can't be saved here. Fix it in the file, and the page follows.";

/// The file as it is: missing (`""`, `true`), or its text.
///
/// # Errors
/// `409 unusable` for a file that is there but unusable.
fn read_text(vault: &dyn Vault, main: &VaultPath) -> Result<(String, bool), ApiError> {
    let path = settings_path(main)?;
    match ledger_file::read(vault, &path) {
        LedgerFile::Missing => Ok((String::new(), true)),
        LedgerFile::Present { document, .. } => Ok((document.to_string(), false)),
        LedgerFile::Unusable(reason) => Err(unusable(unusable_message(reason))),
    }
}

fn settings_path(main: &VaultPath) -> Result<VaultPath, ApiError> {
    ledger_file::ledger_path(main)
        .map_err(|_| ApiError::internal("ledgeline.toml has no place beside the journal"))
}

fn unusable(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "unusable", message)
}

/// `text`'s settings as the page shows them (kebab-case keys, words nested).
fn values(settings: &Settings) -> Result<Map<String, Value>, ApiError> {
    match to_value(settings)? {
        Value::Object(map) => Ok(map),
        _ => Err(ApiError::internal("the settings aren't a table")),
    }
}

/// `GET /api/settings`.
async fn page(State(state): State<AppState>) -> ApiResult {
    let place = place(&state)?;
    blocking(StatusCode::OK, move || to_value(page_of(&place)?)).await
}

fn page_of(place: &Place) -> Result<SettingsPage, ApiError> {
    let path = settings_path(&place.main)?;
    let (file, text, unusable_reason) = match ledger_file::read(&*place.vault, &path) {
        LedgerFile::Missing => (FileState::Missing, String::new(), None),
        LedgerFile::Present { document, .. } => (FileState::Present, document.to_string(), None),
        LedgerFile::Unusable(reason) => (FileState::Unusable, String::new(), Some(reason)),
    };
    let problems: Vec<SettingProblem> = Settings::problems(&text)
        .into_iter()
        .map(SettingProblem::from)
        .collect();
    let problem = match unusable_reason {
        Some(reason) => Some(unusable_message(reason)),
        None if !problems.is_empty() => Some(INVALID_MESSAGE.to_string()),
        None => None,
    };
    let settings = Settings::from_ledgeline_toml(&text).unwrap_or_default();
    let mut page = SettingsPage {
        file,
        read_only: problem.is_some(),
        problem,
        problems,
        revision: if unusable_reason.is_some() {
            None
        } else {
            settings_edit::revision(&text)
        },
        values: values(&settings)?,
        defaults: values(&Settings::default())?,
        set: settings_edit::set_keys(&text),
        inferred: Inferred::default(),
        next_number: None,
        books: BooksFacts::default(),
        books_error: place.books_error.clone(),
        choices: Choices::default(),
        affects_bookings: AFFECTS_BOOKINGS.to_vec(),
        coming: COMING.to_vec(),
    };
    if let Some(books) = &place.books {
        let session = books.session();
        fill_from_books(&mut page, &session);
    }
    Ok(page)
}

/// What only open books can say: what is found for unset accounts, the
/// next number, `[books]`, and the pickers' choices.
fn fill_from_books(page: &mut SettingsPage, session: &Session) {
    let journal = session.journal();
    let roles = AccrualRoles::of(journal);
    let none = Settings::default();
    page.inferred = Inferred {
        receivable_account: receivable_account_in(&none, &roles).ok(),
        revenue_account: inferred_revenue_account(journal),
        deposit_account: Some(deposit_account_in(&none, &roles)),
    };
    page.next_number = session.explain_next_number().ok().map(|next| NextNumber {
        number: next.number.to_string(),
        after: next.after,
        source: next.source.as_str(),
    });
    page.books = BooksFacts {
        basis: Some(match session.basis() {
            Basis::Cash => "cash",
            Basis::Accrual => "accrual",
        }),
        closed_through: session.books_config().closed_through.clone(),
    };
    let index = session.index();
    let dir = &session.settings().templates_dir;
    let own: Vec<TemplateChoice> = index
        .templates()
        .iter()
        .map(|t| TemplateChoice {
            value: t.template_path(),
            name: t.name.clone(),
            kinds: t.kinds.clone(),
            starter: false,
        })
        .collect();
    let starters = starters::STARTERS.iter().filter_map(|starter| {
        let path = starters::path_in(dir, starter).ok()?;
        let value = format!("/{path}");
        if own.iter().any(|t| t.value == value) {
            return None;
        }
        let shipped = Template::from_source(path, starter.source)?;
        Some(TemplateChoice {
            value,
            name: starter.name.to_string(),
            kinds: shipped.kinds,
            starter: true,
        })
    });
    let templates = own.iter().cloned().chain(starters).collect();
    page.choices = Choices {
        templates,
        companies: index
            .site()
            .notes_of_type("organization")
            .map(|note| CompanyChoice {
                title: note.title.clone(),
                link: format!("[[{}]]", note.title),
            })
            .collect(),
        accounts: ledgeline_core::wire::journal_to_accountnames(journal),
    };
}

/// What a change would do, worked out against the file as it is.
struct Planned {
    change: Change,
    /// There is no file yet.
    missing: bool,
    problems: Vec<SettingProblem>,
    effects: Vec<settings_edit::Effect>,
    refused: Option<settings_edit::Refusal>,
}

/// `400` for a change that names no setting or has no shape.
fn patch_error(error: PatchError) -> ApiError {
    match error {
        PatchError::BadRequest(message) => ApiError::bad_request(message),
        PatchError::Unusable(message) => unusable(message),
        PatchError::Invalid(problems) => invalid(&problems),
    }
}

fn invalid(problems: &[SettingProblem]) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid",
        "Some settings have problems.",
    )
    .with(
        "problems",
        serde_json::to_value(problems).unwrap_or_default(),
    )
}

/// Refuse to edit a file whose `[invoicer]` doesn't read (plan §3 rule 7),
/// or whose revision isn't `expected`.
fn check_file(text: &str, expected: Option<&str>) -> Result<(), ApiError> {
    let problems: Vec<SettingProblem> = Settings::problems(text)
        .into_iter()
        .map(SettingProblem::from)
        .collect();
    if !problems.is_empty() {
        return Err(unusable(INVALID_MESSAGE).with(
            "problems",
            serde_json::to_value(&problems).unwrap_or_default(),
        ));
    }
    let revision = settings_edit::revision(text);
    if revision.as_deref() != expected {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "changed",
            "The settings file changed since this page read it. Reload, or keep your changes \
             and save over it.",
        )
        .with(
            "revision",
            serde_json::to_value(revision).unwrap_or_default(),
        ));
    }
    Ok(())
}

fn plan(
    books: &OpenBooks,
    session: &Session,
    request: &SettingsChange,
) -> Result<Planned, ApiError> {
    let change = Change::from_json(&request.set, &request.unset).map_err(patch_error)?;
    let (text, missing) = read_text(&**books.vault(), books.main())?;
    check_file(&text, request.revision.as_deref())?;
    let mut planned = Planned {
        change,
        missing,
        problems: Vec::new(),
        effects: Vec::new(),
        refused: None,
    };
    match settings_edit::patch(&text, &planned.change) {
        Ok(after_text) => {
            let read = |text: &str| {
                Settings::from_ledgeline_toml(text)
                    .map_err(|e| ApiError::internal(format!("the settings don't read: {e}")))
            };
            let (before, after) = (read(&text)?, read(&after_text)?);
            planned.effects = settings_edit::effects(&before, &after, session);
            planned.refused = settings_edit::refusal(&before, &after, session)
                .map_err(|e| super::pdf::books_error(&e))?;
        }
        Err(PatchError::Invalid(problems)) => planned.problems = problems,
        Err(error) => return Err(patch_error(error)),
    }
    Ok(planned)
}

/// `POST /api/settings/check`.
async fn check(
    State(state): State<AppState>,
    payload: Result<Json<SettingsChange>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let planned = plan(books, &session, &request)?;
        to_value(SettingsCheck {
            problems: planned.problems,
            effects: planned.effects,
            refused: planned.refused,
        })
    })
    .await
}

fn no_file() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "no-file",
        "These books have no settings file yet. Create ledgeline.toml beside the journal?",
    )
}

/// `PUT /api/settings`.
async fn save(
    State(state): State<AppState>,
    payload: Result<Json<SettingsChange>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    let prefs_state = state.clone();
    with_books(&state, StatusCode::OK, move |books| {
        let mut session = books.session();
        let planned = plan(books, &session, &request)?;
        if !planned.problems.is_empty() {
            return Err(invalid(&planned.problems));
        }
        if let Some(refused) = &planned.refused {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "refused",
                refused.message.clone(),
            )
            .with("refused", to_value(refused)?));
        }
        if planned
            .effects
            .iter()
            .any(|effect| !request.confirmed.contains(&effect.key))
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "confirm",
                "Some changes affect new bookings. Confirm them to save.",
            )
            .with("effects", to_value(&planned.effects)?));
        }
        if planned.missing && !request.create_file {
            return Err(no_file());
        }
        if !planned.change.is_empty() {
            let vault = Arc::clone(books.vault());
            let path = settings_path(books.main())?;
            prefs::edit_ledger_text(&prefs_state, FILE_HEADER, |text| {
                // Checked again on what is there now: a retry after a race
                // re-checks it, and a file deleted meanwhile isn't created
                // without the user's yes.
                if text.is_empty()
                    && !request.create_file
                    && matches!(ledger_file::read(&*vault, &path), LedgerFile::Missing)
                {
                    return Err(no_file());
                }
                check_file(text, request.revision.as_deref())?;
                settings_edit::patch(text, &planned.change).map_err(patch_error)
            })?;
            reopen(&mut session, books.vault(), books.main()).map_err(|e| {
                ApiError::internal(format!(
                    "the settings were saved, but the books didn't open with them: {e}"
                ))
            })?;
        }
        let (text, _) = read_text(&**books.vault(), books.main())?;
        to_value(SettingsSaved {
            revision: settings_edit::revision(&text).unwrap_or_default(),
            values: values(session.settings())?,
            set: settings_edit::set_keys(&text),
            created: planned.missing && !planned.change.is_empty(),
        })
    })
    .await
}
