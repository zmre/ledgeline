use super::*;
use crate::doc::Money;
use crate::settings::Settings;
use ledgeline_core::vault::MemVault;

const DRAFT: &str = "---
title: 2026-11-17 Customer X Invoice 108
type: invoice
template: /invoices/tpl/t.html
status: draft
invoice_num: 108
date: 2026-11-17
terms: 30
due_date: 2026-12-17
reference: \"PO #1\"
customer: \"[[Customer X]]\"
invoicee_name: Customer X
invoicee_email: ar@customerx.com
support_tier: gold   # a custom field, kept byte for byte
---

Next month's work.

| Code | Description            | Qty | Unit | Rate | Amount    | *Account*          |
|:-----|:-----------------------|----:|:-----|-----:|----------:|:-------------------|
| ARCH | Senior architect hours | 2   | h    | $800 | $1,600.00 | revenue:consulting |
|      |                        |     |      | **Total due** | **$1,600.00** |   |

## Payment

Pay by ACH.
";

const CUSTOMER_X: &str = "---\ntitle: Customer X\ntype: organization\ntags: [customer]\nemails:\n  - billing: ar@customerx.com\n---\n";
const WINGTIP: &str = "---\ntitle: Wingtip\ntype: organization\ntags: [customer]\nemails:\n  - billing: ap@wingtip.example\naddresses:\n  - billing:\n      street: 1 Wingtip Way\n      city: Springfield\n      region: IL\n      postcode: \"62701\"\nterms: 15\n---\n";

const PATH: &str = "invoices/108.md";

fn open(extra: &[(&str, &str)]) -> (MemVault, DocumentIndex) {
    let vault = [
        (PATH, DRAFT),
        ("customers/Customer X.md", CUSTOMER_X),
        ("customers/Wingtip.md", WINGTIP),
    ]
    .iter()
    .chain(extra)
    .fold(MemVault::new(), |v, (p, t)| v.with_file(p, t));
    let index = DocumentIndex::open(&vault, Settings::default(), Money::default());
    (vault, index)
}

fn path() -> VaultPath {
    VaultPath::new(PATH).expect("path")
}

fn parsed(text: &str) -> Invoice {
    Invoice::parse(text, &Settings::default().words)
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| (*c).to_string()).collect()
}

#[test]
fn the_form_of_a_document_reads_its_parts() {
    let form = form_of(&parsed(DRAFT));
    assert_eq!(form.customer, "[[Customer X]]");
    assert_eq!(form.number, "108");
    assert_eq!(form.terms, Some(30));
    assert_eq!(form.reference.as_deref(), Some("PO #1"));
    assert_eq!(form.prose_before, "Next month's work.");
    assert!(form.prose_after.starts_with("## Payment"));
    assert_eq!(
        form.fields,
        [CustomField {
            key: "support_tier".into(),
            value: Value::from("gold")
        }]
    );
    let table = form.table.expect("a table");
    let roles: Vec<&str> = table.columns.iter().map(|c| c.role.as_str()).collect();
    assert_eq!(
        roles,
        [
            "item",
            "description",
            "quantity",
            "unit",
            "rate",
            "amount",
            "books"
        ]
    );
    assert_eq!(table.rows[1][4], "**Total due**");
}

#[test]
fn an_unchanged_form_is_the_file_byte_for_byte() {
    let (_, index) = open(&[]);
    let invoice = parsed(DRAFT);
    let text = apply(&index, &path(), &invoice, &form_of(&invoice)).expect("applies");
    assert_eq!(text, DRAFT);
}

#[test]
fn a_row_edit_recomputes_and_keeps_everything_else() {
    let (_, index) = open(&[]);
    let invoice = parsed(DRAFT);
    let mut form = form_of(&invoice);
    let table = form.table.as_mut().expect("table");
    table.rows[0][2] = "10".into();
    let text = apply(&index, &path(), &invoice, &form).expect("applies");
    assert!(text.contains("$8,000.00"), "{text}");
    assert!(text.contains("**$8,000.00**"), "{text}");
    assert!(
        text.contains("support_tier: gold   # a custom field, kept byte for byte\n"),
        "{text}"
    );
    assert!(text.contains("\nNext month's work.\n\n| Code"), "{text}");
    assert!(text.ends_with("|\n\n## Payment\n\nPay by ACH.\n"), "{text}");
    assert_eq!(
        parsed(&text)
            .header()
            .number
            .as_ref()
            .map(InvoiceNumber::as_str),
        Some("108")
    );
}

