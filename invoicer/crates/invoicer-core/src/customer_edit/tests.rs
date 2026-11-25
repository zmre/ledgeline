use super::*;
use crate::customers::find_customer;
use crate::doc::Money;
use crate::settings::Settings;
use crate::table::words::Words;
use ledgeline_core::vault::MemVault;
use serde_json::json;

/// A hand-written customer: comments, unknown keys (top level and in the
/// portal), a structured address, a body.
const HAND_WRITTEN: &str = "---
title: Fabrikam
type: organization
tags: [customer, vip]   # vip is ours
# Synced from Contacts; the invoicing keys are below.
emails:
  - billing: ap@fabrikam.example
addresses:
  - billing:
      street: 1 Fabrikam Way
      city: Springfield
      region: IL
      postcode: \"62701\"
      country: USA
terms: 30
account_manager: Dana
billing_portal:
  name: Ariba
  url: https://ariba.example/
  vendor_notes: ask for Lee
---

Fabrikam pays by ACH.

- Contact: Lee
";

fn path(p: &str) -> VaultPath {
    VaultPath::new(p).expect("path")
}

fn open(files: &[(&str, &str)]) -> (MemVault, DocumentIndex) {
    let vault = files
        .iter()
        .fold(MemVault::new(), |v, (p, t)| v.with_file(p, t));
    let index = DocumentIndex::open(&vault, Settings::default(), Money::default());
    (vault, index)
}

fn fields(value: serde_json::Value) -> CustomerFields {
    serde_json::from_value(value).expect("fields")
}

fn hash_of(vault: &MemVault, p: &VaultPath) -> String {
    content_hash(&vault.read(p).expect("reads"))
}

#[test]
fn create_writes_a_tagged_note_in_customers_dir() {
    let (vault, mut index) = open(&[]);
    let new = fields(json!({
        "title": "Northwind Traders",
        "kind": "organization",
        "emails": [{"label": "billing", "value": "ap@northwind.example"}, {"label": null, "value": "hello@northwind.example"}],
        "phones": [{"label": "billing", "value": "+1 (303) 555-0142"}],
        "addresses": [{"label": "billing", "value": "1 Main St\nSpringfield, CO 80000\n"}],
        "terms": 15,
        "revenue_account": "revenue:consulting",
        "billing_portal": {"name": "Coupa", "url": "https://coupa.example/", "requires_reference": true},
        "contracts": ["", "[[Northwind MSA]]"],
        "company": ""
    }));
    let made = create(&mut index, &vault, &new).expect("creates");
    assert_eq!(made.path, path("customers/Northwind Traders.md"));
    assert_eq!(made.link, "[[Northwind Traders]]");
    assert_eq!(made.ledger_customer, "Northwind Traders");
    assert_eq!(made.kind, ContactKind::Organization);
    assert!(made.written);
    let text = vault.read_to_string(&made.path).expect("reads");
    assert_eq!(made.hash, content_hash(text.as_bytes()));
    assert_eq!(
        &text[..text.find("billing_portal:\n").expect("portal") + 16],
        "---
title: Northwind Traders
type: organization
tags:
  - customer
emails:
  - billing: ap@northwind.example
  - hello@northwind.example
phones:
  - billing: +1 (303) 555-0142
addresses:
  - billing: |
      1 Main St
      Springfield, CO 80000
terms: 15
revenue_account: revenue:consulting
contracts:
  - \"[[Northwind MSA]]\"
billing_portal:
"
    );
    // The portal's keys, in key order (or insertion order where serde_json
    // keeps it: the app's build, PLAN 3a).
    let portal = &text[text.find("billing_portal:\n").expect("portal") + 16..];
    let mut lines: Vec<&str> = portal.lines().filter(|l| *l != "---").collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            "  name: Coupa",
            "  requires_reference: true",
            "  url: https://coupa.example/"
        ]
    );
    // Indexed at once, and an invoice's link finds it.
    let words = Words::default();
    let found =
        find_customer(index.site(), &VaultPath::root(), &made.link, &words).expect("a customer");
    let bill = found.billing_contact(index.site(), &words);
    assert_eq!(bill.email.as_deref(), Some("ap@northwind.example"));
    assert_eq!(bill.phone.as_deref(), Some("+1 (303) 555-0142"));
    assert_eq!(
        bill.address.as_deref(),
        Some("1 Main St\nSpringfield, CO 80000")
    );
    assert!(found.billing_portal.is_some_and(|p| p.requires_reference));
}

