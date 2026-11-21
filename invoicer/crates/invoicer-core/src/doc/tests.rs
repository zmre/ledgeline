use super::*;
use serde_json::json;

const SPEC: &str = include_str!("../../tests/fixtures/invoice-3-2.md");

fn usd() -> Money {
    let journal =
        ledgeline_core::parse_journal("commodity $1,000.00\n", "books.journal").expect("journal");
    Money::of_journal(&journal)
}

fn parse(source: &str) -> Invoice {
    Invoice::parse(source, &Words::default())
}

fn codes(invoice: &Invoice) -> Vec<ProblemCode> {
    invoice
        .problems(&Words::default(), &usd())
        .iter()
        .map(|p| p.code)
        .collect()
}

#[test]
fn the_spec_invoice_reads_and_has_no_problems() {
    let invoice = parse(SPEC);
    let h = invoice.header();
    assert_eq!(h.doc_type, Some(DocType::Invoice));
    assert_eq!(h.number.as_ref().map(InvoiceNumber::as_str), Some("105"));
    assert_eq!(h.status, Some(Status::Sent));
    assert_eq!(h.terms, Some(30));
    assert_eq!(h.reference.as_deref(), Some("PO #PY75334"));
    assert_eq!(h.invoicee.link.as_deref(), Some("[[Customer X]]"));
    assert_eq!(h.invoicer.name.as_deref(), Some("My Company"));
    assert_eq!(h.paid, Some(Paid::No));
    assert!(invoice.prose_before().contains("Hi Fred"));
    assert!(invoice.prose_after().contains("## Payment"));
    assert!(invoice.bill_table().expect("reads").is_some());
    assert_eq!(invoice.problems(&Words::default(), &usd()), vec![]);
    let summary = invoice.summary(&Words::default(), &usd()).expect("summary");
    assert_eq!(summary.total_text, "$8,350.00");
    assert_eq!(
        summary.accounts,
        [
            "revenue:consulting",
            "revenue:consulting:delivery",
            "liabilities:customer-deposits"
        ]
    );
    assert_eq!(summary.discrepancies, 0);
}

#[test]
fn numbers_keep_their_written_form() {
    let invoice = parse("---\ninvoice_num: 0042\n---\n");
    assert_eq!(
        invoice.header().number.as_ref().map(InvoiceNumber::as_str),
        Some("0042")
    );
    let invoice = parse("---\ninvoice_num: 2026-0042\n---\n");
    assert_eq!(
        invoice.header().number.as_ref().map(InvoiceNumber::as_str),
        Some("2026-0042")
    );
    assert_eq!(InvoiceNumber::new("1,2"), None);
    assert_eq!(InvoiceNumber::new("  "), None);
}

#[test]
fn a_yaml_table_in_frontmatter_is_never_the_bill_table() {
    let doc = "---\nnote: |\n  | Description | Amount |\n  |---|---|\n  | x | $1 |\n---\n\nNo table here.\n";
    let invoice = parse(doc);
    assert!(invoice.bill_table().expect("reads").is_none());
    assert!(codes(&invoice).contains(&ProblemCode::NoBillTable));
}

#[test]
fn supporting_tables_are_located_and_left_alone() {
    let doc = format!("{SPEC}\n| Date | Hours |\n|---|---|\n| 2026-11-01 | 3 |\n");
    let invoice = parse(&doc);
    assert_eq!(invoice.supporting_tables().count(), 1);
    assert!(
        invoice
            .supporting_tables()
            .next()
            .is_some_and(|t| t.starts_with("| Date"))
    );
}

#[test]
fn field_problems_name_lines_and_fixes() {
    let doc = "---
type: invoice
invoice_num: \"10,5\"
date: 2026-13-01
status: paid
template: invoices/tpl/x.htm
customer: Customer X
invoicee_name: X
invoicee_address: >
  1 Main
  Town
invoicer: \"[[Me]]\"
invoicer_name: Me
terms: thirty
po_num: \"PO 1\"
paid: maybe
---

| Description | Amount |
|---|---|
| Work | $5 |
";
    let invoice = parse(doc);
    let problems = invoice.problems(&Words::default(), &usd());
    let find = |code| {
        problems
            .iter()
            .find(|p| p.code == code)
            .unwrap_or_else(|| panic!("{code:?} in {problems:#?}"))
    };
    assert_eq!(find(ProblemCode::BadNumber).line, Some(3));
    assert_eq!(find(ProblemCode::BadDate).line, Some(4));
    assert_eq!(find(ProblemCode::BadStatus).line, Some(5));
    assert_eq!(find(ProblemCode::FoldedText).line, Some(9));
    assert_eq!(find(ProblemCode::BadTerms).line, Some(14));
    assert_eq!(find(ProblemCode::BadPaid).line, Some(16));
    let legacy = find(ProblemCode::LegacyField);
    assert_eq!(
        legacy.fix.as_ref().and_then(|f| f.edit.clone()),
        Some(FixEdit::RenameField {
            from: "po_num".into(),
            to: "reference".into()
        })
    );
    let customer = problems
        .iter()
        .find(|p| p.code == ProblemCode::BadField && p.message.contains("wikilink"))
        .expect("customer");
    assert_eq!(
        customer.fix.as_ref().and_then(|f| f.edit.clone()),
        Some(FixEdit::SetField {
            key: "customer".into(),
            value: json!("[[Customer X]]")
        })
    );
    assert!(
        problems
            .iter()
            .any(|p| p.code == ProblemCode::BadField && p.message.contains("template"))
    );
    assert!(problems.iter().any(|p| p.code == ProblemCode::MissingField
        && p.message.contains("invoicee_email")
        && !p.is_error()));
}

