//! Id-based re-import matching: the classifier, and the one hledger-facing
//! claim it rests on.
//!
//! `reimport.rs`'s own `#[cfg(test)]` module covers the classification rules
//! against hand-written journal text. This suite covers the two things that
//! module cannot:
//!
//! 1. **The scenario**, end to end over the committed
//!    `fixtures/import/reimport/pending-then-cleared/` corpus, converted through
//!    `convert::ofx` exactly as a staged upload is — so the `fitid` column the
//!    rules file interpolates is the one the real conversion emits, not one a
//!    test typed.
//! 2. **The round-trip**, against the real binary: that `comment id:%fitid` in a
//!    rules file becomes a `; id:VALUE` comment in hledger's own output, that
//!    hledger reads it back as the tag `id`, and — the half that actually
//!    matters, since `reimport` reads `Transaction::tags` and never hledger's
//!    JSON — that *this crate's* parser lands the same pair.
//!
//! Gated behind `LEDGELINE_HLEDGER_REIMPORT_CHECK=1`, joining the existing
//! `LEDGELINE_HLEDGER_*_CHECK` suites so `cargo test` stays hermetic.
//!
//! # Safety
//!
//! hledger is run **only** over these committed fixtures. The rules file here
//! contains no `source` directive at all, so the `source … | CMD` shell path
//! `docs/imports.md` § Security describes is unreachable from this suite.

mod common;

use ledgeline_core::convert::{self, SourceFormat};
use ledgeline_core::model::Status;
use ledgeline_core::parse_journal;
use ledgeline_core::reimport::{self, ID_TAG, RowClassification};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The environment variable that opts the hledger half in.
const OPT_IN: &str = "LEDGELINE_HLEDGER_REIMPORT_CHECK";

/// Skip, loudly, unless the opt-in variable is set.
macro_rules! require_hledger {
    () => {
        if std::env::var_os(OPT_IN).is_none() {
            eprintln!("skipping: set {OPT_IN}=1 (or run `just hledger-checks`)");
            return;
        }
    };
}

/// `fixtures/import/reimport/pending-then-cleared/`.
fn scenario_dir() -> PathBuf {
    common::fixtures_dir().join("import/reimport/pending-then-cleared")
}

/// One statement, converted to the canonical CSV exactly as a staged upload is.
fn converted(name: &str) -> String {
    let path = scenario_dir().join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{} readable: {e}", path.display()));
    let format = convert::detect(name, &bytes).expect("a committed fixture converts");
    assert_eq!(format, SourceFormat::Ofx, "{name} is an OFX statement");
    let tabular = convert::convert(format, &bytes).expect("a committed fixture converts");
    convert::to_csv(&tabular)
}

/// The conversion is what puts `fitid` within a rules file's reach at all, so
/// the column list is asserted here rather than assumed by everything below.
#[test]
fn the_ofx_conversion_carries_the_row_id_and_the_hold() {
    let csv = converted("first.ofx");
    let mut lines = csv.lines();
    assert_eq!(
        lines.next(),
        Some("date,amount,name,memo,trntype,fitid,checknum"),
        "the rules file's `fields` line is written against exactly these columns"
    );
    let first = lines.next().expect("a first row");
    assert!(
        first.starts_with("2026-01-05,-4.50,COFFEE SHOP,"),
        "{first}"
    );
    assert!(
        first.contains(",HOLD,FIT0001,"),
        "the hold and the id are the two cells this whole fixture turns on: {first}"
    );
}