#[test]
fn create_follows_customers_dir_and_the_customer_tag() {
    let vault = MemVault::new();
    let mut settings = Settings {
        customers_dir: "clients/".to_string(),
        ..Settings::default()
    };
    settings.words.customer_tag = "client".to_string();
    let mut index = DocumentIndex::open(&vault, settings, Money::default());
    let made = create(
        &mut index,
        &vault,
        &fields(json!({"title": "Ann Lee: Design", "kind": "person"})),
    )
    .expect("creates");
    // `:` can't be in a file name everywhere; the title keeps it.
    assert_eq!(made.path, path("clients/Ann Lee- Design.md"));
    assert_eq!(made.title, "Ann Lee: Design");
    let text = vault.read_to_string(&made.path).expect("reads");
    assert!(text.contains("type: person\ntags:\n  - client\n"), "{text}");
    assert_eq!(index.customers().len(), 1);
}

#[test]
fn create_refuses_a_taken_name_and_bad_fields() {
    let (vault, mut index) = open(&[
        ("customers/Fabrikam.md", HAND_WRITTEN),
        ("people/Dana.md", "---\ntype: person\n---\n"),
    ]);
    let taken = create(
        &mut index,
        &vault,
        &fields(json!({"title": "fabrikam", "kind": "organization"})),
    );
    assert!(
        matches!(&taken, Err(CustomerError::NameTaken { path: p, .. }) if p.as_str() == "customers/Fabrikam.md"),
        "{taken:?}"
    );
    // Any note answering to the name, customer or not.
    assert!(matches!(
        create(
            &mut index,
            &vault,
            &fields(json!({"title": "Dana", "kind": "person"}))
        ),
        Err(CustomerError::NameTaken { .. })
    ));
    for bad in [
        json!({"title": "", "kind": "person"}),
        json!({"title": "Acme"}),
        json!({"title": "A [[B]]", "kind": "person"}),
        json!({"title": "A|B", "kind": "person"}),
        json!({"title": "a/b", "kind": "person"}),
        json!({"title": "Two\nlines", "kind": "person"}),
        json!({"title": "x.md", "kind": "person"}),
        json!({"title": "Acme", "kind": "organization", "terms": 99999}),
        json!({"title": "Acme", "kind": "organization", "revenue_account": "revenue:a  b"}),
        json!({"title": "Acme", "kind": "organization", "ledger_customer": "Acme, Inc"}),
        json!({"title": "Acme", "kind": "organization", "billing_portal": {"url": "ftp://x"}}),
        json!({"title": "Acme", "kind": "organization", "emails": [{"label": "work", "value": "a\nb"}]}),
    ] {
        let refused = create(&mut index, &vault, &fields(bad.clone()));
        assert!(
            matches!(refused, Err(CustomerError::Invalid(_))),
            "{bad}: {refused:?}"
        );
    }
    assert!(!vault.is_dir(&path("customers/x")));
    assert_eq!(index.customers().len(), 1, "nothing was created");
    // Unknown fields and a password are refused before anything is read.
    assert!(
        serde_json::from_value::<CustomerFields>(json!({"title": "A", "colour": "red"})).is_err()
    );
    assert!(
        serde_json::from_value::<CustomerFields>(
            json!({"billing_portal": {"name": "Ariba", "password": "hunter2"}})
        )
        .is_err()
    );
}