#[test]
fn due_dates_are_checked_and_filled() {
    let base = SPEC.replace("due_date: 2026-12-17\n", "");
    let invoice = parse(&base);
    let problems = invoice.problems(&Words::default(), &usd());
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert_eq!(
        problems[0].fix.as_ref().and_then(|f| f.edit.clone()),
        Some(FixEdit::SetField {
            key: "due_date".into(),
            value: json!("2026-12-17")
        })
    );
    let early = SPEC.replace("due_date: 2026-12-17", "due_date: 2026-11-01");
    assert_eq!(codes(&parse(&early)), [ProblemCode::DueBeforeDate]);
}

#[test]
fn table_problems_and_recalc() {
    let wrong = SPEC.replace("| $8,000       |", "| $9,000       |");
    let invoice = parse(&wrong);
    let problems = invoice.problems(&Words::default(), &usd());
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert_eq!(problems[0].code, ProblemCode::Arithmetic);
    assert_eq!(problems[0].line, Some(30));
    assert_eq!(
        problems[0].fix.as_ref().and_then(|f| f.edit.clone()),
        Some(FixEdit::Recalc)
    );
    let fixed = invoice
        .recalc(&Words::default(), &usd())
        .expect("recalcs")
        .expect("changed");
    assert!(
        fixed.starts_with(&SPEC[..SPEC.find("| Senior").expect("row")]),
        "frontmatter and prose untouched"
    );
    assert!(parse(&fixed).problems(&Words::default(), &usd()).is_empty());
    assert_eq!(
        parse(SPEC)
            .recalc(&Words::default(), &usd())
            .expect("recalcs"),
        None
    );

    let after_total = format!(
        "{}| Tip | | | $5 | |\n",
        &SPEC[..SPEC.find("\n## Payment").expect("payment")]
    );
    assert!(codes(&parse(&after_total)).contains(&ProblemCode::RowsAfterTotal));
    let bad_header = SPEC.replace(
        "| Description                  | Qty |",
        "| Description                  | Item |",
    );
    let problems = parse(&bad_header).problems(&Words::default(), &usd());
    assert_eq!(problems[0].code, ProblemCode::Table);
    assert_eq!(problems[0].line, Some(28));
}

#[test]
fn broken_frontmatter_is_one_problem_and_the_body_still_reads() {
    let invoice = parse("---\na: [\n---\n\n| Description | Amount |\n|---|---|\n| x | $1 |\n");
    assert!(invoice.frontmatter_error().is_some());
    assert_eq!(codes(&invoice), [ProblemCode::Frontmatter]);
    assert!(invoice.bill_table().expect("reads").is_some());
}

#[test]
fn paid_mirror_grammar() {
    assert_eq!(Paid::parse(&json!("no")), Some(Paid::No));
    assert_eq!(Paid::parse(&json!(false)), Some(Paid::No));
    assert_eq!(
        Paid::parse(&json!("2026-11-19")),
        Some(Paid::Full {
            date: "2026-11-19".into()
        })
    );
    let partial =
        Paid::parse(&json!("partial 2026-10-28 $1,500, 2026-11-19 $3,500")).expect("partial");
    assert_eq!(
        partial,
        Paid::Partial {
            parts: vec![
                PaidPart {
                    date: "2026-10-28".into(),
                    amount: "$1,500".into()
                },
                PaidPart {
                    date: "2026-11-19".into(),
                    amount: "$3,500".into()
                },
            ]
        }
    );
    assert_eq!(
        partial.to_string(),
        "partial 2026-10-28 $1,500, 2026-11-19 $3,500"
    );
    for bad in [
        json!("yes"),
        json!("partial"),
        json!("partial soon $5"),
        json!(3),
    ] {
        assert_eq!(Paid::parse(&bad), None, "{bad}");
    }
}

#[test]
fn statuses_follow_the_type() {
    let estimate = SPEC
        .replace("type: invoice", "type: estimate")
        .replace("status: sent", "status: estimate-open");
    assert!(!codes(&parse(&estimate)).contains(&ProblemCode::BadStatus));
    let wrong = SPEC.replace("type: invoice", "type: estimate");
    assert!(codes(&parse(&wrong)).contains(&ProblemCode::BadStatus));
    assert_eq!(DocType::parse(" Credit-Note "), Some(DocType::CreditNote));
}
