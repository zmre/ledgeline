//! Customers (ARCHITECTURE §3.4): MBR person and organization notes tagged
//! `customer` (the `[invoicer.words] customer-tag`), with the invoicing keys
//! on top of MBR's contact schema.
//!
//! The contact fields follow `mbr-apple-contact-sync`'s markdown schema:
//! `emails`, `phones` and `addresses` are lists of single-key maps
//! (`- billing: …`; MBR's contact card reads `phones:` the same way),
//! and an address is either text or a map of `street`, `city`, `region`,
//! `postcode`, `country`. Plain strings and a single map are read too, since
//! people write those by hand.

use crate::frontmatter::Fields;
use crate::notes::{Note, Site, resolve_relative, wikilink_target};
use crate::problem::{Problem, ProblemCode};
use crate::settings::Settings;
use crate::table::words::{WordList, Words};
use ledgeline_core::vault::{Vault, VaultPath};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A contact note's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContactKind {
    /// `type: person`.
    Person,
    /// `type: organization`.
    Organization,
}

impl ContactKind {
    /// The `type:` value it is written as.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Organization => "organization",
        }
    }
}

/// One labelled value (`- billing: ar@x.com`); unlabelled when written as a
/// plain string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labelled {
    /// The label, when there is one.
    #[serde(default)]
    pub label: Option<String>,
    /// The value.
    pub value: String,
}

/// A person or organization note's contact fields.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contact {
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// Person or organization.
    pub kind: ContactKind,
    /// `emails:`.
    pub emails: Vec<Labelled>,
    /// `phones:` (`- work: "+1 303 555 0100"`), as written.
    pub phones: Vec<Labelled>,
    /// `addresses:`, each formatted as lines of text.
    pub addresses: Vec<Labelled>,
    /// `company:`, as written (usually `"[[Org]]"`).
    pub company: Option<String>,
}

/// Format a structured address: street, `city, region postcode`, country.
fn address_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim_end_matches('\n').to_string()),
        Value::Object(map) => {
            let get = |k: &str| {
                map.get(k)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            };
            let city_line = [
                get("city").map(str::to_string),
                match (get("region"), get("postcode")) {
                    (Some(r), Some(p)) => Some(format!("{r} {p}")),
                    (r, p) => r.or(p).map(str::to_string),
                },
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
            let lines: Vec<String> = [
                get("street").map(str::to_string),
                Some(city_line),
                get("country").map(str::to_string),
            ]
            .into_iter()
            .flatten()
            .filter(|line| !line.is_empty())
            .collect();
            (!lines.is_empty()).then(|| lines.join("\n"))
        }
        _ => None,
    }
}

/// A list of labelled values, read leniently.
fn labelled(value: Option<&Value>, text: fn(&Value) -> Option<String>) -> Vec<Labelled> {
    let one = |label: Option<&str>, v: &Value| {
        text(v).map(|value| Labelled {
            label: label.map(str::to_string),
            value,
        })
    };
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .flat_map(|item| match item {
                Value::Object(map) => map.iter().filter_map(|(k, v)| one(Some(k), v)).collect(),
                other => one(None, other).into_iter().collect::<Vec<_>>(),
            })
            .collect(),
        Some(Value::Object(map))
            if text(&Value::Object(map.clone())).is_some() && map.contains_key("street") =>
        {
            one(None, &Value::Object(map.clone())).into_iter().collect()
        }
        Some(Value::Object(map)) => map.iter().filter_map(|(k, v)| one(Some(k), v)).collect(),
        Some(other) => one(None, other).into_iter().collect(),
    }
}

fn plain_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The value labelled by the first of `labels` that has one, else the first.
fn pick<'a>(values: &'a [Labelled], labels: &WordList) -> Option<&'a str> {
    labels
        .words()
        .iter()
        .find_map(|label| {
            values.iter().find(|v| {
                v.label
                    .as_deref()
                    .is_some_and(|l| l.eq_ignore_ascii_case(label))
            })
        })
        .or_else(|| values.first())
        .map(|v| v.value.as_str())
}

