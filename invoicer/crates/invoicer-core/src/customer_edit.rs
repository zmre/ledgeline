//! Creating and editing customer notes (invoicer PLAN 4b; ARCHITECTURE
//! §3.4, §5.5). One library call each, shared by the API and, later, the
//! CLI.
//!
//! - [`create`] writes a new person or organization note in
//!   `customers-dir`, tagged with the customer tag. It refuses a title some
//!   note already answers to, so the new `[[Title]]` links only this note.
//! - [`update`] edits a customer note **by key**, through
//!   [`frontmatter::edit`]: only the keys the request names are rewritten;
//!   every other key, comment and the markdown body stay byte for byte. A
//!   named key whose value already reads the same is left alone, so an
//!   address written as MBR's map (`street`, `city`, …) keeps its shape
//!   unless the user changed it.
//! - Both answer the file's content hash ([`Fingerprint::token`]). An
//!   update names the hash it was shown and is refused
//!   ([`CustomerError::Changed`]) when the file is no longer those bytes, and
//!   the write itself is conditional on the revision read, so an outside
//!   edit is never clobbered.
//!
//! [`CustomerFields`] is the request shape: every field optional, absent
//! meaning "leave it". An empty string or list, or `null`, removes the key.

use crate::customers::{BillingPortal, ContactKind, ContractRef, Customer, Labelled, is_customer};
use crate::frontmatter::{self, EditError, FieldEdit};
use crate::index::DocumentIndex;
use crate::notes::{Note, Stamp};
use ledgeline_core::vault::{Fingerprint, Vault, VaultError, VaultPath};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

/// The longest title accepted (it is also the file name).
const MAX_TITLE: usize = 120;

/// Characters a title can't hold: they would break the `[[Title]]` link an
/// invoice writes (`|`, `#`, `^`, brackets) or make it a path (`/`, `\`).
const TITLE_FORBIDDEN: &[char] = &['[', ']', '|', '#', '^', '/', '\\'];

/// `Some(None)` for an explicit `null`, `None` when the field is absent.
#[allow(
    clippy::option_option,
    reason = "absent (leave it) and null (remove it) are different requests"
)]
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// The customer form: MBR's contact fields plus the invoicing keys the core
/// reads (§3.4). Every field is optional: absent leaves the key as it is; an
/// empty string or list, or `null`, removes it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(
    clippy::option_option,
    reason = "absent (leave it) and null (remove it) are different requests"
)]
pub struct CustomerFields {
    /// `title:`, the name invoices link (`[[Title]]`). Required to create.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `type:`. Required to create.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ContactKind>,
    /// `emails:`, labelled (`billing` first is what invoices pick).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emails: Option<Vec<Labelled>>,
    /// `phones:`, labelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phones: Option<Vec<Labelled>>,
    /// `addresses:`, labelled, each as lines of text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<Labelled>>,
    /// `company:` (a person's organization), written as a wikilink.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    /// `terms:`, net days.
    #[serde(deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub terms: Option<Option<u32>>,
    /// `revenue_account:`, the default revenue account for its rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue_account: Option<String>,
    /// `invoice_template:`, the default template (`/invoices/tpl/x.html`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice_template: Option<String>,
    /// `ledger_customer:`, the `customer:` tag in the books when it isn't
    /// the title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_customer: Option<String>,
    /// `contracts:`, wikilinks or file paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contracts: Option<Vec<String>>,
    /// `billing_portal:`. Its keys the form doesn't know are kept.
    #[serde(deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub billing_portal: Option<Option<BillingPortal>>,
}

/// What a create or an update wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomerSaved {
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// What an invoice's `customer:` writes: `[[Title]]`.
    pub link: String,
    /// The `customer:` tag value in the books.
    pub ledger_customer: String,
    /// Person or organization.
    pub kind: ContactKind,
    /// The file's content hash now: what the next update names.
    pub hash: String,
    /// Whether the file was written (`false`: nothing changed).
    pub written: bool,
    /// The keys written, in order.
    pub changed: Vec<String>,
}

