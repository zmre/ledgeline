//! Regression tests for the import-rules **CSV column preview**
//! ([`ledgeline_core::rules::Discovery::preview`], Imports step 6).
//!
//! The preview exists so a mapping screen can show `Col 3  "GROCERY STORE"`
//! instead of a bare `%3`. To do that it has to follow a path taken *out of a
//! file's contents*, which is a new kind of reach for a module whose entire job
//! up to here was deciding what not to touch. Every test below names the guard
//! it exercises and what goes wrong without it.
//!
//! | guard | without it |
//! |---|---|
//! | `source ... \| CMD` refused | **opening a downloaded rules file runs a shell command** |
//! | bare-filename `source` refused | `~/Downloads` becomes readable through a file the user was sent |
//! | `parse::confine` before any `stat` | the refusal reason is an existence oracle for paths outside the root |
//! | `symlink_metadata` on the PRE-canonical path | `confine` has already resolved the link, so nothing is left to refuse |
//! | `file_type().is_file()` | a `read` on a FIFO named `checking.csv` blocks forever and hangs the request |
//! | glob confined to the final component | a pattern out of a file's contents drives a directory walk |
//! | `Read::take(MAX_PREVIEW_BYTES)` | a 40 GB bank export is read into memory to show three rows |
//! | drop the trailing partial line at the cap | a multi-byte character sliced at 64 KiB reports a good file as `NotUtf8` |
//! | `sanitize_display` per cell | a CSV cell is rendered verbatim into a GUI |
//! | name-only `data_label` | the preview becomes the path disclosure the warnings are careful not to be |
//!
//! Most cases are built in a scratch directory: a symlink, a FIFO and a 64 KiB
//! file are portability traps, and half of them cannot be committed to git at
//! all. The happy paths are driven from the committed corpus in `fixtures/rules/`
//! so the column labels asserted here are the ones a real hledger rules file and
//! its real data file actually produce.

mod common;

use ledgeline_core::rules::{Preview, PreviewUnavailable, discover};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Scratch-tree helpers (mirroring tests/rules_security.rs)
// ---------------------------------------------------------------------------

/// A private scratch directory for one test, emptied first so reruns are clean.
/// Named per-test (and per-process) so the suite stays parallel-safe.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ledgeline_rules_preview_{}_{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write `contents` to `dir/name` (creating parents) and return the path.
fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    write_bytes(dir, name, contents.as_bytes())
}

/// The same, for the cases whose whole point is that the bytes are not text.
fn write_bytes(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(&path, contents).expect("write fixture");
    path
}

/// Write the scan's anchor: the journal whose directory becomes the scan root.
fn main_journal(dir: &Path) -> PathBuf {
    write(dir, "main.journal", "2026-01-01 x\n    a  $1.00\n    b\n")
}

/// Preview `id` after a fresh scan of `main`'s directory. Every test goes
/// through the real entry point, so none of them can accidentally exercise a
/// path a client could not reach.
fn preview_of(main: &Path, id: &str) -> Preview {
    discover(main)
        .preview(id)
        .unwrap_or_else(|| panic!("{id} must resolve"))
}

/// The reason a preview is unavailable, asserting that it *is* unavailable and
/// that an unavailable preview shows nothing at all.
fn refusal(preview: &Preview) -> PreviewUnavailable {
    assert!(!preview.available, "expected a refusal: {preview:?}");
    assert_eq!(preview.header, None, "a refusal shows no header");
    assert!(preview.rows.is_empty(), "a refusal shows no rows");
    assert_eq!(preview.columns, 0);
    assert!(!preview.truncated);
    preview
        .reason
        .expect("an unavailable preview names a reason")
}

/// Copy one committed fixture into `dir` under the same name.
///
/// The `simple/` and `advanced/` fixture directories hold no journal of their
/// own, so there is nothing there to anchor a scan root on. Copying keeps the
/// bytes under test the **committed** ones — the header and column counts
/// asserted below are what a real hledger rules file and its real data file
/// produce — without adding a file to a corpus this step must not modify.
fn copy_fixture(dir: &Path, relative: &str) {
    let from = common::fixtures_dir().join("rules").join(relative);
    let name = Path::new(relative)
        .file_name()
        .expect("fixture has a file name");
    std::fs::copy(&from, dir.join(name)).unwrap_or_else(|e| panic!("copy {}: {e}", from.display()));
}

// ---------------------------------------------------------------------------
// The happy path, over the committed corpus
// ---------------------------------------------------------------------------

