use super::*;
use crate::doc::Money;
use crate::settings::Settings;
use ledgeline_core::vault::MemVault;
use serde_json::json;

const ACME: &str = "---
title: Acme
type: organization
tags: [customer]
---
";

const NOT_A_CUSTOMER: &str = "---
title: Globex
type: organization
---
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

fn import(
    vault: &MemVault,
    index: &mut DocumentIndex,
    csv: &str,
    options: ImportOptions,
) -> Result<ImportReport, ImportError> {
    import_csv(index, vault, csv.as_bytes(), &options)
}

fn row_errors(result: Result<ImportReport, ImportError>) -> Vec<RowError> {
    match result {
        Err(ImportError::Rows(rows)) => rows,
        other => panic!("expected row errors, got {other:?}"),
    }
}

#[test]
fn headers_are_forgiving_on_case_spaces_and_separators() {
    for header in ["Revenue Account", " revenue-account ", "REVENUE__ACCOUNT"] {
        assert_eq!(
            Column::from_header(header),
            Some(Column::RevenueAccount),
            "{header}"
        );
    }
    assert_eq!(Column::from_header("Title"), Some(Column::Name));
    assert_eq!(Column::from_header("TYPE"), Some(Column::Kind));
    assert_eq!(
        Column::from_header("Template"),
        Some(Column::InvoiceTemplate)
    );
    assert_eq!(Column::from_header("colour"), None);
    // Every column's own header reads back as it.
    for column in Column::ALL {
        assert_eq!(Column::from_header(column.header()), Some(column));
    }
}

#[test]
fn the_fields_are_what_the_app_sends() {
    let fields = fields_from(&[
        (Column::Name, "Northwind Traders"),
        (Column::Email, " ap@northwind.example "),
        (Column::Address, "1 Main St\nSpringfield"),
        (Column::Terms, "15"),
        (Column::Phone, ""),
    ])
    .expect("reads");
    let app: CustomerFields = serde_json::from_value(json!({
        "title": "Northwind Traders",
        "kind": "organization",
        "emails": [{"label": "billing", "value": "ap@northwind.example"}],
        "addresses": [{"label": "billing", "value": "1 Main St\nSpringfield"}],
        "terms": 15
    }))
    .expect("fields");
    assert_eq!(fields, app);
    let portal = fields_from(&[
        (Column::Name, "X"),
        (Column::Kind, "Person"),
        (Column::PortalName, "Coupa"),
        (Column::PortalRequiresReference, "yes"),
        (Column::Contracts, "[[MSA]]; /customers/sow.pdf;"),
    ])
    .expect("reads");
    assert_eq!(portal.kind, Some(ContactKind::Person));
    let portal_fields = portal.billing_portal.flatten().expect("portal");
    assert_eq!(portal_fields.name.as_deref(), Some("Coupa"));
    assert!(portal_fields.requires_reference);
    assert_eq!(
        portal.contracts,
        Some(vec![
            "[[MSA]]".to_string(),
            "/customers/sow.pdf".to_string()
        ])
    );
    // A blank portal is no portal.
    let none = fields_from(&[(Column::Name, "X"), (Column::PortalName, " ")]).expect("reads");
    assert_eq!(none.billing_portal, None);
    assert!(fields_from(&[(Column::Kind, "company")]).is_err());
    assert!(fields_from(&[(Column::Terms, "net 30")]).is_err());
    assert!(fields_from(&[(Column::PortalRequiresReference, "maybe")]).is_err());
}

#[test]
fn an_import_writes_each_row_as_create_would() {
    let (vault, mut index) = open(&[]);
    let csv = "\u{feff}Name,Kind,E-mail Address,Address,Terms\n\
               \"Smith, Jones & Co\",organization,ap@sj.example,\"1 Main St\nSpringfield, CO\",30\n\
               \n\
               Zoë Müller,person,zoe@example.de,,\n";
    // `E-mail Address` isn't a column: refused, listing the ones there are.
    let refused = import(&vault, &mut index, csv, ImportOptions::default());
    let Err(ImportError::Header(message)) = refused else {
        panic!("{refused:?}")
    };
    assert!(message.contains("\"E-mail Address\""), "{message}");
    assert!(message.contains(&Column::listed()), "{message}");
    assert!(
        vault
            .list_dir(&VaultPath::root(), None)
            .expect("lists")
            .is_empty()
    );

    let csv = csv.replace("E-mail Address", "Email");
    let report = import(&vault, &mut index, &csv, ImportOptions::default()).expect("imports");
    let made: Vec<(&str, u64)> = report
        .created
        .iter()
        .map(|r| (r.saved.title.as_str(), r.line))
        .collect();
    assert_eq!(made, [("Smith, Jones & Co", 2), ("Zoë Müller", 5)]);
    assert!(report.skipped.is_empty());
    let text = vault
        .read_to_string(&path("customers/Smith, Jones & Co.md"))
        .expect("written");
    // The same bytes the app's create writes for the same fields.
    let (other, mut fresh) = open(&[]);
    let app = customer_edit::create(
        &mut fresh,
        &other,
        &serde_json::from_value(json!({
            "title": "Smith, Jones & Co",
            "kind": "organization",
            "emails": [{"label": "billing", "value": "ap@sj.example"}],
            "addresses": [{"label": "billing", "value": "1 Main St\nSpringfield, CO"}],
            "terms": 30
        }))
        .expect("fields"),
    )
    .expect("creates");
    assert_eq!(
        text,
        other.read_to_string(&app.path).expect("reads"),
        "CSV and app write the same note"
    );
    let zoe = vault
        .read_to_string(&path("customers/Zoë Müller.md"))
        .expect("written");
    assert!(zoe.contains("type: person"), "{zoe}");
    // Indexed at once: a second run finds them taken.
    let again = row_errors(import(&vault, &mut index, &csv, ImportOptions::default()));
    assert_eq!(again.len(), 2, "{again:?}");
}