/// Why a create or an update refused. Nothing is written when one is
/// returned.
#[derive(Debug, Error)]
pub enum CustomerError {
    /// A field doesn't read (the message says which and why).
    #[error("{0}")]
    Invalid(String),
    /// No note there.
    #[error("{0}: no such note")]
    NotFound(VaultPath),
    /// The note isn't a tagged person or organization.
    #[error("{path} isn't a customer: a person or organization note tagged `{tag}`")]
    NotACustomer {
        /// The note.
        path: VaultPath,
        /// The customer tag.
        tag: String,
    },
    /// A note already answers to the title, so `[[Title]]` would be taken.
    #[error("a note named {title:?} already exists ({path}); open it, or choose another name")]
    NameTaken {
        /// The title asked for.
        title: String,
        /// The note that has it.
        path: VaultPath,
    },
    /// The file is already there.
    #[error("{0} already exists")]
    Exists(VaultPath),
    /// The file isn't what the caller was shown: read it again.
    #[error("{path} changed on disk since it was opened; reload it to see the change")]
    Changed {
        /// The note.
        path: VaultPath,
        /// Its hash now.
        current: String,
    },
    /// The frontmatter can't be edited safely (edit it by hand).
    #[error(transparent)]
    Frontmatter(#[from] EditError),
    /// The file isn't UTF-8 text.
    #[error("{0} isn't UTF-8 text")]
    NotText(VaultPath),
    /// The file can't be read or written.
    #[error(transparent)]
    Vault(#[from] VaultError),
}

/// The content hash the API hands out and takes back.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
    Fingerprint::of_bytes(bytes).token()
}

/// The form's values for a customer as its note writes them: what an update
/// starts from. `ledger_customer` is the written key only (empty when the
/// title is used); contracts are as written.
#[must_use]
pub fn fields_of(customer: &Customer, note: &Note) -> CustomerFields {
    let text = |key: &str| {
        note.frontmatter
            .get(key)
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    CustomerFields {
        title: Some(customer.contact.title.clone()),
        kind: Some(customer.contact.kind),
        emails: Some(customer.contact.emails.clone()),
        phones: Some(customer.contact.phones.clone()),
        addresses: Some(customer.contact.addresses.clone()),
        company: Some(customer.contact.company.clone().unwrap_or_default()),
        terms: Some(customer.terms),
        revenue_account: Some(customer.revenue_account.clone().unwrap_or_default()),
        invoice_template: Some(customer.invoice_template.clone().unwrap_or_default()),
        ledger_customer: Some(text("ledger_customer")),
        contracts: Some(
            customer
                .contracts
                .iter()
                .map(|c| match c {
                    ContractRef::Note { link } => link.clone(),
                    ContractRef::File { path } => path.clone(),
                })
                .collect(),
        ),
        billing_portal: Some(customer.billing_portal.clone()),
    }
}

// ---------------------------------------------------------------------------
// Validation: each field to the value written, or `None` to remove it.
// ---------------------------------------------------------------------------

fn invalid(message: impl Into<String>) -> CustomerError {
    CustomerError::Invalid(message.into())
}

fn one_line(what: &str, text: &str) -> Result<Option<String>, CustomerError> {
    let text = text.trim();
    if text.chars().any(char::is_control) {
        return Err(invalid(format!("{what}: one line of text, please")));
    }
    Ok((!text.is_empty()).then(|| text.to_string()))
}

fn clean_title(title: &str) -> Result<String, CustomerError> {
    let title = one_line("the name", title)?.ok_or_else(|| invalid("the name can't be empty"))?;
    if let Some(c) = title.chars().find(|c| TITLE_FORBIDDEN.contains(c)) {
        return Err(invalid(format!(
            "the name can't hold {c:?}: invoices link it as [[{title}]]"
        )));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(invalid(format!(
            "the name is longer than {MAX_TITLE} characters"
        )));
    }
    let lower = title.to_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") || title.starts_with('.') {
        return Err(invalid(format!(
            "{title:?} reads as a file name; name the customer"
        )));
    }
    Ok(title)
}

/// The file name for a title: characters file systems refuse become `-`.
fn file_stem(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if matches!(c, ':' | '*' | '?' | '"' | '<' | '>') || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string()
}

fn clean_labelled(
    what: &str,
    items: &[Labelled],
    multi_line: bool,
) -> Result<Vec<Labelled>, CustomerError> {
    let mut out = Vec::new();
    for item in items {
        let value = if multi_line {
            let lines: Vec<&str> = item
                .value
                .trim()
                .lines()
                .map(str::trim_end)
                .filter(|line| !line.trim().is_empty())
                .collect();
            if lines
                .iter()
                .any(|l| l.chars().any(|c| c.is_control() && c != '\t'))
            {
                return Err(invalid(format!(
                    "{what}: a value holds a control character"
                )));
            }
            lines.join("\n")
        } else {
            one_line(what, &item.value)?.unwrap_or_default()
        };
        if value.is_empty() {
            continue;
        }
        let label = match &item.label {
            Some(label) => one_line(&format!("{what}: a label"), label)?,
            None => None,
        };
        out.push(Labelled { label, value });
    }
    Ok(out)
}

fn labelled_value(items: &[Labelled]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| {
                // A multi-line value (an address) is written as a `|` block.
                let value = if item.value.contains('\n') {
                    Value::String(format!("{}\n", item.value))
                } else {
                    Value::String(item.value.clone())
                };
                match &item.label {
                    Some(label) => {
                        let mut map = Map::new();
                        map.insert(label.clone(), value);
                        Value::Object(map)
                    }
                    None => value,
                }
            })
            .collect(),
    )
}

