//! The rules-file **generator**, against a corpus of real-shaped bank headers.
//!
//! `rules/generate.rs`'s own unit tests work on hand-built `Tabular`s, one fact
//! at a time. This suite runs the whole pipeline the server runs — bytes →
//! `convert` → `generate` → `RulesDoc` — over the committed fixtures in
//! `fixtures/import/generate/headers/`, which is where the messy cases live: a
//! header nobody would invent, a currency symbol in every cell, a decimal comma,
//! two date columns, a file that offers three different amount columns at once.
//!
//! # The gated half is the one that matters
//!
//! `docs/imports.md`'s fact 4: **parse success is not a matching signal.** A
//! mismatched rules file frequently parses, exits 0 and produces garbage —
//! postings with no amount, amounts in a commodity of their own, everything in
//! `expenses:unknown`. So "the draft is valid hledger syntax" is worth very
//! little on its own, and `hledger_reads_what_the_generator_drafts` is what
//! actually proves the drafts work: it runs the real binary over each fixture
//! with its generated rules file and checks the transactions that come out.
//!
//! Gated behind `LEDGELINE_HLEDGER_GENERATE_CHECK=1`, following the five
//! existing `LEDGELINE_HLEDGER_*_CHECK` suites — `cargo test` stays hermetic.
//!
//! # Safety
//!
//! hledger is run **only** over these committed fixtures and over rules files
//! this crate just generated, never a user's file. The generator emits no
//! `source` directive at all, so the `source … | CMD` shell-execution path
//! `docs/imports.md` § Security describes cannot be reached from here.

mod common;

use ledgeline_core::convert::{self, SourceFormat, Tabular};
use ledgeline_core::rules::generate::{self, DEFAULT_ACCOUNT2};
use ledgeline_core::rules::{HledgerField, NumberedField, RulesDoc};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The environment variable that opts the hledger half in.
const OPT_IN: &str = "LEDGELINE_HLEDGER_GENERATE_CHECK";

/// The account every draft in this suite is written for.
const ACCOUNT1: &str = "assets:bank:checking";

/// `fixtures/import/generate/headers/`.
fn headers_dir() -> PathBuf {
    common::fixtures_dir().join("import/generate/headers")
}

/// `fixtures/import/generate/isolated/` — the report-litter corpus.
///
/// A sibling directory rather than two more files in `headers/`, because the
/// corpus-wide properties below are stated over `headers/` in absolute terms
/// ("every data row becomes a transaction"), and the whole point of these two
/// is that one of them deliberately imports one row FEWER than it carries.
fn isolated_dir() -> PathBuf {
    common::fixtures_dir().join("import/generate/isolated")
}

/// One fixture, converted exactly as a staged upload is.
fn converted_in(dir: &Path, name: &str) -> Tabular {
    let path = dir.join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{} readable: {e}", path.display()));
    let format = convert::detect(name, &bytes).expect("a committed fixture converts");
    assert_eq!(format, SourceFormat::Csv, "{name} is a CSV");
    convert::convert(format, &bytes).expect("a committed fixture converts")
}

/// One `headers/` fixture, converted exactly as a staged upload is.
fn converted(name: &str) -> Tabular {
    converted_in(&headers_dir(), name)
}

/// The `fields` names a draft settled on, in column order.
fn field_names(doc: &RulesDoc) -> Vec<String> {
    doc.settings()
        .fields
        .map(|setting| setting.value)
        .unwrap_or_default()
}

/// One setting as text, or `None` when the draft did not write it.
fn date_format(doc: &RulesDoc) -> Option<String> {
    doc.settings().date_format.map(|setting| setting.value)
}

fn account1(doc: &RulesDoc) -> Option<String> {
    doc.settings().account1.map(|setting| setting.value)
}

/// Every fixture in the corpus, by name.
fn every_fixture() -> Vec<String> {
    let dir = headers_dir();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} readable: {e}", dir.display()))
        .filter_map(|entry| entry.ok()?.file_name().to_str().map(str::to_string))
        .filter(|name| name.ends_with(".csv"))
        .collect();
    names.sort();
    assert!(
        names.len() >= 6,
        "expected the committed header corpus, found {names:?}"
    );
    names
}

