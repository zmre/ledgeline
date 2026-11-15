//! The `[sync]` tables of `ledgeline.toml` (plan 43 §3.6).
//!
//! ```toml
//! [[sync.connection]]
//! id = "4f6c…"              # keychain key = "ledgeline.sync.<id>"
//! name = "SimpleFIN Bridge"
//! kind = "simplefin"
//!
//! [[sync.account]]
//! connection = "4f6c…"
//! remoteId = "ACT-…"
//! label = "First Tech · Checking ••1234"
//! journalAccount = "assets:bank:firsttech:checking"
//! rules = "imports/rules/firsttech-checking.sync.csv.rules"
//! target = "2026.journal"
//! startDate = 2026-11-01
//! balanceSign = 1
//! enabled = true
//!
//! [[sync.manual]]
//! account = "assets:invest:vanguard"
//! downloadUrl = "https://…"
//! nudge = true
//! ```
//!
//! # Untrusted input
//!
//! `ledgeline.toml` travels with a cloned repo, so it holds no credential and
//! no server address: the access URL (and so the host) lives in the keychain
//! under the connection id. [`parse`] therefore insists on an id that is safe
//! as a keychain key, and drops a `downloadUrl` that isn't `http(s)`. It checks
//! nothing on disk; the server resolves `rules` and `target` inside the
//! journal's folder before using them.
//!
//! [`parse`] never fails as a whole: a malformed entry is skipped with a
//! warning, so one bad hand edit can't hide every other link.
//!
//! # Writing
//!
//! Every writer edits through `toml_edit`'s document, so comments, key order
//! and every other table — `[ui]` above all — come back byte for byte. An
//! updated key keeps its trailing comment. Nothing but these writers ever
//! writes `[sync]`.

use super::normalize::account_short;
use super::sanitize;
use crate::reports::periods::{days_from_iso, iso_from_days};
use toml_edit::{ArrayOfTables, Date, Datetime, DocumentMut, Item, Table, TableLike, Value};

/// The `kind` a connection gets when the file doesn't say.
pub const DEFAULT_KIND: &str = "simplefin";

/// One `[[sync.connection]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionLink {
    /// UUID text; the keychain item is `ledgeline.sync.<id>`.
    pub id: String,
    /// Display only.
    pub name: String,
    /// The provider, `"simplefin"`.
    pub kind: String,
}

/// One `[[sync.account]]`: a provider account linked to a journal account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountLink {
    /// The [`ConnectionLink::id`] it belongs to.
    pub connection: String,
    /// The provider's account id (`remoteId`).
    pub remote_id: String,
    /// Display only (`label`); defaults to `remote_id`.
    pub label: String,
    /// The hledger account its transactions post to (`journalAccount`).
    pub journal_account: String,
    /// Its rules file, journal-relative.
    pub rules: String,
    /// The journal file new transactions go to, journal-relative.
    pub target: String,
    /// ISO date (`startDate`); rows before it are never imported.
    pub start_date: String,
    /// `1`, or `-1` for a bank that reports balances reversed (`balanceSign`).
    pub balance_sign: i8,
    /// `false` stops syncing it without unlinking.
    pub enabled: bool,
}

impl AccountLink {
    /// The account's stable key: [`account_short`] of its connection and
    /// remote id — also the prefix of every id it writes.
    #[must_use]
    pub fn key(&self) -> String {
        account_short(&self.connection, &self.remote_id)
    }
}

/// One `[[sync.manual]]`: an account kept up to date by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAccount {
    /// The hledger account.
    pub account: String,
    /// Where the user downloads its statements (`downloadUrl`); `http(s)` only.
    pub download_url: Option<String>,
    /// Remind when it goes stale.
    pub nudge: bool,
}

/// Everything under `[sync]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncConfig {
    /// `[[sync.connection]]`, in file order.
    pub connections: Vec<ConnectionLink>,
    /// `[[sync.account]]`, in file order.
    pub accounts: Vec<AccountLink>,
    /// `[[sync.manual]]`, in file order.
    pub manual: Vec<ManualAccount>,
}

