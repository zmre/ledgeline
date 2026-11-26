use super::*;
use crate::doc::{Money, Status};
use ledgeline_core::vault::MemVault;

const CUSTOMER_X: &str = "---
title: Customer X
type: organization
tags: [customer]
emails:
  - billing: ar@customerx.com
phones:
  - work: \"+1 303 555 0100\"
addresses:
  - billing: |
      1234 Main St. #555
      Springfield, CO 80000
terms: 30
invoice_template: /invoices/tpl/myco.html
snippets: [\"[[Net 30 ACH]]\"]
---
";

const CATALOG: &str = "---
type: catalog
---

| Code | Description            | Unit | Rate | *Account*          |
|:-----|:-----------------------|:-----|-----:|:-------------------|
| ARCH | Senior architect hours | h    | $800 | revenue:consulting |
";

fn vault() -> MemVault {
    MemVault::new()
        .with_file("ledgeline.toml", "[invoicer]\ncompany = \"[[My Company]]\"\n")
        .with_file("customers/Customer X.md", CUSTOMER_X)
        .with_file(
            "customers/Fred.md",
            "---\ntype: person\ntags: [customer]\ncompany: \"[[Customer X]]\"\n---\n",
        )
        .with_file("customers/Plain.md", "---\ntype: person\n---\n")
        .with_file(
            "orgs/My Company.md",
            "---\ntype: organization\nemails: [finance@mycompany.com]\nphones: [\"(555) 010-0000\"]\naddresses:\n  - work: \"1 Background Blvd.\\nAnywhere, CA 90000\"\n---\n",
        )
        .with_file("invoices/tpl/myco.html", "{# invoicer: name=\"MyCo\" #}\n{{ frontmatter.invoice_num }}")
        .with_file("invoices/tpl/snippets/net-30.md", "---\ntitle: Net 30 ACH\ntype: snippet\n---\n\n## Payment\n\nPay by ACH within 30 days.\n")
        .with_file("invoices/catalog.md", CATALOG)
        .with_file(
            "invoices/old.md",
            "---\ntype: invoice\ninvoice_num: 104\n---\n\n| Description | Amount |\n|---|---|\n| x | $1 |\n",
        )
}

fn index(vault: &MemVault) -> DocumentIndex {
    let settings = Settings::read(vault).expect("settings");
    let journal =
        ledgeline_core::parse_journal("commodity $1,000.00\n", "books.journal").expect("journal");
    DocumentIndex::open(vault, settings, Money::of_journal(&journal))
}

fn arch(vault: &MemVault, index: &DocumentIndex) -> DraftRow {
    let (catalogs, _) =
        crate::catalog::load(vault, index.site(), &index.settings().words, index.money());
    DraftRow::from_catalog(catalogs[0].item("ARCH").expect("ARCH"), "10")
}

#[test]
fn a_draft_snapshots_the_customer_and_company() {
    let vault = vault();
    let index = index(&vault);
    let mut new = NewInvoice::new("[[Customer X]]", "2026-11-17");
    new.rows = vec![
        arch(&vault, &index),
        DraftRow {
            description: "Friends and family discount".into(),
            rate: Some("-10%".into()),
            ..DraftRow::default()
        },
    ];
    new.reference = Some("PO #PY75334".into());
    new.prose_before = Some("Hi Fred.".into());
    let draft = render(&index, &vault, &new, &[]).expect("renders");
    assert_eq!(
        draft.path.as_str(),
        "invoices/2026-11-17 Customer X Invoice 105.md"
    );
    assert_eq!(draft.number.as_str(), "105");
    let expected = "---
title: 2026-11-17 Customer X Invoice 105
type: invoice
template: /invoices/tpl/myco.html
status: draft
invoice_num: 105
date: 2026-11-17
terms: 30
due_date: 2026-12-17
reference: \"PO #PY75334\"
customer: \"[[Customer X]]\"
invoicee_name: Customer X
invoicee_email: ar@customerx.com
invoicee_phone: +1 303 555 0100
invoicee_address: |
  1234 Main St. #555
  Springfield, CO 80000
invoicer: \"[[My Company]]\"
invoicer_name: My Company
invoicer_email: finance@mycompany.com
invoicer_phone: (555) 010-0000
invoicer_address: |
  1 Background Blvd.
  Anywhere, CA 90000
---

Hi Fred.

| Code | Description                 | Qty | Unit | Rate          | Amount        | *Account*          |
|:-----|:----------------------------|----:|:-----|--------------:|--------------:|:-------------------|
| ARCH | Senior architect hours      | 10  | h    | $800          | $8,000.00     | revenue:consulting |
|      | Friends and family discount |     |      | -10%          | -$800.00      |                    |
|      |                             |     |      | **Total due** | **$7,200.00** |                    |

## Payment

Pay by ACH within 30 days.
";
    let invoice = Invoice::parse(draft.text.as_str(), &Settings::default().words);
    assert_eq!(invoice.header().status, Some(Status::Draft));
    assert_eq!(
        invoice.problems(&Settings::default().words, index.money()),
        vec![],
        "{}",
        draft.text
    );
    // The exact text, so a change in layout is a decision, not an accident.
    assert_eq!(draft.text, expected);
}

#[test]
fn a_draft_snapshots_the_company_logo_when_it_has_one() {
    // Without `logo:` (the vault above): no key at all.
    let vault = vault();
    let idx = index(&vault);
    let new = NewInvoice::new("[[Customer X]]", "2026-11-17");
    let draft = render(&idx, &vault, &new, &[]).expect("renders");
    assert!(!draft.text.contains("invoicer_logo"), "{}", draft.text);

    // With one beside the note: its path from the books folder, rooted.
    let vault = vault
        .with_file(
            "orgs/My Company.md",
            "---\ntype: organization\naddresses:\n  - work: 1 Background Blvd.\nlogo: \"[[My Logo.png]]\"\n---\n",
        )
        .with_file("orgs/My Logo.png", "PNG");
    let idx = index(&vault);
    let draft = render(&idx, &vault, &new, &[]).expect("renders");
    assert!(
        draft.text.contains(
            "invoicer_address: 1 Background Blvd.\ninvoicer_logo: /orgs/My Logo.png\n---"
        ),
        "{}",
        draft.text
    );

    // Named but missing: no key (nothing to show).
    let vault = vault.with_file(
        "orgs/My Company.md",
        "---\ntype: organization\nlogo: gone.png\n---\n",
    );
    let idx = index(&vault);
    let draft = render(&idx, &vault, &new, &[]).expect("renders");
    assert!(!draft.text.contains("invoicer_logo"), "{}", draft.text);
}

#[test]
fn a_person_at_a_company_is_billed_as_the_company() {
    let vault = vault();
    let index = index(&vault);
    let draft =
        render(&index, &vault, &NewInvoice::new("Fred", "2026-11-17"), &[]).expect("renders");
    assert_eq!(
        draft.path.as_str(),
        "invoices/2026-11-17 Customer X Invoice 105.md"
    );
    assert!(
        draft
            .text
            .contains("customer: \"[[Fred]]\"\ninvoicee_name: Customer X\n"),
        "{}",
        draft.text
    );
    assert!(!draft.text.contains("terms:"), "Fred has no terms");
}

#[test]
fn numbering_and_refusals() {
    let vault = vault();
    let index = index(&vault);
    let books = render(
        &index,
        &vault,
        &NewInvoice::new("[[Customer X]]", "2026-11-17"),
        &["2004"],
    )
    .expect("renders");
    assert_eq!(books.number.as_str(), "2005");
    let mut taken = NewInvoice::new("[[Customer X]]", "2026-11-17");
    taken.number = Some("104".into());
    assert!(matches!(
        render(&index, &vault, &taken, &[]),
        Err(DraftError::DuplicateNumber { .. })
    ));
    taken.number = Some("0042".into());
    let padded = render(&index, &vault, &taken, &[]).expect("renders");
    assert!(
        padded.text.contains("invoice_num: \"0042\"\n"),
        "{}",
        padded.text
    );
    taken.number = Some("1,2".into());
    assert!(matches!(
        render(&index, &vault, &taken, &[]),
        Err(DraftError::BadNumber(_))
    ));
    assert!(matches!(
        render(
            &index,
            &vault,
            &NewInvoice::new("[[Nobody]]", "2026-11-17"),
            &[]
        ),
        Err(DraftError::UnknownCustomer(_))
    ));
    assert!(matches!(
        render(
            &index,
            &vault,
            &NewInvoice::new("[[Plain]]", "2026-11-17"),
            &[]
        ),
        Err(DraftError::NotACustomer(_))
    ));
    assert!(matches!(
        render(
            &index,
            &vault,
            &NewInvoice::new("[[Customer X]]", "17/11/2026"),
            &[]
        ),
        Err(DraftError::BadDate(_))
    ));
}

#[test]
fn create_writes_once_and_indexes() {
    let vault = vault();
    let mut index = index(&vault);
    let draft = render(
        &index,
        &vault,
        &NewInvoice::new("[[Customer X]]", "2026-11-17"),
        &[],
    )
    .expect("renders");
    assert!(
        draft.text.contains("| **Total due** | **$0.00** |"),
        "an empty draft: {}",
        draft.text
    );
    let revision = create(&mut index, &vault, &draft).expect("creates");
    assert_eq!(
        vault.read_to_string(&draft.path).expect("written"),
        draft.text
    );
    assert_eq!(index.by_number("105").len(), 1);
    assert_eq!(index.next_number([]).expect("next").as_str(), "106");
    assert!(matches!(
        create(&mut index, &vault, &draft),
        Err(DraftError::Exists(_))
    ));

    // Save is conditional on the revision read.
    let edited =
        frontmatter::set(&draft.text, "reference", Value::String("PO 9".into())).expect("edits");
    let saved = save(&mut index, &vault, &draft.path, &edited, &revision).expect("saves");
    assert_eq!(
        index.by_number("105")[0].header.reference.as_deref(),
        Some("PO 9")
    );
    assert!(matches!(
        save(&mut index, &vault, &draft.path, &draft.text, &revision),
        Err(DraftError::Vault(VaultError::Changed { .. }))
    ));
    // A save that wouldn't read back is refused before writing.
    let broken = edited.replace("**$0.00**", "**$5.00**");
    assert!(matches!(
        save(&mut index, &vault, &draft.path, &broken, &saved),
        Err(DraftError::Verify(_))
    ));
    assert_eq!(
        vault.read_to_string(&draft.path).expect("unchanged"),
        edited
    );
    assert!(index.rebuild_and_diff(&vault).is_empty());
}

#[test]
fn filenames_are_cleaned() {
    assert_eq!(
        fill_pattern(
            "{date} {customer} Invoice {invoice_num}.md",
            "2026-11-17",
            "A/B: C",
            "7"
        ),
        "2026-11-17 A-B- C Invoice 7.md"
    );
    assert_eq!(fill_pattern("{invoice_num}  x", "", "", "9"), "9 x");
}

/// Books with a customer who names no terms or template, a second
/// template, an estimate-only one, and `[invoicer]` set to `toml`.
fn defaults_vault(toml: &str) -> MemVault {
    vault()
        .with_file("ledgeline.toml", toml)
        .with_file(
            "customers/Solo.md",
            "---\ntitle: Solo\ntype: organization\ntags: [customer]\n---\n",
        )
        .with_file(
            "invoices/tpl/plain.html",
            "{# invoicer: name=\"Plain\" #}\n{{ frontmatter.invoice_num }}",
        )
        .with_file(
            "invoices/tpl/quote.html",
            "{# invoicer: name=\"Quote\" kinds=\"estimate\" #}\n{{ frontmatter.invoice_num }}",
        )
}

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
}

