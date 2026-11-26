use super::*;
use ledgeline_core::vault::MemVault;

const INVOICE_105: &str = include_str!("../../tests/fixtures/invoice-3-2.md");

fn vault() -> MemVault {
    MemVault::new()
        .with_file("invoices/2026-11-17 Customer X Invoice 105.md", INVOICE_105)
        .with_file(
            "invoices/2026-11-01 Customer X Invoice 104.md",
            INVOICE_105
                .replace("invoice_num: 105", "invoice_num: 104")
                .replace("date: 2026-11-17", "date: 2026-11-01")
                .replace("due_date: 2026-12-17", "due_date: 2026-12-01"),
        )
        .with_file(
            "customers/Customer X.md",
            "---\ntitle: Customer X\ntype: organization\ntags: [customer]\nbilling_portal:\n  name: Coupa\n  requires_reference: true\n---\n",
        )
        .with_file("orgs/My Company.md", "---\ntype: organization\n---\n")
        .with_file("invoices/tpl/myco-default.html", "{{ frontmatter.invoice_num }}")
        .with_file("notes/readme.md", "# not a document\n")
}

fn index(vault: &MemVault) -> DocumentIndex {
    DocumentIndex::open(vault, Settings::default(), Money::default())
}

fn vp(s: &str) -> VaultPath {
    VaultPath::new(s).expect("path")
}

#[test]
fn open_indexes_documents_only() {
    let vault = vault();
    let index = index(&vault);
    assert_eq!(index.docs().count(), 2);
    assert_eq!(index.site().len(), 5);
    let doc = index.by_number("105");
    assert_eq!(doc.len(), 1);
    let summary = doc[0].summary.as_ref().expect("summary");
    assert_eq!(summary.total_text, "$8,350.00");
    assert_eq!(index.next_number([]).expect("next").as_str(), "106");
    assert_eq!(index.next_number(["2004"]).expect("next").as_str(), "2005");
}

#[test]
fn list_filters_and_sorts_newest_first() {
    let vault = vault();
    let index = index(&vault);
    let all: Vec<&str> = index
        .list(&ListFilter::default())
        .iter()
        .filter_map(|d| d.number())
        .collect();
    assert_eq!(all, ["105", "104"]);
    let by_customer = index.list(&ListFilter {
        customer: Some("customer x".into()),
        ..ListFilter::default()
    });
    assert_eq!(by_customer.len(), 2);
    let drafts = index.list(&ListFilter {
        status: Some(Status::Draft),
        ..ListFilter::default()
    });
    assert!(drafts.is_empty());
}

#[test]
fn check_is_clean_on_good_files_and_names_relational_problems() {
    let vault = vault();
    let problems = index(&vault).check(&vault);
    assert!(problems.is_empty(), "{problems:#?}");

    vault.insert_file(
        "invoices/dup.md",
        INVOICE_105
            .replace("reference: \"PO #PY75334\"\n", "")
            .replace("[[Customer X]]", "[[Nobody]]")
            .replace("myco-default", "missing"),
    );
    vault.insert_file(
        "invoices/noref.md",
        INVOICE_105
            .replace("invoice_num: 105", "invoice_num: 107")
            .replace("reference: \"PO #PY75334\"\n", ""),
    );
    let problems = index(&vault).check(&vault);
    let codes: Vec<(String, ProblemCode)> = problems
        .iter()
        .map(|p| {
            (
                p.file.as_ref().map(ToString::to_string).unwrap_or_default(),
                p.code,
            )
        })
        .collect();
    assert!(
        codes.contains(&("invoices/dup.md".into(), ProblemCode::UnknownCustomer)),
        "{codes:#?}"
    );
    assert!(
        codes.contains(&("invoices/dup.md".into(), ProblemCode::UnknownTemplate)),
        "{codes:#?}"
    );
    assert!(
        codes.contains(&("invoices/dup.md".into(), ProblemCode::DuplicateNumber)),
        "{codes:#?}"
    );
    assert!(codes.contains(&(
        "invoices/2026-11-17 Customer X Invoice 105.md".into(),
        ProblemCode::DuplicateNumber
    )));
    assert!(
        codes.contains(&("invoices/noref.md".into(), ProblemCode::ReferenceRequired)),
        "{codes:#?}"
    );
    let dup = problems
        .iter()
        .find(|p| p.code == ProblemCode::UnknownCustomer)
        .expect("unknown");
    assert_eq!(dup.line, Some(10));
}