/// The four-way split, over the journal the first import would have written and
/// the proposal the second one produces.
///
/// This is the classifier's view of the fixture scenario, with hledger's part
/// spelled out as literal text so the test says what it means. The gated
/// `hledger_writes_the_id_tag_our_parser_reads_back` below is what proves that
/// text is what hledger actually emits.
#[test]
fn the_redownload_splits_into_status_only_conflicting_and_new() {
    // What the first import left behind — and then the hand-edit: GROCERY MART's
    // amount corrected from -32.10 to -35.60 by a person, after the fact.
    let journal = parse_journal(
        "\
2026-01-05 ! COFFEE SHOP  ; id:FIT0001
    assets:bank:checking           -4.50
    expenses:unknown                4.50

2026-01-06 * GROCERY MART  ; id:FIT0002
    assets:bank:checking          -35.60
    expenses:unknown               35.60
",
        "main.journal",
    )
    .expect("the journal parses");

    // What the redownload proposes with `.latest` ignored — the only proposal
    // that contains the two rows dedup would hide.
    let proposal = parse_journal(
        "\
2026-01-05 * COFFEE SHOP  ; id:FIT0001
    assets:bank:checking           -4.50
    expenses:unknown                4.50

2026-01-06 * GROCERY MART  ; id:FIT0002
    assets:bank:checking          -32.10
    expenses:unknown               32.10

2026-01-08 * ACME PAYROLL  ; id:FIT0003
    assets:bank:checking         1500.00
    expenses:unknown            -1500.00
",
        "proposed",
    )
    .expect("the proposal parses");

    let index = reimport::build_index(&journal, ID_TAG);
    let rows = reimport::reconcile(&index, &proposal.transactions, ID_TAG)
        .expect("the rules file declares an id");

    assert_eq!(
        rows[0].classification,
        RowClassification::StatusOnly {
            index: journal.transactions[0].index,
            existing_status: Status::Pending,
            new_status: Status::Cleared,
        },
        "a settled hold that changed nothing else is a status sync"
    );

    let RowClassification::Conflicting { diffs, .. } = &rows[1].classification else {
        panic!("the hand-edited amount must be a conflict: {rows:?}");
    };
    assert_eq!(diffs[0].field, "posting 1 amount");
    assert_eq!(diffs[0].existing, "-35.60", "the user's own figure");
    assert_eq!(diffs[0].incoming, "-32.10", "the bank's");

    assert_eq!(rows[2].classification, RowClassification::New);
}

/// The row `.latest` hides is the row worth talking about — so an implementation
/// that classified the ORDINARY proposal would report nothing at all.
///
/// This is `TODO.md`'s bug stated as a test: with the two older rows filtered out
/// by date, there is nothing left for a match to notice, and the settled hold
/// stays pending forever.
#[test]
fn classifying_the_deduped_proposal_would_see_none_of_it() {
    let journal = parse_journal(
        "2026-01-05 ! COFFEE SHOP  ; id:FIT0001\n    a  -4.50\n    b   4.50\n",
        "main.journal",
    )
    .expect("parses");
    // What `hledger import --dry-run` proposes with `.latest` at 2026-01-06:
    // only the genuinely new row.
    let deduped = parse_journal(
        "2026-01-08 * ACME PAYROLL  ; id:FIT0003\n    a  1500.00\n    b -1500.00\n",
        "proposed",
    )
    .expect("parses");
    let index = reimport::build_index(&journal, ID_TAG);
    let rows = reimport::reconcile(&index, &deduped.transactions, ID_TAG).expect("ids declared");
    assert!(
        rows.iter()
            .all(|row| row.classification == RowClassification::New),
        "the settled hold is simply not in this proposal, which is the bug"
    );
}

/// An id may subtract from an import and never add to it.
///
/// A journal imported before its rules file grew a `comment id:` line carries no
/// ids at all. Every row of the next re-download is then `New` — correctly, since
/// this module has nothing to say about them — and `retain_new` must leave the
/// proposal exactly as it found it rather than let anything conclude those rows
/// should now be imported afresh.
#[test]
fn a_journal_with_no_ids_is_left_entirely_alone() {
    let journal = parse_journal(
        "2026-01-05 * COFFEE SHOP\n    a  -4.50\n    b   4.50\n",
        "main.journal",
    )
    .expect("parses");
    let entries = "2026-01-05 * COFFEE SHOP  ; id:FIT0001\n    a  -4.50\n    b   4.50\n\n";
    let proposal = parse_journal(entries, "proposed").expect("parses");
    let index = reimport::build_index(&journal, ID_TAG);
    assert!(index.is_empty());
    assert_eq!(
        reimport::retain_new(entries, &proposal.transactions, &index, ID_TAG),
        entries
    );
}

// ===========================================================================
// The hledger-backed half
// ===========================================================================

