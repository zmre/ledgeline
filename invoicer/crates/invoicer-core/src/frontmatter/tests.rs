use super::*;
use proptest::prelude::*;
use serde_json::json;

const INVOICE: &str = "---
title: \"2026-11-17 Customer X Invoice 105\"
type: invoice   # the document type
template: /invoices/tpl/myco-default.html
status: draft
invoice_num: 0105
date: 2026-11-17
terms: 30
reference: 'PO #PY75334'
customer: \"[[Customer X]]\"
# the snapshot
invoicee_address: |
  1234 Main St. #555
  Springfield, CO 80000
tags:
- customer
- vip
paid: no
---

Hi Fred.

| Description | Amount |
|---|---|
| Work | $1 |
";

fn fields(source: &str) -> Fields {
    Frontmatter::parse(source).expect("parses").fields
}

#[test]
fn locate_follows_mbrs_rules() {
    let block = locate("---\na: 1\n---\nbody\n").expect("block");
    assert_eq!(block.content, 4..9);
    assert_eq!(block.body_start, 13);
    assert!(locate("---\n\na: 1\n---\n").is_none(), "blank first line");
    assert!(locate("---\na: 1\n").is_none(), "unclosed");
    assert!(locate("----\na: 1\n---\n").is_none());
    assert!(
        locate("body\n---\na: 1\n---\n").is_none(),
        "not at the start"
    );
    assert!(locate("---\na: 1\n...\n").is_some(), "dots close it");
    let bom = locate("\u{feff}---\na: 1\n---\n").expect("bom");
    assert_eq!(bom.block.start, 3);
    let crlf = locate("---\r\na: 1\r\n---\r\nbody").expect("crlf");
    assert_eq!(&"---\r\na: 1\r\n---\r\nbody"[crlf.body_start..], "body");
}

#[test]
fn reads_losslessly_as_mbr_does() {
    let fm = Frontmatter::parse(INVOICE).expect("parses");
    assert_eq!(fm.str("date"), Some("2026-11-17"));
    assert_eq!(fm.str("paid"), Some("no"));
    assert_eq!(fm.get("terms"), Some(&json!(30)));
    assert_eq!(fm.get("invoice_num"), Some(&json!(105)));
    assert_eq!(fm.get("tags"), Some(&json!(["customer", "vip"])));
    assert_eq!(
        fm.str("invoicee_address"),
        Some("1234 Main St. #555\nSpringfield, CO 80000\n")
    );
    assert_eq!(fm.order[0], "title");
    assert_eq!(fm.order.last().map(String::as_str), Some("paid"));
    let nested =
        fields("---\nportal:\n  name: Coupa\n  requires_reference: true\nrate: 1.5\nn: ~\n---\n");
    assert_eq!(nested["portal"]["requires_reference"], json!(true));
    assert_eq!(nested["rate"], json!(1.5));
    assert_eq!(nested["n"], Value::Null);
}

#[test]
fn errors_name_the_document_line() {
    let err = Frontmatter::parse("---\na: 1\nb: [\n---\n").expect_err("bad");
    assert!(matches!(err, FrontmatterError::Yaml { .. }), "{err}");
    assert!(err.line() >= 3, "{err:?}");
    let err = Frontmatter::parse("---\na: 1\na: 2\n---\n").expect_err("duplicate");
    assert_eq!(err.line(), 3);
    let err = Frontmatter::parse("---\n- a\n- b\n---\n").expect_err("list");
    assert_eq!(err, FrontmatterError::NotAMapping { line: 2 });
    assert_eq!(
        Frontmatter::parse("no frontmatter\n").expect("ok").fields,
        Fields::new()
    );
    assert!(
        Frontmatter::parse("---\n# only a comment\n---\n")
            .expect("ok")
            .fields
            .is_empty()
    );
}

#[test]
fn key_lines_keep_what_the_parser_normalizes() {
    let keys = key_lines(INVOICE).expect("keys");
    let num = keys.iter().find(|k| k.key == "invoice_num").expect("num");
    assert_eq!(num.raw_value, "0105");
    assert_eq!(num.line, 6);
    let typ = keys.iter().find(|k| k.key == "type").expect("type");
    assert_eq!(typ.raw_value, "invoice");
    let addr = keys
        .iter()
        .find(|k| k.key == "invoicee_address")
        .expect("addr");
    assert_eq!(addr.raw_value, "|");
    assert!(addr.multiline);
    assert_eq!(addr.line, 12);
}

