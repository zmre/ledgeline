//! Editing `ledgeline.toml [invoicer]` from the Settings page (settings
//! plan §3, option C): Save is the explicit action, and writes only what
//! changed.
//!
//! - [`patch`] sets and removes settings through `toml_edit`'s document
//!   model, so comments, key order and every other table come back byte for
//!   byte. A removed setting takes the comment lines directly above it. A
//!   missing `[invoicer]` (or `[invoicer.words]`) is added after what is
//!   there. The result is read back with [`Settings::from_ledgeline_toml`]
//!   and refused unless it reads.
//! - [`revision`] hashes `[invoicer]` alone, so a write to `[ui]`,
//!   `[books]` or `[sync]` (a theme pin, a bank link) never conflicts with
//!   an open form.
//! - [`effects`] says what each booking-related change does, for the
//!   confirmation; [`refusal`] is the one change refused outright (plan
//!   D5).
//!
//! Writing the file (the pinned, capped, revision-checked writer, under the
//! app's prefs lock) is the app's: this module only turns text into text.

use crate::books::Basis;
use crate::books::deposits::WaitingDeposit;
use crate::books::spec::{deposit_account, inferred_revenue_account, receivable_account_in};
use crate::doc::{DocType, Status};
use crate::session::Session;
use crate::settings::{
    CashDeposits, PaymentAllocation, Settings, SettingsError, WORD_KEYS, is_key, label,
};
use ledgeline_core::accrual::AccrualRoles;
use ledgeline_core::edit::Fingerprint;
use serde::Serialize;
use serde_json::{Map, Value as Json};
use std::fmt::Write as _;
use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value};

/// The top of a `ledgeline.toml` the Settings page creates (after the user
/// agreed to: `409 no-file`, then `createFile`).
pub const FILE_HEADER: &str = concat!(
    "# Settings for these books. Ledgeline Invoicer keeps its settings under\n",
    "# [invoicer]; Ledgeline's are under [books] and [ui]. Comments and keys the\n",
    "# apps don't know are kept. Docs: \"Invoicer settings\",\n",
    "# https://zmre.github.io/ledgeline/invoicer/settings/\n",
    "\n",
);

/// The table the invoicer's settings live in.
const TABLE: &str = "invoicer";

/// Its word lists' table, inside [`TABLE`].
const WORDS: &str = "words";

/// The settings whose change alters what new bookings do: Save asks to
/// confirm them ([`effects`]).
pub const AFFECTS_BOOKINGS: [&str; 6] = [
    "receivable-account",
    "revenue-account",
    "deposit-account",
    "discount-account",
    "cash-deposits",
    "payment-allocation",
];

/// A problem with one setting, or with the file (`key` `None`). `key` is
/// the setting's name in the file (`filename`, `words.quantity`); the page
/// shows its label. `message` is a plain sentence that never names a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingProblem {
    /// The setting, as in the file; `None` for the file as a whole.
    pub key: Option<String>,
    /// Why, in plain words.
    pub message: String,
}

impl From<SettingsError> for SettingProblem {
    fn from(error: SettingsError) -> Self {
        match error {
            SettingsError::Invalid { key, message } => Self { key, message },
            SettingsError::Unreadable(_) => Self {
                key: None,
                message: "ledgeline.toml can't be read.".to_string(),
            },
        }
    }
}

/// What a change to a setting that affects bookings does, for the
/// confirmation Save asks for. The page sends `key` back in `confirmed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Effect {
    /// The setting.
    pub key: String,
    /// What changes, and what stays as it was, in plain words.
    pub message: String,
}

/// A change refused whatever is confirmed (plan D5: deposit payments in
/// cash books while a deposit waits for its final invoice).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refusal {
    /// The setting.
    pub key: String,
    /// Why, in plain words.
    pub message: String,
    /// The deposits it waits on.
    pub deposits: Vec<WaitingDeposit>,
}

