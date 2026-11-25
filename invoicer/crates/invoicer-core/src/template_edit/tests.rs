use super::*;
use crate::doc::Money;
use crate::settings::Settings;
use ledgeline_core::vault::MemVault;

/// A small base with the shipped base's comment style and three blocks.
const BASE_TEXT: &str = "{#-
  A base.

  - `settings`: before anything is written.
  - `theme`: the CSS for the sheet (inside a <style>). Colours are
    variables.
  - `footer`: a closing line, empty here.
-#}
{%- block settings %}{% endblock settings -%}
<html><head><style>{% block theme %}.sheet { color: black; }{% endblock theme %}</style></head>
<body><h1>{{ frontmatter.invoice_num }}</h1>{% block footer %}{% endblock footer %}</body></html>
";

const BASE: BaseTemplate<'static> = BaseTemplate {
    file: "_invoice_base.v1.html",
    source: BASE_TEXT,
};

const MINE: &str = "{# invoicer: name=\"Mine\" kinds=\"invoice\" #}
{% extends \"_invoice_base.v1.html\" %}
{% block theme %}{{ super() }} .sheet { color: red; }{% endblock theme %}
";

const WHOLE: &str = "<html><body>{{ frontmatter.invoice_num }}</body></html>\n";

const CUSTOMER: &str = "---
title: Customer X
type: organization
tags: [customer]
invoice_template: /invoices/tpl/mine.html
---
";

const INVOICE: &str = "---
title: Invoice 7
type: invoice
invoice_num: 7
status: draft
customer: \"[[Customer X]]\"
template: /invoices/tpl/mine.html
date: 2026-11-01
---

| Description | Qty | Rate | Amount |
|-------------|----:|-----:|-------:|
| Work        | 1   | $10  | $10.00 |
| **Total**   |     |      | $10.00 |
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

fn books() -> (MemVault, DocumentIndex) {
    open(&[
        ("invoices/tpl/_invoice_base.v1.html", BASE_TEXT),
        ("invoices/tpl/mine.html", MINE),
        ("invoices/tpl/whole.html", WHOLE),
        ("customers/Customer X.md", CUSTOMER),
        ("invoices/Invoice 7.md", INVOICE),
    ])
}

fn hash_of(vault: &MemVault, p: &str) -> String {
    content_hash(&vault.read(&path(p)).expect("reads"))
}

#[test]
fn the_base_lists_its_blocks_with_what_they_are_for() {
    let blocks = base_blocks(BASE_TEXT);
    let names: Vec<&str> = blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["settings", "theme", "footer"]);
    assert_eq!(blocks[0].about, "before anything is written.");
    assert_eq!(
        blocks[1].about,
        "the CSS for the sheet (inside a <style>). Colours are variables."
    );
    assert_eq!(base_version("_invoice_base.v1.html"), Some(1));
    assert_eq!(base_version("_invoice_base.v12.html"), Some(12));
    assert_eq!(base_version("_invoice_base.html"), None);
    assert_eq!(base_version("other.html"), None);
}

#[test]
fn the_list_says_what_each_extends_overrides_and_who_uses_it() {
    let (vault, index) = books();
    let all = list(&index, &vault);
    let names: Vec<&str> = all.iter().map(|t| t.shape.name.as_str()).collect();
    assert_eq!(names, ["Mine", "whole"], "by name; the base never listed");
    let mine = &all[0];
    assert_eq!(mine.template, "/invoices/tpl/mine.html");
    assert_eq!(mine.shape.extends.as_deref(), Some("_invoice_base.v1.html"));
    assert_eq!(mine.shape.base_version, Some(1));
    assert_eq!(mine.shape.overrides, ["theme"]);
    assert_eq!(mine.shape.kinds, ["invoice"]);
    assert_eq!(mine.hash, hash_of(&vault, "invoices/tpl/mine.html"));
    let used: Vec<(UseKind, &str)> = mine
        .used_by
        .iter()
        .map(|u| (u.how, u.title.as_str()))
        .collect();
    assert_eq!(
        used,
        [
            (UseKind::Invoice, "Invoice 7"),
            (UseKind::Customer, "Customer X")
        ]
    );
    assert_eq!(mine.used_by[0].number.as_deref(), Some("7"));
    let whole = &all[1];
    assert_eq!(whole.shape.extends, None);
    assert_eq!(whole.shape.base_version, None);
    assert!(whole.used_by.is_empty());
}