// ---------------------------------------------------------------------------
// Per-fixture expectations
//
// Each fixture exists to make exactly one wrong implementation fail; the
// assertions below say which. See `fixtures/import/README.md` § generate/.
// ---------------------------------------------------------------------------

#[test]
fn chase_reads_posting_date_and_leaves_details_and_type_alone() {
    // "Posting Date", not "Date" — a synonym table that only knows the bare word
    // maps nothing here. "Details" holds DEBIT/CREDIT *words*, which must not
    // become an amount column, and "Type" is nobody's field.
    let table = converted("chase-checking.csv");
    let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");
    assert_eq!(
        field_names(&drafted.doc),
        [
            "details",
            "date",
            "description",
            "amount",
            "type",
            "balance",
            "checkorslip"
        ]
    );
    assert_eq!(date_format(&drafted.doc).as_deref(), Some("%m/%d/%Y"));
    assert_eq!(account1(&drafted.doc).as_deref(), Some(ACCOUNT1));
}

#[test]
fn capital_one_gets_two_dates_and_the_split_amount_scheme() {
    let table = converted("capitalone-card.csv");
    let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");
    assert_eq!(
        field_names(&drafted.doc),
        [
            "date",
            "date2",
            "cardno",
            "description",
            "category",
            "amount-out",
            "amount-in"
        ]
    );
    // `%category` is exactly why an unmapped column keeps its own name: the
    // user's next act is a rule reading it.
    assert_eq!(date_format(&drafted.doc).as_deref(), Some("%Y-%m-%d"));
}

#[test]
fn a_uk_export_is_day_first_and_needs_no_currency_of_its_own() {
    // Every date has a day > 12 somewhere in the sample, so this is NOT
    // ambiguous — and the cells carry `£`, so declaring a currency would
    // produce `££3.60`, a distinct commodity, silently.
    let table = converted("uk-current-account.csv");
    let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");
    assert_eq!(date_format(&drafted.doc).as_deref(), Some("%d/%m/%Y"));
    assert!(!drafted.date_format.expect("a format").ambiguous);
    assert!(
        drafted.doc.settings().currency.is_none(),
        "the cells already carry `£`"
    );
    assert_eq!(
        field_names(&drafted.doc),
        ["date", "description", "amount-out", "amount-in", "balance"]
    );
}

#[test]
fn a_european_export_declares_the_decimal_comma() {
    // THE silent-corruption case. `1.250,00` with no `decimal-mark` is read by
    // hledger as one thousand two hundred and fifty *thousandths* of a unit,
    // and `print` renders it straight back as `1.250,00` — so nothing in
    // hledger's own output shows the 1000x error.
    let table = converted("euro-decimal-comma.csv");
    let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");
    assert_eq!(
        drafted
            .doc
            .settings()
            .decimal_mark
            .map(|setting| setting.value),
        Some(',')
    );
    assert_eq!(date_format(&drafted.doc).as_deref(), Some("%d.%m.%Y"));
    assert_eq!(
        field_names(&drafted.doc),
        ["date", "description", "amount", "currency"]
    );
}

#[test]
fn paypal_maps_what_it_can_and_refuses_to_map_status() {
    // Three amount-shaped columns (`Gross`, `Fee`, `Net`) and not one of them
    // named anything hledger knows. The value-shaped fallback claims the first,
    // low-confidence — and `Status`, whose values are `Completed`/`Pending`,
    // must never reach hledger's own `status` field, which wants `*`/`!`.
    let table = converted("paypal-activity.csv");
    let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");
    let names = field_names(&drafted.doc);
    assert_eq!(names[0], "date");
    assert_eq!(names[3], "description", "PayPal calls the payee `Name`");
    assert_eq!(names[5], "", "`Status` is not written into `fields` at all");
    assert_eq!(names[6], "currency");
    assert_eq!(names[7], "amount", "`Gross`, by the shape of its values");
    assert_eq!(names[8], "fee");
    assert_eq!(names[9], "net");
    assert_eq!(names[10], "balance");

    let gross = drafted
        .columns
        .iter()
        .find(|column| column.field == Some(HledgerField::Amount))
        .expect("an amount column");
    assert!(
        gross.confidence < 0.5,
        "a guess made from values alone is not a confident one: {}",
        gross.confidence
    );
}