#[test]
fn create_never_replaces_a_file() {
    // A file with the name but no title the index knows (not markdown to it).
    let (vault, mut index) = open(&[]);
    vault.create_dir_all(&path("customers")).expect("mkdir");
    vault
        .write_atomic(&path("customers/Acme.md"), b"mine")
        .expect("writes");
    let refused = create(
        &mut index,
        &vault,
        &fields(json!({"title": "Acme", "kind": "organization"})),
    );
    assert!(
        matches!(refused, Err(CustomerError::Exists(_))),
        "{refused:?}"
    );
    assert_eq!(
        vault.read(&path("customers/Acme.md")).expect("reads"),
        b"mine"
    );
}

#[test]
fn update_edits_only_the_keys_named_and_keeps_everything_else() {
    let (vault, mut index) = open(&[("customers/Fabrikam.md", HAND_WRITTEN)]);
    let p = path("customers/Fabrikam.md");
    let hash = hash_of(&vault, &p);
    let done = update(
        &mut index,
        &vault,
        &p,
        &hash,
        &fields(json!({"terms": 45, "phones": [{"label": "billing", "value": "555 0100"}]})),
    )
    .expect("updates");
    assert!(done.written);
    assert_eq!(done.changed, ["phones", "terms"]);
    let text = vault.read_to_string(&p).expect("reads");
    let expected = HAND_WRITTEN.replace("terms: 30\n", "terms: 45\n").replace(
        "  vendor_notes: ask for Lee\n---",
        "  vendor_notes: ask for Lee\nphones:\n  - billing: 555 0100\n---",
    );
    assert_eq!(text, expected);
    assert_eq!(done.hash, content_hash(text.as_bytes()));
    // The index has it.
    let note = index.site().note(&p).expect("indexed");
    assert_eq!(note.frontmatter.get("terms"), Some(&json!(45)));
}

#[test]
fn saving_the_form_unchanged_writes_nothing() {
    let (vault, mut index) = open(&[("customers/Fabrikam.md", HAND_WRITTEN)]);
    let p = path("customers/Fabrikam.md");
    let note = index.site().note(&p).expect("note").clone();
    let customer = Customer::from_note(&note).expect("customer");
    let form = fields_of(&customer, &note);
    assert_eq!(
        form.addresses.as_deref().map(|a| a[0].value.as_str()),
        Some("1 Fabrikam Way\nSpringfield, IL 62701\nUSA")
    );
    assert_eq!(form.ledger_customer.as_deref(), Some(""));
    let done = update(&mut index, &vault, &p, &hash_of(&vault, &p), &form).expect("saves");
    assert!(!done.written, "{:?}", done.changed);
    assert!(done.changed.is_empty());
    // The structured address stays a map.
    assert_eq!(vault.read_to_string(&p).expect("reads"), HAND_WRITTEN);
}

#[test]
fn an_outside_edit_is_never_clobbered() {
    let (vault, mut index) = open(&[("customers/Fabrikam.md", HAND_WRITTEN)]);
    let p = path("customers/Fabrikam.md");
    let shown = hash_of(&vault, &p);
    let outside = HAND_WRITTEN.replace("Fabrikam pays by ACH.", "Fabrikam pays by wire.");
    vault.write_atomic(&p, outside.as_bytes()).expect("edits");
    let refused = update(
        &mut index,
        &vault,
        &p,
        &shown,
        &fields(json!({"terms": 60})),
    );
    let Err(CustomerError::Changed { current, .. }) = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(current, content_hash(outside.as_bytes()));
    assert_eq!(vault.read_to_string(&p).expect("reads"), outside);
    // With the hash it has now, the same change applies over the new text.
    let done = update(
        &mut index,
        &vault,
        &p,
        &current,
        &fields(json!({"terms": 60})),
    )
    .expect("updates");
    assert!(done.written);
    let text = vault.read_to_string(&p).expect("reads");
    assert!(
        text.contains("terms: 60\n") && text.contains("pays by wire"),
        "{text}"
    );
}