/// Everything but `range` of `a` is byte-identical in `b`.
fn only_changed(a: &str, b: &str, key_line_prefix: &str) {
    let start = a.find(key_line_prefix).expect("key present");
    assert_eq!(&a[..start], &b[..start], "text before the key moved");
}

#[test]
fn set_replaces_one_line_keeping_comment_and_quotes() {
    let out = set(INVOICE, "type", json!("estimate")).expect("edits");
    assert!(
        out.contains("type: estimate   # the document type\n"),
        "{out}"
    );
    let out = set(INVOICE, "reference", json!("PO #1")).expect("edits");
    assert!(out.contains("reference: 'PO #1'\n"), "{out}");
    let out = set(INVOICE, "title", json!("Plain title")).expect("edits");
    assert!(out.contains("title: \"Plain title\"\n"), "{out}");
    let out = set(INVOICE, "status", json!("sent")).expect("edits");
    assert_eq!(out, INVOICE.replace("status: draft", "status: sent"));
}

#[test]
fn set_the_paid_mirror_touches_only_its_line() {
    let out = set(
        INVOICE,
        "paid",
        json!("partial 2026-10-28 $1,500, 2026-11-19 $3,500"),
    )
    .expect("edits");
    assert_eq!(
        out,
        INVOICE.replace(
            "paid: no\n",
            "paid: partial 2026-10-28 $1,500, 2026-11-19 $3,500\n"
        )
    );
    only_changed(INVOICE, &out, "paid:");
}

#[test]
fn set_a_multiline_value_writes_a_literal_block() {
    let out = set(INVOICE, "invoicee_address", json!("9 Elm\nTown, ST 1\n")).expect("edits");
    assert!(
        out.contains("invoicee_address: |\n  9 Elm\n  Town, ST 1\ntags:"),
        "{out}"
    );
    let out = set(INVOICE, "invoicee_address", json!("no newline\nat end")).expect("edits");
    assert!(
        out.contains("invoicee_address: |-\n  no newline\n  at end\n"),
        "{out}"
    );
    let out = set(INVOICE, "note", json!("kept\n\n")).expect("edits");
    assert!(out.contains("note: \"kept\\n\\n\"\n"), "never `|+`: {out}");
    let out = set(INVOICE, "note", json!("  leading space\nline")).expect("edits");
    assert!(out.contains("note: \"  leading space\\nline\"\n"), "{out}");
}

#[test]
fn set_replaces_a_block_value_with_a_line_and_back() {
    let out = set(INVOICE, "invoicee_address", json!("One line")).expect("edits");
    assert!(out.contains("invoicee_address: One line\ntags:"), "{out}");
    let out = set(INVOICE, "tags", json!(["customer"])).expect("edits");
    assert!(out.contains("tags:\n  - customer\npaid: no"), "{out}");
    let out = set(INVOICE, "tags", json!([])).expect("edits");
    assert!(out.contains("tags: []\npaid"), "{out}");
}

#[test]
fn set_inserts_an_absent_key_after_the_last_key() {
    let out = set(INVOICE, "sent_date", json!("2026-11-18")).expect("edits");
    assert!(
        out.contains("paid: no\nsent_date: 2026-11-18\n---\n"),
        "{out}"
    );
    let doc = "---\ntitle: x\n# trailing comment\n---\nbody\n";
    let out = set(doc, "a", json!(1)).expect("edits");
    assert_eq!(out, "---\ntitle: x\na: 1\n# trailing comment\n---\nbody\n");
}

#[test]
fn set_writes_nested_maps_and_lists_of_maps() {
    let value = json!([{"reference": "PO #1", "amount": "$40,000"}, {"reference": "PO #2"}]);
    let out = set("---\na: 1\n---\n", "open_pos", value.clone()).expect("edits");
    assert_eq!(fields(&out)["open_pos"], value);
    // Nested maps are written in the JSON object's order: sorted by key,
    // or as inserted when serde_json's `preserve_order` is on (MBR's Tera
    // turns it on in a build that includes the app).
    assert!(
        out.contains("open_pos:\n  - amount: $40,000\n    reference: \"PO #1\"\n")
            || out.contains("open_pos:\n  - reference: \"PO #1\"\n    amount: $40,000\n"),
        "{out}"
    );
    let portal = json!({"name": "Coupa", "requires_reference": true, "instructions": "Upload\nthen attach\n"});
    let out = set("---\na: 1\n---\n", "billing_portal", portal.clone()).expect("edits");
    assert_eq!(fields(&out)["billing_portal"], portal);
}