#[test]
fn the_header_is_split_from_the_body() {
    assert_eq!(
        split_header(MINE),
        (
            "{# invoicer: name=\"Mine\" kinds=\"invoice\" #}\n",
            &MINE[MINE.find('\n').unwrap() + 1..]
        )
    );
    assert_eq!(split_header(WHOLE), ("", WHOLE));
    assert_eq!(
        header_line("A B", &["invoice".into(), "estimate".into()]),
        "{# invoicer: name=\"A B\" kinds=\"invoice,estimate\" #}\n"
    );
    // What it writes, the header reads back.
    let head = templates::header(&header_line("Né ✓", &["credit-note".into()])).unwrap();
    assert_eq!(head.name.as_deref(), Some("Né ✓"));
    assert_eq!(head.kinds, ["credit-note"]);
}

#[test]
fn names_kinds_and_slugs() {
    assert_eq!(clean_name("  Blue  ").unwrap(), "Blue");
    for bad in ["", "  ", "a\"b", "a #} b", "two\nlines"] {
        assert!(clean_name(bad).is_err(), "{bad:?}");
    }
    assert!(clean_name(&"x".repeat(81)).is_err());
    assert_eq!(
        clean_kinds(&["Estimate".into(), "invoice".into(), "estimate".into()]).unwrap(),
        ["invoice", "estimate"]
    );
    assert!(clean_kinds(&[]).is_err());
    assert!(clean_kinds(&["receipt".into()]).is_err());
    assert_eq!(slug("Blue & Gold — 2026"), "blue-gold-2026");
    assert_eq!(slug("_private"), "private");
    assert_eq!(slug("!!!"), "template");
    assert_eq!(slug("Café Noir"), "café-noir");
    assert!(slug(&"word ".repeat(40)).len() <= MAX_SLUG);
    assert!(!slug(&"word ".repeat(40)).ends_with('-'));
}

#[test]
fn a_blank_child_extends_the_base_on_line_two_and_brings_the_base() {
    let (vault, mut index) = open(&[]);
    let made = create(
        &mut index,
        &vault,
        &NewTemplate {
            name: "Blue",
            ..NewTemplate::default()
        },
        &BASE,
    )
    .expect("creates");
    assert_eq!(made.path.as_str(), "invoices/tpl/blue.html");
    assert_eq!(made.template, "/invoices/tpl/blue.html");
    let text = vault.read_to_string(&made.path).unwrap();
    assert_eq!(
        text,
        "{# invoicer: name=\"Blue\" kinds=\"invoice\" #}\n{% extends \"_invoice_base.v1.html\" %}\n"
    );
    assert_eq!(
        text.lines().nth(1),
        Some("{% extends \"_invoice_base.v1.html\" %}")
    );
    assert_eq!(made.hash, content_hash(text.as_bytes()));
    // The base came with it, the shipped text byte for byte.
    assert_eq!(
        made.base.as_ref().map(VaultPath::as_str),
        Some("invoices/tpl/_invoice_base.v1.html")
    );
    assert_eq!(
        vault
            .read_to_string(&path("invoices/tpl/_invoice_base.v1.html"))
            .unwrap(),
        BASE_TEXT
    );
    // Indexed at once: listed, by its name.
    let all = list(&index, &vault);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].shape.name, "Blue");
    assert_eq!(all[0].shape.overrides, Vec::<String>::new());
}

#[test]
fn create_never_replaces_a_file_or_a_users_base() {
    let (vault, mut index) = open(&[
        ("invoices/tpl/blue.html", "<p>someone else's</p>"),
        ("invoices/tpl/_invoice_base.v1.html", "my base"),
    ]);
    let made = create(
        &mut index,
        &vault,
        &NewTemplate {
            name: "Blue",
            kinds: Some(vec!["estimate".into()]),
            from: None,
        },
        &BASE,
    )
    .expect("creates");
    assert_eq!(made.path.as_str(), "invoices/tpl/blue-2.html");
    assert_eq!(made.base, None);
    assert_eq!(
        vault
            .read_to_string(&path("invoices/tpl/blue.html"))
            .unwrap(),
        "<p>someone else's</p>"
    );
    assert_eq!(
        vault
            .read_to_string(&path("invoices/tpl/_invoice_base.v1.html"))
            .unwrap(),
        "my base"
    );
    assert!(
        vault
            .read_to_string(&made.path)
            .unwrap()
            .starts_with("{# invoicer: name=\"Blue\" kinds=\"estimate\" #}\n")
    );
}