#[test]
fn the_committed_tree_fixture_previews_its_real_data_file() {
    // The one fixture that is end-to-end committed: a journal, a rules file two
    // directories below it, and the data file beside that. Nothing is copied or
    // synthesized, so this is the closest thing the suite has to a real user's
    // directory.
    let main = common::fixtures_dir().join("rules/tree/main.journal");
    let preview = preview_of(&main, "import/2026/bank.csv.rules");

    assert!(preview.available);
    assert_eq!(preview.reason, None);
    assert_eq!(
        preview.data_label.as_deref(),
        Some("bank.csv"),
        "the sibling is the rules name with `.rules` off — never `.csv.rules`"
    );
    assert_eq!(preview.separator, ',');
    assert_eq!(
        preview.header,
        Some(vec![
            "Date".to_string(),
            "Description".to_string(),
            "Amount".to_string()
        ]),
        "`skip 1` makes record 0 the header"
    );
    assert_eq!(
        preview.rows,
        vec![
            vec!["2026-01-03", "COFFEE HOUSE", "-6.45"],
            vec!["2026-01-05", "LANDLORD LLC", "-1850.00"],
        ]
    );
    assert_eq!(preview.columns, 3);
    assert!(!preview.truncated, "a 100-byte file is not capped");
}

#[test]
fn the_simple_checking_fixture_previews_its_real_header_and_three_rows() {
    let dir = scratch("checking");
    let main = main_journal(&dir);
    copy_fixture(&dir, "simple/checking.csv.rules");
    copy_fixture(&dir, "simple/checking.csv");

    let preview = preview_of(&main, "checking.csv.rules");
    assert!(preview.available);
    assert_eq!(preview.data_label.as_deref(), Some("checking.csv"));
    assert_eq!(preview.separator, ',');
    assert_eq!(
        preview.header,
        Some(vec![
            "Date".to_string(),
            "Description".to_string(),
            "Amount".to_string()
        ])
    );
    // The file has five data rows; MAX_PREVIEW_ROWS is 3, and the three are the
    // ones immediately after `skip`.
    assert_eq!(
        preview.rows,
        vec![
            vec!["01/15/2024", "ACME PAYROLL", "3000.00"],
            vec!["01/16/2024", "STARBUCKS #4417", "-6.45"],
            vec!["01/17/2024", "LANDLORD LLC", "-1850.00"],
        ],
        "three sample rows, with `skip 1` honoured"
    );
    assert_eq!(preview.columns, 3);
    assert!(!preview.truncated, "a sampled row count is not truncation");
}

#[test]
fn the_credit_card_and_mixed_fixtures_preview_four_columns() {
    // Two more shapes from the committed corpus: ISO dates with separate
    // debit/credit columns, and the file where every construct is deliberately
    // opaque — a preview must not care, because it reads the DATA file.
    let dir = scratch("four_columns");
    let main = main_journal(&dir);
    copy_fixture(&dir, "simple/creditcard1.csv.rules");
    copy_fixture(&dir, "simple/creditcard1.csv");
    copy_fixture(&dir, "advanced/mixed.csv.rules");
    copy_fixture(&dir, "advanced/mixed.csv");

    let card = preview_of(&main, "creditcard1.csv.rules");
    assert!(card.available);
    assert_eq!(
        card.header,
        Some(vec![
            "Date".to_string(),
            "Description".to_string(),
            "Debit".to_string(),
            "Credit".to_string()
        ])
    );
    assert_eq!(card.columns, 4);
    assert_eq!(
        card.rows.first().map(Vec::as_slice),
        Some(
            ["2024-01-15", "ANNUAL FEE", "0", "95.00"]
                .map(String::from)
                .as_slice()
        )
    );

    let mixed = preview_of(&main, "mixed.csv.rules");
    assert!(mixed.available);
    assert_eq!(
        mixed.header,
        Some(vec![
            "Date".to_string(),
            "Description".to_string(),
            "Amount".to_string(),
            "Note".to_string()
        ])
    );
    assert_eq!(mixed.columns, 4);
    assert_eq!(
        mixed.separator, ',',
        "`separator:,` is still a separator directive"
    );
    assert_eq!(mixed.rows.len(), 3);
}