fn clean_account(account: &str) -> Result<Option<String>, CustomerError> {
    let account = one_line("the revenue account", account)?;
    if account
        .as_deref()
        .is_some_and(|a| a.contains("  ") || a.contains('\t'))
    {
        return Err(invalid(
            "the revenue account can't hold two spaces or a tab (hledger ends the name there)",
        ));
    }
    Ok(account)
}

fn clean_company(company: &str) -> Result<Option<String>, CustomerError> {
    let Some(company) = one_line("the company", company)? else {
        return Ok(None);
    };
    if crate::notes::wikilink_target(&company).is_some() {
        return Ok(Some(company));
    }
    if company.contains(['[', ']', '|']) {
        return Err(invalid(
            "the company: a name, or a [[wikilink]] to its note",
        ));
    }
    Ok(Some(format!("[[{company}]]")))
}

/// `billing_portal:` as written: the form's keys over the note's own map,
/// which keeps keys the form doesn't know (in their order).
fn portal_value(
    portal: &BillingPortal,
    existing: Option<&Value>,
) -> Result<Option<Value>, CustomerError> {
    let mut map = match existing {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    let texts = [
        ("name", &portal.name),
        ("url", &portal.url),
        ("login", &portal.login),
        ("supplier_id", &portal.supplier_id),
    ];
    for (key, value) in texts {
        let value = match value {
            Some(v) => one_line(&format!("the portal's {key}"), v)?,
            None => None,
        };
        set_text(&mut map, key, value);
    }
    if let Some(url) = map.get("url").and_then(Value::as_str)
        && !(url.starts_with("https://") || url.starts_with("http://"))
    {
        return Err(invalid(
            "the portal's url must start with https:// (or http://)",
        ));
    }
    if portal.requires_reference || map.contains_key("requires_reference") {
        map.insert(
            "requires_reference".to_string(),
            Value::Bool(portal.requires_reference),
        );
    }
    // Free markdown: several lines are written as a `|` block.
    let instructions = portal
        .instructions
        .as_deref()
        .map(str::trim_end)
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            if s.contains('\n') {
                format!("{s}\n")
            } else {
                s.to_string()
            }
        });
    set_text(&mut map, "instructions", instructions);
    Ok((!map.is_empty()).then_some(Value::Object(map)))
}