/// Why [`patch`] wrote nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    /// An unknown setting, or a value of no shape a setting takes (`400`).
    BadRequest(String),
    /// The file is TOML the page can't edit safely (`409 unusable`).
    Unusable(String),
    /// The values don't read back as settings (`422 invalid`).
    Invalid(Vec<SettingProblem>),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(m) | Self::Unusable(m) => f.write_str(m),
            Self::Invalid(problems) => f.write_str(
                problems
                    .first()
                    .map_or("the settings don't read back", |p| p.message.as_str()),
            ),
        }
    }
}

impl std::error::Error for PatchError {}

/// A change to `[invoicer]`: settings to set (TOML values) and to remove.
/// Names are as the file spells them; a word list is `words.<key>`.
#[derive(Debug, Clone, Default)]
pub struct Change {
    set: Vec<(String, Value)>,
    unset: Vec<String>,
}

impl Change {
    /// A change from the page's JSON: `set` values are text, whole numbers,
    /// or lists of them.
    ///
    /// # Errors
    /// [`PatchError::BadRequest`] for a name that is no setting, one both
    /// set and removed, or a value of another shape (an object, a fraction,
    /// `null`: "Use the default" is `unset`).
    pub fn from_json(set: &Map<String, Json>, unset: &[String]) -> Result<Self, PatchError> {
        let check = |key: &str| {
            if is_key(key) {
                Ok(())
            } else {
                Err(PatchError::BadRequest(format!(
                    "There is no setting called “{key}”."
                )))
            }
        };
        let set = set
            .iter()
            .map(|(key, value)| {
                check(key)?;
                Ok((key.clone(), toml_value(key, value)?))
            })
            .collect::<Result<Vec<_>, PatchError>>()?;
        for key in unset {
            check(key)?;
            if set.iter().any(|(k, _)| k == key) {
                return Err(PatchError::BadRequest(format!(
                    "{} is both changed and set back to its default.",
                    sentence(label(key))
                )));
            }
        }
        Ok(Self {
            set,
            unset: unset.to_vec(),
        })
    }

    /// Whether the change changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.unset.is_empty()
    }
}