#[test]
fn starting_from_a_template_keeps_its_body_and_kinds_under_a_new_name() {
    let (vault, mut index) = books();
    let made = create(
        &mut index,
        &vault,
        &NewTemplate {
            name: "Mine, copy",
            kinds: None,
            from: Some(
                "{# invoicer: name=\"Classic\" kinds=\"invoice,estimate\" #}\n{% extends \"_invoice_base.v1.html\" %}\n{% block footer %}Thanks{% endblock footer %}\n",
            ),
        },
        &BASE,
    )
    .expect("duplicates");
    assert_eq!(made.path.as_str(), "invoices/tpl/mine-copy.html");
    assert_eq!(
        vault.read_to_string(&made.path).unwrap(),
        "{# invoicer: name=\"Mine, copy\" kinds=\"invoice,estimate\" #}\n{% extends \"_invoice_base.v1.html\" %}\n{% block footer %}Thanks{% endblock footer %}\n"
    );
    assert_eq!(made.base, None, "the books have it");
    // A name another template has is refused, whatever its case.
    let taken = create(
        &mut index,
        &vault,
        &NewTemplate {
            name: "mine",
            ..NewTemplate::default()
        },
        &BASE,
    );
    assert!(
        matches!(taken, Err(TemplateError::NameTaken { ref path, .. }) if path.as_str() == "invoices/tpl/mine.html"),
        "{taken:?}"
    );
    // Something the invoicer wouldn't find is refused, before any write.
    let before = vault.list_dir(&path("invoices/tpl"), None).unwrap().len();
    let lost = create(
        &mut index,
        &vault,
        &NewTemplate {
            name: "Lost",
            kinds: None,
            from: Some("<p>no number, no base</p>"),
        },
        &BASE,
    );
    assert!(matches!(lost, Err(TemplateError::Invalid(_))), "{lost:?}");
    assert_eq!(
        vault.list_dir(&path("invoices/tpl"), None).unwrap().len(),
        before
    );
}

#[test]
fn a_save_writes_only_what_changed_over_the_hash_it_was_shown() {
    let (vault, mut index) = books();
    let mine = path("invoices/tpl/mine.html");
    let hash = hash_of(&vault, mine.as_str());

    // The same form: nothing written.
    let same = save(
        &mut index,
        &vault,
        &mine,
        &hash,
        &TemplateFields {
            name: Some("Mine".into()),
            kinds: Some(vec!["invoice".into()]),
            body: Some(body_of(MINE).to_string()),
        },
        Some(&BASE),
    )
    .expect("saves");
    assert!(!same.written);
    assert!(same.changed.is_empty());
    assert_eq!(vault.read_to_string(&mine).unwrap(), MINE);

    // A new body keeps the header line as it is.
    let body =
        "{% extends \"_invoice_base.v1.html\" %}\n{% block footer %}Bye{% endblock footer %}\n";
    let saved = save(
        &mut index,
        &vault,
        &mine,
        &hash,
        &TemplateFields {
            body: Some(body.into()),
            ..TemplateFields::default()
        },
        Some(&BASE),
    )
    .expect("saves");
    assert_eq!(saved.changed, ["body"]);
    assert_eq!(
        vault.read_to_string(&mine).unwrap(),
        format!("{{# invoicer: name=\"Mine\" kinds=\"invoice\" #}}\n{body}")
    );

    // The old hash is refused now: nothing written.
    let stale = save(
        &mut index,
        &vault,
        &mine,
        &hash,
        &TemplateFields {
            name: Some("Other".into()),
            ..TemplateFields::default()
        },
        Some(&BASE),
    );
    assert!(
        matches!(stale, Err(TemplateError::Changed { ref current, .. }) if *current == saved.hash),
        "{stale:?}"
    );

    // A rename rewrites the header only; the name is listed at once.
    let renamed = save(
        &mut index,
        &vault,
        &mine,
        &saved.hash,
        &TemplateFields {
            name: Some("Ocean".into()),
            kinds: Some(vec!["invoice".into(), "estimate".into()]),
            body: None,
        },
        Some(&BASE),
    )
    .expect("renames");
    assert_eq!(renamed.changed, ["name", "kinds"]);
    assert_eq!(
        vault.read_to_string(&mine).unwrap(),
        format!("{{# invoicer: name=\"Ocean\" kinds=\"invoice,estimate\" #}}\n{body}")
    );
    assert!(list(&index, &vault).iter().any(|t| t.shape.name == "Ocean"));

    // Taking another template's name is refused.
    let taken = save(
        &mut index,
        &vault,
        &mine,
        &renamed.hash,
        &TemplateFields {
            name: Some("WHOLE".into()),
            ..TemplateFields::default()
        },
        Some(&BASE),
    );
    assert!(
        matches!(taken, Err(TemplateError::NameTaken { .. })),
        "{taken:?}"
    );
    // A body the invoicer wouldn't find is refused.
    let lost = save(
        &mut index,
        &vault,
        &mine,
        &renamed.hash,
        &TemplateFields {
            body: Some("<p>plain</p>".into()),
            ..TemplateFields::default()
        },
        Some(&BASE),
    );
    assert!(matches!(lost, Err(TemplateError::Invalid(_))), "{lost:?}");
}