#[test]
fn renaming_keeps_the_books_name() {
    let (vault, mut index) = open(&[("customers/Fabrikam.md", HAND_WRITTEN)]);
    let p = path("customers/Fabrikam.md");
    let done = update(
        &mut index,
        &vault,
        &p,
        &hash_of(&vault, &p),
        &fields(json!({"title": "Fabrikam Inc"})),
    )
    .expect("renames");
    assert_eq!(done.changed, ["title", "ledger_customer"]);
    assert_eq!(done.title, "Fabrikam Inc");
    assert_eq!(done.ledger_customer, "Fabrikam");
    assert_eq!(done.path, p, "the file keeps its name");
    let text = vault.read_to_string(&p).expect("reads");
    assert!(text.starts_with("---\ntitle: Fabrikam Inc\n"), "{text}");
    assert!(text.contains("ledger_customer: Fabrikam\n---\n"), "{text}");
    // Invoices' `[[Fabrikam]]` still resolve, by the file name.
    let words = Words::default();
    let found =
        find_customer(index.site(), &VaultPath::root(), "[[Fabrikam]]", &words).expect("found");
    assert_eq!(found.contact.title, "Fabrikam Inc");

    // Renaming onto another note's name is refused.
    let (vault, mut index) = open(&[
        ("customers/Fabrikam.md", HAND_WRITTEN),
        ("orgs/Contoso.md", "---\ntype: organization\n---\n"),
    ]);
    let refused = update(
        &mut index,
        &vault,
        &p,
        &hash_of(&vault, &p),
        &fields(json!({"title": "Contoso"})),
    );
    assert!(
        matches!(refused, Err(CustomerError::NameTaken { .. })),
        "{refused:?}"
    );
}

#[test]
fn the_portal_keeps_its_unknown_keys_and_empty_values_remove() {
    let (vault, mut index) = open(&[("customers/Fabrikam.md", HAND_WRITTEN)]);
    let p = path("customers/Fabrikam.md");
    let done = update(
        &mut index,
        &vault,
        &p,
        &hash_of(&vault, &p),
        &fields(json!({
            "billing_portal": {"name": "Ariba", "url": "https://ariba.example/", "login": "ar@myco.example",
                               "instructions": "Upload under Invoices.\nAttach the timesheet.\n"},
            "emails": [],
            "terms": null,
            "revenue_account": ""
        })),
    )
    .expect("updates");
    assert_eq!(done.changed, ["emails", "terms", "billing_portal"]);
    let text = vault.read_to_string(&p).expect("reads");
    assert!(
        !text.contains("emails:") && !text.contains("terms:"),
        "{text}"
    );
    assert!(text.contains("  vendor_notes: ask for Lee\n"), "{text}");
    assert!(text.contains("  login: ar@myco.example\n"), "{text}");
    assert!(
        text.contains("  instructions: |\n    Upload under Invoices.\n    Attach the timesheet.\n"),
        "{text}"
    );
    assert!(text.ends_with("---\n\nFabrikam pays by ACH.\n\n- Contact: Lee\n"));

    let done = update(
        &mut index,
        &vault,
        &p,
        &done.hash,
        &fields(json!({"billing_portal": null})),
    )
    .expect("removes");
    assert_eq!(done.changed, ["billing_portal"]);
    let text = vault.read_to_string(&p).expect("reads");
    assert!(!text.contains("billing_portal"), "{text}");
    assert!(text.contains("account_manager: Dana\n"), "{text}");
}

#[test]
fn only_customers_are_edited_and_odd_layouts_are_refused() {
    let flow = "---\ntitle: Odd\ntype: organization\ntags: [customer]\nemails:\n  - billing: a@b.example\n# the old one\n  - work: c@d.example\n---\nbody\n";
    let (vault, mut index) = open(&[
        ("people/Dana.md", "---\ntype: person\n---\n"),
        ("customers/Odd.md", flow),
    ]);
    let dana = path("people/Dana.md");
    assert!(matches!(
        update(
            &mut index,
            &vault,
            &dana,
            &hash_of(&vault, &dana),
            &fields(json!({"terms": 1}))
        ),
        Err(CustomerError::NotACustomer { .. })
    ));
    assert!(matches!(
        update(
            &mut index,
            &vault,
            &path("nope.md"),
            "x",
            &CustomerFields::default()
        ),
        Err(CustomerError::NotFound(_))
    ));
    let odd = path("customers/Odd.md");
    let refused = update(
        &mut index,
        &vault,
        &odd,
        &hash_of(&vault, &odd),
        &fields(json!({"terms": 1})),
    );
    assert!(
        matches!(refused, Err(CustomerError::Frontmatter(_))),
        "{refused:?}"
    );
    assert_eq!(vault.read_to_string(&odd).expect("reads"), flow);
}