#[test]
fn refresh_reparses_only_changed_files() {
    let vault = vault();
    let mut index = index(&vault);
    let path = vp("invoices/2026-11-17 Customer X Invoice 105.md");
    // Same bytes: unchanged.
    vault.insert_file(path.as_str(), INVOICE_105);
    let done = index.refresh(&vault, std::slice::from_ref(&path));
    assert_eq!(done.unchanged, std::slice::from_ref(&path));
    // A change: re-parsed.
    vault.insert_file(
        path.as_str(),
        INVOICE_105.replace("status: sent", "status: void"),
    );
    let done = index.refresh(&vault, std::slice::from_ref(&path));
    assert_eq!(done.updated, std::slice::from_ref(&path));
    assert_eq!(
        index.doc(&path).and_then(|d| d.header.status),
        Some(Status::Void)
    );
    // A new file and a deletion.
    let new = vp("invoices/new.md");
    vault.insert_file(
        new.as_str(),
        INVOICE_105.replace("invoice_num: 105", "invoice_num: 200"),
    );
    let old = vp("invoices/2026-11-01 Customer X Invoice 104.md");
    vault.remove_file(&old).expect("removes");
    let done = index.refresh(&vault, &[new.clone(), old.clone()]);
    assert_eq!(done.updated, std::slice::from_ref(&new));
    assert_eq!(done.removed, std::slice::from_ref(&old));
    assert_eq!(index.next_number([]).expect("next").as_str(), "201");
    assert!(index.doc(&old).is_none());
    // A document that stops being one leaves the index but stays a note.
    vault.insert_file(new.as_str(), "# just a note\n");
    index.refresh(&vault, std::slice::from_ref(&new));
    assert!(index.doc(&new).is_none());
    assert!(index.site().note(&new).is_some());
    assert!(index.rebuild_and_diff(&vault).is_empty());
}

#[test]
fn rebuild_and_diff_finds_what_refresh_missed() {
    let vault = vault();
    let index = index(&vault);
    assert!(index.rebuild_and_diff(&vault).is_empty());
    vault.insert_file(
        "invoices/sneaky.md",
        INVOICE_105.replace("invoice_num: 105", "invoice_num: 300"),
    );
    vault.insert_file(
        "invoices/2026-11-17 Customer X Invoice 105.md",
        INVOICE_105.replace("status: sent", "status: draft"),
    );
    vault.remove_file(&vp("notes/readme.md")).expect("removes");
    let drift = index.rebuild_and_diff(&vault);
    assert_eq!(
        drift,
        [
            IndexDrift::Changed {
                path: vp("invoices/2026-11-17 Customer X Invoice 105.md")
            },
            IndexDrift::Missing {
                path: vp("invoices/sneaky.md")
            },
            IndexDrift::Stale {
                path: vp("notes/readme.md")
            },
        ]
    );
}

#[test]
fn ignored_paths_are_never_indexed() {
    let vault = vault();
    vault.insert_file("invoices/pdf/x.md", INVOICE_105);
    vault.insert_file(".git/x.md", INVOICE_105);
    let mut index = index(&vault);
    assert_eq!(index.docs().count(), 2);
    let done = index.refresh(&vault, &[vp("invoices/pdf/x.md")]);
    assert_eq!(done, Refreshed::default());
}

/// PLAN Q1: the base beside the templates is never a template, so it is
/// never reported as an unknown or broken one; an invoice naming it is told
/// to name a template that extends it.
#[test]
fn a_base_is_not_a_template_and_naming_one_says_so() {
    let vault = vault().with_file(
        "invoices/tpl/_invoice_base.v1.html",
        "{{ frontmatter.invoice_num }}{% block theme %}{% endblock theme %}",
    );
    assert!(index(&vault).check(&vault).is_empty());
    vault.insert_file(
        "invoices/2026-11-17 Customer X Invoice 105.md",
        INVOICE_105.replace("myco-default.html", "_invoice_base.v1.html"),
    );
    let problems = index(&vault).check(&vault);
    let unknown: Vec<&Problem> = problems
        .iter()
        .filter(|p| p.code == ProblemCode::UnknownTemplate)
        .collect();
    assert_eq!(unknown.len(), 1, "{problems:#?}");
    assert!(unknown[0].message.contains("is a base"), "{unknown:#?}");
}