#[test]
fn a_template_without_a_header_gets_one_only_when_named() {
    let (vault, mut index) = books();
    let whole = path("invoices/tpl/whole.html");
    let hash = hash_of(&vault, whole.as_str());
    let body = "<html><body><b>{{ frontmatter.invoice_num }}</b></body></html>\n";
    let saved = save(
        &mut index,
        &vault,
        &whole,
        &hash,
        &TemplateFields {
            name: Some("whole".into()),
            kinds: Some(vec!["invoice".into()]),
            body: Some(body.into()),
        },
        None,
    )
    .expect("saves");
    assert_eq!(saved.changed, ["body"]);
    assert_eq!(vault.read_to_string(&whole).unwrap(), body);
    let named = save(
        &mut index,
        &vault,
        &whole,
        &saved.hash,
        &TemplateFields {
            name: Some("Whole page".into()),
            ..TemplateFields::default()
        },
        None,
    )
    .expect("names");
    assert_eq!(
        vault.read_to_string(&whole).unwrap(),
        format!("{{# invoicer: name=\"Whole page\" kinds=\"invoice\" #}}\n{body}")
    );
    assert_eq!(named.name, "Whole page");
}

#[test]
fn a_save_brings_back_the_base_it_extends() {
    let (vault, mut index) = open(&[("invoices/tpl/mine.html", MINE)]);
    let mine = path("invoices/tpl/mine.html");
    let saved = save(
        &mut index,
        &vault,
        &mine,
        &hash_of(&vault, mine.as_str()),
        &TemplateFields::default(),
        Some(&BASE),
    )
    .expect("saves");
    assert!(!saved.written);
    assert_eq!(
        saved.base.as_ref().map(VaultPath::as_str),
        Some("invoices/tpl/_invoice_base.v1.html")
    );
    assert_eq!(
        vault
            .read_to_string(&path("invoices/tpl/_invoice_base.v1.html"))
            .unwrap(),
        BASE_TEXT
    );
}

#[test]
fn delete_is_refused_while_invoices_or_customers_use_it() {
    let (vault, mut index) = books();
    let mine = path("invoices/tpl/mine.html");
    let refused = delete(&mut index, &vault, &mine, &hash_of(&vault, mine.as_str()));
    match refused {
        Err(TemplateError::InUse {
            ref invoices,
            ref customers,
        }) => {
            assert_eq!(invoices, &["7"]);
            assert_eq!(customers, &["Customer X"]);
            let message = refused.as_ref().unwrap_err().to_string();
            assert_eq!(
                message,
                "Invoice 7 uses it and Customer X starts new invoices with it; choose another template for them first"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(vault.stat(&mine).unwrap().is_some());

    // Unused: removed, and gone from the list; the base stays.
    let whole = path("invoices/tpl/whole.html");
    let stale = delete(&mut index, &vault, &whole, "not-the-hash");
    assert!(matches!(stale, Err(TemplateError::Changed { .. })));
    delete(&mut index, &vault, &whole, &hash_of(&vault, whole.as_str())).expect("deletes");
    assert!(vault.stat(&whole).unwrap().is_none());
    assert!(list(&index, &vault).iter().all(|t| t.path != whole));
    assert!(
        vault
            .stat(&path("invoices/tpl/_invoice_base.v1.html"))
            .unwrap()
            .is_some()
    );
    // A base isn't a template: never deleted or saved here.
    let base = path("invoices/tpl/_invoice_base.v1.html");
    assert!(matches!(
        delete(&mut index, &vault, &base, &hash_of(&vault, base.as_str())),
        Err(TemplateError::NotATemplate(_))
    ));
    assert!(matches!(
        read(&index, &vault, &path("invoices/tpl/nope.html")),
        Err(TemplateError::NotFound(_))
    ));
}

#[test]
fn read_gives_the_body_without_the_header() {
    let (vault, index) = books();
    let file = read(&index, &vault, &path("invoices/tpl/mine.html")).expect("reads");
    assert_eq!(file.body, body_of(MINE));
    assert!(file.body.starts_with("{% extends"));
    assert_eq!(file.entry.shape.name, "Mine");
}
