use super::*;
use crate::doc::Money;
use crate::settings::Settings;
use ledgeline_core::vault::MemVault;

/// A hand-written snippet: a comment, an unknown key, a body with a list.
const NET_30: &str = "---
title: Net 30 ACH
type: snippet
snippet_kind: payment   # shown under Payment
reviewed_by: Dana
---

## Payment

Pay by ACH within 30 days.
";

const CUSTOMER: &str = "---
title: Customer X
type: organization
tags: [customer]
snippets: [\"[[Net 30 ACH]]\"]
---
";

const INVOICE: &str = "---
title: Invoice 7
type: invoice
invoice_num: 7
status: draft
customer: \"[[Customer X]]\"
date: 2026-11-01
---

| Description | Qty | Rate | Amount |
|-------------|----:|-----:|-------:|
| Work        | 1   | $10  | $10.00 |
| **Total**   |     |      | $10.00 |

## Payment

Pay by ACH within 30 days.
";

fn path(p: &str) -> VaultPath {
    VaultPath::new(p).expect("path")
}

const SNIPPET: &str = "invoices/tpl/snippets/net-30.md";

fn open(files: &[(&str, &str)]) -> (MemVault, DocumentIndex) {
    let vault = files
        .iter()
        .fold(MemVault::new(), |v, (p, t)| v.with_file(p, t));
    let index = DocumentIndex::open(&vault, Settings::default(), Money::default());
    (vault, index)
}

fn books() -> (MemVault, DocumentIndex) {
    open(&[
        (SNIPPET, NET_30),
        ("customers/Customer X.md", CUSTOMER),
        ("invoices/7.md", INVOICE),
    ])
}

fn hash(vault: &MemVault, p: &str) -> String {
    content_hash(&vault.read(&path(p)).expect("reads"))
}

fn fields(title: Option<&str>, kind: Option<&str>, body: Option<&str>) -> SnippetFields {
    SnippetFields {
        title: title.map(str::to_string),
        kind: kind.map(str::to_string),
        body: body.map(str::to_string),
    }
}

#[test]
fn the_list_says_who_names_a_snippet_and_who_holds_a_copy() {
    let (vault, index) = books();
    let list = list(&index, &vault);
    assert_eq!(list.len(), 1);
    let net = &list[0];
    assert_eq!(net.title, "Net 30 ACH");
    assert_eq!(net.kind.as_deref(), Some("payment"));
    assert_eq!(net.body, "## Payment\n\nPay by ACH within 30 days.\n");
    assert_eq!(net.hash, hash(&vault, SNIPPET));
    let uses: Vec<(&str, UseKind, Option<&str>)> = net
        .used_by
        .iter()
        .map(|u| (u.path.as_str(), u.how, u.number.as_deref()))
        .collect();
    assert_eq!(
        uses,
        [
            ("customers/Customer X.md", UseKind::Default, None),
            ("invoices/7.md", UseKind::Copy, Some("7")),
        ]
    );
}

#[test]
fn update_edits_by_key_and_replaces_the_body() {
    let (vault, mut index) = books();
    let saved = update(
        &mut index,
        &vault,
        &path(SNIPPET),
        &hash(&vault, SNIPPET),
        &fields(None, Some("terms"), Some("Pay within **30** days.\r\n\r\n")),
    )
    .expect("saves");
    assert!(saved.written);
    assert_eq!(saved.changed, ["snippet_kind", "body"]);
    let text = vault.read_to_string(&path(SNIPPET)).expect("reads");
    assert_eq!(
        text,
        "---
title: Net 30 ACH
type: snippet
snippet_kind: terms   # shown under Payment
reviewed_by: Dana
---

Pay within **30** days.
"
    );
    assert_eq!(saved.hash, content_hash(text.as_bytes()));
    // The same again writes nothing.
    let again = update(
        &mut index,
        &vault,
        &path(SNIPPET),
        &saved.hash,
        &fields(
            Some("Net 30 ACH"),
            Some("terms"),
            Some("\nPay within **30** days."),
        ),
    )
    .expect("saves");
    assert!(!again.written);
    assert!(again.changed.is_empty());
    // An empty kind removes the key; an empty body leaves the frontmatter.
    let cleared = update(
        &mut index,
        &vault,
        &path(SNIPPET),
        &again.hash,
        &fields(None, Some(""), Some("  \n")),
    )
    .expect("saves");
    assert_eq!(
        vault.read_to_string(&path(SNIPPET)).expect("reads"),
        "---\ntitle: Net 30 ACH\ntype: snippet\nreviewed_by: Dana\n---\n"
    );
    assert_eq!(cleared.kind, None);
}

#[test]
fn a_rename_keeps_the_old_title_as_an_alias() {
    let (vault, mut index) = books();
    let saved = update(
        &mut index,
        &vault,
        &path(SNIPPET),
        &hash(&vault, SNIPPET),
        &fields(Some("ACH, net 30"), None, None),
    )
    .expect("renames");
    assert_eq!(saved.link, "[[ACH, net 30]]");
    assert_eq!(saved.changed, ["title", "aliases"]);
    let text = vault.read_to_string(&path(SNIPPET)).expect("reads");
    assert!(text.contains("title: ACH, net 30\n"), "{text}");
    assert!(text.contains("aliases:"), "{text}");
    assert!(
        text.ends_with("---\n\n## Payment\n\nPay by ACH within 30 days.\n"),
        "{text}"
    );
    // The customer's [[Net 30 ACH]] still finds it.
    let list = list(&index, &vault);
    assert_eq!(list[0].aliases, ["Net 30 ACH"]);
    assert_eq!(list[0].used_by[0].how, UseKind::Default);
    // Taking the old name back leaves the aliases.
    let back = update(
        &mut index,
        &vault,
        &path(SNIPPET),
        &saved.hash,
        &fields(Some("Net 30 ACH"), None, None),
    )
    .expect("renames");
    assert_eq!(back.changed, ["title", "aliases"]);
    let list = super::list(&index, &vault);
    assert_eq!(list[0].aliases, ["ACH, net 30"]);
}

#[test]
fn names_must_be_free_and_link_safe() {
    let (vault, mut index) = books();
    let h = hash(&vault, SNIPPET);
    for bad in ["", "A [link]", "x.md", "a|b"] {
        assert!(
            matches!(
                update(
                    &mut index,
                    &vault,
                    &path(SNIPPET),
                    &h,
                    &fields(Some(bad), None, None)
                ),
                Err(SnippetError::Invalid(_))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        update(
            &mut index,
            &vault,
            &path(SNIPPET),
            &h,
            &fields(Some("Customer X"), None, None)
        ),
        Err(SnippetError::NameTaken { .. })
    ));
    assert!(matches!(
        create(&mut index, &vault, &fields(Some("net 30 ach"), None, None)),
        Err(SnippetError::NameTaken { .. })
    ));
    assert!(matches!(
        update(
            &mut index,
            &vault,
            &path(SNIPPET),
            &h,
            &fields(None, Some("a\nb"), None)
        ),
        Err(SnippetError::Invalid(_))
    ));
    assert_eq!(hash(&vault, SNIPPET), h, "nothing written");
}

#[test]
fn an_outside_edit_is_never_overwritten() {
    let (vault, mut index) = books();
    let h = hash(&vault, SNIPPET);
    let outside = NET_30.replace("30 days", "45 days");
    vault
        .write_atomic(&path(SNIPPET), outside.as_bytes())
        .expect("writes");
    let Err(SnippetError::Changed { current, .. }) = update(
        &mut index,
        &vault,
        &path(SNIPPET),
        &h,
        &fields(None, None, Some("Mine")),
    ) else {
        panic!("refused");
    };
    assert_eq!(current, content_hash(outside.as_bytes()));
    assert_eq!(
        vault.read_to_string(&path(SNIPPET)).expect("reads"),
        outside
    );
    assert!(matches!(
        delete(&mut index, &vault, &path(SNIPPET), &h, true),
        Err(SnippetError::Changed { .. })
    ));
}

#[test]
fn create_writes_a_snippet_in_the_snippets_folder() {
    let (vault, mut index) = books();
    let made = create(
        &mut index,
        &vault,
        &fields(
            Some("Late fee: 1.5%"),
            Some("terms"),
            Some("A late fee of 1.5% a month applies."),
        ),
    )
    .expect("creates");
    assert_eq!(made.path, path("invoices/tpl/snippets/Late fee- 1.5%.md"));
    assert_eq!(made.link, "[[Late fee: 1.5%]]");
    let text = vault.read_to_string(&made.path).expect("reads");
    assert_eq!(
        text,
        "---\ntitle: \"Late fee: 1.5%\"\ntype: snippet\nsnippet_kind: terms\n---\n\nA late fee of 1.5% a month applies.\n"
    );
    assert_eq!(made.hash, content_hash(text.as_bytes()));
    let list = list(&index, &vault);
    assert_eq!(list.len(), 2);
    assert!(
        list.iter()
            .any(|s| s.title == "Late fee: 1.5%" && s.used_by.is_empty())
    );
    assert!(matches!(
        create(&mut index, &vault, &fields(None, None, None)),
        Err(SnippetError::Invalid(_))
    ));
}

#[test]
fn delete_is_refused_while_a_customer_names_it_unless_forced() {
    let (vault, mut index) = books();
    let h = hash(&vault, SNIPPET);
    let Err(SnippetError::InUse { customers }) =
        delete(&mut index, &vault, &path(SNIPPET), &h, false)
    else {
        panic!("refused");
    };
    assert_eq!(customers, ["Customer X"]);
    assert!(vault.read(&path(SNIPPET)).is_ok());
    delete(&mut index, &vault, &path(SNIPPET), &h, true).expect("deletes");
    assert!(vault.read(&path(SNIPPET)).is_err());
    assert!(list(&index, &vault).is_empty());
    // The invoice's copy is untouched.
    assert_eq!(
        vault.read_to_string(&path("invoices/7.md")).expect("reads"),
        INVOICE
    );
    // Not a snippet.
    let customer = hash(&vault, "customers/Customer X.md");
    assert!(matches!(
        delete(
            &mut index,
            &vault,
            &path("customers/Customer X.md"),
            &customer,
            true
        ),
        Err(SnippetError::NotASnippet(_))
    ));
}
