use super::*;
use crate::doc::Money;
use crate::sent;
use ledgeline_core::vault::MemVault;

const COMPANY: &str = "---
type: organization
emails:
  - billing: finance@mycompany.com
phones:
  - billing: \"+1 (555) 010-2000\"
addresses:
  - work: |
      1234 Background Blvd.
      Anywhere, CA 90000
---
";

/// The company as it is now: it gained a logo since the documents below
/// were made.
fn company_with_logo() -> String {
    COMPANY.replacen("addresses:", "logo: \"[[logo.png]]\"\naddresses:", 1)
}

/// A document made before the company had a logo: every company key but
/// `invoicer_logo`, then a custom field, then the body.
fn document(status: &str) -> String {
    format!(
        "---
title: Invoice 108
type: invoice
status: {status}
invoice_num: 108
date: 2026-11-17
customer: \"[[Customer X]]\"
invoicee_name: Customer X
invoicer: \"[[My Company]]\"
invoicer_name: My Company
invoicer_email: finance@mycompany.com
invoicer_phone: +1 (555) 010-2000
invoicer_address: |
  1234 Background Blvd.
  Anywhere, CA 90000
support_tier: gold   # kept byte for byte
---

Hello.

| Description | Qty | Rate | Amount  |
|:------------|----:|-----:|--------:|
| Hours       | 1   | $100 | $100.00 |
|             |     | **Total due** | **$100.00** |
"
    )
}

fn vault(company: Option<&str>) -> MemVault {
    let vault = MemVault::new()
        .with_file(
            "ledgeline.toml",
            "[invoicer]\ncompany = \"[[My Company]]\"\n",
        )
        .with_file("company/logo.png", "PNG");
    match company {
        Some(text) => vault.with_file("company/My Company.md", text),
        None => vault,
    }
}

fn index(vault: &MemVault) -> DocumentIndex {
    let settings = Settings::read(vault).expect("settings");
    let journal =
        ledgeline_core::parse_journal("commodity $1,000.00\n", "books.journal").expect("journal");
    DocumentIndex::open(vault, settings, Money::of_journal(&journal))
}

#[test]
fn the_company_fields_are_what_new_writes() {
    let vault = vault(Some(&company_with_logo()));
    let idx = index(&vault);
    let fields = fields(idx.site(), idx.settings(), &vault).expect("a company");
    let keys: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, KEYS);
    assert_eq!(fields[5].1, Value::String("/company/logo.png".into()));
    assert_eq!(
        fields[4].1,
        Value::String("1234 Background Blvd.\nAnywhere, CA 90000\n".into())
    );
    // No note by that name: no company.
    let vault = self::vault(None);
    let idx = index(&vault);
    assert_eq!(super::fields(idx.site(), idx.settings(), &vault), None);
}

#[test]
fn edits_set_what_differs_and_remove_what_the_company_dropped() {
    let vault = vault(Some(COMPANY));
    let idx = index(&vault);
    let company = fields(idx.site(), idx.settings(), &vault).expect("a company");
    let current = |text: &str| {
        Invoice::parse(text, &idx.settings().words)
            .frontmatter()
            .fields
            .clone()
    };
    // The same details: nothing, whatever the block style of the address.
    assert_eq!(edits(&company, &current(&document("draft"))), vec![]);
    let chomped = document("draft").replace("invoicer_address: |", "invoicer_address: |-");
    assert_eq!(edits(&company, &current(&chomped)), vec![]);
    // A phone the company no longer has goes; a new address is set.
    let moved: Vec<(&str, Value)> = company
        .iter()
        .filter(|(k, _)| *k != "invoicer_phone")
        .map(|(k, v)| match *k {
            "invoicer_address" => (*k, Value::String("9 New St.\n".into())),
            _ => (*k, v.clone()),
        })
        .collect();
    assert_eq!(
        edits(&moved, &current(&document("draft"))),
        vec![
            FieldEdit::Set(
                "invoicer_address".into(),
                Value::String("9 New St.\n".into())
            ),
            FieldEdit::Remove("invoicer_phone".into()),
        ]
    );
}

#[test]
fn a_draft_is_shown_with_the_company_as_it_is_now() {
    let vault = vault(Some(&company_with_logo()));
    let idx = index(&vault);
    let draft = document("draft");
    let shown = as_shown(&idx, &vault, &draft);
    // Only the new key: everything else byte for byte.
    assert_eq!(
        shown,
        draft.replace(
            "support_tier: gold   # kept byte for byte\n",
            "support_tier: gold   # kept byte for byte\ninvoicer_logo: /company/logo.png\n"
        )
    );
    // Current already: as written, borrowed.
    assert!(matches!(as_shown(&idx, &vault, &shown), Cow::Borrowed(_)));
    // Sent and void keep what they were sent with.
    for status in ["sent", "void"] {
        let text = document(status);
        assert!(matches!(as_shown(&idx, &vault, &text), Cow::Borrowed(t) if t == text));
    }
}