#[test]
fn one_bad_row_writes_nothing_and_every_problem_is_listed() {
    let (vault, mut index) = open(&[("customers/Acme.md", ACME)]);
    let csv = "name,terms,portal_url\n\
               Good One,30,\n\
               ,30,\n\
               Bad Terms,soon,\n\
               Acme,,\n\
               Bad Portal,,ftp://portal.example\n\
               Short Row\n\
               good one,,\n";
    let errors = row_errors(import(&vault, &mut index, csv, ImportOptions::default()));
    let lines: Vec<u64> = errors.iter().map(|e| e.line).collect();
    assert_eq!(lines, [3, 4, 5, 6, 7, 8], "{errors:?}");
    assert!(errors[0].error.contains("can't be empty"), "{errors:?}");
    assert!(errors[1].error.contains("whole net days"), "{errors:?}");
    assert!(errors[2].error.contains("already exists"), "{errors:?}");
    assert!(errors[3].error.contains("https://"), "{errors:?}");
    assert!(
        errors[4].error.contains("1 field, but the header has 3"),
        "{errors:?}"
    );
    assert_eq!(errors[5].error, "the same customer as line 2");
    assert_eq!(errors[5].name, "good one");
    assert!(
        vault.read(&path("customers/Good One.md")).is_err(),
        "all or nothing"
    );
    let message = ImportError::Rows(errors).to_string();
    assert!(
        message.starts_with("6 rows don't read, so nothing was written:"),
        "{message}"
    );
    assert!(message.contains("  line 4 (Bad Terms): terms"), "{message}");
}

#[test]
fn skip_existing_skips_customers_but_not_other_notes() {
    let files = [
        ("customers/Acme.md", ACME),
        ("company/Globex.md", NOT_A_CUSTOMER),
    ];
    let (vault, mut index) = open(&files);
    let skip = ImportOptions {
        skip_existing: true,
        dry_run: false,
    };
    let report = import(&vault, &mut index, "name\nACME\nInitech\n", skip).expect("imports");
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].path, path("customers/Acme.md"));
    assert_eq!(report.skipped[0].line, 2);
    assert_eq!(report.created.len(), 1);
    assert_eq!(report.created[0].saved.title, "Initech");
    assert_eq!(
        vault
            .read_to_string(&path("customers/Acme.md"))
            .expect("reads"),
        ACME,
        "untouched"
    );
    let errors = row_errors(import(&vault, &mut index, "name\nGlobex\n", skip));
    assert!(errors[0].error.contains("isn't a customer"), "{errors:?}");
}

#[test]
fn a_file_the_index_doesnt_know_is_refused_before_writing() {
    let (vault, mut index) = open(&[]);
    vault.create_dir_all(&path("customers")).expect("mkdir");
    vault
        .write_atomic(&path("customers/Acme.md"), b"mine")
        .expect("writes");
    let errors = row_errors(import(
        &vault,
        &mut index,
        "name\nFirst\nAcme\n",
        ImportOptions::default(),
    ));
    assert_eq!(errors.len(), 1);
    assert!(errors[0].error.contains("already exists"), "{errors:?}");
    assert!(vault.read(&path("customers/First.md")).is_err());
}

#[test]
fn a_dry_run_renders_and_writes_nothing() {
    let (vault, mut index) = open(&[]);
    let dry = ImportOptions {
        skip_existing: false,
        dry_run: true,
    };
    let report = import(
        &vault,
        &mut index,
        "name,email\nInitech,ap@initech.example\n",
        dry,
    )
    .expect("checks");
    assert!(report.dry_run);
    let row = &report.created[0];
    assert!(!row.saved.written);
    let text = row.text.as_deref().expect("text");
    assert!(text.contains("  - billing: ap@initech.example\n"), "{text}");
    assert!(vault.read(&row.saved.path).is_err(), "nothing written");
}

#[test]
fn the_file_itself_is_checked() {
    let (vault, mut index) = open(&[]);
    let mut run = |bytes: &[u8]| import_csv(&mut index, &vault, bytes, &ImportOptions::default());
    assert!(matches!(run(b""), Err(ImportError::Empty)));
    assert!(matches!(
        run(b"name\nok\n\xff\xfe\n"),
        Err(ImportError::NotText { line: 3 })
    ));
    let big = vec![b'a'; MAX_IMPORT_BYTES + 1];
    assert!(matches!(run(&big), Err(ImportError::TooLarge { .. })));
    let Err(ImportError::Header(message)) = run(b"email\nx@y.example\n") else {
        panic!("no name column")
    };
    assert!(message.contains("no name column"), "{message}");
    let Err(ImportError::Header(message)) = run(b"name,Name\na,b\n") else {
        panic!("twice")
    };
    assert!(message.contains("twice"), "{message}");
    let many = format!(
        "name\n{}",
        (0..=MAX_IMPORT_ROWS)
            .map(|i| format!("C{i}\n"))
            .collect::<String>()
    );
    assert!(matches!(
        run(many.as_bytes()),
        Err(ImportError::TooManyRows { .. })
    ));
    let report = run(b"name,email\n").expect("a header alone");
    assert!(report.created.is_empty());
}
