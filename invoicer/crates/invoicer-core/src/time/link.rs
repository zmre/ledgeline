//! What links the time book to the Invoicer's notes.
//!
//! - A time account's `customer:` (nearest ancestor) is a customer's
//!   `ledger_customer`, the books' `customer:` tag value (ARCHITECTURE §3.4),
//!   falling back to the note's title.
//! - A customer note's time keys (plan 46 B-unbilled, B-invoice, B-round,
//!   B-rates): `cadence`, `time_detail`, `time_increment`, `time_rounding`,
//!   `time_round_scope`, `time_items`.
//! - A **timekeeper** (T18) is a `type: person` or `type: organization` note
//!   tagged `timekeeper`: whose time a card is (`time_person`, else the
//!   title's short form) and who is paid for it (`pay_rate`, `vendor`,
//!   `expense_account`, `payable_account`, `pay_terms`, `pay_to`).
//!
//! Every key is read with the frontmatter machinery customers use, so a note
//! reads the same here as in MBR.

use super::TIMEKEEPER_TAG;
use super::round::Rounding;
use crate::customers::{ContactKind, Customer, find_customer};
use crate::index::DocumentIndex;
use crate::notes::{Note, Site};
use ledgeline_core::time::book::suggest_person;
use ledgeline_core::time::model::is_under;
use ledgeline_core::time::query::Cadence;
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use serde_json::Value;

/// How much of the time an invoice shows beside its bill table
/// (`time_detail`, B-invoice step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Detail {
    /// No timesheet.
    None,
    /// One row per day and person (the default).
    #[default]
    Daily,
    /// One row per entry.
    Entries,
}

impl Detail {
    /// `none`, `daily`, `entries`; anything else is `None` (the caller's
    /// default applies).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "daily" => Some(Self::Daily),
            "entries" => Some(Self::Entries),
            _ => None,
        }
    }
}

/// One `time_items` rule (B-rates): time on `account` (or below it), by
/// `person` when given, bills as `item`. The longest account wins, and a
/// rule naming the person beats one that doesn't.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemRule {
    /// The account (a branch).
    pub account: String,
    /// The person, when the rule is theirs.
    pub person: Option<String>,
    /// The catalog code.
    pub item: String,
}

/// A customer with its time keys.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CustomerTime {
    /// The customer.
    #[serde(skip)]
    pub customer: Customer,
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// The `customer:` value time accounts link it by.
    pub ledger_customer: String,
    /// `cadence`, as written.
    pub cadence: Option<String>,
    /// `time_detail` (default daily).
    pub detail: Detail,
    /// `time_increment`, `time_rounding`, `time_round_scope`.
    pub rounding: Rounding,
    /// `time_items`.
    pub items: Vec<ItemRule>,
}

impl CustomerTime {
    /// The cadence, read.
    #[must_use]
    pub fn cadence(&self) -> Cadence {
        Cadence::parse(self.cadence.as_deref())
    }

    /// The `time_items` rule for `account` and `person` (B-rates), if any.
    #[must_use]
    pub fn item_rule(&self, account: &str, person: Option<&str>) -> Option<&ItemRule> {
        self.items
            .iter()
            .filter(|rule| is_under(account, &rule.account))
            .filter(|rule| rule.person.is_none() || rule.person.as_deref() == person)
            .max_by_key(|rule| (rule.account.len(), rule.person.is_some()))
    }
}