#[test]
fn terms_come_from_the_invoice_then_the_customer_then_the_default() {
    let vault = defaults_vault("[invoicer]\ndefault-terms = 14\n");
    let index = index(&vault);
    let draft = |customer: &str, terms: Option<u32>| {
        let mut new = NewInvoice::new(customer, "2026-11-01");
        new.terms = terms;
        render(&index, &vault, &new, &[]).expect("renders")
    };
    let own = draft("[[Solo]]", Some(7));
    assert_eq!(field(&own.text, "terms"), Some("7"));
    assert_eq!(field(&own.text, "due_date"), Some("2026-11-08"));
    // Customer X's own terms (30) win over the default.
    assert_eq!(
        field(&draft("[[Customer X]]", None).text, "terms"),
        Some("30")
    );
    let defaulted = draft("[[Solo]]", None);
    assert_eq!(field(&defaulted.text, "terms"), Some("14"));
    assert_eq!(field(&defaulted.text, "due_date"), Some("2026-11-15"));
    assert!(defaulted.warnings.is_empty());
    // No default: no terms, no due date.
    let bare = defaults_vault("[invoicer]\n");
    let bare_index = self::index(&bare);
    let none = render(
        &bare_index,
        &bare,
        &NewInvoice::new("[[Solo]]", "2026-11-01"),
        &[],
    )
    .expect("renders");
    assert_eq!(field(&none.text, "terms"), None);
    assert_eq!(field(&none.text, "due_date"), None);
}