#[test]
fn absent_and_null_are_different_requests() {
    let absent = fields(json!({}));
    assert_eq!(absent.terms, None);
    assert_eq!(absent.billing_portal, None);
    let null = fields(json!({"terms": null, "billing_portal": null}));
    assert_eq!(null.terms, Some(None));
    assert_eq!(null.billing_portal, Some(None));
    // The form round-trips through JSON (the API hands it out and takes it back).
    let form = fields(
        json!({"title": "A", "kind": "person", "terms": 30, "emails": [{"label": "work", "value": "a@b"}]}),
    );
    let back: CustomerFields =
        serde_json::from_value(serde_json::to_value(&form).expect("json")).expect("reads");
    assert_eq!(back, form);
}

#[test]
fn a_company_name_becomes_a_wikilink() {
    let (vault, mut index) = open(&[("customers/Fabrikam.md", HAND_WRITTEN)]);
    let made = create(
        &mut index,
        &vault,
        &fields(json!({"title": "Lee Park", "kind": "person", "company": "Fabrikam"})),
    )
    .expect("creates");
    let text = vault.read_to_string(&made.path).expect("reads");
    assert!(text.contains("company: \"[[Fabrikam]]\"\n"), "{text}");
    let words = Words::default();
    let lee = find_customer(index.site(), &VaultPath::root(), &made.link, &words).expect("found");
    assert_eq!(lee.billing_contact(index.site(), &words).name, "Fabrikam");
}

#[test]
fn the_fixture_customer_saves_unchanged() {
    // Every invoicing key the core reads, a trailing comment, quoted phones,
    // a `|` address and portal instructions, `open_pos` and `snippets`.
    const CUSTOMER_X: &str = include_str!("../../../../fixtures/accrual/customers/Customer X.md");
    let (vault, mut index) = open(&[("customers/Customer X.md", CUSTOMER_X)]);
    let p = path("customers/Customer X.md");
    let note = index.site().note(&p).expect("note").clone();
    let customer = Customer::from_note(&note).expect("customer");
    let form = fields_of(&customer, &note);
    let done = update(&mut index, &vault, &p, &hash_of(&vault, &p), &form).expect("saves");
    assert!(!done.written, "{:?}", done.changed);
    assert_eq!(vault.read_to_string(&p).expect("reads"), CUSTOMER_X);
    // One portal key changed: the rest of the portal, and the file, stay.
    let mut portal = form.billing_portal.clone().flatten().expect("portal");
    portal.supplier_id = Some("SUP-1".into());
    let done = update(
        &mut index,
        &vault,
        &p,
        &done.hash,
        &CustomerFields {
            billing_portal: Some(Some(portal)),
            ..CustomerFields::default()
        },
    )
    .expect("saves");
    assert_eq!(done.changed, ["billing_portal"]);
    let text = vault.read_to_string(&p).expect("reads");
    assert!(text.contains("  supplier_id: SUP-1\n"), "{text}");
    assert!(
        text.contains("  instructions: |\n    Upload the PDF under Invoices"),
        "{text}"
    );
    let outside = |t: &str| {
        let start = t.find("billing_portal:").expect("portal");
        let end = t.find("snippets:").expect("snippets");
        format!("{}{}", &t[..start], &t[end..])
    };
    assert_eq!(outside(&text), outside(CUSTOMER_X));
}

/// A customer with hand-written open POs: a quoted reference, an amount as
/// a number, a key the form doesn't know, a comment after the list.
const WITH_POS: &str = "---
title: Contoso
type: organization
tags: [customer]
open_pos:
  - reference: \"PO #105\"
    amount: 40000
    expires: 2027-06-30
    buyer: Lee        # who signed it
  - reference: \"PO #200\"