impl SyncConfig {
    /// The connection with `id`.
    #[must_use]
    pub fn connection(&self, id: &str) -> Option<&ConnectionLink> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// The linked account whose [`AccountLink::key`] is `key`.
    #[must_use]
    pub fn account(&self, key: &str) -> Option<&AccountLink> {
        self.accounts.iter().find(|a| a.key() == key)
    }
}

/// Why a writer refused to change the file. The file is never half-changed:
/// a writer returns new text or an error, and the caller writes only text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The existing text isn't TOML.
    #[error("ledgeline.toml isn't valid TOML, so Ledgeline won't change it: {0}")]
    InvalidToml(String),
    /// A key Ledgeline writes holds something else (`sync = 3`).
    #[error(
        "ledgeline.toml's `{0}` isn't laid out the way Ledgeline expects, so it won't change it."
    )]
    Shape(String),
    /// The value to write would not survive [`parse`].
    #[error("Can't save this link: {0}")]
    Invalid(String),
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Read the `[sync]` tables. Never fails: problems come back as warnings, and
/// the entries they concern are skipped. A later entry repeating an earlier
/// one's key (connection id, `(connection, remoteId)`, manual `account`) is
/// skipped too.
#[must_use]
pub fn parse(text: &str) -> (SyncConfig, Vec<String>) {
    let document = match text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(error) => {
            return (
                SyncConfig::default(),
                vec![format!(
                    "ledgeline.toml isn't valid TOML, so no bank links were read: {}",
                    first_line(&error.to_string())
                )],
            );
        }
    };
    let Some(sync) = document.get("sync") else {
        return (SyncConfig::default(), Vec::new());
    };
    let Some(sync) = sync.as_table_like() else {
        return (
            SyncConfig::default(),
            vec!["ledgeline.toml: `sync` isn't a table, so no bank links were read.".into()],
        );
    };

    let mut warnings = Vec::new();
    let mut read = |key: &str| -> Vec<&dyn TableLike> {
        match entries(sync, key) {
            Ok(entries) => entries,
            Err(()) => {
                warnings.push(format!(
                    "ledgeline.toml: `sync.{key}` isn't a list of tables, so it was skipped."
                ));
                Vec::new()
            }
        }
    };
    let (connection_tables, account_tables, manual_tables) =
        (read("connection"), read("account"), read("manual"));

    let connections = collect(
        "connection",
        &connection_tables,
        read_connection,
        |c: &ConnectionLink| c.id.clone(),
        &mut warnings,
    );
    let accounts = collect(
        "account",
        &account_tables,
        read_account,
        |a: &AccountLink| format!("{}\u{0}{}", a.connection, a.remote_id),
        &mut warnings,
    );
    let manual = collect(
        "manual",
        &manual_tables,
        |table| read_manual(table),
        |m: &ManualAccount| m.account.clone(),
        &mut warnings,
    );
    (
        SyncConfig {
            connections,
            accounts,
            manual,
        },
        warnings,
    )
}

/// The tables under `sync.<key>`: an array of tables, or an inline array of
/// inline tables. `Err` when it is something else.
fn entries<'a>(sync: &'a dyn TableLike, key: &str) -> Result<Vec<&'a dyn TableLike>, ()> {
    match sync.get(key) {
        None => Ok(Vec::new()),
        Some(Item::ArrayOfTables(tables)) => {
            Ok(tables.iter().map(|t| t as &dyn TableLike).collect())
        }
        Some(Item::Value(Value::Array(array))) => array
            .iter()
            .map(|value| value.as_inline_table().map(|t| t as &dyn TableLike))
            .collect::<Option<Vec<_>>>()
            .ok_or(()),
        Some(_) => Err(()),
    }
}

/// Read every entry, warning about and skipping the malformed and the
/// repeated.
fn collect<T>(
    section: &str,
    tables: &[&dyn TableLike],
    read: impl Fn(&dyn TableLike) -> Result<(T, Vec<String>), String>,
    key: impl Fn(&T) -> String,
    warnings: &mut Vec<String>,
) -> Vec<T> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (at, table) in tables.iter().enumerate() {
        let entry = at + 1;
        match read(*table) {
            Ok((value, notes)) => {
                warnings.extend(
                    notes
                        .into_iter()
                        .map(|note| format!("ledgeline.toml: [[sync.{section}]] #{entry}: {note}")),
                );
                if seen.insert(key(&value)) {
                    out.push(value);
                } else {
                    warnings.push(format!(
                        "ledgeline.toml: [[sync.{section}]] #{entry} repeats an earlier entry, so it was skipped."
                    ));
                }
            }
            Err(reason) => warnings.push(format!(
                "ledgeline.toml: [[sync.{section}]] #{entry} was skipped: {reason}."
            )),
        }
    }
    out
}