/// Set `key` to `value` (remove it for `None`), leaving a value that only
/// differs in trailing white space as written (a `|` block stays one).
fn set_text(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    match value {
        Some(v) => {
            let same = map
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|old| old.trim_end() == v.trim_end());
            if !same {
                map.insert(key.to_string(), Value::String(v));
            }
        }
        None => {
            map.remove(key);
        }
    }
}

/// What one form maps to: each named key's new value (`None`: remove it), in
/// the order a new note writes them.
struct Wanted {
    keys: Vec<(&'static str, Option<Value>)>,
}

impl Wanted {
    fn put(&mut self, key: &'static str, value: Option<Value>) {
        self.keys.push((key, value));
    }
}

fn wanted(
    fields: &CustomerFields,
    existing_portal: Option<&Value>,
) -> Result<Wanted, CustomerError> {
    let mut out = Wanted { keys: Vec::new() };
    let text = |s: String| Some(Value::String(s));
    if let Some(title) = &fields.title {
        out.put("title", text(clean_title(title)?));
    }
    if let Some(kind) = fields.kind {
        out.put("type", text(kind.as_str().to_string()));
    }
    let lists = [
        ("emails", &fields.emails, false),
        ("phones", &fields.phones, false),
        ("addresses", &fields.addresses, true),
    ];
    for (key, items, multi_line) in lists {
        if let Some(items) = items {
            let items = clean_labelled(key, items, multi_line)?;
            out.put(key, (!items.is_empty()).then(|| labelled_value(&items)));
        }
    }
    if let Some(company) = &fields.company {
        out.put("company", clean_company(company)?.map(Value::String));
    }
    if let Some(terms) = fields.terms {
        if terms.is_some_and(|t| t > 3650) {
            return Err(invalid("terms: net days, at most 3650"));
        }
        out.put("terms", terms.map(Value::from));
    }
    if let Some(account) = &fields.revenue_account {
        out.put(
            "revenue_account",
            clean_account(account)?.map(Value::String),
        );
    }
    if let Some(template) = &fields.invoice_template {
        out.put(
            "invoice_template",
            one_line("the template", template)?.map(Value::String),
        );
    }
    if let Some(ledger) = &fields.ledger_customer {
        let ledger = one_line("the name in the books", ledger)?;
        if ledger.as_deref().is_some_and(|l| l.contains(',')) {
            return Err(invalid(
                "the name in the books can't hold a comma (it is a tag value)",
            ));
        }
        out.put("ledger_customer", ledger.map(Value::String));
    }
    if let Some(contracts) = &fields.contracts {
        let mut list = Vec::new();
        for entry in contracts {
            if let Some(entry) = one_line("a contract", entry)? {
                list.push(Value::String(entry));
            }
        }
        out.put(
            "contracts",
            (!list.is_empty()).then_some(Value::Array(list)),
        );
    }
    if let Some(portal) = &fields.billing_portal {
        let value = match portal {
            Some(portal) => portal_value(portal, existing_portal)?,
            None => None,
        };
        out.put("billing_portal", value);
    }
    Ok(out)
}

/// Whether `key`'s new value reads the same as the note already does: then
/// it isn't rewritten (an address map stays a map).
fn reads_the_same(customer: &Customer, note: &Note, key: &str, value: Option<&Value>) -> bool {
    let contact = &customer.contact;
    let same_list = |current: &[Labelled], value: Option<&Value>| {
        let wanted: Vec<Labelled> = match value {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| match item {
                    Value::Object(map) => map
                        .iter()
                        .next()
                        .map(|(k, v)| Labelled {
                            label: Some(k.clone()),
                            value: v.as_str().unwrap_or_default().to_string(),
                        })
                        .unwrap_or(Labelled {
                            label: None,
                            value: String::new(),
                        }),
                    other => Labelled {
                        label: None,
                        value: other.as_str().unwrap_or_default().to_string(),
                    },
                })
                .collect(),
            Some(_) => return false,
        };
        let text = |l: &Labelled| (l.label.clone(), l.value.trim_end_matches('\n').to_string());
        wanted.len() == current.len() && wanted.iter().zip(current).all(|(a, b)| text(a) == text(b))
    };
    match key {
        "emails" => same_list(&contact.emails, value),
        "phones" => same_list(&contact.phones, value),
        "addresses" => same_list(&contact.addresses, value),
        "title" => {
            note.frontmatter.contains_key("title")
                && value.and_then(Value::as_str) == Some(contact.title.as_str())
        }
        _ => note.frontmatter.get(key) == value,
    }
}