terms: 30   # net
---

Body stays.
";

fn pos(value: serde_json::Value) -> CustomerFields {
    fields(json!({ "open_pos": value }))
}

#[test]
fn open_pos_save_unchanged_and_keep_unknown_keys() {
    let (vault, mut index) = open(&[("customers/Contoso.md", WITH_POS)]);
    let p = path("customers/Contoso.md");
    let note = index.site().note(&p).expect("note").clone();
    let customer = Customer::from_note(&note).expect("customer");
    let form = fields_of(&customer, &note);
    let listed = form.open_pos.clone().expect("open_pos");
    assert_eq!(listed[0].reference, "PO #105");
    assert_eq!(listed[0].amount.as_deref(), Some("40000"));
    assert_eq!(listed[0].was.as_deref(), Some("PO #105"));
    // The form as read writes nothing: `amount: 40000` stays a number.
    let done = update(&mut index, &vault, &p, &hash_of(&vault, &p), &form).expect("saves");
    assert!(!done.written, "{:?}", done.changed);
    assert_eq!(vault.read_to_string(&p).expect("reads"), WITH_POS);

    // Edit the first PO's expiry: its `buyer` stays, the second PO stays, and
    // every other key, comment and the body stay byte for byte.
    let done = update(
        &mut index,
        &vault,
        &p,
        &done.hash,
        &pos(json!([
            {"reference": "PO #105", "amount": "40000", "expires": "2027-12-31", "was": "PO #105"},
            {"reference": "PO #200", "was": "PO #200"}
        ])),
    )
    .expect("saves");
    assert_eq!(done.changed, ["open_pos"]);
    let text = vault.read_to_string(&p).expect("reads");
    assert!(text.contains("expires: 2027-12-31"), "{text}");
    assert!(text.contains("buyer: Lee"), "{text}");
    assert!(text.contains("PO #200"), "{text}");
    assert!(
        text.ends_with("terms: 30   # net\n---\n\nBody stays.\n"),
        "{text}"
    );
    let reread = index.site().note(&p).cloned().expect("indexed");
    let po = &Customer::from_note(&reread).expect("customer").open_pos;
    assert_eq!(po.len(), 2);
    assert_eq!(po[0].expires.as_deref(), Some("2027-12-31"));
    assert_eq!(po[0].amount.as_deref(), Some("40000"));
}

#[test]
fn open_pos_add_rename_and_close() {
    let (vault, mut index) = open(&[("customers/Contoso.md", WITH_POS)]);
    let p = path("customers/Contoso.md");
    // Rename PO #105 (its `buyer` follows by `was`), close PO #200, add one.
    let done = update(
        &mut index,
        &vault,
        &p,
        &hash_of(&vault, &p),
        &pos(json!([
            {"reference": "PO #105-A", "amount": "$40,000", "expires": "2027-06-30", "was": "PO #105"},
            {"reference": "PO #300", "amount": "$5,000.50", "expires": ""}
        ])),
    )
    .expect("saves");
    assert_eq!(done.changed, ["open_pos"]);
    let note = index.site().note(&p).cloned().expect("indexed");
    let customer = Customer::from_note(&note).expect("customer");
    let refs: Vec<&str> = customer
        .open_pos
        .iter()
        .map(|po| po.reference.as_str())
        .collect();
    assert_eq!(refs, ["PO #105-A", "PO #300"]);
    assert_eq!(customer.open_pos[1].amount.as_deref(), Some("$5,000.50"));
    assert_eq!(customer.open_pos[1].expires, None);
    assert_eq!(
        note.frontmatter["open_pos"][0]["buyer"],
        json!("Lee"),
        "the renamed PO keeps its other keys"
    );
    // Closing the last ones removes the key; nothing else moves but the
    // body's note of them (S5; `update` has no date to give).
    let done = update(&mut index, &vault, &p, &done.hash, &pos(json!([]))).expect("saves");
    assert_eq!(done.changed, ["open_pos"]);
    let text = vault.read_to_string(&p).expect("reads");
    assert_eq!(
        text,
        "---\ntitle: Contoso\ntype: organization\ntags: [customer]\nterms: 30   # net\n---\n\nBody stays.\n\n## Closed POs\n\n- PO #200; nothing invoiced; closed.\n- PO #105-A — $40,000, expires 2027-06-30; nothing invoiced; closed.\n- PO #300 — $5,000.50; nothing invoiced; closed.\n"
    );
}