impl Contact {
    /// The contact in `note`, when it is a person or organization note.
    #[must_use]
    pub fn from_note(note: &Note) -> Option<Self> {
        let kind = if note.is_type("person") {
            ContactKind::Person
        } else if note.is_type("organization") {
            ContactKind::Organization
        } else {
            return None;
        };
        Some(Self {
            path: note.path.clone(),
            title: note.title.clone(),
            kind,
            emails: labelled(note.frontmatter.get("emails"), plain_text),
            phones: labelled(note.frontmatter.get("phones"), plain_text),
            addresses: labelled(note.frontmatter.get("addresses"), address_text),
            company: note.str("company").map(str::to_string),
        })
    }

    /// The billing email: labelled by the first billing label that has one
    /// (`billing`, then `work`), else the first.
    #[must_use]
    pub fn email(&self, labels: &WordList) -> Option<&str> {
        pick(&self.emails, labels)
    }

    /// The billing phone number, chosen as [`Contact::email`] is.
    #[must_use]
    pub fn phone(&self, labels: &WordList) -> Option<&str> {
        pick(&self.phones, labels)
    }

    /// The billing address, chosen as [`Contact::email`] is.
    #[must_use]
    pub fn address(&self, labels: &WordList) -> Option<&str> {
        pick(&self.addresses, labels)
    }
}

/// A `contracts:` entry: a note, or a file (usually an executed PDF).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ContractRef {
    /// `"[[Customer X MSA 2026]]"`.
    Note {
        /// As written.
        link: String,
    },
    /// `/customers/contracts/SOW 3.pdf`, or relative to the customer note.
    File {
        /// As written.
        path: String,
    },
}

/// Where a contract is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractTarget {
    /// The note it links.
    Note(VaultPath),
    /// The file it names.
    File(VaultPath),
}

/// `billing_portal:`: how invoices reach a customer that only takes them
/// through a vendor portal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct BillingPortal {
    /// Coupa, Ariba, …
    pub name: Option<String>,
    /// Where to upload.
    pub url: Option<String>,
    /// The username (never a password).
    pub login: Option<String>,
    /// Your vendor number in their system.
    pub supplier_id: Option<String>,
    /// The portal rejects invoices with no `reference`.
    pub requires_reference: bool,
    /// Free markdown.
    pub instructions: Option<String>,
}

/// An `open_pos:` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenPo {
    /// What the customer quotes back (`PO #PY75334`).
    pub reference: String,
    /// The PO's amount, as written.
    pub amount: Option<String>,
    /// When it expires.
    pub expires: Option<String>,
}

/// A customer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Customer {
    /// The contact.
    pub contact: Contact,
    /// The `customer:` tag value in the books: `ledger_customer`, else the
    /// title.
    pub ledger_customer: String,
    /// Net days.
    pub terms: Option<u32>,
    /// The default revenue account.
    pub revenue_account: Option<String>,
    /// The default template (`/invoices/tpl/x.html`).
    pub invoice_template: Option<String>,
    /// `contracts:`.
    pub contracts: Vec<ContractRef>,
    /// `billing_portal:`.
    pub billing_portal: Option<BillingPortal>,
    /// `open_pos:`.
    pub open_pos: Vec<OpenPo>,
    /// Default snippets (`[[Net 30 ACH]]`), as written.
    pub snippets: Vec<String>,
}

/// Who an invoice is addressed to: the snapshot `invoicee_*` (or
/// `invoicer_*`) fields copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BillingContact {
    /// The note the name comes from (an org, for a person at a company).
    pub note: VaultPath,
    /// The name printed.
    pub name: String,
    /// The billing email.
    pub email: Option<String>,
    /// The billing phone number.
    pub phone: Option<String>,
    /// The billing address.
    pub address: Option<String>,
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items.iter().filter_map(plain_text).collect(),
        _ => Vec::new(),
    }
}