#[test]
fn strings_that_look_like_other_types_are_quoted() {
    for text in [
        "true",
        "no",
        "null",
        "~",
        "105",
        "1.5",
        "0x1F",
        ".inf",
        "",
        " x",
        "a: b",
        "- x",
        "#x",
        "[[Customer X]]",
        "x:",
    ] {
        let out = set("---\na: 1\n---\n", "v", json!(text)).expect("edits");
        assert_eq!(fields(&out)["v"], json!(text), "{text}: {out}");
    }
    let out = set("---\na: 1\n---\n", "v", json!("2026-11-17")).expect("edits");
    assert!(out.contains("v: 2026-11-17\n"), "dates stay plain: {out}");
    let out = set("---\na: 1\n---\n", "v", json!("tab\there \"q\" \\ \u{1}")).expect("edits");
    assert_eq!(fields(&out)["v"], json!("tab\there \"q\" \\ \u{1}"));
}

#[test]
fn numbers_bools_and_null() {
    let out = set("---\na: 1\n---\n", "terms", json!(45)).expect("edits");
    assert!(out.contains("terms: 45\n"));
    let out = set(&out, "flag", json!(false)).expect("edits");
    assert!(out.contains("flag: false\n"));
    let out = set(&out, "nothing", Value::Null).expect("edits");
    assert_eq!(fields(&out)["nothing"], Value::Null);
}

#[test]
fn remove_deletes_the_entry_lines_only() {
    let out = remove(INVOICE, "invoicee_address").expect("edits");
    assert!(out.contains("# the snapshot\ntags:\n"), "{out}");
    assert!(!fields(&out).contains_key("invoicee_address"));
    assert_eq!(remove(INVOICE, "absent").expect("no-op"), INVOICE);
    assert_eq!(
        remove("---\na: 1\n---\n\nbody\n", "a").expect("edits"),
        "\nbody\n"
    );
    assert_eq!(
        remove("---\na: 1\n\nb: 2\n---\nbody\n", "a").expect("edits"),
        "---\nb: 2\n---\nbody\n"
    );
}

#[test]
fn several_edits_apply_in_order() {
    let out = edit(
        INVOICE,
        &[
            FieldEdit::Set("status".into(), json!("sent")),
            FieldEdit::Set("sent_date".into(), json!("2026-11-18")),
            FieldEdit::Remove("tags".into()),
        ],
    )
    .expect("edits");
    let f = fields(&out);
    assert_eq!(f["status"], json!("sent"));
    assert_eq!(f["sent_date"], json!("2026-11-18"));
    assert!(!f.contains_key("tags"));
    assert!(out.ends_with(&INVOICE[INVOICE.find("\nHi Fred").expect("body")..]));
}

#[test]
fn crlf_documents_stay_crlf() {
    let doc = "---\r\ntitle: x\r\naddr: |\r\n  a\r\n  b\r\n---\r\nbody\r\n";
    let out = set(doc, "addr", json!("c\nd\n")).expect("edits");
    assert_eq!(
        out,
        "---\r\ntitle: x\r\naddr: |\r\n  c\r\n  d\r\n---\r\nbody\r\n"
    );
    let out = set(doc, "new", json!("v")).expect("edits");
    assert!(
        out.contains("addr: |\r\n  a\r\n  b\r\nnew: v\r\n---"),
        "{out:?}"
    );
}

#[test]
fn a_document_without_frontmatter_gets_a_block() {
    assert_eq!(
        set("Hello\n", "type", json!("invoice")).expect("edits"),
        "---\ntype: invoice\n---\n\nHello\n"
    );
    assert_eq!(set("", "a", json!(1)).expect("edits"), "---\na: 1\n---\n");
    assert_eq!(remove("Hello\n", "a").expect("no-op"), "Hello\n");
    assert!(matches!(
        set("---\n---\nbody", "a", json!(1)),
        Err(EditError::Layout { .. })
    ));
}

