//! Time tracking's routes (plan 46 Phase 2). Every operation is one
//! [`invoicer_core::time`] call, shared with the CLI's `time` commands.
//!
//! | Route                                   | Does                                                                     |
//! |-----------------------------------------|--------------------------------------------------------------------------|
//! | `GET /api/time/state`                   | Track and the lamp: slots 1–9, 0 with today's totals, the running timer and its guard, today, the resume hint, the week; setup's suggestions before setup. |
//! | `POST /api/time/toggle`                 | `{slot}` or `{account}`: stop the running slot, or switch to this one.  |
//! | `POST /api/time/start`                  | `{account}`: start it (stopping another); nothing when it runs.        |
//! | `POST /api/time/stop`                   | `{at?, discard?}`: stop (or discard) my timer.                           |
//! | `POST /api/time/adjust`                 | `{slot\|account, minutes}`: `+`/`−`; on the running slot, its start.     |
//! | `POST /api/time/snap`                   | `{to?}`: the running start back to the previous entry's end.            |
//! | `POST /api/time/note`                   | `{slot\|account, note, date?}`: today's open line's note.                |
//! | `POST /api/time/setup`                  | `{person?, seed?, dry_run?}`: create the default files (T2).            |
//! | `POST /api/time/adopt`                  | `{file, dry_run?}`: use an existing roll-up or time file (T15).         |
//! | `GET/POST/PUT/DELETE /api/time/entries` | Entries: list (filters), add, edit, delete (`?id=`).                    |
//! | `POST /api/time/entries/bulk`           | `{ids, op, …}`: billable, nobill, move, person.                          |
//! | `GET /api/time/accounts`                | The account tree, tags resolved with their source, hours.               |
//! | `POST /api/time/accounts/{declare,set-tag,archive,rename}` | The Accounts page's actions.                         |
//! | `GET /api/time/summary`                 | Time by any groupings (note too), depth, top N, a period series; the roll-up's `file`. |
//! | `GET /api/time/unbilled`                | Per customer, with suggested periods; needs-a-customer; orphan tags.    |
//! | `POST /api/time/bill`                   | Preview (`preview: true`) or create an invoice draft from time.         |
//! | `POST /api/time/release`                | `{invoice, how}`: take `invoice: N` off its time (or make it nobill), refused (`invoice-exists`) while N is a draft or sent and not void; `{bill, how}`: void AP bill `bill` and unlock its time. |
//! | `GET /api/time/unpaid`                  | Per person (timekeepers), with suggested periods.                       |
//! | `POST /api/time/pay`                    | Preview (needs) or book an accounts-payable bill from time.             |
//!
//! Errors are the API's envelope (`{"error", "code"}`, [`ApiError`]):
//! `not-set-up` (409), `note-tag` (422, with `offending` and
//! `suggestion`), `locked` (409), `conflict` (409), `invoice-exists`
//! (409), `external-change`
//! (409), `exists` (409), `invalid`/`verify`/`settings` (422), `not-found`
//! (404), `bad-request` (400), and the books bridge's codes for a bill.
//!
//! `now` (`YYYY-MM-DDTHH:MM:SS`) and `today` may be given on every route,
//! for tests and scripts; else the local clock.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use invoicer_core::time::accounts as time_accounts;
use invoicer_core::time::bill::{self, BillRequest, Release};
use invoicer_core::time::entries::{self, BulkOp, EntryEdit, EntryQuery, NewEntry};
use invoicer_core::time::pay::{self, PayRequest};
use invoicer_core::time::report::{self, Group, SummaryQuery};
use invoicer_core::time::track::{self, SlotKind};
use invoicer_core::time::{Locks, TimeBooks, TimeOpError, TimePrefs, Written, parse_moment};
use ledgeline_core::time::TagRole;
use ledgeline_core::time::book;
use ledgeline_core::time::model::TimeBook;
use ledgeline_core::time::query::Filter;
use ledgeline_core::time::timeclock::Moment;
use ledgeline_core::ui_prefs::Patch;
use serde::Deserialize;
use serde_json::{Value, json};

use super::api::{ApiError, ApiResult, blocking, body, to_value, today as today_of};
use super::books::OpenBooks;
use super::{AppState, prefs};