#[test]
fn open_pos_refuse_what_doesnt_read() {
    let (vault, mut index) = open(&[("customers/Contoso.md", WITH_POS)]);
    let p = path("customers/Contoso.md");
    let hash = hash_of(&vault, &p);
    for (bad, says) in [
        (
            json!([{"reference": "", "amount": "$5"}]),
            "needs its reference",
        ),
        (
            json!([{"reference": "PO 1"}, {"reference": "po 1"}]),
            "both",
        ),
        (
            json!([{"reference": "PO 1", "amount": "lots"}]),
            "isn't money",
        ),
        (
            json!([{"reference": "PO 1", "expires": "June"}]),
            "isn't a date",
        ),
        (json!([{"reference": "PO\n1"}]), "one line"),
    ] {
        let refused = update(&mut index, &vault, &p, &hash, &pos(bad.clone()));
        let Err(CustomerError::Invalid(message)) = refused else {
            panic!("{bad}: {refused:?}");
        };
        assert!(message.contains(says), "{bad}: {message}");
    }
    assert_eq!(vault.read_to_string(&p).expect("reads"), WITH_POS);
    // A blank row (no reference, amount or date) is dropped, not refused.
    let done = update(
        &mut index,
        &vault,
        &p,
        &hash,
        &pos(json!([{"reference": "PO #105", "amount": "40000", "expires": "2027-06-30", "was": "PO #105"}, {"reference": " "}])),
    )
    .expect("saves");
    let note = index.site().note(&p).cloned().expect("indexed");
    let left = Customer::from_note(&note).expect("customer").open_pos;
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(done.noted, ["PO #200"]);
    assert!(done.written);
}

#[test]
fn open_pos_conflict_on_an_outside_edit() {
    let (vault, mut index) = open(&[("customers/Contoso.md", WITH_POS)]);
    let p = path("customers/Contoso.md");
    let shown = hash_of(&vault, &p);
    let outside = WITH_POS.replace("Body stays.", "Edited elsewhere.");
    vault.write_atomic(&p, outside.as_bytes()).expect("edits");
    let refused = update(&mut index, &vault, &p, &shown, &pos(json!([])));
    assert!(
        matches!(refused, Err(CustomerError::Changed { .. })),
        "{refused:?}"
    );
    assert_eq!(vault.read_to_string(&p).expect("reads"), outside);
}

#[test]
fn a_new_customer_can_have_open_pos() {
    let (vault, mut index) = open(&[]);
    let made = create(
        &mut index,
        &vault,
        &fields(json!({
            "title": "Northwind",
            "kind": "organization",
            "open_pos": [{"reference": "PO #1", "amount": "$1,000", "expires": "2027-01-31"}]
        })),
    )
    .expect("creates");
    let note = index.site().note(&made.path).cloned().expect("indexed");
    let customer = Customer::from_note(&note).expect("customer");
    assert_eq!(customer.open_pos.len(), 1);
    assert_eq!(customer.open_pos[0].reference, "PO #1");
    assert_eq!(customer.open_pos[0].amount.as_deref(), Some("$1,000"));
    assert_eq!(customer.open_pos[0].expires.as_deref(), Some("2027-01-31"));
}