#[test]
fn an_id_that_does_not_resolve_previews_nothing() {
    // `None` means exactly one thing — the id is not in this scan's set — so a
    // GUI can tell "no such rules file" apart from "no data file". Resolution is
    // the same exact string equality `resolve` uses, so a traversal id misses
    // here for the same reason it misses there.
    let dir = scratch("unknown_id");
    let main = main_journal(&dir);
    write(
        &dir,
        "a.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    write(&dir, "a.csv", "Date,Description,Amount\n2024-01-01,X,1\n");

    let found = discover(&main);
    assert!(found.preview("a.csv.rules").is_some());
    assert!(found.preview("nope.rules").is_none());
    assert!(found.preview("../a.csv.rules").is_none());
    assert!(found.preview("/etc/passwd").is_none());
    assert!(found.preview("").is_none());
}

// ---------------------------------------------------------------------------
// `source ... | CMD` — the refusal that matters most
// ---------------------------------------------------------------------------

#[test]
fn a_source_command_is_refused_and_nothing_is_executed() {
    // A `source` containing a `|` is a SHELL COMMAND hledger runs on import. A
    // rules file is a document the user downloaded or was sent, so running it
    // would make "look at my import rules" a remote-code-execution primitive.
    //
    // Asserting the reason is not enough — a wrong implementation could run the
    // command *and* return the right enum. The command therefore creates a
    // sentinel file, and the assertion is that the sentinel does not exist.
    let dir = scratch("source_command");
    let main = main_journal(&dir);
    let sentinel = dir.join("PWNED");
    write(
        &dir,
        "evil.csv.rules",
        &format!(
            "source /bin/sh -c 'touch {}' |\nskip 1\nfields date, description, amount\n",
            sentinel.display()
        ),
    );
    // A perfectly good data file sits right beside it, so a preview that fell
    // back to the sibling instead of refusing would look like it worked.
    write(
        &dir,
        "evil.csv",
        "Date,Description,Amount\n2024-01-01,X,1\n",
    );

    let preview = preview_of(&main, "evil.csv.rules");
    assert_eq!(refusal(&preview), PreviewUnavailable::SourceIsCommand);
    assert_eq!(
        preview.data_label, None,
        "there is no file here to name; a command is not a path"
    );
    assert!(
        !sentinel.exists(),
        "THE assertion of this suite: the `source` command must never run"
    );
    // Give a wrongly-spawned child a moment to lose the race, then look again.
    std::thread::sleep(std::time::Duration::from_millis(250));
    assert!(!sentinel.exists(), "nothing may run the command, ever");
}

// ---------------------------------------------------------------------------
// Escaping the scan root
// ---------------------------------------------------------------------------

#[test]
fn a_bare_filename_source_is_refused_as_outside_the_root() {
    // hledger resolves a `source` with no directory part against `~/Downloads`.
    // That is outside the journal directory this whole feature is confined to,
    // so it is refused rather than quietly re-pointed at a sibling of the same
    // name — which would be Ledgeline previewing a file the rules file did not
    // name.
    let dir = scratch("bare_source");
    let main = main_journal(&dir);
    write(
        &dir,
        "bare.csv.rules",
        "source statements.csv\nskip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "statements.csv",
        "Date,Description,Amount\n2024-01-01,X,1\n",
    );

    let preview = preview_of(&main, "bare.csv.rules");
    assert_eq!(refusal(&preview), PreviewUnavailable::SourceOutsideRoot);
    assert_eq!(
        preview.data_label, None,
        "a refused source names no file, not even the one it would have meant"
    );
}

#[test]
fn a_traversing_source_is_refused_whether_or_not_the_target_exists() {
    // The reason must not change with the target's existence: a message that
    // says "not found" for one path outside the root and "not a regular file"
    // for another is a filesystem existence oracle, which is the SEC-6 failure
    // `parse.rs` documents for `include` diagnostics.
    let dir = scratch("traversal");
    let main = main_journal(&dir);
    let outside = dir
        .parent()
        .expect("scratch has a parent")
        .join(format!("ledgeline_preview_outside_{}", std::process::id()));
    std::fs::create_dir_all(&outside).expect("create out-of-tree dir");
    std::fs::write(outside.join("outside.csv"), "Date,Secret\n2024-01-01,x\n")
        .expect("write out-of-tree csv");

    for (name, source) in [
        ("real.csv.rules", "../outside.csv"),
        ("gone.csv.rules", "../not-there.csv"),
        ("etc.csv.rules", "/etc/passwd"),
        ("nodir.csv.rules", "/etc/definitely-not-here"),
        ("deep.csv.rules", "./a/../../outside.csv"),
    ] {
        write(
            &dir,
            name,
            &format!("source {source}\nskip 1\nfields date, description, amount\n"),
        );
        let preview = preview_of(&main, name);
        assert_eq!(
            refusal(&preview),
            PreviewUnavailable::SourceOutsideRoot,
            "`source {source}` must be refused for CONTAINMENT, not for existence"
        );
        assert_eq!(preview.data_label, None);
    }
}

// ---------------------------------------------------------------------------
// Things that are not regular files
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn a_symlinked_data_file_is_refused_rather_than_resolved() {
    // Same rule as the scan: a symlink is refused outright, not followed. The
    // link points at a real CSV *inside* the root, so containment cannot be what
    // refuses it — only looking at the link itself can.
    //
    // `parse::confine` canonicalizes, which resolves the link, so the file-type
    // test has to run on the path as CONSTRUCTED. If it ever runs on `confine`'s
    // output instead, this test previews `real.csv` and passes silently for the
    // wrong reason, which is why the row content is asserted absent too.
    let dir = scratch("symlink_data");
    let main = main_journal(&dir);
    write(
        &dir,
        "link.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    let real = write(
        &dir,
        "real.csv",
        "Date,Description,Amount\n2024-01-01,SECRET,1\n",
    );
    std::os::unix::fs::symlink(&real, dir.join("link.csv")).expect("symlink");

    let preview = preview_of(&main, "link.csv.rules");
    assert_eq!(refusal(&preview), PreviewUnavailable::NotRegularFile);
    assert!(
        !format!("{preview:?}").contains("SECRET"),
        "the link's target must not have been read: {preview:?}"
    );
}

#[test]
#[cfg(unix)]
fn a_fifo_data_file_is_refused_and_the_call_returns() {
    // THE reason the file-type check exists. A `read` on a FIFO with no writer
    // blocks forever, so a preview that treated one as a data file would not
    // fail the request — it would hang it, and hold the handler thread for good.
    //
    // The assertion that matters is therefore not "refused" but "returned at
    // all", so the call runs on its own thread against a wall clock.
    let dir = scratch("fifo_data");
    let main = main_journal(&dir);
    write(
        &dir,
        "pipe.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    let fifo = dir.join("pipe.csv");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !made {
        eprintln!("skipping: mkfifo is unavailable on this platform");
        return;
    }

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(preview_of(&main, "pipe.csv.rules").reason);
    });
    let reason = receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the preview must RETURN; a `read` on the FIFO would block forever");

    assert_eq!(reason, Some(PreviewUnavailable::NotRegularFile));
}