/// A frontmatter value as text (a number reads as its digits).
pub(crate) fn field_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A whole number of minutes or days, written as a number or text.
fn field_whole(value: Option<&Value>) -> Option<u32> {
    match value? {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn item_rules(value: Option<&Value>) -> Vec<ItemRule> {
    let Some(Value::Array(rules)) = value else {
        return Vec::new();
    };
    rules
        .iter()
        .filter_map(|rule| {
            let map = rule.as_object()?;
            Some(ItemRule {
                account: field_text(map.get("account"))?,
                person: field_text(map.get("person")),
                item: field_text(map.get("item"))?,
            })
        })
        .collect()
}

/// The time keys of `customer`'s note.
#[must_use]
pub fn customer_time(site: &Site, customer: &Customer) -> CustomerTime {
    let note = site.note(&customer.contact.path);
    let get = |key: &str| note.and_then(|n| n.frontmatter.get(key));
    CustomerTime {
        path: customer.contact.path.clone(),
        title: customer.contact.title.clone(),
        ledger_customer: customer.ledger_customer.clone(),
        cadence: field_text(get("cadence")),
        detail: field_text(get("time_detail"))
            .and_then(|d| Detail::parse(&d))
            .unwrap_or_default(),
        rounding: Rounding::of_fields(
            field_whole(get("time_increment")),
            field_text(get("time_rounding")).as_deref(),
            field_text(get("time_round_scope")).as_deref(),
        ),
        items: item_rules(get("time_items")),
        customer: customer.clone(),
    }
}

/// The customer a time account's `customer:` names: the customer whose
/// `ledger_customer` (or title) it is, else the one its `[[link]]` or title
/// resolves to.
#[must_use]
pub fn customer_named(index: &DocumentIndex, name: &str) -> Option<Customer> {
    let name = name.trim();
    let customers = index.customers();
    customers
        .iter()
        .find(|c| c.ledger_customer == name)
        .or_else(|| {
            customers
                .iter()
                .find(|c| c.contact.title.eq_ignore_ascii_case(name))
        })
        .cloned()
        .or_else(|| {
            find_customer(
                index.site(),
                &VaultPath::root(),
                name,
                &index.settings().words,
            )
            .ok()
        })
}

/// A timekeeper note (T18).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timekeeper {
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// Person or organization.
    pub kind: ContactKind,
    /// The `person:` value its time cards carry: `time_person`, else the
    /// title's short form as P-person makes it (`Riley Chen` → `riley`).
    pub person: String,
    /// `pay_rate` (money, as written).
    pub pay_rate: Option<String>,
    /// `vendor`: the books' `vendor:` tag; defaults to the title.
    pub vendor: Option<String>,
    /// `expense_account`.
    pub expense_account: Option<String>,
    /// `payable_account`.
    pub payable_account: Option<String>,
    /// `pay_terms`, in days.
    pub pay_terms: Option<u32>,
    /// `pay_to`: another note who is paid instead (a contractor's company).
    pub pay_to: Option<String>,
}

impl Timekeeper {
    /// The timekeeper fields of `note`, when it is a person or organization
    /// note (the tag is not checked: see [`is_timekeeper`]).
    #[must_use]
    pub fn from_note(note: &Note) -> Option<Self> {
        let kind = if note.is_type("person") {
            ContactKind::Person
        } else if note.is_type("organization") {
            ContactKind::Organization
        } else {
            return None;
        };
        let get = |key: &str| field_text(note.frontmatter.get(key));
        Some(Self {
            path: note.path.clone(),
            title: note.title.clone(),
            kind,
            person: get("time_person")
                .or_else(|| suggest_person(Some(&note.title), None))
                .unwrap_or_default(),
            pay_rate: get("pay_rate"),
            vendor: get("vendor"),
            expense_account: get("expense_account"),
            payable_account: get("payable_account"),
            pay_terms: field_whole(note.frontmatter.get("pay_terms")),
            pay_to: get("pay_to"),
        })
    }

    /// The vendor name the books know this payee by.
    #[must_use]
    pub fn vendor_name(&self) -> &str {
        self.vendor.as_deref().unwrap_or(&self.title)
    }
}

/// Whether `note` is a timekeeper: a person or organization note tagged
/// [`TIMEKEEPER_TAG`].
#[must_use]
pub fn is_timekeeper(note: &Note) -> bool {
    (note.is_type("person") || note.is_type("organization")) && note.has_tag(TIMEKEEPER_TAG)
}

/// Every timekeeper note, by path.
#[must_use]
pub fn timekeepers(site: &Site) -> Vec<Timekeeper> {
    site.notes()
        .filter(|note| is_timekeeper(note))
        .filter_map(Timekeeper::from_note)
        .collect()
}

/// The timekeeper whose time cards carry `person:`.
#[must_use]
pub fn timekeeper_for(site: &Site, person: &str) -> Option<Timekeeper> {
    timekeepers(site)
        .into_iter()
        .find(|keeper| keeper.person == person)
}

/// The note `link` (a `[[wikilink]]` or a title) names, as a timekeeper,
/// tagged or not: the bill dialog's *Pick a note*, and `pay_to`.
#[must_use]
pub fn note_as_timekeeper(site: &Site, from: &VaultPath, link: &str) -> Option<Timekeeper> {
    site.resolve_wikilink(from, link)
        .and_then(Timekeeper::from_note)
}