#[test]
fn closed_pos_are_noted_in_a_new_section_and_the_body_stays() {
    let (vault, mut index) = open(&[("customers/Contoso.md", WITH_POS)]);
    let p = path("customers/Contoso.md");
    let done = update_on(
        &mut index,
        &vault,
        &p,
        &hash_of(&vault, &p),
        &pos(json!([{"reference": "PO #200", "was": "PO #200"}])),
        Some("2026-11-24"),
    )
    .expect("saves");
    assert_eq!(done.noted, ["PO #105"]);
    let text = vault.read_to_string(&p).expect("reads");
    assert!(
        text.ends_with(
            "---\n\nBody stays.\n\n## Closed POs\n\n- PO #105 — 40000, expires 2027-06-30; nothing invoiced; closed 2026-11-24.\n"
        ),
        "{text}"
    );
    assert!(!text.contains("buyer: Lee"), "{text}");

    // A second close appends to the section, after its last line.
    let done = update_on(
        &mut index,
        &vault,
        &p,
        &done.hash,
        &pos(json!([])),
        Some("2026-11-25"),
    )
    .expect("saves");
    assert_eq!(done.noted, ["PO #200"]);
    let text = vault.read_to_string(&p).expect("reads");
    assert!(
        text.ends_with(
            "Body stays.\n\n## Closed POs\n\n- PO #105 — 40000, expires 2027-06-30; nothing invoiced; closed 2026-11-24.\n- PO #200; nothing invoiced; closed 2026-11-25.\n"
        ),
        "{text}"
    );
    assert!(!text.contains("open_pos:"), "{text}");
}

#[test]
fn a_rename_or_an_edit_is_not_a_close() {
    let (vault, mut index) = open(&[("customers/Contoso.md", WITH_POS)]);
    let p = path("customers/Contoso.md");
    let done = update_on(
        &mut index,
        &vault,
        &p,
        &hash_of(&vault, &p),
        &pos(json!([
            {"reference": "PO #105-A", "amount": "40000", "was": "PO #105"},
            {"reference": "PO #200", "was": "PO #200"}
        ])),
        Some("2026-11-24"),
    )
    .expect("saves");
    assert!(done.noted.is_empty());
    assert!(
        !vault
            .read_to_string(&p)
            .expect("reads")
            .contains("Closed POs")
    );
}

#[test]
fn the_closed_section_is_found_mid_body_and_the_rest_stays() {
    let body = "\nIntro.\n\n## Closed POs\n\n- PO #1 — $5; nothing invoiced; closed 2026-01-01.\n\n## Contacts\n\nLee.\n";
    let text = format!("---\ntitle: X\n---\n{body}");
    let out = note_closed_pos(
        &text,
        &["- PO #2; nothing invoiced; closed 2026-11-24.".to_string()],
    );
    assert_eq!(
        out,
        "---\ntitle: X\n---\n\nIntro.\n\n## Closed POs\n\n- PO #1 — $5; nothing invoiced; closed 2026-01-01.\n- PO #2; nothing invoiced; closed 2026-11-24.\n\n## Contacts\n\nLee.\n"
    );
    // An empty section, and a body without a final newline.
    let out = note_closed_pos(
        "---\ntitle: X\n---\nText\n## Closed POs",
        &["- A.".to_string()],
    );
    assert_eq!(out, "---\ntitle: X\n---\nText\n## Closed POs\n\n- A.\n");
    let out = note_closed_pos("---\ntitle: X\n---\nText", &["- A.".to_string()]);
    assert_eq!(out, "---\ntitle: X\n---\nText\n\n## Closed POs\n\n- A.\n");
    let out = note_closed_pos("---\ntitle: X\n---\n", &["- A.".to_string()]);
    assert_eq!(out, "---\ntitle: X\n---\n\n## Closed POs\n\n- A.\n");
}

#[test]
fn a_closed_po_line_reads_its_figures() {
    let usage = crate::views::PoUsage {
        reference: "PO #PY75334".into(),
        amount: Some("$40,000".into()),
        expires: Some("2027-06-30".into()),
        expired: false,
        invoices: vec!["104".into()],
        invoiced: Some("$16,700.00".into()),
        remaining: Some("$23,300.00".into()),
    };
    assert_eq!(
        closed_po_line(&usage, Some("2026-11-24")),
        "- PO #PY75334 — $40,000, expires 2027-06-30; invoiced $16,700.00; closed 2026-11-24."
    );
}