/// A text field: `Ok(None)` when absent, `Err` when not text.
fn optional_text(table: &dyn TableLike, key: &str) -> Result<Option<String>, String> {
    match table.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_str()
            .map(|text| Some(text.to_string()))
            .ok_or_else(|| format!("`{key}` isn't text")),
    }
}

/// A text field that must be present, not blank, and free of control
/// characters.
fn required_text(table: &dyn TableLike, key: &str) -> Result<String, String> {
    let text = optional_text(table, key)?.ok_or_else(|| format!("`{key}` is missing"))?;
    check_text(key, &text)?;
    Ok(text)
}

fn optional_bool(table: &dyn TableLike, key: &str, default: bool) -> Result<bool, String> {
    match table.get(key) {
        None => Ok(default),
        Some(item) => item
            .as_bool()
            .ok_or_else(|| format!("`{key}` isn't true or false")),
    }
}

fn read_connection(table: &dyn TableLike) -> Result<(ConnectionLink, Vec<String>), String> {
    let id = required_text(table, "id")?;
    check_connection_id(&id)?;
    let name = optional_text(table, "name")?
        .map(|name| sanitize(&name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "SimpleFIN".to_string());
    let kind = optional_text(table, "kind")?.unwrap_or_else(|| DEFAULT_KIND.to_string());
    check_kind(&kind)?;
    Ok((ConnectionLink { id, name, kind }, Vec::new()))
}

fn read_account(table: &dyn TableLike) -> Result<(AccountLink, Vec<String>), String> {
    let connection = required_text(table, "connection")?;
    check_connection_id(&connection)?;
    let remote_id = required_text(table, "remoteId")?;
    let label = optional_text(table, "label")?
        .map(|label| sanitize(&label))
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| sanitize(&remote_id));
    let start_date = match table.get("startDate") {
        None => return Err("`startDate` is missing".into()),
        Some(item) => item
            .as_datetime()
            .and_then(|datetime| match (datetime.date, datetime.time) {
                (Some(date), None) => Some(format!(
                    "{:04}-{:02}-{:02}",
                    date.year, date.month, date.day
                )),
                _ => None,
            })
            .or_else(|| item.as_str().map(str::to_string))
            .filter(|date| is_iso_date(date))
            .ok_or("`startDate` isn't a date like 2026-11-01")?,
    };
    let balance_sign = match table.get("balanceSign") {
        None => 1,
        Some(item) => match item.as_integer() {
            Some(1) => 1,
            Some(-1) => -1,
            _ => return Err("`balanceSign` isn't 1 or -1".into()),
        },
    };
    Ok((
        AccountLink {
            journal_account: required_text(table, "journalAccount")?,
            rules: required_text(table, "rules")?,
            target: required_text(table, "target")?,
            enabled: optional_bool(table, "enabled", true)?,
            connection,
            remote_id,
            label,
            start_date,
            balance_sign,
        },
        Vec::new(),
    ))
}

fn read_manual(table: &dyn TableLike) -> Result<(ManualAccount, Vec<String>), String> {
    let account = required_text(table, "account")?;
    let mut notes = Vec::new();
    let download_url = match optional_text(table, "downloadUrl")? {
        Some(url) if is_web_url(&url) => Some(url),
        Some(_) => {
            notes.push("`downloadUrl` isn't an http(s) address, so it was ignored.".to_string());
            None
        }
        None => None,
    };
    Ok((
        ManualAccount {
            account,
            download_url,
            nudge: optional_bool(table, "nudge", false)?,
        },
        notes,
    ))
}

// ---------------------------------------------------------------------------
// Validation, shared by reading and writing
// ---------------------------------------------------------------------------

