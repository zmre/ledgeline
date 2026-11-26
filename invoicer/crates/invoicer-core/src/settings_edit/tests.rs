use super::*;
use serde_json::json;
use std::path::PathBuf;

/// A hand-written file: comments everywhere, `[books]` before, `[ui]` and
/// `[sync]` after.
const FILE: &str = "\
# My books' settings.

[books]
basis = \"cash\"   # kept

[invoicer]
# The folder new invoices go in.
invoices-dir = \"billing\"
# Net 30 unless the customer says otherwise.
default-terms = 30  # days
dso-days = 60

[invoicer.words]
# German headers.
quantity = [\"Menge\", \"Qty\"]

[ui]
theme = \"nord\"

[sync]
# Bank links.
x = 1
";

fn change(set: serde_json::Value, unset: &[&str]) -> Change {
    let set = match set {
        Json::Object(map) => map,
        _ => Map::new(),
    };
    let unset: Vec<String> = unset.iter().map(|s| (*s).to_string()).collect();
    Change::from_json(&set, &unset).expect("a change")
}

fn patched(set: serde_json::Value, unset: &[&str]) -> String {
    patch(FILE, &change(set, unset)).expect("patches")
}

#[test]
fn setting_a_key_keeps_its_comments_and_everything_else() {
    let after = patched(json!({"default-terms": 45}), &[]);
    assert_eq!(
        after,
        FILE.replace("default-terms = 30  # days", "default-terms = 45  # days")
    );
    assert_eq!(
        Settings::from_ledgeline_toml(&after)
            .expect("reads")
            .default_terms,
        Some(45)
    );
}

#[test]
fn adding_a_key_appends_it_to_the_table() {
    let after = patched(json!({"payment-allocation": "pro-rata"}), &[]);
    assert_eq!(
        after,
        FILE.replace(
            "dso-days = 60\n",
            "dso-days = 60\npayment-allocation = \"pro-rata\"\n"
        )
    );
}

#[test]
fn unsetting_a_key_takes_the_comment_above_it() {
    let after = patched(json!({}), &["default-terms"]);
    assert_eq!(
        after,
        FILE.replace(
            "# Net 30 unless the customer says otherwise.\ndefault-terms = 30  # days\n",
            ""
        )
    );
    // A key the file doesn't set: nothing to do.
    assert_eq!(patched(json!({}), &["catalog"]), FILE);
}

#[test]
fn the_same_value_however_written_is_no_change() {
    assert_eq!(patched(json!({"dso-days": 60}), &[]), FILE);
    assert_eq!(
        patched(json!({"words.quantity": ["Menge", "Qty"]}), &[]),
        FILE
    );
}

#[test]
fn word_lists_go_in_their_table() {
    let after = patched(json!({"words.total": ["Summe"]}), &[]);
    assert_eq!(
        after,
        FILE.replace(
            "quantity = [\"Menge\", \"Qty\"]\n",
            "quantity = [\"Menge\", \"Qty\"]\ntotal = [\"Summe\"]\n"
        )
    );
    assert_eq!(
        Settings::from_ledgeline_toml(&after)
            .expect("reads")
            .words
            .total
            .first(),
        "Summe"
    );
    // The last word list removed takes its table with it.
    let after = patched(json!({}), &["words.quantity"]);
    assert_eq!(
        after,
        FILE.replace(
            "[invoicer.words]\n# German headers.\nquantity = [\"Menge\", \"Qty\"]\n\n",
            ""
        )
    );
    // A words table where there was none.
    let added = patch(
        "[invoicer]\ndso-days = 30\n",
        &change(json!({"words.tax": ["VAT"]}), &[]),
    )
    .expect("patches");
    assert_eq!(
        added,
        "[invoicer]\ndso-days = 30\n\n[invoicer.words]\ntax = [\"VAT\"]\n"
    );
}

#[test]
fn a_missing_table_is_added_after_the_others() {
    let text = "# Mine.\n\n[books]\nbasis = \"cash\"\n\n[ui]\ntheme = \"nord\"\n";
    let after = patch(text, &change(json!({"default-terms": 14}), &[])).expect("patches");
    assert_eq!(after, format!("{text}\n[invoicer]\ndefault-terms = 14\n"));
    // Only `[invoicer.words]`: `[invoicer]` becomes explicit above it.
    let words = "[invoicer.words]\ntax = [\"VAT\"]\n";
    let after = patch(words, &change(json!({"dso-days": 45}), &[])).expect("patches");
    assert!(
        Settings::from_ledgeline_toml(&after)
            .expect("reads")
            .dso_days
            == 45,
        "{after}"
    );
    assert!(
        after.contains("[invoicer.words]\ntax = [\"VAT\"]"),
        "{after}"
    );
}

#[test]
fn a_new_file_is_the_header_then_the_table() {
    // `ledger_file::edit` puts the header first and edits "".
    let body = patch("", &change(json!({"default-terms": 30}), &[])).expect("patches");
    let text = format!("{FILE_HEADER}{body}");
    assert!(text.starts_with("# Settings for these books. Ledgeline Invoicer"));
    assert!(
        text.ends_with("\n\n[invoicer]\ndefault-terms = 30\n"),
        "{text}"
    );
    assert_eq!(
        Settings::from_ledgeline_toml(&text)
            .expect("reads")
            .default_terms,
        Some(30)
    );
    // A comment-only file keeps its comments above the new table.
    let comment = "# Hand-made.\n";
    let after = patch(comment, &change(json!({"dso-days": 30}), &[])).expect("patches");
    assert_eq!(after, "# Hand-made.\n[invoicer]\ndso-days = 30\n");
}

#[test]
fn a_result_that_does_not_read_is_refused() {
    let refused = patch(
        FILE,
        &change(
            json!({"filename": "{date}.md", "dso-days": 0, "words.total": []}),
            &[],
        ),
    );
    let Err(PatchError::Invalid(problems)) = refused else {
        panic!("{refused:?}");
    };
    let mut keys: Vec<&str> = problems.iter().filter_map(|p| p.key.as_deref()).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["dso-days", "filename", "words.total"]);
    for problem in &problems {
        assert!(!problem.message.contains("dso-days"), "{}", problem.message);
        assert!(!problem.message.contains('{'), "{}", problem.message);
    }
    assert!(matches!(
        patch("not = = toml", &change(json!({"dso-days": 3}), &[])),
        Err(PatchError::Unusable(_))
    ));
    assert!(matches!(
        patch(
            "invoicer = { dso-days = 30 }\n",
            &change(json!({"dso-days": 3}), &[])
        ),
        Err(PatchError::Unusable(_))
    ));
}