/// The ARCHITECTURE §3.3 worked example, built row by row as the grid's
/// row-type buttons do, amounts left blank for the engine.
#[test]
fn row_types_compute_as_the_worked_example() {
    let (_, index) = open(&[]);
    let invoice = parsed(DRAFT);
    let mut form = form_of(&invoice);
    form.table = Some(GridForm {
        columns: ["Description", "Qty", "Rate", "Amount", "*Account*"]
            .iter()
            .map(|h| ColumnForm {
                header: (*h).into(),
                role: String::new(),
            })
            .collect(),
        rows: vec![
            row(&["Website redesign", "1", "$12,000", "", "revenue:projects"]),
            row(&[
                "Senior architect hours",
                "10",
                "$250",
                "",
                "revenue:consulting",
            ]),
            row(&["Loyalty discount", "", "-10%", "", ""]),
            row(&[
                "Annual support plan",
                "1",
                "$6,000",
                "",
                "liabilities:deferred-revenue → revenue:support, 12 months from 2026-12-01",
            ]),
            row(&["", "", "", "", ""]),
            row(&["", "", "**Subtotal**", "", ""]),
            row(&["Early-payment discount", "", "-2%", "", ""]),
            row(&["", "", "**Subtotal**", "", ""]),
            row(&["Sales tax", "", "8.25%", "", "liabilities:sales-tax"]),
            row(&[
                "Less deposit (Invoice 104)",
                "",
                "",
                "-$5,000.00",
                "liabilities:customer-deposits",
            ]),
            row(&["", "", "**Total due**", "", ""]),
        ],
    });
    let text = apply(&index, &path(), &invoice, &form).expect("applies");
    for amount in [
        "$12,000.00",
        "-$250.00",
        "**$20,250.00**",
        "-$405.00",
        "**$19,845.00**",
        "$1,637.21",
        "-$5,000.00",
        "**$16,482.21**",
    ] {
        assert!(text.contains(amount), "{amount} in {text}");
    }
    let summary = parsed(&text)
        .summary(&index.settings().words, index.money())
        .expect("adds up");
    assert_eq!(summary.total_text, "$16,482.21");
    assert_eq!(summary.discrepancies, 0);
}

#[test]
fn prose_and_custom_fields_change_by_diff() {
    let (_, index) = open(&[]);
    let invoice = parsed(DRAFT);
    let mut form = form_of(&invoice);
    form.prose_before = String::new();
    form.prose_after = "Thanks!".into();
    form.fields = vec![CustomField {
        key: "project".into(),
        value: Value::from("Apollo"),
    }];
    let text = apply(&index, &path(), &invoice, &form).expect("applies");
    assert!(!text.contains("support_tier"), "{text}");
    assert!(text.contains("project: Apollo\n"), "{text}");
    assert!(text.contains("---\n\n| Code"), "{text}");
    assert!(text.ends_with("|\n\nThanks!\n"), "{text}");
    assert!(!text.contains("Next month's work"));

    form.fields = vec![CustomField {
        key: "status".into(),
        value: Value::from("sent"),
    }];
    assert!(matches!(
        apply(&index, &path(), &invoice, &form),
        Err(EditorError::Invalid { .. })
    ));
}

#[test]
fn a_new_customer_is_snapshotted_and_terms_give_the_due_date() {
    let (_, index) = open(&[]);
    let invoice = parsed(DRAFT);
    let mut form = form_of(&invoice);
    form.customer = "Wingtip".into();
    form.terms = Some(15);
    form.due_date = None;
    let text = apply(&index, &path(), &invoice, &form).expect("applies");
    let header = parsed(&text).header().clone();
    assert_eq!(header.invoicee.link.as_deref(), Some("[[Wingtip]]"));
    assert_eq!(header.invoicee.name.as_deref(), Some("Wingtip"));
    assert_eq!(header.invoicee.email.as_deref(), Some("ap@wingtip.example"));
    assert!(
        header
            .invoicee
            .address
            .as_deref()
            .is_some_and(|a| a.contains("1 Wingtip Way")),
        "{text}"
    );
    assert_eq!(header.due_date.as_deref(), Some("2026-12-02"));

    form.customer = "Nobody".into();
    assert!(matches!(
        apply(&index, &path(), &invoice, &form),
        Err(EditorError::Customer(_))
    ));
}