/// Not blank, and no control characters (a newline would end the TOML string
/// as the user sees it, and the value ends up in rules and journals).
fn check_text(key: &str, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        Err(format!("`{key}` is empty"))
    } else if text.chars().any(char::is_control) {
        Err(format!("`{key}` contains control characters"))
    } else {
        Ok(())
    }
}

/// 1–64 of `[A-Za-z0-9_-]`: safe inside the keychain key
/// `ledgeline.sync.<id>`, whatever a hostile file says.
fn check_connection_id(id: &str) -> Result<(), String> {
    let safe = (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'));
    if safe {
        Ok(())
    } else {
        Err("the connection id isn't 1–64 letters, digits, `-` or `_`".into())
    }
}

/// 1–32 of `[a-z0-9-]`.
fn check_kind(kind: &str) -> Result<(), String> {
    let plain = (1..=32).contains(&kind.len())
        && kind
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if plain {
        Ok(())
    } else {
        Err("`kind` isn't a provider name".into())
    }
}

/// A real calendar date written `YYYY-MM-DD`.
fn is_iso_date(text: &str) -> bool {
    text.len() == 10 && iso_from_days(days_from_iso(text)) == text
}

fn is_web_url(text: &str) -> bool {
    url::Url::parse(text)
        .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.host_str().is_some())
}

fn first_line(text: &str) -> &str {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(text)
}

fn invalid(reason: String) -> ConfigError {
    ConfigError::Invalid(reason)
}

fn validate_connection(c: &ConnectionLink) -> Result<(), ConfigError> {
    check_connection_id(&c.id)
        .and_then(|()| check_text("name", &c.name))
        .and_then(|()| check_kind(&c.kind))
        .map_err(invalid)
}

fn validate_account(a: &AccountLink) -> Result<(), ConfigError> {
    check_connection_id(&a.connection)
        .and_then(|()| check_text("remoteId", &a.remote_id))
        .and_then(|()| check_text("label", &a.label))
        .and_then(|()| check_text("journalAccount", &a.journal_account))
        .and_then(|()| check_text("rules", &a.rules))
        .and_then(|()| check_text("target", &a.target))
        .and_then(|()| {
            if is_iso_date(&a.start_date) {
                Ok(())
            } else {
                Err("`startDate` isn't a date like 2026-11-01".into())
            }
        })
        .and_then(|()| {
            if matches!(a.balance_sign, 1 | -1) {
                Ok(())
            } else {
                Err("`balanceSign` isn't 1 or -1".into())
            }
        })
        .map_err(invalid)
}