#[test]
fn only_settings_of_a_known_shape_are_taken() {
    let bad = |set: serde_json::Value, unset: &[&str]| {
        let set = match set {
            Json::Object(map) => map,
            _ => Map::new(),
        };
        let unset: Vec<String> = unset.iter().map(|s| (*s).to_string()).collect();
        Change::from_json(&set, &unset)
    };
    for (set, unset) in [
        (json!({"invoice-dir": "x"}), vec![]),
        (json!({"words.nope": ["x"]}), vec![]),
        (json!({"dso-days": 1.5}), vec![]),
        (json!({"company": null}), vec![]),
        (json!({"company": {"a": 1}}), vec![]),
        (json!({"company": "[[A]]"}), vec!["company"]),
        (json!({}), vec!["words"]),
    ] {
        assert!(
            matches!(bad(set.clone(), &unset), Err(PatchError::BadRequest(_))),
            "{set} {unset:?}"
        );
    }
    assert!(bad(json!({"aging-buckets": [15, 45]}), &[]).is_ok());
    assert!(bad(json!({}), &[]).expect("empty").is_empty());
}

#[test]
fn the_revision_is_the_invoicer_table_alone() {
    let rev = revision(FILE).expect("toml");
    // Other tables, comments and key order are no change.
    assert_eq!(
        revision(&FILE.replace("theme = \"nord\"", "theme = \"github\"")),
        Some(rev.clone())
    );
    assert_eq!(
        revision(&FILE.replace("x = 1", "x = 2\ny = 3")),
        Some(rev.clone())
    );
    assert_eq!(
        revision(&FILE.replace("# Net 30", "# Thirty")),
        Some(rev.clone())
    );
    assert_eq!(
        revision(&FILE.replace(
            "default-terms = 30  # days\ndso-days = 60",
            "dso-days = 60\ndefault-terms = 30"
        )),
        Some(rev.clone())
    );
    // A setting is.
    assert_ne!(revision(&patched(json!({"dso-days": 61}), &[])), Some(rev));
    // No table and no file read the same; not TOML has none.
    assert_eq!(revision(""), revision("[books]\nbasis = \"cash\"\n"));
    assert_eq!(revision("not = = toml"), None);
}