#[test]
fn with_no_company_nothing_follows_and_nothing_differs() {
    let vault = vault(None);
    let idx = index(&vault);
    for status in ["draft", "sent"] {
        let text = document(status);
        assert!(matches!(as_shown(&idx, &vault, &text), Cow::Borrowed(_)));
        let invoice = Invoice::parse(text.as_str(), &idx.settings().words);
        assert!(!differs(&idx, &vault, &invoice));
        assert!(current_text(&idx, &vault, &text).expect("reads").is_none());
    }
    // And the setting unset entirely.
    let vault = MemVault::new().with_file("company/My Company.md", COMPANY);
    let idx = index(&vault);
    let text = document("draft");
    assert!(matches!(as_shown(&idx, &vault, &text), Cow::Borrowed(_)));
}

#[test]
fn only_a_sent_invoice_with_an_older_copy_differs() {
    let vault = vault(Some(&company_with_logo()));
    let idx = index(&vault);
    let parsed = |text: String| Invoice::parse(text, &idx.settings().words);
    assert!(differs(&idx, &vault, &parsed(document("sent"))));
    // A draft follows; a void invoice is history.
    assert!(!differs(&idx, &vault, &parsed(document("draft"))));
    assert!(!differs(&idx, &vault, &parsed(document("void"))));
    // The same company: nothing to offer.
    let vault = self::vault(Some(COMPANY));
    let idx = index(&vault);
    assert!(!differs(
        &idx,
        &vault,
        &Invoice::parse(document("sent"), &idx.settings().words)
    ));
}

#[test]
fn update_rewrites_only_the_company_keys_and_leaves_sent_hash() {
    let original = document("sent").replace(
        "support_tier:",
        &format!(
            "sent_hash: {}\nsupport_tier:",
            sent::hash(&document("sent"))
        ),
    );
    let moved = company_with_logo().replace("1234 Background Blvd.", "77 Harbor Way");
    let vault = vault(Some(&moved)).with_file("invoices/108.md", &original);
    let mut idx = index(&vault);
    let path = VaultPath::new("invoices/108.md").unwrap();
    let (source, revision) = crate::edit::read_text(&vault, &path).expect("reads");
    assert_eq!(
        sent::changed_since_sent(&Invoice::parse(source.as_str(), &idx.settings().words)),
        Some(false)
    );

    let outcome = update(&mut idx, &vault, &path, &source, &revision).expect("updates");
    assert!(outcome.written);
    assert_eq!(outcome.changed, ["invoicer_address", "invoicer_logo"]);
    let written = vault.read_to_string(&path).expect("reads");
    assert_eq!(
        written,
        original
            .replace("  1234 Background Blvd.\n", "  77 Harbor Way\n")
            .replace(
                "support_tier: gold   # kept byte for byte\n",
                "support_tier: gold   # kept byte for byte\ninvoicer_logo: /company/logo.png\n"
            )
    );
    // `sent_hash` is the last kept PDF's: untouched, so the next export
    // says the invoice differs from what the customer has.
    let after = Invoice::parse(written.as_str(), &idx.settings().words);
    assert_eq!(
        sent::recorded(&after),
        sent::recorded(&Invoice::parse(original.as_str(), &idx.settings().words))
    );
    assert_eq!(sent::changed_since_sent(&after), Some(true));
    assert!(!differs(&idx, &vault, &after));

    // Again: nothing to write.
    let (source, revision) = crate::edit::read_text(&vault, &path).expect("reads");
    let again = update(&mut idx, &vault, &path, &source, &revision).expect("updates");
    assert!(!again.written && again.changed.is_empty());
    assert_eq!(vault.read_to_string(&path).unwrap(), written);
}

#[test]
fn update_with_no_company_writes_nothing() {
    let vault = vault(None).with_file("invoices/108.md", document("sent"));
    let mut idx = index(&vault);
    let path = VaultPath::new("invoices/108.md").unwrap();
    let (source, revision) = crate::edit::read_text(&vault, &path).expect("reads");
    let outcome = update(&mut idx, &vault, &path, &source, &revision).expect("updates");
    assert!(!outcome.written && outcome.changed.is_empty());
    assert_eq!(vault.read_to_string(&path).unwrap(), document("sent"));
}
