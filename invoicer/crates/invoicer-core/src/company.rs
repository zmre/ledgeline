//! Your company's details on a document: `invoicer` and the `invoicer_*`
//! keys (ARCHITECTURE §3.2, PLAN D18).
//!
//! **Drafts follow the company.** A draft's `invoicer_*` keys are derived:
//! wherever a draft is shown (the app's page, MBR's page, `show`), previewed
//! or exported, it carries the company note's details as they are now
//! ([`as_shown`]), whatever its file says. Nothing is written for that: the
//! file keeps the copy `new` made until something writes it.
//!
//! **Sending freezes the copy.** Send writes the company's current details
//! into the file with `status: sent` ([`edits`], in
//! [`InvoiceBooks::send`](crate::books::InvoiceBooks::send)), before any PDF
//! is kept or hashed, so the sent copy is what the customer got. A hand-written
//! `invoicer_*` value in a draft is overwritten then.
//!
//! **Sent invoices keep what was sent.** A later change to the company note
//! never reaches them on its own; [`differs`] says when it would, and
//! [`update`] (Update from company, `update-company`) writes it on request.
//!
//! With no `[invoicer] company`, or one naming no note, nothing follows and
//! nothing is written: every document keeps the copy it has.

use std::borrow::Cow;

use crate::customers;
use crate::doc::{Invoice, Status};
use crate::draft::{self, block_text};
use crate::edit::{SetOutcome, WriteError};
use crate::frontmatter::{self, FieldEdit, Fields};
use crate::index::DocumentIndex;
use crate::notes::Site;
use crate::settings::Settings;
use ledgeline_core::vault::{Revision, Vault, VaultPath};
use serde_json::Value;

/// Every key the company's details are written under, in the order `new`
/// writes them.
pub const KEYS: [&str; 6] = [
    "invoicer",
    "invoicer_name",
    "invoicer_email",
    "invoicer_phone",
    "invoicer_address",
    "invoicer_logo",
];

/// The company's details as `new` writes them, from its note as it is now:
/// the link, the name, and the billing email, phone and address and the
/// logo when it has them. `None` when there is no company.
#[must_use]
pub fn fields(
    site: &Site,
    settings: &Settings,
    vault: &dyn Vault,
) -> Option<Vec<(&'static str, Value)>> {
    let company = customers::company(site, settings)?;
    let labels = &settings.words.billing_labels;
    let text = |s: &str| Value::String(s.to_string());
    let mut fields = vec![
        ("invoicer", text(&format!("[[{}]]", company.title))),
        ("invoicer_name", text(&company.title)),
    ];
    fields.extend(company.email(labels).map(|e| ("invoicer_email", text(e))));
    fields.extend(company.phone(labels).map(|p| ("invoicer_phone", text(p))));
    fields.extend(
        company
            .address(labels)
            .map(|a| ("invoicer_address", block_text(a))),
    );
    // Rooted, so the page finds it from any folder (MBR serves the books at `/`).
    fields.extend(
        customers::company_logo(site, vault, &company)
            .map(|p| ("invoicer_logo", text(&format!("/{}", p.as_str())))),
    );
    Some(fields)
}

/// A value as text, for comparing: a string without its trailing line
/// breaks (a `|` block keeps one, `|-` none), a number or flag as written.
fn as_text(value: &Value) -> Option<Cow<'_, str>> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(Cow::Borrowed(s.trim_end_matches(['\n', '\r']))),
        other => Some(Cow::Owned(other.to_string())),
    }
}

/// The edits that make `current` (a document's frontmatter) carry
/// `company`: a set for each key whose value differs, a removal for each
/// company key the company no longer gives. Empty when it already does.
#[must_use]
pub fn edits(company: &[(&str, Value)], current: &Fields) -> Vec<FieldEdit> {
    let sets = company
        .iter()
        .filter(|(key, value)| current.get(*key).and_then(as_text) != as_text(value))
        .map(|(key, value)| FieldEdit::Set((*key).to_string(), value.clone()));
    let removals = KEYS
        .iter()
        .filter(|key| current.contains_key(**key) && !company.iter().any(|(k, _)| k == *key))
        .map(|key| FieldEdit::Remove((*key).to_string()));
    sets.chain(removals).collect()
}

/// [`edits`] for `invoice` against the company as it is now; `None` when
/// there is no company.
fn edits_now(
    index: &DocumentIndex,
    vault: &dyn Vault,
    invoice: &Invoice,
) -> Option<Vec<FieldEdit>> {
    fields(index.site(), index.settings(), vault)
        .map(|company| edits(&company, &invoice.frontmatter().fields))
}

/// `source` with its copy of the company made current: `Ok(None)` when there
/// is no company or it is current already.
///
/// # Errors
/// [`WriteError::Frontmatter`] when its frontmatter can't be edited safely.
pub fn current_text(
    index: &DocumentIndex,
    vault: &dyn Vault,
    source: &str,
) -> Result<Option<(String, Vec<FieldEdit>)>, WriteError> {
    let invoice = Invoice::parse(source, &index.settings().words);
    match edits_now(index, vault, &invoice) {
        Some(edits) if !edits.is_empty() => {
            let text = frontmatter::edit(source, &edits)?;
            Ok(Some((text, edits)))
        }
        _ => Ok(None),
    }
}

/// A document as it is shown, previewed and exported: a draft carries the
/// company's details as they are now; anything else (and a draft whose
/// frontmatter can't be edited safely) is as written.
#[must_use]
pub fn as_shown<'a>(index: &DocumentIndex, vault: &dyn Vault, source: &'a str) -> Cow<'a, str> {
    let is_draft = Invoice::parse(source, &index.settings().words)
        .header()
        .status
        == Some(Status::Draft);
    if !is_draft {
        return Cow::Borrowed(source);
    }
    match current_text(index, vault, source) {
        Ok(Some((text, _))) => Cow::Owned(text),
        _ => Cow::Borrowed(source),
    }
}

/// Whether `invoice` is sent and its copy of the company differs from the
/// company note now: the hint offering Update from company. Never for a
/// draft (it follows), a void invoice (history) or books with no company.
#[must_use]
pub fn differs(index: &DocumentIndex, vault: &dyn Vault, invoice: &Invoice) -> bool {
    invoice.header().status == Some(Status::Sent)
        && edits_now(index, vault, invoice).is_some_and(|edits| !edits.is_empty())
}

/// **Update from company**: write the company's current details into the
/// document at `path`, read at `revision` as `source`. Only the company keys
/// change; every other byte stays. `sent_hash` is left as it was, as for
/// every edit, so a sent invoice's next export says it differs from what
/// the customer has. Re-booking a sent invoice is the caller's (the app's
/// action and `update-company` check it before and run it after).
///
/// # Errors
/// [`WriteError`]: the frontmatter can't be edited safely, or the file
/// changed since it was read.
pub fn update(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    source: &str,
    revision: &Revision,
) -> Result<SetOutcome, WriteError> {
    let found = current_text(index, vault, source)?;
    let changed: Vec<String> = found
        .as_ref()
        .map(|(_, edits)| {
            edits
                .iter()
                .map(|e| match e {
                    FieldEdit::Set(key, _) | FieldEdit::Remove(key) => key.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let written = match &found {
        Some((text, _)) => {
            draft::save_fields(index, vault, path, source, text, revision)?;
            true
        }
        None => false,
    };
    let doc = index.doc(path);
    Ok(SetOutcome {
        path: path.clone(),
        number: doc.and_then(|d| d.number()).map(str::to_string),
        changed,
        written,
        problems: doc.map(|d| d.problems.clone()).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests;