/// **The round-trip this whole phase rests on**, verified against the binary.
///
/// Three claims, and the third is the one that matters:
///
/// 1. `comment id:%fitid` makes hledger write `; id:FIT0001` onto the
///    transaction — no new rules grammar, just a field it already assigns;
/// 2. hledger reads it back as the tag `id` (`ttags` in `print -O json`);
/// 3. **this crate's own parser** lands the same `("id", "FIT0001")` in
///    `Transaction::tags` — which is what `reimport` actually reads. Steps 1 and
///    2 could both hold while step 3 failed, and then nothing would ever match.
///
/// It also pins the status mapping in the same run, because a status hledger did
/// not write is a status-only difference that can never occur.
#[test]
fn hledger_writes_the_id_tag_our_parser_reads_back() {
    require_hledger!();
    let scratch = Scratch::new("roundtrip");
    let csv = scratch.0.join("bank.csv");
    std::fs::write(&csv, converted("first.ofx")).expect("write the converted CSV");
    std::fs::copy(
        scenario_dir().join("bank.csv.rules"),
        scratch.0.join("bank.csv.rules"),
    )
    .expect("copy the rules fixture");

    let printed = hledger_print(&csv).expect("hledger prints the fixture");
    let first = &printed.as_array().expect("an array")[0];
    assert_eq!(
        first["ttags"],
        serde_json::json!([["id", "FIT0001"]]),
        "hledger's own reading of the comment: {first}"
    );

    // …and now the half that matters. Re-parse hledger's TEXT output, which is
    // the shape `import` appends to a journal and therefore the shape
    // `reimport::build_index` will meet.
    let text = hledger_print_text(&csv).expect("hledger prints the fixture");
    let journal = parse_journal(&text, "proposed").expect("hledger's output is a journal");
    assert_eq!(
        journal.transactions[0].tags,
        vec![("id".to_string(), "FIT0001".to_string())],
        "the tag `reimport` reads must survive OUR parser too:\n{text}"
    );
    assert_eq!(
        reimport::id_of(&journal.transactions[0], ID_TAG),
        Some("FIT0001")
    );
    assert_eq!(
        journal.transactions[0].status,
        Status::Pending,
        "TRNTYPE HOLD must reach hledger's `status` field, or a settled hold is invisible:\n{text}"
    );
    assert_eq!(journal.transactions[1].status, Status::Cleared);
    assert!(
        reimport::maps_status(&journal.transactions),
        "a proposal carrying markers is how we know the rules file assigns them"
    );
}

/// The redownload's own proposal, from the real binary, classified against the
/// real first import's own output. The scenario with nothing hand-written.
#[test]
fn the_real_binary_produces_the_split_the_classifier_expects() {
    require_hledger!();
    let scratch = Scratch::new("split");
    std::fs::copy(
        scenario_dir().join("bank.csv.rules"),
        scratch.0.join("bank.csv.rules"),
    )
    .expect("copy the rules fixture");

    // Import one: exactly the text a commit would append.
    let first_csv = scratch.0.join("bank.csv");
    std::fs::write(&first_csv, converted("first.ofx")).expect("write");
    let first = hledger_print_text(&first_csv).expect("hledger prints");
    let journal = parse_journal(&first, "main.journal").expect("hledger's output is a journal");

    // Import two, dedup-free: the redownload as `bare_proposal` would see it.
    let second_csv = scratch.0.join("redownload.csv");
    std::fs::write(&second_csv, converted("redownload.ofx")).expect("write");
    std::fs::copy(
        scenario_dir().join("bank.csv.rules"),
        scratch.0.join("redownload.csv.rules"),
    )
    .expect("copy the rules fixture");
    let second = hledger_print_text(&second_csv).expect("hledger prints");
    let proposal = parse_journal(&second, "proposed").expect("hledger's output is a journal");

    let index = reimport::build_index(&journal, ID_TAG);
    let rows = reimport::reconcile(&index, &proposal.transactions, ID_TAG).expect("ids declared");
    let kinds: Vec<&str> = rows
        .iter()
        .map(|row| match row.classification {
            RowClassification::New => "new",
            RowClassification::Unchanged => "unchanged",
            RowClassification::StatusOnly { .. } => "status-only",
            RowClassification::Conflicting { .. } => "conflicting",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["status-only", "unchanged", "new"],
        "the hold settled, the settled row did not move, and the payroll is new\n{second}"
    );
}

/// A scratch directory that removes itself on drop. Mirrors `rules_generate.rs`,
/// which is the other core suite that has to hand a real hledger a real file.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ledgeline-reimport-{name}-{}", std::process::id()));
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

/// `hledger --no-conf -I -f CSV print -O json`, over a fixture CSV whose sibling
/// `.rules` file hledger finds by name.
///
/// `--no-conf` first, ahead of the subcommand, for the reason `docs/imports.md`
/// § *No hledger we run reads a config file* gives at length: a config file can
/// replace the command.
fn hledger_print(csv: &Path) -> Result<serde_json::Value, String> {
    serde_json::from_str(&hledger_print_raw(csv, &["-O", "json"])?)
        .map_err(|e| format!("hledger JSON: {e}"))
}

/// The same run, as journal text — the shape `hledger import` appends.
fn hledger_print_text(csv: &Path) -> Result<String, String> {
    hledger_print_raw(csv, &[])
}

fn hledger_print_raw(csv: &Path, extra: &[&str]) -> Result<String, String> {
    let output = Command::new("hledger")
        .arg("--no-conf")
        .arg("-I")
        .arg("-f")
        .arg(csv)
        .arg("print")
        .args(extra)
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
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