/// `label` with its first letter in capitals, to start a sentence.
fn sentence(label: &str) -> String {
    let mut chars = label.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// A JSON value as the TOML value a setting holds.
fn toml_value(key: &str, json: &Json) -> Result<Value, PatchError> {
    let shape = || {
        PatchError::BadRequest(format!(
            "{} must be text, a whole number, or a list of them.",
            sentence(label(key))
        ))
    };
    let scalar = |json: &Json| match json {
        Json::String(text) => Ok(Value::from(text.as_str())),
        Json::Number(n) => n.as_i64().map(Value::from).ok_or_else(shape),
        _ => Err(shape()),
    };
    match json {
        Json::Array(items) => items
            .iter()
            .map(scalar)
            .collect::<Result<Array, _>>()
            .map(Value::Array),
        other => scalar(other),
    }
}

/// A hash of `text`'s `[invoicer]` table alone (its values, not its
/// comments or layout), as an opaque token: what Save must still find.
/// `None` when `text` isn't TOML. A file with no `[invoicer]`, and no file,
/// have the same revision.
#[must_use]
pub fn revision(text: &str) -> Option<String> {
    let doc: toml::Table = toml::from_str(text).ok()?;
    let mut canonical = String::new();
    if let Some(table) = doc.get(TABLE) {
        write_canonical(table, &mut canonical);
    }
    Some(Fingerprint::of_bytes(canonical.as_bytes()).token())
}

/// `value` with every table's keys sorted, so key order is no change.
fn write_canonical(value: &toml::Value, out: &mut String) {
    match value {
        toml::Value::Table(table) => {
            let mut keys: Vec<&String> = table.keys().collect();
            keys.sort();
            out.push('{');
            for key in keys {
                let _ = write!(out, "{}=", Json::from(key.as_str()));
                write_canonical(&table[key.as_str()], out);
                out.push(',');
            }
            out.push('}');
        }
        toml::Value::Array(items) => {
            out.push('[');
            for item in items {
                write_canonical(item, out);
                out.push(',');
            }
            out.push(']');
        }
        toml::Value::String(text) => {
            let _ = write!(out, "{}", Json::from(text.as_str()));
        }
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

/// The settings `text`'s `[invoicer]` sets, in the page's order
/// ([`crate::settings::KEYS`], then `words.<key>`): the rows that offer
/// "Use the default". Unknown keys are not listed. Empty when `text` isn't
/// TOML.
#[must_use]
pub fn set_keys(text: &str) -> Vec<String> {
    let Ok(doc) = toml::from_str::<toml::Table>(text) else {
        return Vec::new();
    };
    let Some(toml::Value::Table(table)) = doc.get(TABLE) else {
        return Vec::new();
    };
    let words = match table.get(WORDS) {
        Some(toml::Value::Table(words)) => Some(words),
        _ => None,
    };
    crate::settings::KEYS
        .iter()
        .filter(|key| table.contains_key(**key))
        .map(|key| (*key).to_string())
        .chain(
            WORD_KEYS
                .iter()
                .filter(|word| words.is_some_and(|w| w.contains_key(**word)))
                .map(|word| format!("{WORDS}.{word}")),
        )
        .collect()
}

/// Apply `change` to a whole `ledgeline.toml`'s `text` (`""` for a file
/// that isn't there yet), keeping everything else byte for byte. A setting
/// set to the value the file already holds is left as written.
///
/// # Errors
/// [`PatchError::Unusable`] when `text` isn't TOML or `[invoicer]` isn't a
/// table the page can edit (written inline); [`PatchError::Invalid`] when
/// the result doesn't read back as settings (every problem, by setting).
pub fn patch(text: &str, change: &Change) -> Result<String, PatchError> {
    let mut doc: DocumentMut = text.parse().map_err(|_| {
        PatchError::Unusable("ledgeline.toml isn't valid TOML, so it can't be saved.".into())
    })?;
    if !change.set.is_empty() {
        ensure_table(&mut doc);
    }
    for (key, value) in &change.set {
        let table = invoicer_mut(&mut doc)?;
        match key.strip_prefix("words.") {
            Some(word) => put(words_mut(table)?, word, value.clone())?,
            None => put(table, key, value.clone())?,
        }
    }
    for key in &change.unset {
        let Some(table) = doc.get_mut(TABLE).and_then(Item::as_table_like_mut) else {
            continue;
        };
        match key.strip_prefix("words.") {
            Some(word) => {
                let emptied = match table.get_mut(WORDS).and_then(Item::as_table_like_mut) {
                    Some(words) => {
                        words.remove(word);
                        words.is_empty()
                    }
                    None => false,
                };
                if emptied {
                    table.remove(WORDS);
                }
            }
            None => {
                table.remove(key);
            }
        }
    }
    let after = doc.to_string();
    let problems: Vec<SettingProblem> = Settings::problems(&after)
        .into_iter()
        .map(SettingProblem::from)
        .collect();
    if problems.is_empty() && after.parse::<DocumentMut>().is_ok() {
        Ok(after)
    } else {
        Err(PatchError::Invalid(problems))
    }
}

/// Add `[invoicer]` after what `doc` holds when it has none. A file's
/// trailing comments (all of a comment-only file) stay above it.
fn ensure_table(doc: &mut DocumentMut) {
    if doc.contains_key(TABLE) {
        return;
    }
    let trailing = doc.trailing().as_str().unwrap_or_default().to_string();
    doc.set_trailing("");
    let mut table = Table::new();
    // A blank line after the last table; a comment-only (or new) file's
    // text is the prefix as it is.
    let prefix = if trailing.is_empty() && !doc.is_empty() {
        "\n".to_string()
    } else {
        trailing
    };
    table.decor_mut().set_prefix(prefix);
    doc.insert(TABLE, Item::Table(table));
}

/// `[invoicer]`, as a table the page may edit.
fn invoicer_mut(doc: &mut DocumentMut) -> Result<&mut Table, PatchError> {
    let item = doc
        .get_mut(TABLE)
        .ok_or_else(|| PatchError::Unusable("ledgeline.toml has no [invoicer] table.".into()))?;
    let table = item.as_table_mut().ok_or_else(|| {
        PatchError::Unusable(
            "The invoicer settings in ledgeline.toml are written on one line, so they can't be \
             saved here. Edit them in the file."
                .into(),
        )
    })?;
    // `[invoicer.words]` alone makes `[invoicer]` implicit; it now holds a key.
    table.set_implicit(false);
    Ok(table)
}

/// `[invoicer.words]`, added after `[invoicer]`'s keys when missing.
fn words_mut(table: &mut Table) -> Result<&mut dyn TableLike, PatchError> {
    if !table.contains_key(WORDS) {
        table.insert(WORDS, Item::Table(Table::new()));
    }
    table
        .get_mut(WORDS)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| {
            PatchError::Unusable(
                "The invoicer's words in ledgeline.toml aren't a table, so they can't be saved \
                 here. Edit them in the file."
                    .into(),
            )
        })
}

/// Set `key` to `value` in `table`, keeping its comments and spacing when
/// it is already there, and leaving it alone when it already holds the
/// value.
fn put(table: &mut dyn TableLike, key: &str, mut value: Value) -> Result<(), PatchError> {
    match table.get_mut(key) {
        Some(item) => {
            let existing = item.as_value_mut().ok_or_else(|| {
                PatchError::Unusable(format!(
                    "{} isn't written as a value in ledgeline.toml, so it can't be saved here.",
                    sentence(label(key))
                ))
            })?;
            if same_value(existing, &value) {
                return Ok(());
            }
            *value.decor_mut() = existing.decor().clone();
            *existing = value;
        }
        None => {
            table.insert(key, Item::Value(value));
        }
    }
    Ok(())
}

/// Whether two TOML values are the same value, however written.
fn same_value(a: &Value, b: &Value) -> bool {
    let read = |v: &Value| {
        toml::from_str::<toml::Table>(&format!("v = {}", v.clone().decorated("", "")))
            .ok()
            .and_then(|mut t| t.remove("v"))
    };
    read(a).is_some_and(|a| Some(a) == read(b))
}

/// What each changed setting that affects bookings does, from `before` to
/// `after`, in the books `session` has open: an effect only where what new
/// bookings use changes (setting an account to the one already found is
/// none).
#[must_use]
pub fn effects(before: &Settings, after: &Settings, session: &Session) -> Vec<Effect> {
    let journal = session.journal();
    let roles = AccrualRoles::of(journal);
    let cash = session.basis() == Basis::Cash;
    let accrual_only = "These books use accrual, so this applies only if they switch to cash.";
    let none = "no account";
    let mut out: Vec<Effect> = Vec::new();
    let mut push = |key: &str, message: String| {
        out.push(Effect {
            key: key.to_string(),
            message,
        });
    };

    let receivable = |s: &Settings| receivable_account_in(s, &roles).ok();
    let (old, new) = (receivable(before), receivable(after));
    if old != new {
        let new = new.as_deref().unwrap_or(none);
        let old = old.as_deref().unwrap_or(none);
        push(
            "receivable-account",
            if cash {
                format!(
                    "Cash books book no receivable when an invoice is sent, so nothing in your \
                     books changes. If these books switch to accrual, invoices will owe to {new}."
                )
            } else {
                let sent = sent_invoices(session);
                format!(
                    "Invoices you send from now on owe to {new}.{}",
                    stay(sent, &format!("on {old}"))
                )
            },
        );
    }

    let inferred = inferred_revenue_account(journal);
    let revenue = |s: &Settings| s.revenue_account.clone().or_else(|| inferred.clone());
    let (old, new) = (revenue(before), revenue(after));
    if old != new {
        let sent = sent_invoices(session);
        push(
            "revenue-account",
            format!(
                "Rows with no account of their own book to {} from now on.{}",
                new.as_deref().unwrap_or(none),
                stay(sent, &format!("with {}", old.as_deref().unwrap_or(none)))
            ),
        );
    }

    let (old, new) = (
        deposit_account(before, journal),
        deposit_account(after, journal),
    );
    if old != new {
        push(
            "deposit-account",
            format!(
                "Deposit invoices book to {new} from now on. Deposits already in your books \
                 stay on {old}."
            ),
        );
    }

    let discount = match (&before.discount_account, &after.discount_account) {
        (old, Some(new)) if old.as_ref() != Some(new) => Some(format!(
            "Discount rows book to {new} from now on, instead of {}. Invoices already in your \
             books keep how they were booked.",
            old.as_deref()
                .map_or_else(|| "reducing each row pro rata".to_string(), str::to_string)
        )),
        (Some(old), None) => Some(format!(
            "Discount rows reduce each row pro rata from now on, instead of booking to {old}. \
             Invoices already in your books keep how they were booked."
        )),
        _ => None,
    };
    if let Some(message) = discount {
        push("discount-account", message);
    }

    if before.cash_deposits != after.cash_deposits {
        push(
            "cash-deposits",
            if !cash {
                accrual_only.to_string()
            } else if after.cash_deposits == CashDeposits::LiabilityUntilFinal {
                format!(
                    "Deposit payments you apply from now on are held in {} until the final \
                     invoice uses them. Deposits already received keep where they were booked.",
                    deposit_account(after, journal)
                )
            } else {
                "Deposit payments you apply from now on count as income when they arrive. \
                 Deposits already received keep where they were booked."
                    .to_string()
            },
        );
    }

    if before.payment_allocation != after.payment_allocation {
        push(
            "payment-allocation",
            if !cash {
                accrual_only.to_string()
            } else if after.payment_allocation == PaymentAllocation::ProRata {
                "Payments you apply from now on are split pro rata. Payments already in your \
                 books keep the split they were booked with."
                    .to_string()
            } else {
                "Payments you apply from now on move each row toward its invoiced amount. \
                 Payments already in your books keep the split they were booked with."
                    .to_string()
            },
        );
    }
    out
}

/// " N sent invoices stay {where}; re-booking one moves it.", or nothing.
fn stay(sent: usize, place: &str) -> String {
    match sent {
        0 => String::new(),
        1 => format!(" 1 sent invoice stays {place}; re-booking it moves it."),
        n => format!(" {n} sent invoices stay {place}; re-booking one moves it."),
    }
}

/// How many invoices and credit notes are sent (booked, in accrual books).
fn sent_invoices(session: &Session) -> usize {
    session
        .index()
        .docs()
        .filter(|d| d.header.status == Some(Status::Sent))
        .filter(|d| {
            matches!(
                d.header.doc_type.unwrap_or(DocType::Invoice),
                DocType::Invoice | DocType::CreditNote
            )
        })
        .count()
}

/// The change refused whatever is confirmed (plan D5): `cash-deposits` in
/// cash books while a deposit waits for its final invoice, read from where
/// each deposit's payments actually booked.
///
/// # Errors
/// When the books can't be loaded.
pub fn refusal(
    before: &Settings,
    after: &Settings,
    session: &Session,
) -> Result<Option<Refusal>, crate::books::BooksError> {
    if before.cash_deposits == after.cash_deposits || session.basis() != Basis::Cash {
        return Ok(None);
    }
    let waiting = session
        .invoice_books()
        .deposits_awaiting_final(session.index(), session.book_states())?;
    Ok((!waiting.is_empty()).then(|| Refusal {
        key: "cash-deposits".to_string(),
        message: match waiting.len() {
            1 => "1 deposit is waiting for its final invoice. Change how deposit payments book \
                  once the final invoice that uses it is paid: changing it now would count the \
                  deposit twice, or not at all."
                .to_string(),
            n => format!(
                "{n} deposits are waiting for their final invoices. Change how deposit payments \
                 book once the final invoices that use them are paid: changing it now would \
                 count the deposits twice, or not at all."
            ),
        },
        deposits: waiting,
    }))
}

#[cfg(test)]
mod tests;