fn saved(path: VaultPath, text: &str, written: bool, changed: Vec<String>) -> CustomerSaved {
    let note = Note::from_source(
        path.clone(),
        text,
        Stamp {
            modified: None,
            size: text.len() as u64,
            fingerprint: Fingerprint::of_bytes(text.as_bytes()),
        },
    );
    let customer = Customer::from_note(&note);
    CustomerSaved {
        title: note.title.clone(),
        link: format!("[[{}]]", note.title),
        ledger_customer: customer
            .as_ref()
            .map_or_else(|| note.title.clone(), |c| c.ledger_customer.clone()),
        kind: customer.map_or(ContactKind::Organization, |c| c.contact.kind),
        hash: content_hash(text.as_bytes()),
        written,
        changed,
        path,
    }
}

/// Create a customer: a new person or organization note in `customers-dir`
/// named after its title, tagged with the customer tag, holding the form's
/// fields in a fixed order, with an empty body. Indexed at once.
///
/// # Errors
/// [`CustomerError::Invalid`] (no title or kind, or a field that doesn't
/// read), [`CustomerError::NameTaken`] (a note answers to the title),
/// [`CustomerError::Exists`] (the file is there), or a vault error.
pub fn create(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    fields: &CustomerFields,
) -> Result<CustomerSaved, CustomerError> {
    let title = clean_title(fields.title.as_deref().unwrap_or_default())?;
    if fields.kind.is_none() {
        return Err(invalid("choose person or organization"));
    }
    if let Some(note) = index.site().resolve_wikilink(&VaultPath::root(), &title) {
        return Err(CustomerError::NameTaken {
            title,
            path: note.path.clone(),
        });
    }
    let settings = index.settings();
    let dir = settings.customers_dir.trim_matches('/');
    let stem = file_stem(&title);
    let name = if dir.is_empty() {
        format!("{stem}.md")
    } else {
        format!("{dir}/{stem}.md")
    };
    let path = VaultPath::new(&name)
        .map_err(|e| invalid(format!("{name:?} can't be a file in the books: {e}")))?;
    let tag = settings.words.customer_tag.clone();
    let wanted = wanted(fields, None)?;
    let mut block: Vec<(&str, Value)> = Vec::new();
    for (key, value) in wanted.keys {
        if let Some(value) = value {
            block.push((key, value));
        }
        if key == "type" {
            block.push(("tags", Value::Array(vec![Value::String(tag.clone())])));
        }
    }
    let text = frontmatter::render_block(&block)?;
    if let Some(parent) = path.parent() {
        vault.create_dir_all(&parent)?;
    }
    vault
        .create_new_shared(&path, text.as_bytes())
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                CustomerError::Exists(path.clone())
            } else {
                CustomerError::Vault(e)
            }
        })?;
    index.refresh(vault, std::slice::from_ref(&path));
    let changed = block.iter().map(|(k, _)| (*k).to_string()).collect();
    Ok(saved(path, &text, true, changed))
}