#[test]
fn an_ambiguous_date_file_says_so_and_picks_one_amount_scheme() {
    let table = converted("ambiguous-dates.csv");
    let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");
    let guess = drafted.date_format.clone().expect("a format");
    assert!(guess.ambiguous, "every component is <= 12");
    assert_eq!(guess.format, "%m/%d/%Y");
    assert_eq!(
        field_names(&drafted.doc),
        ["date", "description", "amount", "debit", "credit"],
        "the signed total wins; the other two are named, not mapped"
    );
    assert!(
        drafted
            .warnings
            .iter()
            .any(|warning| warning.contains("more than one way")),
        "{:?}",
        drafted.warnings
    );
}

// ---------------------------------------------------------------------------
// `isolated/` — a label the original report left in the data area
// ---------------------------------------------------------------------------

#[test]
fn the_report_label_survives_conversion_and_is_excluded_by_the_draft() {
    // First half: `convert` cannot help here, and that is why this exists at
    // all. Preamble and trailer trimming work from the ENDS of a table, so a
    // one-cell row sandwiched between the header and the transactions is not
    // preamble and not a trailer -- it arrives in the `Tabular` as a record.
    let table = converted_in(&isolated_dir(), "quickbooks-label.csv");
    assert_eq!(table.rows.len(), 8, "the label row is still a row");
    assert_eq!(table.rows[0][0], "General Ledger");

    // Second half: the draft names it, excludes it, and says so.
    let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");
    assert_eq!(
        generate::isolated_columns(&table),
        vec![generate::IsolatedColumn {
            column: 0,
            rows: vec![0]
        }]
    );
    assert_eq!(
        field_names(&drafted.doc),
        ["column1", "date", "description", "amount"],
        "the header-less column gets a machine name so `%column1` resolves"
    );
    assert!(
        drafted.doc.text().ends_with("if %column1 .\n    skip\n\n"),
        "{}",
        drafted.doc.text()
    );
    // The guesses are the ones the data supports, not the ones the label
    // dragged them to: every date sample is now a real date.
    assert_eq!(date_format(&drafted.doc).as_deref(), Some("%m/%d/%Y"));
    assert!(
        drafted
            .warnings
            .iter()
            .any(|warning| warning.contains("data row 1") && warning.contains("General Ledger")),
        "{:?}",
        drafted.warnings
    );
}

#[test]
fn a_sometimes_blank_real_column_is_never_excluded() {
    // THE false positive. `Check Number` is populated on three rows of eight --
    // sparser than the label column in the fixture above -- and a detector
    // keyed on column sparsity alone would draft a rule dropping every check
    // the user wrote, silently and at exit 0. Those rows carry a date, a payee
    // and an amount, so they are not isolated however empty the column is.
    let table = converted_in(&isolated_dir(), "check-number.csv");
    assert_eq!(table.rows.len(), 8);
    assert_eq!(generate::isolated_columns(&table), Vec::new());

    let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");
    assert!(
        !drafted.doc.text().contains("if "),
        "no exclusion rule: {}",
        drafted.doc.text()
    );
    assert_eq!(
        field_names(&drafted.doc),
        ["date", "description", "code", "amount"],
        "the column is real data, and is mapped as such"
    );
}

// ---------------------------------------------------------------------------
// Properties every draft has, whatever the fixture
// ---------------------------------------------------------------------------

#[test]
fn every_draft_is_writable_through_the_create_route() {
    // The create `PUT` can only name TYPED items. `ItemBody` has no comment
    // variant and a `trivia` item's only save form is `{kind:"keep", id}`,
    // which needs a file that already exists — so a draft carrying one would be
    // a draft nobody could save. This is the corpus-wide version of
    // `generate.rs`'s own `every_drafted_item_can_be_written_back`.
    for name in every_fixture() {
        let drafted = generate::generate(&converted(&name), ACCOUNT1).expect("drafts");
        for item in drafted.doc.items() {
            assert!(
                item.opaque().is_none(),
                "{name}: a draft may not contain an opaque item"
            );
            assert!(
                !matches!(item.kind, ledgeline_core::rules::ItemKind::Trivia),
                "{name}: a draft may not contain trivia"
            );
        }
    }
}