#[test]
fn save_is_conditional_on_the_revision_read() {
    let (vault, mut index) = open(&[]);
    let invoice = parsed(DRAFT);
    let mut form = form_of(&invoice);
    form.reference = Some("PO #2".into());
    let read = revision_of(DRAFT);

    // An outside edit since the editor read it: refused, file untouched.
    let theirs = DRAFT.replace("Pay by ACH.", "Pay by wire.");
    vault
        .write_atomic(&path(), theirs.as_bytes())
        .expect("outside edit");
    let refused = save(&mut index, &vault, &path(), &form, &read, false);
    match refused {
        Err(error @ EditorError::Changed { .. }) => assert_eq!(error.code(), "changed"),
        other => panic!("{other:?}"),
    }
    assert_eq!(vault.read_to_string(&path()).expect("reads"), theirs);

    // Over their revision, deliberately: written, the rest of theirs kept.
    let saved = save(
        &mut index,
        &vault,
        &path(),
        &form,
        &revision_of(&theirs),
        false,
    )
    .expect("saves");
    assert!(saved.written);
    let text = vault.read_to_string(&path()).expect("reads");
    assert!(text.contains("reference: \"PO #2\""), "{text}");
    assert_eq!(saved.revision, revision_of(&text));
    assert_eq!(
        index
            .doc(&path())
            .and_then(|d| d.header.reference.clone())
            .as_deref(),
        Some("PO #2")
    );

    // Saving the same form again writes nothing.
    let again = save(&mut index, &vault, &path(), &form, &saved.revision, false).expect("saves");
    assert!(!again.written);
}

#[test]
fn sent_and_void_documents_and_numbers_follow_the_lifecycle() {
    let other = DRAFT
        .replace("invoice_num: 108", "invoice_num: 109")
        .replace("status: draft", "status: sent");
    let (vault, mut index) = open(&[("invoices/109.md", &other)]);
    let mut form = form_of(&parsed(DRAFT));
    form.number = "109".into();
    let taken = save(
        &mut index,
        &vault,
        &path(),
        &form,
        &revision_of(DRAFT),
        false,
    );
    assert!(
        matches!(&taken, Err(e) if e.code() == "refused"),
        "{taken:?}"
    );
    form.number = "110".into();
    let renumbered = save(
        &mut index,
        &vault,
        &path(),
        &form,
        &revision_of(DRAFT),
        false,
    )
    .expect("renumbers");
    assert_eq!(renumbered.number.as_deref(), Some("110"));

    let sent_path = VaultPath::new("invoices/109.md").expect("path");
    let mut sent_form = form_of(&parsed(&other));
    sent_form.reference = Some("PO #9".into());
    let asked = save(
        &mut index,
        &vault,
        &sent_path,
        &sent_form,
        &revision_of(&other),
        false,
    );
    assert!(
        matches!(&asked, Err(e) if e.code() == "confirm-rebook"),
        "{asked:?}"
    );
    assert_eq!(vault.read_to_string(&sent_path).expect("reads"), other);
    save(
        &mut index,
        &vault,
        &sent_path,
        &sent_form,
        &revision_of(&other),
        true,
    )
    .expect("saves");

    let void = DRAFT.replace("status: draft", "status: void");
    vault
        .write_atomic(&path(), void.as_bytes())
        .expect("writes");
    let refused = save(
        &mut index,
        &vault,
        &path(),
        &form,
        &revision_of(&void),
        true,
    );
    assert!(
        matches!(&refused, Err(e) if e.code() == "void"),
        "{refused:?}"
    );
}

#[test]
fn a_broken_grid_is_refused_with_its_field() {
    let (_, index) = open(&[]);
    let invoice = parsed(DRAFT);
    let mut form = form_of(&invoice);
    form.table.as_mut().expect("table").rows[0].push("extra".into());
    assert!(matches!(
        apply(&index, &path(), &invoice, &form),
        Err(EditorError::Invalid { field, .. }) if field == "table row 1"
    ));
    let mut form = form_of(&invoice);
    form.date = "17/11/2026".into();
    assert!(matches!(
        apply(&index, &path(), &invoice, &form),
        Err(EditorError::Invalid { field, .. }) if field == "date"
    ));
}