/// The routes (state still to be supplied).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/time/state", get(state))
        .route("/api/time/toggle", post(toggle))
        .route("/api/time/start", post(start))
        .route("/api/time/stop", post(stop))
        .route("/api/time/adjust", post(adjust))
        .route("/api/time/snap", post(snap))
        .route("/api/time/note", post(note))
        .route("/api/time/setup", post(setup))
        .route("/api/time/adopt", post(adopt))
        .route(
            "/api/time/entries",
            get(list_entries)
                .post(add_entry)
                .put(edit_entry)
                .delete(delete_entry),
        )
        .route("/api/time/entries/bulk", post(bulk))
        .route("/api/time/accounts", get(accounts))
        .route("/api/time/accounts/declare", post(declare))
        .route("/api/time/accounts/set-tag", post(set_tag))
        .route("/api/time/accounts/archive", post(archive))
        .route("/api/time/accounts/rename", post(rename))
        .route("/api/time/summary", get(summary))
        .route("/api/time/unbilled", get(unbilled))
        .route("/api/time/bill", post(bill_route))
        .route("/api/time/release", post(release))
        .route("/api/time/unpaid", get(unpaid))
        .route("/api/time/pay", post(pay_route))
}

impl From<TimeOpError> for ApiError {
    fn from(error: TimeOpError) -> Self {
        if let TimeOpError::Books(books) = &error {
            return ApiError::from(books);
        }
        let code = error.code();
        let status = match code {
            "not-set-up" | "locked" | "conflict" | "invoice-exists" | "external-change"
            | "exists" => StatusCode::CONFLICT,
            "note-tag" | "invalid" | "verify" | "settings" => StatusCode::UNPROCESSABLE_ENTITY,
            "not-found" => StatusCode::NOT_FOUND,
            "bad-request" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = error.to_string();
        let failure = Self::new(status, code, message);
        match error {
            TimeOpError::Note {
                offending,
                suggestion,
                ..
            } => failure
                .with("offending", json!(offending))
                .with("suggestion", json!(suggestion)),
            _ => failure,
        }
    }
}

/// `now`, else the local clock.
fn now_of(given: Option<&str>) -> Result<Moment, ApiError> {
    match given {
        Some(text) => parse_moment(text).ok_or_else(|| {
            ApiError::bad_request(format!("now={text}: not a moment (YYYY-MM-DDTHH:MM:SS)"))
        }),
        None => {
            let local = chrono::Local::now();
            let date = local.format("%Y-%m-%d").to_string();
            let seconds = local.format("%H:%M:%S").to_string();
            Ok(Moment::new(
                date,
                invoicer_core::time::parse_time_of_day(&seconds).unwrap_or(0),
            ))
        }
    }
}

/// The tracker's preferences, as the shared ui-prefs merge them.
fn time_prefs(state: &AppState) -> TimePrefs {
    TimePrefs::from_prefs(prefs::current(state).prefs())
}

/// Save `time` keys (`{"other": …}`) to ui-prefs; a failure is logged, as
/// the write it follows is done.
fn save_prefs(state: &AppState, time: Value) {
    let saved = Patch::from_json(&prefs::SCHEMA, &json!({ "time": time }))
        .map_err(|e| format!("{e:?}"))
        .and_then(|patch| prefs::apply(state, &patch).map_err(|e| format!("{e:?}")));
    if let Err(error) = saved {
        eprintln!(
            "{}: the time preference wasn't saved: {error}",
            super::APP.log_prefix
        );
    }
}

/// The open books' time book and its cached read.
fn time_of(books: &OpenBooks) -> Result<(TimeBooks, Option<Arc<TimeBook>>), ApiError> {
    let time = books.time_books()?;
    let book = books.time_book(&time)?;
    Ok((time, book))
}

/// The book, or `409 not-set-up`.
fn required(time: &TimeBooks, book: Option<&Arc<TimeBook>>) -> Result<Arc<TimeBook>, ApiError> {
    book.cloned()
        .ok_or_else(|| TimeOpError::NotSetUp(time.paths.relative(&time.paths.rollup)).into())
}

/// The locks the open invoices say.
fn locks_of(books: &OpenBooks) -> Locks {
    let session = books.session();
    Locks::of(session.index(), session.booked_numbers())
}

/// What a write answers: its result, the files written, and whether it was
/// retried; the cache dropped.
fn written<T: serde::Serialize>(
    books: &OpenBooks,
    time: &TimeBooks,
    done: &Written<T>,
) -> Result<Value, ApiError> {
    books.forget_time();
    Ok(json!({
        "done": to_value(&done.done)?,
        "files": done.saved.iter().map(|p| time.paths.relative(p)).collect::<Vec<_>>(),
        "retried": done.retried,
        "included": done.included,
    }))
}

/// The state at `now`, for `prefs`, with setup's suggestions when there is
/// no roll-up.
fn state_value(books: &OpenBooks, prefs: &TimePrefs, now: &Moment) -> Result<Value, ApiError> {
    let (time, book) = time_of(books)?;
    let stopped = books
        .time_state()
        .last_stop
        .get(prefs.person.as_deref().unwrap_or(""))
        .map(|(_, account)| account.clone());
    let state = track::state(&time, book.as_deref(), prefs, now, stopped.as_deref());
    let mut value = to_value(&state)?;
    if book.is_none() {
        value["setup"] = json!({
            "suggested_person": suggested_person(),
            "file": time.config.file(),
            "accounts_file": time.config.accounts_file(),
            "track_file": time.config.track_file(),
            "found": book::find_time_files(&**books.vault(), books.main()),
        });
    }
    value["pinned"] = json!(prefs.pinned);
    Ok(value)
}

/// The OS login's short form (P-person); the full name isn't read.
fn suggested_person() -> Option<String> {
    let login = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok();
    book::suggest_person(None, login.as_deref())
}

#[derive(Debug, Default, Deserialize)]
struct NowParams {
    now: Option<String>,
}

/// `GET /api/time/state`.
async fn state(State(state): State<AppState>, Query(params): Query<NowParams>) -> ApiResult {
    let now = now_of(params.now.as_deref())?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let prefs = time_prefs(&state);
        state_value(&books, &prefs, &now)
    })
    .await
}