#[test]
fn set_keys_are_the_rows_with_use_the_default() {
    assert_eq!(
        set_keys(FILE),
        [
            "default-terms",
            "dso-days",
            "invoices-dir",
            "words.quantity"
        ]
    );
    assert!(set_keys("").is_empty());
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .join("books.journal")
}

fn with(settings: &Settings, edit: impl FnOnce(&mut Settings)) -> Settings {
    let mut after = settings.clone();
    edit(&mut after);
    after
}

#[test]
fn booking_changes_say_what_they_do() {
    let cash = Session::open_local(&fixture("cash")).expect("opens");
    let before = cash.settings().clone();
    let effects = effects(
        &before,
        &with(&before, |s| {
            s.payment_allocation = PaymentAllocation::ProRata;
            s.revenue_account = Some("revenue:other".into());
            s.dso_days = 30;
        }),
        &cash,
    );
    let keys: Vec<&str> = effects.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, ["revenue-account", "payment-allocation"]);
    assert_eq!(
        effects[1].message,
        "Payments you apply from now on are split pro rata. Payments already in your books \
         keep the split they were booked with."
    );
    assert!(
        effects[0]
            .message
            .starts_with("Rows with no account of their own book to revenue:other from now on."),
        "{}",
        effects[0].message
    );
    // Naming the account already in use changes nothing.
    assert!(
        super::effects(
            &before,
            &with(&before, |s| s.deposit_account =
                Some("liabilities:customer-deposits".into())),
            &cash,
        )
        .is_empty()
    );

    let accrual = Session::open_local(&fixture("accrual")).expect("opens");
    let before = accrual.settings().clone();
    let effects = super::effects(
        &before,
        &with(&before, |s| {
            s.receivable_account = Some("assets:ar2".into());
            s.payment_allocation = PaymentAllocation::ProRata;
        }),
        &accrual,
    );
    assert_eq!(effects.len(), 2, "{effects:?}");
    assert!(
        effects[0]
            .message
            .starts_with("Invoices you send from now on owe to assets:ar2."),
        "{}",
        effects[0].message
    );
    assert!(
        effects[0].message.contains("sent invoices stay on"),
        "{}",
        effects[0].message
    );
    assert_eq!(
        effects[1].message,
        "These books use accrual, so this applies only if they switch to cash."
    );
}

#[test]
fn deposit_payments_are_refused_while_a_deposit_waits() {
    let cash = Session::open_local(&fixture("cash")).expect("opens");
    let before = cash.settings().clone();
    let after = with(&before, |s| {
        s.cash_deposits = CashDeposits::LiabilityUntilFinal;
    });
    // 206's deposit wire went to revenue; 207, applying it, is unpaid.
    let refused = refusal(&before, &after, &cash)
        .expect("reads")
        .expect("refused");
    assert_eq!(refused.key, "cash-deposits");
    assert_eq!(refused.deposits.len(), 1, "{refused:?}");
    let waiting = &refused.deposits[0];
    assert_eq!(waiting.number, "206");
    assert_eq!(waiting.customer, "Customer X");
    assert_eq!(waiting.waiting, "$5,000.00");
    assert_eq!(waiting.booked_to, ["revenue:consulting"]);
    assert!(refused.message.starts_with("1 deposit is waiting"));
    // Nothing else is refused, and accrual books never are.
    assert_eq!(
        refusal(&before, &with(&before, |s| s.dso_days = 30), &cash).expect("reads"),
        None
    );
    let accrual = Session::open_local(&fixture("accrual")).expect("opens");
    let before = accrual.settings().clone();
    assert_eq!(
        refusal(
            &before,
            &with(&before, |s| s.cash_deposits =
                CashDeposits::LiabilityUntilFinal),
            &accrual
        )
        .expect("reads"),
        None
    );
}
