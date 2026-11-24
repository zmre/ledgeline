//! A minimal new customer note (ARCHITECTURE §3.4), for the invoice
//! editor's inline "Add new customer" (invoicer PLAN 4a). Phase 4b's
//! customer editor writes the full contact schema; this writes only what an
//! invoice needs to be sent: a person or organization note in
//! `customers-dir`, tagged with the customer tag, with a billing email,
//! address and terms when given.
//!
//! The note is created exclusively (never over a file already there) and
//! verified to read back as a customer before the index sees it.

use crate::customers::{self, Customer};
use crate::frontmatter::{self, EditError};
use crate::index::DocumentIndex;
use crate::notes::{Note, Stamp};
use crate::settings::Settings;
use ledgeline_core::vault::{Fingerprint, Vault, VaultError, VaultPath};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

/// Person or organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// `type: person`.
    Person,
    /// `type: organization`.
    Organization,
}

/// What to write.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewCustomer {
    /// The title (and file name).
    pub name: String,
    /// Person or organization.
    pub kind: Kind,
    /// The billing email.
    #[serde(default)]
    pub email: Option<String>,
    /// The billing address, as lines of text.
    #[serde(default)]
    pub address: Option<String>,
    /// Net days.
    #[serde(default)]
    pub terms: Option<u32>,
}

/// Why no note was written.
#[derive(Debug, Error)]
pub enum CustomerNoteError {
    /// The name can't be a title or a file name.
    #[error("{0}")]
    Invalid(String),
    /// A note with that name is already there.
    #[error("{0} already exists")]
    Exists(VaultPath),
    /// The frontmatter wouldn't write.
    #[error(transparent)]
    Frontmatter(#[from] EditError),
    /// The vault refused.
    #[error(transparent)]
    Vault(#[from] VaultError),
}

fn clean(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The note's text and path for `new`.
///
/// # Errors
/// [`CustomerNoteError::Invalid`], [`CustomerNoteError::Frontmatter`].
pub fn render(
    settings: &Settings,
    new: &NewCustomer,
) -> Result<(VaultPath, String), CustomerNoteError> {
    let name = new.name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(CustomerNoteError::Invalid(
            "the customer needs a name".into(),
        ));
    }
    if name.starts_with('.')
        || name.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '[' | ']' | '#'
                )
        })
    {
        return Err(CustomerNoteError::Invalid(format!(
            "{name:?} can't be a note's name (no / \\ : * ? \" < > | [ ] #, nor a leading dot)"
        )));
    }
    let dir = Settings::vault_path(&settings.customers_dir)
        .map_err(|e| CustomerNoteError::Invalid(e.to_string()))?;
    let path = dir
        .join(&format!("{name}.md"))
        .map_err(|e| CustomerNoteError::Invalid(e.to_string()))?;
    let mut fields: Vec<(&str, Value)> = vec![
        ("title", Value::String(name.clone())),
        (
            "type",
            Value::from(match new.kind {
                Kind::Person => "person",
                Kind::Organization => "organization",
            }),
        ),
        ("tags", json!([settings.words.customer_tag])),
    ];
    let billing = settings.words.billing_labels.first().to_string();
    if let Some(email) = clean(new.email.as_deref()) {
        fields.push(("emails", json!([{ billing.clone(): email }])));
    }
    if let Some(address) = clean(new.address.as_deref()) {
        let address = if address.contains('\n') {
            format!("{address}\n")
        } else {
            address
        };
        fields.push(("addresses", json!([{ billing: address }])));
    }
    if let Some(terms) = new.terms {
        fields.push(("terms", Value::Number(terms.into())));
    }
    Ok((path, frontmatter::render_block(&fields)?))
}

/// Write `new` as a customer note and index it. Returns the note's path and
/// the customer as the index now reads it.
///
/// # Errors
/// [`CustomerNoteError`]; nothing is written then.
pub fn create(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    new: &NewCustomer,
) -> Result<(VaultPath, Customer), CustomerNoteError> {
    let settings = index.settings();
    let (path, text) = render(settings, new)?;
    let stamp = Stamp {
        modified: None,
        size: text.len() as u64,
        fingerprint: Fingerprint::of_bytes(text.as_bytes()),
    };
    let note = Note::from_source(path.clone(), &text, stamp);
    let customer = customers::is_customer(&note, &settings.words)
        .then(|| Customer::from_note(&note))
        .flatten()
        .ok_or_else(|| {
            CustomerNoteError::Invalid("the note wouldn't read back as a customer".into())
        })?;
    if let Some(existing) = index
        .site()
        .notes()
        .find(|n| n.title.eq_ignore_ascii_case(&customer.contact.title))
    {
        return Err(CustomerNoteError::Exists(existing.path.clone()));
    }
    if let Some(dir) = path.parent() {
        vault.create_dir_all(&dir)?;
    }
    vault
        .create_new_shared(&path, text.as_bytes())
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                CustomerNoteError::Exists(path.clone())
            } else {
                CustomerNoteError::Vault(e)
            }
        })?;
    index.refresh(vault, std::slice::from_ref(&path));
    Ok((path, customer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Money;
    use ledgeline_core::vault::MemVault;

    #[test]
    fn a_new_customer_reads_back_and_bills() {
        let vault = MemVault::new();
        let mut index = DocumentIndex::open(&vault, Settings::default(), Money::default());
        let (path, customer) = create(
            &mut index,
            &vault,
            &NewCustomer {
                name: "  Acme   Corp ".into(),
                kind: Kind::Organization,
                email: Some("ap@acme.example".into()),
                address: Some("1 Road\nTown, ST 00000".into()),
                terms: Some(45),
            },
        )
        .expect("creates");
        assert_eq!(path.as_str(), "customers/Acme Corp.md");
        assert_eq!(customer.terms, Some(45));
        let found = customers::find_customer(
            index.site(),
            &VaultPath::new("invoices/x.md").expect("path"),
            "[[Acme Corp]]",
            &index.settings().words,
        )
        .expect("a customer");
        let bill_to = found.billing_contact(index.site(), &index.settings().words);
        assert_eq!(bill_to.email.as_deref(), Some("ap@acme.example"));
        assert_eq!(bill_to.address.as_deref(), Some("1 Road\nTown, ST 00000"));

        let again = create(
            &mut index,
            &vault,
            &NewCustomer {
                name: "acme corp".into(),
                kind: Kind::Person,
                email: None,
                address: None,
                terms: None,
            },
        );
        assert!(matches!(again, Err(CustomerNoteError::Exists(_))));
        for bad in ["", "a/b", "[[x]]", ".hidden"] {
            let refused = render(
                index.settings(),
                &NewCustomer {
                    name: bad.into(),
                    kind: Kind::Person,
                    email: None,
                    address: None,
                    terms: None,
                },
            );
            assert!(
                matches!(refused, Err(CustomerNoteError::Invalid(_))),
                "{bad}"
            );
        }
    }
}