/// A body that may be absent (`POST` with nothing).
fn optional<T: Default>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    match payload {
        Ok(Json(body)) => Ok(body),
        Err(JsonRejection::MissingJsonContentType(_)) => Ok(T::default()),
        Err(e) => Err(ApiError::bad_request(e.body_text())),
    }
}

/// `{slot}` or `{account}`, and when.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotBody {
    slot: Option<u8>,
    account: Option<String>,
    now: Option<String>,
    minutes: Option<i64>,
    note: Option<String>,
    date: Option<String>,
}

/// The account `body` names: its `account`, or the account in its slot.
fn slot_account(book: &TimeBook, prefs: &TimePrefs, body: &SlotBody) -> Result<String, ApiError> {
    if let Some(account) = body
        .account
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        return Ok(account.to_string());
    }
    let slot = body
        .slot
        .ok_or_else(|| ApiError::bad_request("name a slot (0–9) or an account"))?;
    if slot > 9 {
        return Err(ApiError::bad_request("slot: 0–9"));
    }
    track::slot_accounts(book, prefs)
        .into_iter()
        .find(|(key, _, _)| *key == slot)
        .and_then(|(_, _, account)| account)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "conflict",
                format!("slot {slot} is empty: pin an account, or choose one through Other…"),
            )
        })
}

/// A tracker write: the account resolved, `op` run under the write lock,
/// slot 0 remembered when the account came through Other…, and the outcome
/// with the new state.
async fn tracker_write<F>(state: AppState, body: SlotBody, needs_account: bool, op: F) -> ApiResult
where
    F: FnOnce(
            &TimeBooks,
            &TimeBook,
            &TimePrefs,
            Option<String>,
            &Moment,
        ) -> Result<Written<track::TrackOutcome>, TimeOpError>
        + Send
        + 'static,
{
    let now = now_of(body.now.as_deref())?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let prefs = time_prefs(&state);
        let _guard = books.time_writes();
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        let account = if needs_account {
            Some(slot_account(&book, &prefs, &body)?)
        } else {
            None
        };
        let done = op(&time, &book, &prefs, account.clone(), &now)?;
        if let Some(account) = &account {
            let in_slots = track::slot_accounts(&book, &prefs)
                .iter()
                .any(|(_, kind, a)| *kind != SlotKind::Other && a.as_deref() == Some(account));
            if (body.slot == Some(0) || body.account.is_some())
                && !in_slots
                && done.done.started.is_some()
            {
                save_prefs(&state, json!({ "other": account }));
            }
        }
        if let Some(stopped) = &done.done.stopped {
            let at = parse_moment(&stopped.at).unwrap_or_else(|| now.clone());
            books.time_state().last_stop.insert(
                prefs.person.clone().unwrap_or_default(),
                (at, stopped.account.clone()),
            );
        }
        let mut out = written(&books, &time, &done)?;
        out["state"] = state_value(&books, &time_prefs(&state), &now)?;
        Ok(out)
    })
    .await
}