#[test]
fn keep_chomped_blocks_keep_their_blank_lines() {
    let doc = "---\nt: |+\n  a\n\n\nu: 1\n---\n";
    assert_eq!(fields(doc)["t"], json!("a\n\n\n"));
    let out = set(doc, "u", json!(2)).expect("edits");
    assert_eq!(fields(&out)["t"], json!("a\n\n\n"));
    let out = set(doc, "t", json!("b\n")).expect("edits");
    assert_eq!(out, "---\nt: |\n  b\nu: 1\n---\n");
}

#[test]
fn quoted_keys_and_odd_spellings() {
    let doc = "---\n\"odd key\": 1\n'it''s': 2\nplain: 3\n---\n";
    let out = set(doc, "odd key", json!(9)).expect("edits");
    assert!(out.contains("\"odd key\": 9\n"), "{out}");
    let out = set(doc, "it's", json!("x")).expect("edits");
    assert!(out.contains("'it''s': x\n"), "{out}");
    let out = set(doc, "new: key", json!(1)).expect("edits");
    assert_eq!(fields(&out)["new: key"], json!(1));
}

#[test]
fn refuses_layouts_it_does_not_understand() {
    for doc in [
        "---\n{a: 1, b: 2}\n---\n",
        "---\n? complex\n: v\n---\n",
        "---\ntags:\n# a comment inside a list\n- a\n---\n",
    ] {
        if Frontmatter::parse(doc).is_ok() {
            assert!(
                matches!(set(doc, "a", json!(5)), Err(EditError::Layout { .. })),
                "{doc}"
            );
        }
    }
    assert!(matches!(
        set("---\na: [\n---\n", "a", json!(1)),
        Err(EditError::Read(_))
    ));
    for key in ["", " a", "a\nb"] {
        assert!(
            matches!(set(INVOICE, key, json!(1)), Err(EditError::BadKey(_))),
            "{key:?}"
        );
    }
}

#[test]
fn flow_lists_and_anchors_still_edit_around() {
    let doc = "---\ntags: [a, b]\nbase: &b 1\ncopy: *b\nother: x\n---\n";
    let out = set(doc, "other", json!("y")).expect("edits");
    assert_eq!(out, doc.replace("other: x", "other: y"));
    let multi = "---\nlist: [a,\n  b]\nz: 1\n---\n";
    let out = set(multi, "z", json!(2)).expect("edits");
    assert_eq!(out, multi.replace("z: 1", "z: 2"));
}

#[test]
fn render_block_writes_a_verified_block() {
    let block = render_block(&[
        ("title", json!("2026-11-17 Customer X Invoice 105")),
        ("invoice_num", json!("105")),
        ("customer", json!("[[Customer X]]")),
        ("invoicee_address", json!("1 Main\nTown\n")),
    ])
    .expect("renders");
    assert_eq!(
        block,
        "---\ntitle: 2026-11-17 Customer X Invoice 105\ninvoice_num: \"105\"\ncustomer: \"[[Customer X]]\"\ninvoicee_address: |\n  1 Main\n  Town\n---\n"
    );
    assert!(matches!(
        render_block(&[("", json!(1))]),
        Err(EditError::BadKey(_))
    ));
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

fn key() -> impl Strategy<Value = String> {
    "[a-z][a-z_]{0,8}"
}

fn scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<i32>().prop_map(|n| json!(n)),
        any::<bool>().prop_map(|b| json!(b)),
        Just(Value::Null),
        "[ -~]{0,20}".prop_map(Value::String),
        "[a-z #:\\-\\[\\]'\"]{1,12}(\n[a-z ]{0,8}){0,3}\n?".prop_map(Value::String),
        "\\PC{0,12}".prop_map(Value::String),
    ]
}