fn validate_manual(m: &ManualAccount) -> Result<(), ConfigError> {
    check_text("account", &m.account)
        .and_then(|()| match &m.download_url {
            Some(url) if !is_web_url(url) => Err("`downloadUrl` isn't an http(s) address".into()),
            _ => Ok(()),
        })
        .map_err(invalid)
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Add the connection, or update the one with the same `id` in place.
///
/// # Errors
/// [`ConfigError`] when the text isn't TOML, `[sync]` has an unexpected
/// shape, or `c` wouldn't survive [`parse`].
pub fn upsert_connection(text: &str, c: &ConnectionLink) -> Result<String, ConfigError> {
    validate_connection(c)?;
    edit_array(text, "connection", |tables| {
        upsert_table(
            tables,
            |t| str_of(t, "id") == Some(c.id.as_str()),
            |table| {
                set(table, "id", c.id.as_str());
                set(table, "name", c.name.as_str());
                set(table, "kind", c.kind.as_str());
            },
        );
    })
}

/// Add the account link, or update the one with the same
/// `(connection, remoteId)` in place.
///
/// # Errors
/// As [`upsert_connection`].
pub fn upsert_account(text: &str, a: &AccountLink) -> Result<String, ConfigError> {
    validate_account(a)?;
    let date =
        toml_date(&a.start_date).ok_or_else(|| invalid("`startDate` isn't a date".into()))?;
    edit_array(text, "account", |tables| {
        upsert_table(
            tables,
            |t| is_account(t, &a.connection, &a.remote_id),
            |table| {
                set(table, "connection", a.connection.as_str());
                set(table, "remoteId", a.remote_id.as_str());
                set(table, "label", a.label.as_str());
                set(table, "journalAccount", a.journal_account.as_str());
                set(table, "rules", a.rules.as_str());
                set(table, "target", a.target.as_str());
                set(table, "startDate", date);
                set(table, "balanceSign", i64::from(a.balance_sign));
                set(table, "enabled", a.enabled);
            },
        );
    })
}

/// Remove the link for `(connection, remote_id)`. Removing one that isn't
/// there returns the text unchanged.
///
/// # Errors
/// As [`upsert_connection`].
pub fn remove_account(
    text: &str,
    connection: &str,
    remote_id: &str,
) -> Result<String, ConfigError> {
    remove_where(text, "account", |t| is_account(t, connection, remote_id))
}

/// Remove the connection with `id`. Its accounts stay: they read as "not
/// connected" until a connection with that id comes back.
///
/// # Errors
/// As [`upsert_connection`].
pub fn remove_connection(text: &str, id: &str) -> Result<String, ConfigError> {
    remove_where(text, "connection", |t| str_of(t, "id") == Some(id))
}

/// Add the manual account, or update the one with the same `account` in
/// place. `download_url: None` removes the key.
///
/// # Errors
/// As [`upsert_connection`].
pub fn upsert_manual(text: &str, m: &ManualAccount) -> Result<String, ConfigError> {
    validate_manual(m)?;
    edit_array(text, "manual", |tables| {
        upsert_table(
            tables,
            |t| str_of(t, "account") == Some(m.account.as_str()),
            |table| {
                set(table, "account", m.account.as_str());
                match &m.download_url {
                    Some(url) => set(table, "downloadUrl", url.as_str()),
                    None => {
                        table.remove("downloadUrl");
                    }
                }
                set(table, "nudge", m.nudge);
            },
        );
    })
}

fn str_of<'a>(table: &'a Table, key: &str) -> Option<&'a str> {
    table.get(key).and_then(Item::as_str)
}

fn is_account(table: &Table, connection: &str, remote_id: &str) -> bool {
    str_of(table, "connection") == Some(connection) && str_of(table, "remoteId") == Some(remote_id)
}

fn toml_date(iso: &str) -> Option<Datetime> {
    let year = iso.get(0..4)?.parse().ok()?;
    let month = iso.get(5..7)?.parse().ok()?;
    let day = iso.get(8..10)?.parse().ok()?;
    Some(Datetime {
        date: Some(Date { year, month, day }),
        time: None,
        offset: None,
    })
}

/// Set `key` to `value`, keeping the old value's decor (its trailing comment
/// and spacing) and its place in the table.
fn set(table: &mut Table, key: &str, value: impl Into<Value>) {
    let mut value = value.into();
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    table[key] = Item::Value(value);
}

/// Run `fill` on the first table `matches`, or on a new one pushed at the
/// end. A new table takes its place in the file right after the array's last
/// entry (`toml_edit` places an unpositioned table after the one before it).
fn upsert_table(
    tables: &mut ArrayOfTables,
    matches: impl Fn(&Table) -> bool,
    fill: impl FnOnce(&mut Table),
) {
    if let Some(table) = tables.iter_mut().find(|t| matches(t)) {
        fill(table);
        return;
    }
    let mut table = Table::new();
    fill(&mut table);
    tables.push(table);
}

fn parse_document(text: &str) -> Result<DocumentMut, ConfigError> {
    text.parse::<DocumentMut>()
        .map_err(|error| ConfigError::InvalidToml(first_line(&error.to_string()).to_string()))
}

/// Run `edit` on `sync.<key>`, creating `[sync]` (implicit, so no bare
/// `[sync]` header appears) and the array when missing.
fn edit_array(
    text: &str,
    key: &str,
    edit: impl FnOnce(&mut ArrayOfTables),
) -> Result<String, ConfigError> {
    let mut document = parse_document(text)?;
    let sync = document
        .as_table_mut()
        .entry("sync")
        .or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| ConfigError::Shape("sync".into()))?;
    let tables = sync
        .entry(key)
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| ConfigError::Shape(format!("sync.{key}")))?;
    edit(tables);
    Ok(document.to_string())
}