/// Edit the customer note at `path` by key: the keys `fields` names, and
/// only when their values change. `expected` is the hash the caller was
/// shown ([`CustomerSaved::hash`], or the detail's): when the file is no
/// longer those bytes nothing is written.
///
/// Renaming (a new `title`) of a customer whose note has no
/// `ledger_customer` writes the old name as `ledger_customer` too, unless
/// the form sets it: the books know the customer by that name, and a rename
/// doesn't rename history (§3.4). The file keeps its name; invoices'
/// `[[Old name]]` links still resolve by the file name.
///
/// # Errors
/// [`CustomerError::Changed`] (an outside edit), [`CustomerError::Invalid`],
/// [`CustomerError::NotFound`], [`CustomerError::NotACustomer`],
/// [`CustomerError::Frontmatter`] (a layout the editor can't change safely),
/// or a vault error.
pub fn update(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    expected: &str,
    fields: &CustomerFields,
) -> Result<CustomerSaved, CustomerError> {
    let (bytes, revision) = vault.read_pinned(path, None).map_err(|e| {
        if e.is_not_found() {
            CustomerError::NotFound(path.clone())
        } else {
            CustomerError::Vault(e)
        }
    })?;
    let current = revision.fingerprint.token();
    if current != expected {
        return Err(CustomerError::Changed {
            path: path.clone(),
            current,
        });
    }
    let source = String::from_utf8(bytes).map_err(|_| CustomerError::NotText(path.clone()))?;
    let note = Note::from_source(
        path.clone(),
        &source,
        Stamp {
            modified: None,
            size: source.len() as u64,
            fingerprint: revision.fingerprint.clone(),
        },
    );
    let tag = index.settings().words.customer_tag.clone();
    let customer = is_customer(&note, &index.settings().words)
        .then(|| Customer::from_note(&note))
        .flatten()
        .ok_or_else(|| CustomerError::NotACustomer {
            path: path.clone(),
            tag,
        })?;
    if let Some(title) = &fields.title
        && clean_title(title)? != customer.contact.title
    {
        // The new name must not be another note's already.
        if let Some(other) = index
            .site()
            .resolve_wikilink(&VaultPath::root(), title.trim())
            .filter(|n| &n.path != path)
        {
            return Err(CustomerError::NameTaken {
                title: title.trim().to_string(),
                path: other.path.clone(),
            });
        }
    }
    let mut wanted = wanted(fields, note.frontmatter.get("billing_portal"))?;
    let renamed = wanted.keys.iter().any(|(k, v)| {
        *k == "title" && v.as_ref().and_then(Value::as_str) != Some(customer.contact.title.as_str())
    });
    if renamed
        && !note.frontmatter.contains_key("ledger_customer")
        && fields.ledger_customer.is_none()
    {
        wanted.put(
            "ledger_customer",
            Some(Value::String(customer.ledger_customer.clone())),
        );
    }
    let edits: Vec<FieldEdit> = wanted
        .keys
        .into_iter()
        .filter(|(key, value)| !reads_the_same(&customer, &note, key, value.as_ref()))
        .map(|(key, value)| match value {
            Some(value) => FieldEdit::Set(key.to_string(), value),
            None => FieldEdit::Remove(key.to_string()),
        })
        .collect();
    let text = frontmatter::edit(&source, &edits)?;
    let written = text != source;
    let changed: Vec<String> = edits
        .iter()
        .filter(|e| match e {
            FieldEdit::Remove(key) => note.frontmatter.contains_key(key),
            FieldEdit::Set(..) => true,
        })
        .map(|e| match e {
            FieldEdit::Set(key, _) | FieldEdit::Remove(key) => key.clone(),
        })
        .collect();
    if written {
        vault
            .replace(path, text.as_bytes(), &revision)
            .map_err(|e| match e {
                VaultError::Changed { .. } => CustomerError::Changed {
                    path: path.clone(),
                    current: vault
                        .read(path)
                        .map(|b| content_hash(&b))
                        .unwrap_or_default(),
                },
                other => CustomerError::Vault(other),
            })?;
        index.refresh(vault, std::slice::from_ref(path));
    }
    Ok(saved(path.clone(), &text, written, changed))
}

#[cfg(test)]
mod tests;