fn value() -> impl Strategy<Value = Value> {
    scalar().prop_recursive(2, 12, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::btree_map(key(), inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

/// How a scalar line is written in the generated source, for variety.
fn styled(value: &Value, style: u8) -> Option<String> {
    let s = value.as_str()?;
    if s.contains('\n') || s.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(match style % 3 {
        0 => double_quoted(s),
        1 => single_quoted(s),
        _ => return None,
    })
}

fn document() -> impl Strategy<Value = (String, Vec<String>)> {
    prop::collection::btree_map(key(), (value(), any::<u8>(), any::<bool>()), 1..7).prop_map(|m| {
        let mut out = String::from("---\n");
        let keys: Vec<String> = m.keys().cloned().collect();
        for (k, (v, style, comment)) in &m {
            let line = match styled(v, *style) {
                Some(text) => format!("{k}: {text}\n"),
                None => render_entry(k, v, Quote::Auto, None),
            };
            if *comment && line.matches('\n').count() == 1 {
                out.push_str(&format!("{}  # note\n", line.trim_end_matches('\n')));
            } else {
                out.push_str(&line);
            }
            if *style % 5 == 0 {
                out.push_str("# a comment between keys\n");
            }
            if *style % 7 == 0 {
                out.push('\n');
            }
        }
        out.push_str("---\n\nBody | text\n");
        (out, keys)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn set_reads_back_and_leaves_every_other_key_alone(
        (doc, keys) in document(),
        pick in any::<prop::sample::Index>(),
        fresh in any::<bool>(),
        new_value in value(),
    ) {
        let before = Frontmatter::parse(&doc).expect("generated documents parse");
        let key = if fresh { "zz_new".to_string() } else { keys[pick.index(keys.len())].clone() };
        let out = set(&doc, &key, new_value.clone()).expect("edits");
        let after = Frontmatter::parse(&out).expect("re-parses");
        prop_assert_eq!(after.fields.get(&key), Some(&new_value));
        for k in before.order.iter().filter(|k| **k != key) {
            prop_assert_eq!(before.fields.get(k), after.fields.get(k));
        }
        // Every other entry's text is byte-identical: only one splice.
        let lines: Vec<&str> = doc.lines().collect();
        let out_lines: Vec<&str> = out.lines().collect();
        let key_prefix = format!("{key}:");
        if let Some(at) = lines.iter().position(|l| l.starts_with(&key_prefix)) {
            prop_assert_eq!(&lines[..at], &out_lines[..at]);
        }
        prop_assert!(out.ends_with("---\n\nBody | text\n"));
    }

    #[test]
    fn remove_then_parse_drops_only_that_key((doc, keys) in document(), pick in any::<prop::sample::Index>()) {
        let key = keys[pick.index(keys.len())].clone();
        let before = Frontmatter::parse(&doc).expect("parses");
        let out = remove(&doc, &key).expect("edits");
        let after = Frontmatter::parse(&out).expect("re-parses");
        prop_assert!(!after.fields.contains_key(&key));
        prop_assert_eq!(after.fields.len(), before.fields.len() - 1);
    }

    #[test]
    fn rendered_values_read_back(v in value()) {
        let block = render_block(&[("k", v.clone())]).expect("renders");
        let parsed = Frontmatter::parse(&block).expect("parses");
        prop_assert_eq!(parsed.fields.get("k"), Some(&v));
    }
}

/// SEC-4: nested aliases ("billion laughs") are refused before yaml-rust2
/// deep-copies them; ordinary anchors still read.
#[test]
fn alias_bombs_are_refused_and_ordinary_aliases_read() {
    // Six levels of ten: a million values from a few hundred bytes.
    let mut yaml = String::from("a0: &a0 [x,x,x,x,x,x,x,x,x,x]\n");
    for level in 1..6 {
        let prev = format!("*a{}", level - 1);
        let items = [prev.as_str(); 10].join(",");
        yaml.push_str(&format!("a{level}: &a{level} [{items}]\n"));
    }
    let source = format!("---\ntitle: t\n{yaml}---\nbody\n");
    assert!(source.len() < 1024);
    let err = Frontmatter::parse(&source).expect_err("refused");
    assert!(
        matches!(err, FrontmatterError::Yaml { line: 7, .. }),
        "{err}"
    );
    assert!(err.to_string().contains("aliases expand"), "{err}");

    let fine = fields("---\nbase: &b {rate: 150}\ncopy: *b\nlist: [*b, *b]\n---\n");
    assert_eq!(fine["copy"], json!({"rate": 150}));
    assert_eq!(fine["list"], json!([{"rate": 150}, {"rate": 150}]));
}