fn text_field(fields: &Fields, key: &str) -> Option<String> {
    fields.get(key).and_then(plain_text)
}

/// Whether `note` is a customer: a person or organization note carrying the
/// customer tag.
#[must_use]
pub fn is_customer(note: &Note, words: &Words) -> bool {
    (note.is_type("person") || note.is_type("organization")) && note.has_tag(&words.customer_tag)
}

/// Every customer in the site, sorted by path.
#[must_use]
pub fn customers(site: &Site, words: &Words) -> Vec<Customer> {
    site.notes()
        .filter(|note| is_customer(note, words))
        .filter_map(Customer::from_note)
        .collect()
}

/// Why a `customer:` link doesn't give a customer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomerLookup {
    /// It names no note.
    NotFound,
    /// It names a note that isn't a tagged person or organization.
    NotACustomer(VaultPath),
}

/// The customer a `customer:` link names, seen from the note at `from`.
///
/// # Errors
/// [`CustomerLookup`].
pub fn find_customer(
    site: &Site,
    from: &VaultPath,
    link: &str,
    words: &Words,
) -> Result<Customer, CustomerLookup> {
    let note = site
        .resolve_wikilink(from, link)
        .ok_or(CustomerLookup::NotFound)?;
    if !is_customer(note, words) {
        return Err(CustomerLookup::NotACustomer(note.path.clone()));
    }
    Customer::from_note(note).ok_or_else(|| CustomerLookup::NotACustomer(note.path.clone()))
}

/// Your company: the organization (or person) note `[invoicer] company`
/// names, by wikilink or title. Unlike a customer it needs no tag.
#[must_use]
pub fn company(site: &Site, settings: &Settings) -> Option<Contact> {
    let name = settings.company.as_deref()?;
    let note = site.resolve_wikilink(&VaultPath::root(), name)?;
    Contact::from_note(note)
}