/// `POST /api/time/toggle`.
async fn toggle(
    State(state): State<AppState>,
    payload: Result<Json<SlotBody>, JsonRejection>,
) -> ApiResult {
    let body = body(payload)?;
    tracker_write(state, body, true, |time, book, prefs, account, now| {
        let account = account.unwrap_or_default();
        track::toggle(time, Some(book), prefs.person.as_deref(), &account, now)
    })
    .await
}

/// `POST /api/time/start`.
async fn start(
    State(state): State<AppState>,
    payload: Result<Json<SlotBody>, JsonRejection>,
) -> ApiResult {
    let body = body(payload)?;
    tracker_write(state, body, true, |time, book, prefs, account, now| {
        let account = account.unwrap_or_default();
        track::start(time, Some(book), prefs.person.as_deref(), &account, now)
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StopBody {
    now: Option<String>,
    /// Stop at this moment instead (*Stop at…*).
    at: Option<String>,
    #[serde(default)]
    discard: bool,
}

/// `POST /api/time/stop`.
async fn stop(
    State(state): State<AppState>,
    payload: Result<Json<StopBody>, JsonRejection>,
) -> ApiResult {
    let request = optional(payload)?;
    let at = request.at.as_deref().map(|t| now_of(Some(t))).transpose()?;
    let body = SlotBody {
        now: request.now,
        ..SlotBody::default()
    };
    let discard = request.discard;
    tracker_write(state, body, false, move |time, book, prefs, _, now| {
        track::stop(
            time,
            Some(book),
            prefs.person.as_deref(),
            at.as_ref().unwrap_or(now),
            discard,
        )
    })
    .await
}

/// `POST /api/time/adjust`: `{slot|account, minutes}` (signed; default the
/// step).
async fn adjust(
    State(state): State<AppState>,
    payload: Result<Json<SlotBody>, JsonRejection>,
) -> ApiResult {
    let body = body(payload)?;
    let minutes = body.minutes;
    tracker_write(state, body, true, move |time, book, prefs, account, now| {
        let account = account.unwrap_or_default();
        track::adjust(
            time,
            Some(book),
            prefs.person.as_deref(),
            &account,
            minutes.unwrap_or(prefs.step),
            now,
        )
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapBody {
    now: Option<String>,
    to: Option<String>,
}

/// `POST /api/time/snap`: to `to`, else (timedot) this app's last stop
/// today, else (timeclock) the end of the latest session.
async fn snap(
    State(state): State<AppState>,
    payload: Result<Json<SnapBody>, JsonRejection>,
) -> ApiResult {
    let request = optional(payload)?;
    let to = request.to.as_deref().map(|t| now_of(Some(t))).transpose()?;
    let books = state.open_books()?;
    let body = SlotBody {
        now: request.now,
        ..SlotBody::default()
    };
    tracker_write(state, body, false, move |time, book, prefs, _, now| {
        let remembered = books
            .time_state()
            .last_stop
            .get(prefs.person.as_deref().unwrap_or(""))
            .map(|(at, _)| at)
            .filter(|at| at.date == now.date)
            .cloned();
        track::snap(
            time,
            Some(book),
            prefs.person.as_deref(),
            to.or(remembered).as_ref(),
        )
    })
    .await
}

/// `POST /api/time/note`: `{slot|account, note, date?}`.
async fn note(
    State(state): State<AppState>,
    payload: Result<Json<SlotBody>, JsonRejection>,
) -> ApiResult {
    let body = body(payload)?;
    let now = now_of(body.now.as_deref())?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let prefs = time_prefs(&state);
        let _guard = books.time_writes();
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        let account = slot_account(&book, &prefs, &body)?;
        let date = body.date.clone().unwrap_or_else(|| now.date.clone());
        let done = track::note(
            &time,
            Some(&book),
            prefs.person.as_deref(),
            &account,
            &date,
            body.note.as_deref().unwrap_or(""),
        )?;
        written(&books, &time, &done)
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupBody {
    /// The person (P-person): saved to ui-prefs; `""` for none.
    person: Option<String>,
    /// Seed `clients:<customer>` per customer and `internal:admin`.
    #[serde(default = "yes")]
    seed: bool,
    #[serde(default)]
    dry_run: bool,
    today: Option<String>,
}

const fn yes() -> bool {
    true
}

/// `POST /api/time/setup`: create the roll-up, the accounts file and the
/// first tracker file (T2), or with `dry_run` show them.
async fn setup(
    State(state): State<AppState>,
    payload: Result<Json<SetupBody>, JsonRejection>,
) -> ApiResult {
    let request = optional(payload)?;
    let today = today_of(request.today.as_deref())?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let _guard = books.time_writes();
        let time = books.time_books()?;
        let person = match request.person.as_deref().map(str::trim) {
            Some("") => None,
            Some(person) => Some(person.to_string()),
            None => time_prefs(&state).person,
        };
        let customers: Vec<String> = if request.seed {
            books
                .session()
                .index()
                .customers()
                .into_iter()
                .map(|c| c.ledger_customer)
                .collect()
        } else {
            Vec::new()
        };
        let names: Vec<&str> = customers.iter().map(String::as_str).collect();
        let seed = if request.seed {
            book::default_seed(&names, &time.config)
        } else {
            Vec::new()
        };
        let plan = book::setup_plan(
            &**books.vault(),
            books.main(),
            &time.config,
            person.as_deref(),
            &today,
            &seed,
        )
        .map_err(TimeOpError::from)?;
        let files: Vec<Value> = plan
            .files
            .iter()
            .map(|(path, text)| json!({ "path": time.paths.relative(path), "text": text }))
            .collect();
        if !request.dry_run {
            book::create(&**books.vault(), &plan).map_err(TimeOpError::from)?;
            if request.person.is_some() {
                save_prefs(&state, json!({ "person": person }));
            }
            books.forget_time();
        }
        Ok(json!({ "files": files, "person": person, "written": !request.dry_run }))
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdoptBody {
    file: String,
    #[serde(default)]
    dry_run: bool,
}

/// `POST /api/time/adopt`: use an existing roll-up or time file; writes
/// `[time] file` into an existing `ledgeline.toml` when it isn't the
/// default name (never creating the file: `settings` says what to add).
/// `missing` lists the `commodity` lines `hledger -s` needs that the
/// roll-up lacks (plan 46 fact 10), for the person to add.
async fn adopt(
    State(state): State<AppState>,
    payload: Result<Json<AdoptBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let plan = book::adopt_plan(&**books.vault(), books.main(), &request.file)
            .map_err(TimeOpError::from)?;
        let written = if request.dry_run || !plan.needs_setting {
            false
        } else {
            book::set_time_file(&**books.vault(), books.main(), &plan.file)
                .map_err(TimeOpError::from)?
        };
        books.forget_time();
        Ok(json!({
            "file": plan.file,
            "format": plan.format.map(|f| f.name()),
            "needs_setting": plan.needs_setting,
            "written": written,
            "missing": plan.missing,
        }))
    })
    .await
}

/// `GET /api/time/entries`: [`EntryQuery`]'s filters.
async fn list_entries(State(state): State<AppState>, Query(query): Query<EntryQuery>) -> ApiResult {
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        to_value(entries::list(&book, &query, &locks_of(&books)))
    })
    .await
}

/// A time write over the cached book and the invoices' locks, answered as
/// [`written`].
async fn time_write<T, F>(state: AppState, op: F) -> ApiResult
where
    T: serde::Serialize,
    F: FnOnce(&TimeBooks, &TimeBook, &Locks, &TimePrefs) -> Result<Written<T>, TimeOpError>
        + Send
        + 'static,
{
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let prefs = time_prefs(&state);
        let _guard = books.time_writes();
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        let locks = locks_of(&books);
        let done = op(&time, &book, &locks, &prefs)?;
        written(&books, &time, &done)
    })
    .await
}

/// `POST /api/time/entries`: Add.
async fn add_entry(
    State(state): State<AppState>,
    payload: Result<Json<NewEntry>, JsonRejection>,
) -> ApiResult {
    let new = body(payload)?;
    time_write(state, move |time, book, _, prefs| {
        entries::add(time, Some(book), prefs.person.as_deref(), &new)
    })
    .await
}

/// `PUT /api/time/entries`: Edit.
async fn edit_entry(
    State(state): State<AppState>,
    payload: Result<Json<EntryEdit>, JsonRejection>,
) -> ApiResult {
    let edit = body(payload)?;
    time_write(state, move |time, book, locks, _| {
        entries::update(time, Some(book), locks, &edit)
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
struct IdParam {
    id: String,
}

/// `DELETE /api/time/entries?id=`.
async fn delete_entry(State(state): State<AppState>, Query(param): Query<IdParam>) -> ApiResult {
    time_write(state, move |time, book, locks, _| {
        entries::delete(time, Some(book), locks, &param.id)
    })
    .await
}

#[derive(Debug, Deserialize)]
struct BulkBody {
    ids: Vec<String>,
    #[serde(flatten)]
    op: BulkOp,
}

/// `POST /api/time/entries/bulk`: `{ids, op: billable|nobill|move|person, …}`.
async fn bulk(
    State(state): State<AppState>,
    payload: Result<Json<BulkBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    time_write(state, move |time, book, locks, _| {
        entries::bulk(time, Some(book), locks, &request.ids, &request.op)
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
struct MonthParams {
    /// `YYYY-MM` for the month column; default this month.
    month: Option<String>,
    today: Option<String>,
}

/// `GET /api/time/accounts`.
async fn accounts(State(state): State<AppState>, Query(params): Query<MonthParams>) -> ApiResult {
    let today = today_of(params.today.as_deref())?;
    let month = params.month.unwrap_or_else(|| today[..7].to_string());
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        let session = books.session();
        to_value(time_accounts::view(&time, &book, session.index(), &month))
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclareBody {
    account: String,
    #[serde(default)]
    tags: std::collections::BTreeMap<String, String>,
}

/// `POST /api/time/accounts/declare`: `{account, tags?}`.
async fn declare(
    State(state): State<AppState>,
    payload: Result<Json<DeclareBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    time_write(state, move |time, book, _, _| {
        let tags: Vec<(String, String)> = request.tags.into_iter().collect();
        time_accounts::declare(time, Some(book), &request.account, &tags)
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetTagBody {
    account: String,
    /// `customer`, `billable`, `item` or `archived`.
    tag: String,
    /// The value; `null` removes it.
    value: Option<String>,
}

fn account_role(tag: &str) -> Result<TagRole, ApiError> {
    match tag {
        "customer" => Ok(TagRole::Customer),
        "billable" => Ok(TagRole::Billable),
        "item" => Ok(TagRole::Item),
        "archived" => Ok(TagRole::Archived),
        other => Err(ApiError::bad_request(format!(
            "tag={other}: an account takes customer, billable, item or archived"
        ))),
    }
}

/// `POST /api/time/accounts/set-tag`: `{account, tag, value}`.
async fn set_tag(
    State(state): State<AppState>,
    payload: Result<Json<SetTagBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    let role = account_role(&request.tag)?;
    time_write(state, move |time, book, _, _| {
        time_accounts::set_tag(
            time,
            Some(book),
            &request.account,
            role,
            request.value.as_deref(),
        )
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveBody {
    account: String,
    #[serde(default = "yes")]
    archived: bool,
}

/// `POST /api/time/accounts/archive`: `{account, archived}`.
async fn archive(
    State(state): State<AppState>,
    payload: Result<Json<ArchiveBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    time_write(state, move |time, book, _, _| {
        time_accounts::set_tag(
            time,
            Some(book),
            &request.account,
            TagRole::Archived,
            request.archived.then_some("yes"),
        )
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameBody {
    from: String,
    to: String,
    /// Write it; else preview.
    #[serde(default)]
    apply: bool,
}

/// `POST /api/time/accounts/rename`: `{from, to, apply}`.
async fn rename(
    State(state): State<AppState>,
    payload: Result<Json<RenameBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    time_write(state, move |time, book, _, _| {
        time_accounts::rename(time, Some(book), &request.from, &request.to, request.apply)
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
struct SummaryParams {
    from: Option<String>,
    to: Option<String>,
    person: Option<String>,
    customer: Option<String>,
    billable: Option<bool>,
    account: Option<String>,
    /// `account,person`: one or more of account, branch, customer, person,
    /// billable, billed, note, day, week, month, quarter, year.
    group: Option<String>,
    depth: Option<usize>,
    top: Option<usize>,
    /// `day`, `week`, `month`, `quarter`, `year`.
    series: Option<String>,
    /// `tree`: the account tree (`hledger bal --tree`) instead.
    view: Option<String>,
}

/// `GET /api/time/summary`.
async fn summary(State(state): State<AppState>, Query(params): Query<SummaryParams>) -> ApiResult {
    let group: Vec<Group> = params
        .group
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(|g| Group::parse(g).ok_or_else(|| super::api::bad("group", g)))
        .collect::<Result<_, _>>()?;
    let series = params
        .series
        .as_deref()
        .map(|s| report::interval(s).ok_or_else(|| super::api::bad("series", s)))
        .transpose()?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        let filter = Filter {
            period: entries::period(params.from.as_deref(), params.to.as_deref()),
            person: params.person.as_deref(),
            customer: params.customer.as_deref(),
            billable: params.billable,
            branch: params.account.as_deref(),
        };
        if params.view.as_deref() == Some("tree") {
            let rows: Vec<Value> = ledgeline_core::time::query::account_tree(&book, &filter)
                .into_iter()
                .map(|row| {
                    json!({
                        "account": row.account,
                        "depth": row.depth,
                        "own": invoicer_core::time::Qty::from(row.own),
                        "total": invoicer_core::time::Qty::from(row.total),
                    })
                })
                .collect();
            return Ok(json!({ "tree": rows }));
        }
        let mut value = to_value(report::summary(
            &book,
            &SummaryQuery {
                filter,
                group,
                depth: params.depth,
                top: params.top,
                series,
            },
        ))?;
        // The roll-up, for the `hledger -f …` commands the dashboard shows.
        value["file"] = json!(time.config.file());
        Ok(value)
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
struct TodayParams {
    today: Option<String>,
    #[serde(default)]
    everyone: bool,
}

/// `GET /api/time/unbilled`.
async fn unbilled(State(state): State<AppState>, Query(params): Query<TodayParams>) -> ApiResult {
    let today = today_of(params.today.as_deref())?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        let locks = locks_of(&books);
        let session = books.session();
        to_value(report::unbilled(
            &book,
            session.index(),
            &locks.known,
            &today,
        ))
    })
    .await
}

/// `GET /api/time/unpaid` (`everyone`: people with no timekeeper note too).
async fn unpaid(State(state): State<AppState>, Query(params): Query<TodayParams>) -> ApiResult {
    let today = today_of(params.today.as_deref())?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        let session = books.session();
        to_value(report::unpaid(
            &book,
            session.index(),
            &today,
            params.everyone,
        ))
    })
    .await
}

/// `POST /api/time/bill`: with `preview`, the bill as it would be; else
/// the draft made and its time tagged (the starter template copied in on
/// first use, as Add invoice does).
async fn bill_route(
    State(state): State<AppState>,
    payload: Result<Json<Value>, JsonRejection>,
) -> ApiResult {
    let raw = body(payload)?;
    let today_given = raw.get("today").and_then(Value::as_str).map(str::to_string);
    let mut stripped = raw;
    if let Some(map) = stripped.as_object_mut() {
        map.remove("today");
    }
    let request: BillRequest =
        serde_json::from_value(stripped).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let today = today_of(today_given.as_deref())?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let _guard = books.time_writes();
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        let mut session = books.session();
        if request.preview {
            return to_value(bill::preview(
                &time,
                &book,
                session.index(),
                session.vault(),
                &request,
                &today,
            )?);
        }
        let customer =
            invoicer_core::time::link::customer_named(session.index(), &request.customer)
                .map(|c| c.wikilink())
                .unwrap_or_else(|| request.customer.clone());
        let (starter_template, starter) =
            crate::starter_for_new(&session, &customer, request.template.as_deref());
        let request = BillRequest {
            template: request.template.clone().or(starter_template),
            ..request
        };
        let booked: Vec<String> = session.booked_numbers().to_vec();
        let booked: Vec<&str> = booked.iter().map(String::as_str).collect();
        let (index, vault) = session.parts_mut();
        let created = bill::create(&time, &book, index, vault, &booked, &request, &today);
        books.forget_time();
        let mut created = created?;
        if let Some(starter) = starter {
            let dir = session.settings().templates_dir.clone();
            match crate::starters::install(session.vault(), &dir, starter) {
                Ok(installed) => {
                    let (index, vault) = session.parts_mut();
                    index.refresh(vault, &installed.written_files());
                }
                Err(e) => created.warnings.push(format!(
                    "the draft is written, but the {} template wasn't copied in: {e}",
                    starter.name
                )),
            }
        }
        to_value(created)
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseBody {
    #[serde(default)]
    invoice: Option<String>,
    #[serde(default)]
    bill: Option<String>,
    #[serde(default)]
    how: Release,
    #[serde(default)]
    today: Option<String>,
}

/// `POST /api/time/release`, one of:
///
/// - `{invoice, how: release|nobill}`: take `invoice: N` off its time (or
///   make it nobill) — an orphan tag's fix. Refused, 409 `invoice-exists`,
///   while invoice N is a draft (delete it) or sent and not void (void it):
///   those flows release the time themselves. `{done, files, retried,
///   included}`.
/// - `{bill, how}`: void the accounts-payable bill whose reference is
///   `bill`, then take `bill:` off its time (or swap it for `nobill`).
///   Refused, 409 `conflict`, when the bill is paid. `{bill, released, how,
///   voided, files}`.
async fn release(
    State(state): State<AppState>,
    payload: Result<Json<ReleaseBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    match (request.invoice, request.bill) {
        (Some(invoice), None) => {
            let books = state.open_books()?;
            blocking(StatusCode::OK, move || {
                let _guard = books.time_writes();
                let (time, book) = time_of(&books)?;
                let book = required(&time, book.as_ref())?;
                let session = books.session();
                let done = bill::release_by_hand(
                    &time,
                    Some(&book),
                    session.index(),
                    session.journal(),
                    &invoice,
                    request.how,
                )?;
                drop(session);
                written(&books, &time, &done)
            })
            .await
        }
        (None, Some(reference)) => {
            let today = today_of(request.today.as_deref())?;
            let books = state.open_books()?;
            blocking(StatusCode::OK, move || {
                let _guard = books.time_writes();
                let (time, book) = time_of(&books)?;
                let book = required(&time, book.as_ref())?;
                let mut session = books.session();
                let invoice_books = session.invoice_books();
                let done = pay::unlock_bill(
                    &time,
                    Some(&book),
                    &invoice_books,
                    &reference,
                    request.how,
                    &today,
                );
                books.forget_time();
                let done = done?;
                super::books::reload_after_write(&mut session);
                to_value(done)
            })
            .await
        }
        _ => Err(ApiError::bad_request(
            "give `invoice` or `bill` (one of them)".to_string(),
        )),
    }
}

/// `POST /api/time/pay`: with `preview` (or when something is needed), the
/// bill as it would be and its `needs`; else booked and its time tagged.
async fn pay_route(
    State(state): State<AppState>,
    payload: Result<Json<Value>, JsonRejection>,
) -> ApiResult {
    let raw = body(payload)?;
    let today_given = raw.get("today").and_then(Value::as_str).map(str::to_string);
    let mut stripped = raw;
    if let Some(map) = stripped.as_object_mut() {
        map.remove("today");
    }
    let request: PayRequest =
        serde_json::from_value(stripped).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let today = today_of(today_given.as_deref())?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        let _guard = books.time_writes();
        let (time, book) = time_of(&books)?;
        let book = required(&time, book.as_ref())?;
        let mut session = books.session();
        let preview = pay::preview(&book, session.index(), session.journal(), &request, &today)?;
        if request.preview || (!preview.ready && request.create_note.is_none()) {
            return to_value(preview);
        }
        let invoice_books = session.invoice_books();
        let (index, vault) = session.parts_mut();
        let created = pay::create(&time, &book, index, vault, &invoice_books, &request, &today);
        books.forget_time();
        let created = created?;
        super::books::reload_after_write(&mut session);
        to_value(created)
    })
    .await
}

/// After an invoice draft is deleted or an invoice voided: its time is
/// released (or made nobill). A failure is reported, not fatal: the
/// invoice step is done, and Unbilled lists the tag as an orphan.
pub(crate) fn release_after(books: &OpenBooks, number: &str, how: Release) -> Value {
    let _guard = books.time_writes();
    let result = books.time_books().and_then(|time| {
        let book = books.time_book(&time)?;
        if book.is_none() {
            return Ok(0);
        }
        bill::release(&time, book.as_deref(), number, how).map(|w| w.done)
    });
    books.forget_time();
    match result {
        Ok(lines) => json!({ "released": lines, "how": how }),
        Err(error) => json!({ "released": 0, "error": error.to_string(), "code": error.code() }),
    }
}