#[test]
fn every_draft_re_parses_as_itself_and_raises_no_warning() {
    for name in every_fixture() {
        let drafted = generate::generate(&converted(&name), ACCOUNT1).expect("drafts");
        let text = drafted.doc.text().to_string();
        let reparsed = RulesDoc::parse(&text);
        assert_eq!(reparsed.text(), text, "{name}: round trip");
        assert!(
            reparsed.warnings().is_empty(),
            "{name}: {:?}",
            reparsed.warnings()
        );
        assert_eq!(
            account1(&reparsed).as_deref(),
            Some(ACCOUNT1),
            "{name}: account1 survives"
        );
        assert_eq!(
            reparsed.settings().account2.map(|s| s.value).as_deref(),
            Some(DEFAULT_ACCOUNT2),
            "{name}: the fallback category is deliberately dumb"
        );
    }
}

#[test]
fn every_draft_maps_a_date_and_some_amount() {
    // Not a formality: a rules file with no date column is one hledger refuses
    // outright, and one with no amount column produces postings with no amount
    // at all — `docs/imports.md`'s fact 4, which exits 0.
    for name in every_fixture() {
        let drafted = generate::generate(&converted(&name), ACCOUNT1).expect("drafts");
        let names = field_names(&drafted.doc);
        assert!(names.iter().any(|n| n == "date"), "{name}: no date column");
        assert!(
            names
                .iter()
                .any(|n| n == "amount" || n == "amount-in" || n == "amount-out"),
            "{name}: no amount column"
        );
        assert!(
            drafted.date_format.is_some(),
            "{name}: no date format was recognised"
        );
    }
}

#[test]
fn no_draft_ever_declares_a_separator_or_an_encoding() {
    // The file a draft describes is `convert::to_csv`'s output: always commas,
    // always UTF-8. Copying the download's own delimiter or code page across
    // would describe a file that no longer exists — and an `encoding` line
    // would make hledger mis-decode a UTF-8 file.
    for name in every_fixture() {
        let drafted = generate::generate(&converted(&name), ACCOUNT1).expect("drafts");
        let settings = drafted.doc.settings();
        assert!(settings.separator.is_none(), "{name}");
        assert!(settings.encoding.is_none(), "{name}");
        assert_eq!(settings.skip.map(|s| s.value), Some(1), "{name}");
    }
}

#[test]
fn no_draft_ever_names_the_same_column_twice() {
    // hledger resolves duplicate `fields` names as "first one wins", silently
    // (verified against 1.52), so a duplicate is a column that vanishes with no
    // diagnostic anywhere.
    for name in every_fixture() {
        let drafted = generate::generate(&converted(&name), ACCOUNT1).expect("drafts");
        let names: Vec<String> = field_names(&drafted.doc)
            .into_iter()
            .filter(|n| !n.is_empty())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "{name}: duplicate in {names:?}");
    }
}

// ---------------------------------------------------------------------------
// The gated half: what hledger actually does with these drafts
// ---------------------------------------------------------------------------

