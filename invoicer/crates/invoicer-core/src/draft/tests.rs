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