/// Templates and catalogs are read at open and again only when a refresh
/// names them (review RA-3): `check` and the sidebar read no file.
#[test]
fn templates_and_catalogs_follow_refresh() {
    let vault = vault();
    let catalog = vp("invoices/catalog.md");
    vault.insert_file(
        catalog.as_str(),
        "---\ntype: catalog\n---\n\n| Code | Description | Rate |\n|---|---|---|\n| ARCH | Architect | $800 |\n",
    );
    let mut index = index(&vault);
    let names = |index: &DocumentIndex| -> Vec<String> {
        index.templates().iter().map(|t| t.name.clone()).collect()
    };
    let codes = |index: &DocumentIndex| -> Vec<String> {
        index
            .catalogs()
            .flat_map(|c| c.items.iter().map(|i| i.code.clone()))
            .collect()
    };
    assert_eq!(names(&index), ["myco-default"]);
    assert_eq!(codes(&index), ["ARCH"]);
    assert_eq!(index.catalog_problems().count(), 0);

    // Written on disk but not refreshed yet: the index is as it was.
    let tpl = vp("invoices/tpl/myco-default.html");
    vault.insert_file(
        tpl.as_str(),
        "{# invoicer: name=\"Renamed\" #}\n{{ frontmatter.invoice_num }}",
    );
    let added = vp("invoices/tpl/other.html");
    vault.insert_file(added.as_str(), "<p>{{ frontmatter.invoice_num }}</p>");
    let partial = vp("invoices/tpl/_part.html");
    vault.insert_file(partial.as_str(), "{{ frontmatter.invoice_num }}");
    assert_eq!(names(&index), ["myco-default"]);
    assert!(
        !index.rebuild_and_diff(&vault).is_empty(),
        "check --full sees it"
    );

    // Refreshed: an edit, a new file, and a partial (never a template).
    index.refresh(&vault, &[tpl.clone(), added.clone(), partial.clone()]);
    assert_eq!(names(&index), ["Renamed", "other"]);
    assert!(index.rebuild_and_diff(&vault).is_empty());

    // An edit that makes it no template, and a removal.
    vault.insert_file(tpl.as_str(), "<p>a page</p>");
    vault.remove_file(&added).expect("removes");
    index.refresh(&vault, &[tpl.clone(), added.clone()]);
    assert!(index.templates().is_empty());
    let problems = index.check(&vault);
    assert!(
        problems
            .iter()
            .any(|p| p.code == ProblemCode::UnknownTemplate),
        "{problems:#?}"
    );

    // A catalog edit, a bad row, and a note that stops being a catalog.
    vault.insert_file(
        catalog.as_str(),
        "---\ntype: catalog\n---\n\n| Code | Description | Rate |\n|---|---|---|\n| ARCH | Architect | $800 |\n| JR | Junior | ten |\n",
    );
    index.refresh(&vault, std::slice::from_ref(&catalog));
    assert_eq!(codes(&index), ["ARCH", "JR"]);
    assert_eq!(index.catalog_problems().count(), 1);
    vault.insert_file(catalog.as_str(), "# notes now\n");
    index.refresh(&vault, std::slice::from_ref(&catalog));
    assert!(codes(&index).is_empty());
    assert_eq!(index.catalog_problems().count(), 0);
    assert!(index.rebuild_and_diff(&vault).is_empty());
}

/// The number and customer lookups are rebuilt after any refresh that
/// changed something (review RC-5, RC-15).
#[test]
fn number_and_customer_lookups_follow_refresh() {
    let vault = vault();
    let mut index = index(&vault);
    let customer = vp("customers/Customer X.md");
    let numbers = |index: &DocumentIndex| -> Vec<String> {
        index
            .billed_to(&customer)
            .iter()
            .filter_map(|d| d.number().map(str::to_string))
            .collect()
    };
    assert_eq!(numbers(&index), ["105", "104"], "newest first");
    assert_eq!(index.by_number(" 104 ").len(), 1);
    assert!(index.by_number("300").is_empty());

    // A new document, a renumbered one, and one billed elsewhere.
    let new = vp("invoices/new.md");
    vault.insert_file(
        new.as_str(),
        INVOICE_105
            .replace("invoice_num: 105", "invoice_num: 300")
            .replace("date: 2026-11-17", "date: 2026-12-01"),
    );
    let old = vp("invoices/2026-11-01 Customer X Invoice 104.md");
    vault.insert_file(
        old.as_str(),
        INVOICE_105.replace("[[Customer X]]", "[[My Company]]"),
    );
    index.refresh(&vault, &[new.clone(), old.clone()]);
    assert_eq!(numbers(&index), ["300", "105"]);
    assert_eq!(index.by_number("300").len(), 1);
    assert!(index.by_number("104").is_empty());
    assert_eq!(index.by_number("105").len(), 2, "two documents share it");

    // A removal.
    vault.remove_file(&new).expect("removes");
    index.refresh(&vault, std::slice::from_ref(&new));
    assert_eq!(numbers(&index), ["105"]);
    assert!(index.by_number("300").is_empty());
}