/// A scratch directory that removes itself on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ledgeline-generate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `hledger -I -f <csv> print -O json`, or hledger's own complaint.
///
/// `--no-conf` first, ahead of the subcommand, for the reason `docs/imports.md`
/// § *No hledger we run reads a config file* gives: a config's first bare word
/// **replaces the command**, and this repository is a directory someone may well
/// have one above.
///
/// `-I` because a draft that maps a running-balance column emits real balance
/// assertions, and a CSV read on its own starts from zero — so every assertion
/// would fail for a reason that says nothing about the mapping. It is the same
/// flag, for the same reason, that `import_api` puts on its own import.
///
/// `-O json` rather than the human-readable output for `matching.rs`'s reason:
/// `print`'s layout is a display format, and the JSON is the one hledger commits
/// to.
fn hledger_print(csv: &Path) -> Result<serde_json::Value, String> {
    let output = Command::new("hledger")
        .arg("--no-conf")
        .arg("-I")
        .arg("-f")
        .arg(csv)
        .args(["print", "-O", "json"])
        .output()
        .map_err(|e| format!("could not run hledger: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "hledger exited {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("hledger JSON: {e}"))
}

/// The `accountN` field for posting `n`.
fn account_field(n: u8) -> HledgerField {
    HledgerField::Numbered {
        base: NumberedField::Account,
        n,
    }
}

#[test]
fn hledger_reads_what_the_generator_drafts() {
    if std::env::var_os(OPT_IN).is_none_or(|value| value.is_empty()) {
        eprintln!("skipping: set {OPT_IN}=1 to run the hledger generator check");
        return;
    }
    // Named so the assertions below cannot pass by accident: nothing else in
    // this repository writes these accounts.
    assert_eq!(
        account_field(1),
        HledgerField::Numbered {
            base: NumberedField::Account,
            n: 1
        }
    );

    for name in every_fixture() {
        let table = converted(&name);
        let rows = table.rows.len();
        let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");

        // Write exactly what a commit would: the CONVERTED CSV, plus the
        // drafted rules file under the sibling name hledger looks for. Not the
        // fixture's own bytes -- the whole premise of the generator is that it
        // describes `convert::to_csv`'s output.
        let scratch = Scratch::new(name.trim_end_matches(".csv"));
        let csv = scratch.0.join(&name);
        std::fs::write(&csv, convert::to_csv(&table)).expect("write the converted CSV");
        std::fs::write(scratch.0.join(format!("{name}.rules")), drafted.doc.text())
            .expect("write the drafted rules file");

        let json = hledger_print(&csv)
            .unwrap_or_else(|error| panic!("{name}: the drafted rules file failed:\n{error}"));
        let transactions = json.as_array().expect("hledger emits an array");

        // 1. EVERY row arrives. A `skip` that is one out silently drops the
        //    first transaction (or eats a header as data), exit 0 either way.
        assert_eq!(
            transactions.len(),
            rows,
            "{name}: {rows} data rows became {} transactions",
            transactions.len()
        );

        for transaction in transactions {
            let postings = transaction["tpostings"]
                .as_array()
                .unwrap_or_else(|| panic!("{name}: no postings in {transaction}"));
            assert_eq!(postings.len(), 2, "{name}: {transaction}");

            // 2. The statement's own account is posting 1, so the balance the
            //    import screen reconciles against is a balance OF something.
            assert_eq!(
                postings[0]["paccount"].as_str(),
                Some(ACCOUNT1),
                "{name}: {transaction}"
            );
            assert_eq!(
                postings[1]["paccount"].as_str(),
                Some(DEFAULT_ACCOUNT2),
                "{name}: {transaction}"
            );

            // 3. Fact 4, both halves. A posting hledger accepted with NO amount
            //    at all, and an amount with no commodity -- which forms a
            //    separate commodity, so the `$` balance never moves. Both exit
            //    0, and neither is visible in a percentage.
            let amount = postings[0]["pamount"]
                .as_array()
                .unwrap_or_else(|| panic!("{name}: {transaction}"));
            assert_eq!(
                amount.len(),
                1,
                "{name}: a posting with no amount:\n{transaction}"
            );

            // 4. A date came out. hledger will not produce a transaction
            //    without one, but a WRONG date-format is a hard error rather
            //    than a wrong date, so this pins that the read succeeded at all.
            assert!(
                transaction["tdate"].as_str().is_some_and(|d| d.len() == 10),
                "{name}: {transaction}"
            );
        }
    }
}

/// hledger's parsed value for transaction `at`'s first posting, as
/// `(mantissa, decimal places)` — the pair that says what the number *is*.
fn parsed_amount(json: &serde_json::Value, at: usize) -> (i64, i64) {
    let quantity =
        &json.as_array().expect("an array")[at]["tpostings"][0]["pamount"][0]["aquantity"];
    (
        quantity["decimalMantissa"]
            .as_i64()
            .unwrap_or_else(|| panic!("a decimal mantissa in {quantity}")),
        quantity["decimalPlaces"]
            .as_i64()
            .unwrap_or_else(|| panic!("decimal places in {quantity}")),
    )
}

/// Write `table` and `rules` into `scratch` under `name`, and print it.
fn print_with(scratch: &Scratch, name: &str, table: &Tabular, rules: &str) -> serde_json::Value {
    let csv = scratch.0.join(name);
    std::fs::write(&csv, convert::to_csv(table)).expect("write the converted CSV");
    std::fs::write(scratch.0.join(format!("{name}.rules")), rules).expect("write the rules file");
    hledger_print(&csv).unwrap_or_else(|error| panic!("{name}:\n{error}"))
}

/// The draft's own text with the `decimal-mark` line taken out — what a
/// generator that never asked the question would have written.
fn without_decimal_mark(text: &str) -> String {
    text.lines()
        .filter(|line| !line.starts_with("decimal-mark"))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// The draft's own text with every conditional block taken off the end — what a
/// generator that never looked for a report label would have written.
///
/// The exclusions are the last items in a draft and nothing follows them, so
/// truncating at the first `if` line takes exactly them and nothing else.
fn without_exclusions(text: &str) -> String {
    text.lines()
        .take_while(|line| !line.starts_with("if "))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// `hledger print`'s answer, or its complaint, without deciding which is wrong.
fn print_or_error(
    scratch: &Scratch,
    name: &str,
    table: &Tabular,
    rules: &str,
) -> Result<Value, String> {
    let csv = scratch.0.join(name);
    std::fs::write(&csv, convert::to_csv(table)).expect("write the converted CSV");
    std::fs::write(scratch.0.join(format!("{name}.rules")), rules).expect("write the rules file");
    hledger_print(&csv)
}

#[test]
fn the_drafted_exclusion_leaves_out_the_label_row_and_nothing_else() {
    if std::env::var_os(OPT_IN).is_none_or(|value| value.is_empty()) {
        eprintln!("skipping: set {OPT_IN}=1 to run the hledger generator check");
        return;
    }
    // The test that would have caught the reported bug, and — like the
    // decimal-mark one above — it has to assert the WRONG answer too. Without
    // the exclusion this is not "seven transactions and one odd one": hledger
    // abandons the ENTIRE read on the first record it cannot date, so the
    // answer is a hard failure and zero transactions. A test that only checked
    // "seven came back" would pass against a generator that had instead learnt
    // to drop the row during conversion, which is a different and much more
    // dangerous fix.
    let scratch = Scratch::new("isolated");

    let table = converted_in(&isolated_dir(), "quickbooks-label.csv");
    let rows = table.rows.len();
    let drafted = generate::generate(&table, ACCOUNT1).expect("drafts");

    let right = print_or_error(&scratch, "label.csv", &table, drafted.doc.text())
        .unwrap_or_else(|error| panic!("the drafted rules file failed:\n{error}"));
    let transactions = right.as_array().expect("hledger emits an array");
    assert_eq!(
        transactions.len(),
        rows - 1,
        "{rows} rows, one of them a label, became {} transactions",
        transactions.len()
    );
    // Exactly the right one is gone: every transaction has a real date, a real
    // amount and the two accounts, and none of them is the label.
    for transaction in transactions {
        let postings = transaction["tpostings"]
            .as_array()
            .unwrap_or_else(|| panic!("no postings in {transaction}"));
        assert_eq!(postings.len(), 2, "{transaction}");
        assert_eq!(postings[0]["paccount"].as_str(), Some(ACCOUNT1));
        assert_eq!(postings[1]["paccount"].as_str(), Some(DEFAULT_ACCOUNT2));
        assert_eq!(postings[0]["pamount"].as_array().map(Vec::len), Some(1));
        assert!(
            transaction["tdate"].as_str().is_some_and(|d| d.len() == 10),
            "{transaction}"
        );
        assert_ne!(
            transaction["tdescription"].as_str(),
            Some("General Ledger"),
            "the label imported as a transaction"
        );
    }
    // And the payees that ARE there are the file's own, so nothing was dropped
    // along with the label.
    let payees: Vec<&str> = transactions
        .iter()
        .filter_map(|transaction| transaction["tdescription"].as_str())
        .collect();
    assert!(payees.contains(&"COFFEE ROASTERS"), "{payees:?}");
    assert!(payees.contains(&"INTEREST PAID"), "{payees:?}");

    // The wrong answer, asserted: without the rule there is no import at all.
    let wrong = print_or_error(
        &scratch,
        "label.csv",
        &table,
        &without_exclusions(drafted.doc.text()),
    );
    let complaint = wrong.expect_err("without the exclusion hledger refuses the file");
    assert!(
        complaint.contains("General Ledger"),
        "hledger's own complaint names the row: {complaint}"
    );

    // The other half of the corpus: a sparse but REAL column changes nothing.
    // Every row imports, including the three carrying a check number.
    let checks = converted_in(&isolated_dir(), "check-number.csv");
    let drafted = generate::generate(&checks, ACCOUNT1).expect("drafts");
    let json = print_or_error(&scratch, "checks.csv", &checks, drafted.doc.text())
        .unwrap_or_else(|error| panic!("the drafted rules file failed:\n{error}"));
    assert_eq!(
        json.as_array().map(Vec::len),
        Some(checks.rows.len()),
        "a sometimes-blank real column must not cost a single transaction"
    );
}

#[test]
fn the_decimal_mark_the_generator_chose_is_the_one_that_changes_the_number() {
    if std::env::var_os(OPT_IN).is_none_or(|value| value.is_empty()) {
        eprintln!("skipping: set {OPT_IN}=1 to run the hledger generator check");
        return;
    }
    // The check that could not be written any other way, and the one that has
    // to assert the WRONG answer as well as the right one.
    //
    // A wrong `decimal-mark` is not an error and leaves no visible trace:
    // hledger re-renders `-1,200` as `-1,200` whichever way it read it, so
    // `print`'s TEXT is byte-identical and only the parsed VALUE differs by a
    // factor of a thousand. Both fixtures are therefore run TWICE — once with
    // the draft as generated, once with its `decimal-mark` line removed — and
    // the second run is asserted to be wrong. Without that half this test would
    // pass against a generator that emitted no `decimal-mark` at all, because
    // hledger resolves a value carrying BOTH separators (`2.400,00`) correctly
    // on its own. Only a lone separator followed by exactly three digits
    // discriminates, which is why each fixture has a whole-thousands row.
    let scratch = Scratch::new("decimal-mark");

    // European: `-1.500` is fifteen hundred euros, and reads as 1.5 unless the
    // decimal mark is declared to be the comma.
    let euro = converted("euro-decimal-comma.csv");
    let drafted = generate::generate(&euro, ACCOUNT1).expect("drafts");
    assert_eq!(
        drafted
            .doc
            .settings()
            .decimal_mark
            .map(|setting| setting.value),
        Some(',')
    );
    let right = print_with(&scratch, "euro.csv", &euro, drafted.doc.text());
    // hledger keeps the cell's own precision, so this is -1500 exactly, with no
    // decimal places -- not -1500.00. The pair is the point: the WRONG reading
    // below has the same mantissa and a different exponent.
    assert_eq!(parsed_amount(&right, 3), (-1_500, 0), "-1.500 is -1500");
    let wrong = print_with(
        &scratch,
        "euro.csv",
        &euro,
        &without_decimal_mark(drafted.doc.text()),
    );
    assert_eq!(
        parsed_amount(&wrong, 3),
        (-1_500, 3),
        "without the directive hledger reads -1.500 as -1.5, silently"
    );

    // US: the mirror image. `-1,200` is twelve hundred dollars, and reads as
    // 1.2 unless the decimal mark is declared to be the point.
    let us = converted("thousands-trap.csv");
    let drafted = generate::generate(&us, ACCOUNT1).expect("drafts");
    assert_eq!(
        drafted
            .doc
            .settings()
            .decimal_mark
            .map(|setting| setting.value),
        Some('.')
    );
    let right = print_with(&scratch, "us.csv", &us, drafted.doc.text());
    assert_eq!(parsed_amount(&right, 1), (-1_200, 0), "-1,200 is -1200");
    let wrong = print_with(
        &scratch,
        "us.csv",
        &us,
        &without_decimal_mark(drafted.doc.text()),
    );
    assert_eq!(
        parsed_amount(&wrong, 1),
        (-1_200, 3),
        "without the directive hledger reads -1,200 as -1.2, silently"
    );
}