/// Remove every `sync.<key>` table `matches`. Text with nothing to remove comes
/// back unchanged, byte for byte.
fn remove_where(
    text: &str,
    key: &str,
    matches: impl Fn(&Table) -> bool,
) -> Result<String, ConfigError> {
    let mut document = parse_document(text)?;
    let Some(sync) = document.get_mut("sync") else {
        return Ok(text.to_string());
    };
    let sync = sync
        .as_table_mut()
        .ok_or_else(|| ConfigError::Shape("sync".into()))?;
    let Some(tables) = sync.get_mut(key) else {
        return Ok(text.to_string());
    };
    let tables = tables
        .as_array_of_tables_mut()
        .ok_or_else(|| ConfigError::Shape(format!("sync.{key}")))?;
    if !tables.iter().any(&matches) {
        return Ok(text.to_string());
    }
    tables.retain(|table| !matches(table));
    Ok(document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> AccountLink {
        AccountLink {
            connection: "4f6c1a2e-0000-4000-8000-000000000001".into(),
            remote_id: "ACT-12ab".into(),
            label: "First Tech · Checking ••1234".into(),
            journal_account: "assets:bank:firsttech:checking".into(),
            rules: "imports/rules/firsttech-checking.sync.csv.rules".into(),
            target: "2026.journal".into(),
            start_date: "2026-11-01".into(),
            balance_sign: 1,
            enabled: true,
        }
    }

    #[test]
    fn iso_dates_are_checked_for_real() {
        assert!(is_iso_date("2026-11-01"));
        assert!(is_iso_date("2024-02-29"));
        for bad in [
            "2026-02-30",
            "2025-02-29",
            "2026-13-01",
            "2026-1-01",
            "",
            "abcdefghij",
            "2026/11/01",
        ] {
            assert!(!is_iso_date(bad), "{bad}");
        }
    }

    #[test]
    fn connection_ids_must_be_keychain_safe() {
        assert!(check_connection_id("4f6c1a2e-0000-4000-8000-000000000001").is_ok());
        for bad in ["", "a b", "../x", "x.y", &"a".repeat(65), "ünï"] {
            assert!(check_connection_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_key_is_account_short() {
        assert_eq!(link().key(), account_short(&link().connection, "ACT-12ab"));
        assert_eq!(link().key(), "2562b58c16");
    }

    #[test]
    fn writers_refuse_values_parse_would_skip() {
        let bad_date = AccountLink {
            start_date: "2026-02-30".into(),
            ..link()
        };
        assert!(matches!(
            upsert_account("", &bad_date),
            Err(ConfigError::Invalid(_))
        ));
        let bad_sign = AccountLink {
            balance_sign: 0,
            ..link()
        };
        assert!(matches!(
            upsert_account("", &bad_sign),
            Err(ConfigError::Invalid(_))
        ));
        let newline = AccountLink {
            label: "a\nb".into(),
            ..link()
        };
        assert!(matches!(
            upsert_account("", &newline),
            Err(ConfigError::Invalid(_))
        ));
        let manual = ManualAccount {
            account: "assets:x".into(),
            download_url: Some("javascript:alert(1)".into()),
            nudge: true,
        };
        assert!(matches!(
            upsert_manual("", &manual),
            Err(ConfigError::Invalid(_))
        ));
        let connection = ConnectionLink {
            id: "has space".into(),
            name: "x".into(),
            kind: "simplefin".into(),
        };
        assert!(matches!(
            upsert_connection("", &connection),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn writers_refuse_unexpected_shapes() {
        assert!(matches!(
            upsert_account("sync = 3\n", &link()),
            Err(ConfigError::Shape(_))
        ));
        assert!(matches!(
            upsert_account("[sync]\naccount = 3\n", &link()),
            Err(ConfigError::Shape(_))
        ));
        assert!(matches!(
            upsert_account("not = [toml", &link()),
            Err(ConfigError::InvalidToml(_))
        ));
        assert!(matches!(
            remove_account("sync = 3\n", "a", "b"),
            Err(ConfigError::Shape(_))
        ));
    }
}