#[test]
fn the_template_comes_from_the_invoice_then_the_customer_then_the_default() {
    let vault = defaults_vault("[invoicer]\ndefault-template = \"/invoices/tpl/plain.html\"\n");
    let index = index(&vault);
    let draft = |customer: &str, template: Option<&str>| {
        let mut new = NewInvoice::new(customer, "2026-11-01");
        new.template = template.map(str::to_string);
        render(&index, &vault, &new, &[]).expect("renders")
    };
    assert_eq!(
        field(&draft("[[Solo]]", Some("/x.html")).text, "template"),
        Some("/x.html")
    );
    assert_eq!(
        field(&draft("[[Customer X]]", None).text, "template"),
        Some("/invoices/tpl/myco.html")
    );
    let defaulted = draft("[[Solo]]", None);
    assert_eq!(
        field(&defaulted.text, "template"),
        Some("/invoices/tpl/plain.html")
    );
    assert!(defaulted.warnings.is_empty());
    // Without a default: the first template for invoices (myco, by path).
    let bare = defaults_vault("[invoicer]\n");
    let bare_index = self::index(&bare);
    let first = render(
        &bare_index,
        &bare,
        &NewInvoice::new("[[Solo]]", "2026-11-01"),
        &[],
    )
    .expect("renders");
    assert_eq!(
        field(&first.text, "template"),
        Some("/invoices/tpl/myco.html")
    );
}

#[test]
fn a_missing_default_template_falls_back_and_says_so() {
    for (default, says) in [
        ("/invoices/tpl/gone.html", "isn't in the books"),
        ("/invoices/tpl/quote.html", "isn't for invoices"),
    ] {
        let vault = defaults_vault(&format!("[invoicer]\ndefault-template = \"{default}\"\n"));
        let index = index(&vault);
        let draft = render(
            &index,
            &vault,
            &NewInvoice::new("[[Solo]]", "2026-11-01"),
            &[],
        )
        .expect("renders");
        assert_eq!(
            field(&draft.text, "template"),
            Some("/invoices/tpl/myco.html"),
            "{default}"
        );
        assert_eq!(draft.warnings.len(), 1, "{default}");
        assert!(draft.warnings[0].contains(says), "{}", draft.warnings[0]);
    }
}