impl Customer {
    /// The customer fields of a contact note (tag not checked: see
    /// [`is_customer`]). Fields that don't read are left empty here and
    /// reported by [`Customer::problems`].
    #[must_use]
    pub fn from_note(note: &Note) -> Option<Self> {
        let contact = Contact::from_note(note)?;
        let fields = &note.frontmatter;
        let contracts = strings(fields.get("contracts"))
            .into_iter()
            .map(|entry| {
                if wikilink_target(&entry).is_some() {
                    ContractRef::Note { link: entry }
                } else {
                    ContractRef::File { path: entry }
                }
            })
            .collect();
        let billing_portal = match fields.get("billing_portal") {
            Some(Value::Object(map)) => {
                let get = |k: &str| map.get(k).and_then(plain_text);
                Some(BillingPortal {
                    name: get("name"),
                    url: get("url"),
                    login: get("login"),
                    supplier_id: get("supplier_id"),
                    requires_reference: map
                        .get("requires_reference")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    instructions: map
                        .get("instructions")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            }
            _ => None,
        };
        let open_pos = match fields.get("open_pos") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| {
                    let map = item.as_object()?;
                    Some(OpenPo {
                        reference: map.get("reference").and_then(plain_text)?,
                        amount: map.get("amount").and_then(plain_text),
                        expires: map.get("expires").and_then(plain_text),
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        Some(Self {
            ledger_customer: text_field(fields, "ledger_customer")
                .unwrap_or_else(|| contact.title.clone()),
            terms: fields
                .get("terms")
                .and_then(Value::as_u64)
                .and_then(|t| u32::try_from(t).ok()),
            revenue_account: text_field(fields, "revenue_account"),
            invoice_template: text_field(fields, "invoice_template"),
            contracts,
            billing_portal,
            open_pos,
            snippets: strings(fields.get("snippets")),
            contact,
        })
    }

    /// The `customer:` link an invoice writes: `[[Title]]`.
    #[must_use]
    pub fn wikilink(&self) -> String {
        format!("[[{}]]", self.contact.title)
    }

    /// Who invoices go to (§3.4): the billing email and address, else `work`,
    /// else the first. A person at a `company: "[[Org]]"` bills to the org:
    /// its name, and its billing email and address when it has them.
    #[must_use]
    pub fn billing_contact(&self, site: &Site, words: &Words) -> BillingContact {
        let labels = &words.billing_labels;
        let own = BillingContact {
            note: self.contact.path.clone(),
            name: self.contact.title.clone(),
            email: self.contact.email(labels).map(str::to_string),
            phone: self.contact.phone(labels).map(str::to_string),
            address: self.contact.address(labels).map(str::to_string),
        };
        let org = (self.contact.kind == ContactKind::Person)
            .then_some(self.contact.company.as_deref())
            .flatten()
            .and_then(|company| site.resolve_wikilink(&self.contact.path, company))
            .and_then(Contact::from_note)
            .filter(|org| org.kind == ContactKind::Organization);
        match org {
            None => own,
            Some(org) => BillingContact {
                note: org.path.clone(),
                name: org.title.clone(),
                email: org.email(labels).map(str::to_string).or(own.email),
                phone: org.phone(labels).map(str::to_string).or(own.phone),
                address: org.address(labels).map(str::to_string).or(own.address),
            },
        }
    }

    /// Where each contract is, or `None` for one that doesn't exist.
    #[must_use]
    pub fn resolve_contract(
        &self,
        contract: &ContractRef,
        site: &Site,
        vault: &dyn Vault,
    ) -> Option<ContractTarget> {
        match contract {
            ContractRef::Note { link } => site
                .resolve_wikilink(&self.contact.path, link)
                .map(|note| ContractTarget::Note(note.path.clone())),
            ContractRef::File { path } => resolve_relative(&self.contact.path, path.trim())
                .filter(|p| vault.is_file(p))
                .map(ContractTarget::File),
        }
    }

    /// What `check` reports about this customer: contracts that don't exist,
    /// a portal that doesn't read or holds a secret, terms that aren't days.
    #[must_use]
    pub fn problems(&self, note: &Note, site: &Site, vault: &dyn Vault) -> Vec<Problem> {
        let line = |key: &str| note.line_of(key);
        let mut problems: Vec<Problem> = self
            .contracts
            .iter()
            .filter(|c| self.resolve_contract(c, site, vault).is_none())
            .map(|c| {
                let shown = match c {
                    ContractRef::Note { link } => link,
                    ContractRef::File { path } => path,
                };
                Problem::warning(
                    ProblemCode::MissingContract,
                    format!("the contract {shown} doesn't exist"),
                )
                .with_fix(
                    "fix the path: `/` is the books folder; otherwise it is relative to this note",
                    None,
                )
                .at_line(line("contracts"))
            })
            .collect();
        match note.frontmatter.get("billing_portal") {
            None | Some(Value::Object(_)) => {}
            Some(_) => problems.push(
                Problem::error(
                    ProblemCode::BadPortal,
                    "billing_portal must be a map (name, url, login, …)",
                )
                .at_line(line("billing_portal")),
            ),
        }
        if let Some(Value::Object(map)) = note.frontmatter.get("billing_portal") {
            let secrets: Vec<&String> = map
                .keys()
                .filter(|k| {
                    ["password", "secret", "token", "api_key"]
                        .iter()
                        .any(|s| k.to_lowercase().contains(s))
                })
                .collect();
            if !secrets.is_empty() {
                problems.push(
                    Problem::error(
                        ProblemCode::BadPortal,
                        format!(
                            "billing_portal has {}: secrets never go in notes",
                            secrets[0]
                        ),
                    )
                    .with_fix("delete it and keep the password in the keychain", None)
                    .at_line(line("billing_portal")),
                );
            }
            if map
                .get("requires_reference")
                .is_some_and(|v| !v.is_boolean())
            {
                problems.push(
                    Problem::warning(
                        ProblemCode::BadPortal,
                        "billing_portal.requires_reference must be true or false",
                    )
                    .at_line(line("billing_portal")),
                );
            }
        }
        if note.frontmatter.get("terms").is_some() && self.terms.is_none() {
            problems.push(
                Problem::error(
                    ProblemCode::BadTerms,
                    "terms must be a whole number of days",
                )
                .at_line(line("terms")),
            );
        }
        problems
            .into_iter()
            .map(|p| p.in_file(&note.path))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::SiteConfig;
    use ledgeline_core::vault::MemVault;

    const CUSTOMER_X: &str = "---
title: Customer X
type: organization
tags: [customer]
emails:
  - billing: accounts_receivable@customerx.com
  - work: hello@customerx.com
phones:
  - work: \"+1 303 555 0100\"
  - billing: \"+1 303 555 0142\"
addresses:
  - billing: |
      1234 Main St. #555
      Springfield, CO 80000
terms: 30
revenue_account: revenue:consulting
invoice_template: /invoices/tpl/myco-default.html
ledger_customer: Customer X Inc
contracts:
  - \"[[Customer X MSA 2026]]\"
  - \"/customers/contracts/Customer X SOW 3 renewal (signed).pdf\"
  - contracts/missing.pdf
open_pos:
  - reference: \"PO #PY75334\"
    amount: $40,000
    expires: 2027-06-30
billing_portal:
  name: Coupa
  url: https://customerx.coupahost.com/
  login: finance@mycompany.com
  supplier_id: SUP-44821
  requires_reference: true
snippets: [\"[[Net 30 ACH]]\"]
---

Fred Smith is the champion.
";

    fn vault() -> MemVault {
        MemVault::new()
            .with_file("customers/Customer X.md", CUSTOMER_X)
            .with_file("customers/Customer X MSA 2026.md", "# MSA\n")
            .with_file("customers/contracts/Customer X SOW 3 renewal (signed).pdf", "%PDF-")
            .with_file(
                "customers/Fred Smith.md",
                "---\ntype: person\ntags: [customer]\ncompany: \"[[Acme]]\"\nemails:\n  - home: fred@home.example\nphones: \"555 0199\"\n---\n",
            )
            .with_file(
                "orgs/Acme.md",
                "---\ntitle: Acme\ntype: organization\naddresses:\n  - work:\n      street: 1 Main St\n      city: Springfield\n      region: IL\n      postcode: \"62701\"\n      country: USA\n---\n",
            )
            .with_file("people/Not A Customer.md", "---\ntype: person\n---\n")
            .with_file("orgs/My Company.md", "---\ntype: organization\nemails: [finance@mycompany.com]\n---\n")
    }

    fn site(vault: &MemVault) -> Site {
        Site::open(vault, SiteConfig::default())
    }

    #[test]
    fn customers_are_tagged_contacts() {
        let vault = vault();
        let site = site(&vault);
        let all = customers(&site, &Words::default());
        let titles: Vec<&str> = all.iter().map(|c| c.contact.title.as_str()).collect();
        assert_eq!(titles, ["Customer X", "Fred Smith"]);
        let x = &all[0];
        assert_eq!(x.ledger_customer, "Customer X Inc");
        assert_eq!(x.terms, Some(30));
        assert_eq!(x.open_pos[0].reference, "PO #PY75334");
        assert!(
            x.billing_portal
                .as_ref()
                .is_some_and(|p| p.requires_reference)
        );
        assert_eq!(x.snippets, ["[[Net 30 ACH]]"]);
        assert_eq!(x.wikilink(), "[[Customer X]]");
    }

    #[test]
    fn billing_contact_prefers_billing_labels() {
        let vault = vault();
        let site = site(&vault);
        let x = find_customer(
            &site,
            &VaultPath::root(),
            "[[Customer X]]",
            &Words::default(),
        )
        .expect("customer");
        let bill = x.billing_contact(&site, &Words::default());
        assert_eq!(
            bill.email.as_deref(),
            Some("accounts_receivable@customerx.com")
        );
        assert_eq!(
            bill.phone.as_deref(),
            Some("+1 303 555 0142"),
            "billing before work, as for emails"
        );
        assert_eq!(x.contact.phones.len(), 2);
        assert_eq!(
            bill.address.as_deref(),
            Some("1234 Main St. #555\nSpringfield, CO 80000")
        );
    }

    #[test]
    fn a_person_at_a_company_bills_to_the_org() {
        let vault = vault();
        let site = site(&vault);
        let fred = find_customer(
            &site,
            &VaultPath::root(),
            "[[Fred Smith]]",
            &Words::default(),
        )
        .expect("customer");
        let bill = fred.billing_contact(&site, &Words::default());
        assert_eq!(bill.name, "Acme");
        assert_eq!(
            bill.email.as_deref(),
            Some("fred@home.example"),
            "the org has none"
        );
        assert_eq!(
            bill.phone.as_deref(),
            Some("555 0199"),
            "a bare phone, and the org has none"
        );
        assert_eq!(
            bill.address.as_deref(),
            Some("1 Main St\nSpringfield, IL 62701\nUSA")
        );
    }

    #[test]
    fn lookups_say_why_they_fail() {
        let vault = vault();
        let site = site(&vault);
        let words = Words::default();
        assert_eq!(
            find_customer(&site, &VaultPath::root(), "[[Nobody]]", &words).err(),
            Some(CustomerLookup::NotFound)
        );
        assert!(matches!(
            find_customer(&site, &VaultPath::root(), "[[Not A Customer]]", &words),
            Err(CustomerLookup::NotACustomer(_))
        ));
        let custom = Words {
            customer_tag: "client".into(),
            ..Words::default()
        };
        assert!(customers(&site, &custom).is_empty());
    }

    #[test]
    fn contracts_resolve_as_notes_or_files() {
        let vault = vault();
        let site = site(&vault);
        let x = find_customer(
            &site,
            &VaultPath::root(),
            "[[Customer X]]",
            &Words::default(),
        )
        .expect("customer");
        let resolved: Vec<Option<ContractTarget>> = x
            .contracts
            .iter()
            .map(|c| x.resolve_contract(c, &site, &vault))
            .collect();
        assert!(matches!(resolved[0], Some(ContractTarget::Note(_))));
        assert!(matches!(resolved[1], Some(ContractTarget::File(_))));
        assert_eq!(resolved[2], None);
        let note = site.note(&x.contact.path).expect("note");
        let problems = x.problems(note, &site, &vault);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].code, ProblemCode::MissingContract);
    }

    #[test]
    fn portals_never_hold_secrets() {
        let vault = MemVault::new().with_file(
            "c.md",
            "---\ntype: organization\ntags: [customer]\nterms: soon\nbilling_portal:\n  name: Ariba\n  password: hunter2\n  requires_reference: maybe\n---\n",
        );
        let site = site(&vault);
        let note = site.notes().next().expect("note");
        let c = Customer::from_note(note).expect("contact");
        let codes: Vec<ProblemCode> = c
            .problems(note, &site, &vault)
            .iter()
            .map(|p| p.code)
            .collect();
        assert_eq!(
            codes,
            [
                ProblemCode::BadPortal,
                ProblemCode::BadPortal,
                ProblemCode::BadTerms
            ]
        );
    }

    #[test]
    fn the_company_needs_no_tag() {
        let vault = vault();
        let site = site(&vault);
        let settings = Settings {
            company: Some("[[My Company]]".into()),
            ..Settings::default()
        };
        let me = company(&site, &settings).expect("company");
        assert_eq!(
            me.email(&Words::default().billing_labels),
            Some("finance@mycompany.com")
        );
        assert_eq!(company(&site, &Settings::default()), None);
    }
}