#[test]
fn a_directory_named_like_the_data_file_is_not_a_data_file() {
    let dir = scratch("dir_data");
    let main = main_journal(&dir);
    write(
        &dir,
        "d.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    std::fs::create_dir_all(dir.join("d.csv")).expect("create dir");

    let preview = preview_of(&main, "d.csv.rules");
    assert_eq!(refusal(&preview), PreviewUnavailable::NotRegularFile);
}

// ---------------------------------------------------------------------------
// Files that exist but cannot be previewed
// ---------------------------------------------------------------------------

#[test]
fn a_missing_data_file_says_so_and_names_what_it_looked_for() {
    // Inside the root, "not there" is a useful answer and discloses nothing: the
    // user owns this directory, and a file NAME is not a path.
    let dir = scratch("missing");
    let main = main_journal(&dir);
    write(
        &dir,
        "lonely.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );

    let preview = preview_of(&main, "lonely.csv.rules");
    assert_eq!(refusal(&preview), PreviewUnavailable::NoDataFile);
    assert_eq!(preview.data_label.as_deref(), Some("lonely.csv"));
}

#[test]
fn a_non_utf8_data_file_is_refused() {
    let dir = scratch("not_utf8");
    let main = main_journal(&dir);
    write(
        &dir,
        "latin1.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    // Latin-1 `é` in a description: enough to make the file undecodable.
    write_bytes(
        &dir,
        "latin1.csv",
        b"Date,Description,Amount\n2024-01-01,caf\xe9,1\n",
    );

    let preview = preview_of(&main, "latin1.csv.rules");
    assert_eq!(refusal(&preview), PreviewUnavailable::NotUtf8);
    assert_eq!(
        preview.data_label.as_deref(),
        Some("latin1.csv"),
        "a file we could not decode is still a file we can name"
    );
}

#[test]
fn an_empty_data_file_is_refused() {
    let dir = scratch("empty");
    let main = main_journal(&dir);
    write(
        &dir,
        "e.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    write(&dir, "e.csv", "");

    assert_eq!(
        refusal(&preview_of(&main, "e.csv.rules")),
        PreviewUnavailable::Empty
    );
}

#[test]
fn a_data_file_with_nothing_past_the_skip_is_refused_as_empty() {
    // `skip 3` over a two-record file leaves no header and no rows. Reporting
    // that as an empty preview beats reporting a working one with nothing in it.
    let dir = scratch("all_skipped");
    let main = main_journal(&dir);
    write(
        &dir,
        "s.csv.rules",
        "skip 3\nfields date, description, amount\n",
    );
    write(&dir, "s.csv", "junk\nmore junk\n");

    assert_eq!(
        refusal(&preview_of(&main, "s.csv.rules")),
        PreviewUnavailable::Empty
    );
}

#[test]
fn a_header_with_no_data_rows_still_previews() {
    // The header IS the column labels, which is the whole point of the feature.
    // A file that has one and no data yet must still label the mapping screen.
    let dir = scratch("header_only");
    let main = main_journal(&dir);
    write(
        &dir,
        "h.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    write(&dir, "h.csv", "Date,Description,Amount\n");

    let preview = preview_of(&main, "h.csv.rules");
    assert!(preview.available);
    assert_eq!(preview.header.as_ref().map(Vec::len), Some(3));
    assert!(preview.rows.is_empty());
    assert_eq!(preview.columns, 3);
}

#[test]
fn a_file_with_no_skip_has_no_header_and_still_previews_its_rows() {
    // `skip 0` means the file has no header row. Labelling the columns with
    // record 0's values would present data as names.
    let dir = scratch("no_skip");
    let main = main_journal(&dir);
    write(&dir, "n.csv.rules", "fields date, description, amount\n");
    write(&dir, "n.csv", "2024-01-01,COFFEE,-1\n2024-01-02,RENT,-2\n");

    let preview = preview_of(&main, "n.csv.rules");
    assert!(preview.available);
    assert_eq!(preview.header, None);
    assert_eq!(preview.rows.len(), 2);
    assert_eq!(preview.columns, 3);
}

// ---------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------

#[test]
fn an_over_size_data_file_previews_from_the_cap_and_says_so() {
    // A large CSV is the NORMAL case, not a suspicious one, so the read is
    // capped and the preview still works — unlike an over-size rules file, which
    // is refused. `truncated` is how the GUI knows it is looking at a prefix.
    let dir = scratch("oversize");
    let main = main_journal(&dir);
    write(
        &dir,
        "big.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );

    let mut csv = String::from("Date,Description,Amount\n");
    let mut n = 0u32;
    while csv.len() <= 64 * 1024 {
        csv.push_str(&format!("2024-01-01,ROW {n},{n}.00\n"));
        n += 1;
    }
    assert!(csv.len() > 64 * 1024, "the fixture must exceed the cap");
    write(&dir, "big.csv", &csv);

    let preview = preview_of(&main, "big.csv.rules");
    assert!(preview.available, "an over-size CSV still previews");
    assert!(preview.truncated, "and says it is a prefix");
    assert_eq!(
        preview.header.as_deref(),
        Some(
            ["Date", "Description", "Amount"]
                .map(String::from)
                .as_slice()
        ),
        "the preview is of the file's START, so the header is intact"
    );
    assert_eq!(preview.rows.len(), 3);
    assert_eq!(preview.rows[0][1], "ROW 0");
}

#[test]
fn a_capped_read_that_lands_mid_character_is_not_reported_as_invalid_utf8() {
    // The cap is a BYTE count, and a multi-byte character straddling it would
    // leave a lone continuation byte at the end of the buffer. Decoding that
    // would report a perfectly good UTF-8 file as `NotUtf8`, so the trailing
    // partial line is dropped first. The padding is sized to march the boundary
    // across every offset inside a 3-byte character.
    let dir = scratch("cap_boundary");
    let main = main_journal(&dir);
    write(
        &dir,
        "u.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );

    for pad in 0..6usize {
        let mut csv = String::from("Date,Description,Amount\n");
        csv.push_str(&"x".repeat(pad));
        while csv.len() < 80 * 1024 {
            // `€` is three bytes, so a byte cap lands inside one for two offsets
            // out of every three.
            csv.push_str("2024-01-01,CAFÉ €€€ MOKA,-1.00\n");
        }
        write(&dir, "u.csv", &csv);

        let preview = preview_of(&main, "u.csv.rules");
        assert!(
            preview.available,
            "padding {pad} must not turn a UTF-8 file into {:?}",
            preview.reason
        );
        assert!(preview.truncated);
        assert_eq!(preview.rows.len(), 3);
    }
}

#[test]
fn a_very_wide_record_is_capped_at_the_column_limit() {
    // MAX_PREVIEW_COLUMNS is 64. `columns` reports the width actually kept, so a
    // GUI laying out one control per index cannot be told to draw more controls
    // than there are cells.
    let dir = scratch("wide");
    let main = main_journal(&dir);
    write(
        &dir,
        "w.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    let header = (0..500)
        .map(|i| format!("c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let row = (0..500)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    write(&dir, "w.csv", &format!("{header}\n{row}\n"));

    let preview = preview_of(&main, "w.csv.rules");
    assert!(preview.available);
    assert_eq!(preview.columns, 64);
    assert_eq!(preview.header.as_ref().map(Vec::len), Some(64));
    assert_eq!(preview.rows[0].len(), 64);
}

// ---------------------------------------------------------------------------
// RFC 4180 — the reason this uses a real CSV reader
// ---------------------------------------------------------------------------

#[test]
fn quoted_fields_containing_the_separator_and_a_newline_both_survive() {
    // Exactly what a bank CSV has, and exactly what a hand-rolled splitter gets
    // wrong: a quoted comma is one field, not two, and a quoted newline is one
    // record, not two. Getting either wrong shifts every column label after it.
    let dir = scratch("quoting");
    let main = main_journal(&dir);
    write(
        &dir,
        "q.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "q.csv",
        "Date,Description,Amount\n\
         2024-01-01,\"ACME, INC.\",-10.00\n\
         2024-01-02,\"TWO\nLINES\",-20.00\n\
         2024-01-03,\"A \"\"quoted\"\" word\",-30.00\n",
    );

    let preview = preview_of(&main, "q.csv.rules");
    assert!(preview.available);
    assert_eq!(preview.columns, 3, "a quoted comma is not a column break");
    assert_eq!(
        preview.rows,
        vec![
            vec!["2024-01-01", "ACME, INC.", "-10.00"],
            // The embedded newline is one record, and `sanitize_display`
            // collapses it for display — a GUI cell is one line.
            vec!["2024-01-02", "TWO LINES", "-20.00"],
            vec!["2024-01-03", "A \"quoted\" word", "-30.00"],
        ]
    );
}

#[test]
fn a_ragged_record_previews_instead_of_failing() {
    // `flexible(true)`. A bank CSV with a short trailer row must still show the
    // user what is in it; refusing the whole preview over one ragged record
    // would be refusing to answer the question they actually asked.
    let dir = scratch("ragged");
    let main = main_journal(&dir);
    write(
        &dir,
        "r.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "r.csv",
        "Date,Description,Amount\n2024-01-01,A,-1.00\n2024-01-02\n2024-01-03,C,-3.00,EXTRA\n",
    );

    let preview = preview_of(&main, "r.csv.rules");
    assert!(preview.available);
    assert_eq!(preview.rows.len(), 3);
    assert_eq!(preview.rows[1], vec!["2024-01-02"]);
    assert_eq!(
        preview.columns, 4,
        "`columns` is the WIDEST record, so no cell is left without a column"
    );
}

// ---------------------------------------------------------------------------
// The delimiter
// ---------------------------------------------------------------------------

#[test]
fn the_separator_directive_beats_the_file_extension() {
    // The extension is a convention about how a file was named; the directive is
    // the user saying so about this very file, and it is what hledger obeys.
    let dir = scratch("separator_directive");
    let main = main_journal(&dir);
    write(
        &dir,
        "semi.csv.rules",
        "separator ;\nskip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "semi.csv",
        "Date;Description;Amount\n2024-01-01;COFFEE, LARGE;-1.00\n",
    );

    let preview = preview_of(&main, "semi.csv.rules");
    assert!(preview.available);
    assert_eq!(preview.separator, ';', "the directive wins over `.csv`");
    assert_eq!(preview.columns, 3);
    assert_eq!(
        preview.rows,
        vec![vec!["2024-01-01", "COFFEE, LARGE", "-1.00"]],
        "and the comma inside a field is just a comma"
    );
}

#[test]
fn the_tab_and_space_separator_words_are_honoured() {
    let dir = scratch("separator_words");
    let main = main_journal(&dir);
    write(
        &dir,
        "t.csv.rules",
        "separator TAB\nskip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "t.csv",
        "Date\tDescription\tAmount\n2024-01-01\tA B\t-1\n",
    );

    let preview = preview_of(&main, "t.csv.rules");
    assert!(preview.available);
    assert_eq!(
        preview.separator, '\t',
        "`TAB` is matched case-insensitively"
    );
    assert_eq!(preview.columns, 3);
    assert_eq!(preview.rows, vec![vec!["2024-01-01", "A B", "-1"]]);
}

#[test]
fn the_extension_picks_the_delimiter_when_the_rules_file_is_silent() {
    let dir = scratch("extensions");
    let main = main_journal(&dir);
    for (rules, data, contents, expected) in [
        (
            "a.ssv.rules",
            "a.ssv",
            "Date;Description\n2024-01-01;A\n",
            ';',
        ),
        (
            "b.tsv.rules",
            "b.tsv",
            "Date\tDescription\n2024-01-01\tB\n",
            '\t',
        ),
        (
            "c.csv.rules",
            "c.csv",
            "Date,Description\n2024-01-01,C\n",
            ',',
        ),
        // No extension at all falls back to a comma, which is what an
        // unlabelled data file overwhelmingly is.
        ("d.rules", "d", "Date,Description\n2024-01-01,D\n", ','),
    ] {
        write(&dir, rules, "skip 1\nfields date, description\n");
        write(&dir, data, contents);
        let preview = preview_of(&main, rules);
        assert!(preview.available, "{rules}: {:?}", preview.reason);
        assert_eq!(preview.separator, expected, "{rules}");
        assert_eq!(preview.columns, 2, "{rules}");
    }
}

// ---------------------------------------------------------------------------
// Globs
// ---------------------------------------------------------------------------

/// Stamp `path`'s modification time, so "newest wins" is decided by the test and
/// not by how fast the filesystem's clock ticks.
fn set_mtime(path: &Path, seconds_ago: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(seconds_ago);
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .expect("reopen for set_times");
    file.set_times(std::fs::FileTimes::new().set_modified(when))
        .expect("set mtime");
}

#[test]
fn a_glob_source_picks_the_newest_match() {
    // hledger reads the newest match of a `source` glob, so this does too —
    // otherwise the preview labels the mapping screen from last month's export.
    let dir = scratch("glob_newest");
    let main = main_journal(&dir);
    write(
        &dir,
        "bank.csv.rules",
        "source ./bank*.csv\nskip 1\nfields date, description, amount\n",
    );
    let old = write(
        &dir,
        "bank-2024-01.csv",
        "Date,Description,Amount\n2024-01-01,OLD,-1\n",
    );
    let new = write(
        &dir,
        "bank-2026-07.csv",
        "Date,Description,Amount\n2026-07-01,NEW,-2\n",
    );
    // A near-miss that must not match, and a dotfile that would otherwise win on
    // mtime the way a `.DS_Store` does in a real download directory.
    write(
        &dir,
        "statement.csv",
        "Date,Description,Amount\n2024-01-01,WRONG,-3\n",
    );
    let dotfile = write(
        &dir,
        ".bank-hidden.csv",
        "Date,Description,Amount\n2030-01-01,HIDDEN,-4\n",
    );
    set_mtime(&old, 86_400);
    set_mtime(&new, 60);
    set_mtime(&dotfile, 0);

    let preview = preview_of(&main, "bank.csv.rules");
    assert!(preview.available, "{:?}", preview.reason);
    assert_eq!(preview.data_label.as_deref(), Some("bank-2026-07.csv"));
    assert_eq!(preview.rows, vec![vec!["2026-07-01", "NEW", "-2"]]);

    // `?` matches exactly one character, so this pattern reaches neither file.
    write(
        &dir,
        "one.csv.rules",
        "source ./bank-?.csv\nskip 1\nfields date, description, amount\n",
    );
    assert_eq!(
        refusal(&preview_of(&main, "one.csv.rules")),
        PreviewUnavailable::NoDataFile
    );
}

#[test]
fn a_bare_glob_is_still_a_bare_filename() {
    // `source bank*.csv` has no path separator, so hledger resolves it against
    // `~/Downloads`. The bare-filename refusal is a containment rule and takes
    // precedence over glob support — a pattern is not an exemption from it.
    let dir = scratch("glob_bare");
    let main = main_journal(&dir);
    write(
        &dir,
        "bare.csv.rules",
        "source bank*.csv\nskip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "bank-1.csv",
        "Date,Description,Amount\n2024-01-01,X,-1\n",
    );

    assert_eq!(
        refusal(&preview_of(&main, "bare.csv.rules")),
        PreviewUnavailable::SourceOutsideRoot
    );
}

#[test]
fn a_glob_above_the_final_component_is_refused() {
    // Matching a directory name means WALKING to find the directory to look in,
    // driven by a pattern out of a file's contents. Refused rather than walked —
    // hledger allows it; we deliberately do not.
    let dir = scratch("glob_dir");
    let main = main_journal(&dir);
    write(
        &dir,
        "import/2026/bank.csv",
        "Date,Description,Amount\n2026-01-01,X,-1\n",
    );
    for (name, source) in [
        ("a.csv.rules", "./import/*/bank.csv"),
        ("b.csv.rules", "./import/202?/bank.csv"),
        ("c.csv.rules", "./*/2026/bank.csv"),
        ("d.csv.rules", "../*/bank.csv"),
    ] {
        write(
            &dir,
            name,
            &format!("source {source}\nskip 1\nfields date, description, amount\n"),
        );
        assert_eq!(
            refusal(&preview_of(&main, name)),
            PreviewUnavailable::SourceOutsideRoot,
            "`source {source}` globs above the final component"
        );
    }
}

#[test]
fn a_source_naming_a_subdirectory_file_is_read() {
    // The other half of the glob test: an ordinary relative `source` pointing
    // into a subdirectory is exactly what the `import/YYYY/` layout produces,
    // and must work.
    let dir = scratch("source_subdir");
    let main = main_journal(&dir);
    write(
        &dir,
        "bank.csv.rules",
        "source ./import/2026/bank.csv\nskip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "import/2026/bank.csv",
        "Date,Description,Amount\n2026-01-01,COFFEE,-6.45\n",
    );

    let preview = preview_of(&main, "bank.csv.rules");
    assert!(preview.available, "{:?}", preview.reason);
    assert_eq!(
        preview.data_label.as_deref(),
        Some("bank.csv"),
        "the NAME only — the `import/2026/` it lives in is not disclosed"
    );
    assert_eq!(preview.rows, vec![vec!["2026-01-01", "COFFEE", "-6.45"]]);
}

// ---------------------------------------------------------------------------
// Sanitization — these strings go straight into a GUI
// ---------------------------------------------------------------------------

#[test]
fn cells_with_control_characters_are_sanitized() {
    let dir = scratch("control_chars");
    let main = main_journal(&dir);
    write(
        &dir,
        "c.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "c.csv",
        "Date,Description,Amount\n2024-01-01,\u{7}BE\u{1b}LL\u{0},  spaced   out  \n",
    );

    let preview = preview_of(&main, "c.csv.rules");
    assert!(preview.available);
    let row = &preview.rows[0];
    assert_eq!(row[1], "BELL", "control characters are dropped");
    assert_eq!(
        row[2], "spaced out",
        "whitespace runs collapse and the ends are trimmed"
    );
    for cell in preview.header.iter().flatten().chain(row) {
        assert!(
            !cell.chars().any(|c| c.is_control()),
            "a control character reached a GUI cell: {cell:?}"
        );
    }
}

#[test]
fn a_very_long_cell_is_truncated_on_a_char_boundary() {
    // MAX_CELL_CHARS is 120, counted in `char`s. A byte-offset truncation would
    // panic on the first non-ASCII description, and a bank's is not ASCII.
    let dir = scratch("long_cell");
    let main = main_journal(&dir);
    write(
        &dir,
        "l.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    // 400 three-byte characters: every candidate byte offset is mid-character.
    let long = "é".repeat(400);
    write(
        &dir,
        "l.csv",
        &format!("Date,Description,Amount\n2024-01-01,{long},-1.00\n"),
    );

    let preview = preview_of(&main, "l.csv.rules");
    assert!(preview.available);
    let cell = &preview.rows[0][1];
    assert_eq!(cell.chars().count(), 120, "120 chars, ellipsis included");
    assert!(cell.ends_with('…'), "the truncation is visible: {cell}");
    assert!(
        cell.chars().take(119).all(|c| c == 'é'),
        "and it did not split a code point"
    );
}

// ---------------------------------------------------------------------------
// Disclosure
// ---------------------------------------------------------------------------

#[test]
fn no_field_of_any_preview_contains_a_path() {
    // A preview reaches the same user-facing dialog the scan's warnings do, and
    // is a better oracle than they are: it follows a path out of a FILE'S
    // CONTENTS, so a leak here can be steered by whoever wrote the rules file.
    let dir = scratch("disclosure");
    let main = main_journal(&dir);
    let outside = dir
        .parent()
        .expect("scratch has a parent")
        .join(format!("ledgeline_preview_secret_{}", std::process::id()));
    std::fs::create_dir_all(&outside).expect("create out-of-tree dir");
    std::fs::write(outside.join("secret.csv"), "a,b\n1,2\n").expect("write secret");

    // Every shape at once: a working preview, each refusal, and a data file
    // whose own CELLS contain the scratch path — a preview that echoed a cell
    // without sanitizing could otherwise be fed one.
    write(
        &dir,
        "ok.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "ok.csv",
        &format!("Date,Description,Amount\n2024-01-01,{},-1\n", dir.display()),
    );
    write(
        &dir,
        "missing.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "out.csv.rules",
        "source ../secret.csv\nskip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "cmd.csv.rules",
        "source cat /etc/passwd |\nskip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "bare.csv.rules",
        "source secret.csv\nskip 1\nfields date, description\n",
    );
    write(
        &dir,
        "deep.csv.rules",
        "source ./sub/deep.csv\nskip 1\nfields date, description\n",
    );
    write(&dir, "sub/deep.csv", "Date,Description\n2024-01-01,X\n");

    let found = discover(&main);
    assert!(!found.files.is_empty());
    let root = dir.canonicalize().expect("canonical scratch root");
    let outside_secret = outside.display().to_string();

    for file in &found.files {
        let preview = found.preview(&file.id).expect("a listed id resolves");

        // (1) Every string LEDGELINE produces. A cell is deliberately not in
        // this set — see `a_cell_that_is_itself_a_path_is_shown_but_never_a
        // _field_of_ours`: a description may legitimately contain a path, and
        // that is the user's own data coming back to them. What must never
        // happen is this module minting one. `ok.csv` holds the scratch path in
        // a cell precisely so the two cannot be confused for each other.
        let ours = preview.data_label.iter();
        for value in ours {
            assert!(
                !value.contains(std::path::MAIN_SEPARATOR),
                "{}: data_label carries a path separator: {value:?}",
                file.id
            );
            assert!(
                !value.contains(root.display().to_string().as_str()),
                "{}: data_label discloses the scan root: {value:?}",
                file.id
            );
        }
        // The reason is an enum, so it cannot carry a path at all — asserted
        // here so a future change to a string-carrying reason is caught.
        assert!(
            !format!("{:?}", preview.reason).contains(std::path::MAIN_SEPARATOR),
            "{}: the reason carries a path: {:?}",
            file.id,
            preview.reason
        );

        // (2) Nothing out of tree was ever READ. A cell may contain a path the
        // user typed; it may never contain one only this process could know,
        // because that would mean the out-of-tree file was opened.
        for cell in preview
            .header
            .iter()
            .flatten()
            .chain(preview.rows.iter().flatten())
        {
            assert!(
                !cell.contains(outside_secret.as_str()),
                "{}: a cell came from outside the root: {cell:?}",
                file.id
            );
            // (3) Whatever a cell holds, it is still safe to render.
            assert!(
                !cell.chars().any(|c| c.is_control()),
                "{}: an unsanitized cell reached a GUI: {cell:?}",
                file.id
            );
            assert!(cell.chars().count() <= 120, "{}: {cell:?}", file.id);
        }
    }

    // The out-of-tree file must not have been previewable at all.
    assert_eq!(
        found
            .preview("out.csv.rules")
            .and_then(|preview| preview.reason),
        Some(PreviewUnavailable::SourceOutsideRoot)
    );
}

#[test]
fn a_cell_that_is_itself_a_path_is_shown_but_never_a_field_of_ours() {
    // The converse of the test above, stated on its own: a data file may
    // legitimately contain a path in a description, and that is the user's own
    // data. What must never happen is Ledgeline PRODUCING one.
    let dir = scratch("path_cell");
    let main = main_journal(&dir);
    write(
        &dir,
        "p.csv.rules",
        "skip 1\nfields date, description, amount\n",
    );
    write(
        &dir,
        "p.csv",
        "Date,Description,Amount\n2024-01-01,PAYMENT FOR /Users/someone/x,-1\n",
    );

    let preview = preview_of(&main, "p.csv.rules");
    assert!(preview.available);
    assert_eq!(preview.rows[0][1], "PAYMENT FOR /Users/someone/x");
    assert_eq!(
        preview.data_label.as_deref(),
        Some("p.csv"),
        "our own field is still a bare name"
    );
}
