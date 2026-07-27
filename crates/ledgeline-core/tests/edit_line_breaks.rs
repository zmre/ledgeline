//! DL-1 regression: line addressing must agree with the parser across every
//! Unicode line break that is **not** `\n`.
//!
//! The parser numbers lines with `str::lines()`, which splits on `\n` only (and
//! strips a paired `\r`). The editor used to convert those line numbers to
//! buffer offsets with ropey's line index, which ADDITIONALLY treats `U+000B`
//! VT, `U+000C` FF, a lone `U+000D` CR, `U+0085` NEL, `U+2028` LS and `U+2029`
//! PS as line breaks. One such character anywhere in a file shifted every
//! following `source_span.line`, so every edit below it hit the wrong lines.
//!
//! The triggers are mundane: a form feed is the standard Emacs/ledger `^L` page
//! separator, and a bare `\r` is routine CSV-import residue.
//!
//! Each case here builds the three-transaction journal from the code review,
//! deletes the middle transaction, and requires the other two to survive
//! byte-identically. Before the fix, deleting `B` destroyed a posting belonging
//! to `A` and left a journal `hledger` refuses to load — with no error from the
//! write path, because the reparse guard only counted transactions.
//!
//! Set `LEDGELINE_DL1_DUMP=<dir>` to also write each case's before/after journal
//! there for an out-of-band `hledger -f … print` check.

use ledgeline_core::edit::JournalEditor;
use ledgeline_core::model::Tindex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique scratch directory that removes itself on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("ledgeline-dl1-{name}-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every non-LF character ropey treats as a line break, named for the failure
/// message. `\r` is written un-paired: a `\r\n` pair is ONE break to both ropey
/// and `str::lines()`, so only a lone `\r` diverges.
const BREAKS: &[(&str, &str)] = &[
    ("form_feed", "\u{000C}"),
    ("lone_cr", "\r"),
    ("vertical_tab", "\u{000B}"),
    ("next_line", "\u{0085}"),
    ("line_separator", "\u{2028}"),
    ("paragraph_separator", "\u{2029}"),
];

/// The review's journal, with `break_char` embedded in the leading comment —
/// exactly where an Emacs `^L` page separator or CSV-import `\r` lands. The
/// transactions are deliberately not blank-line separated, as in the report.
fn journal_with(break_char: &str) -> String {
    format!(
        "; page{break_char}marker\n\
         2026-01-01 A\n    \
         expenses:a  $1.00\n    \
         assets:cash\n\
         2026-01-02 B\n    \
         expenses:b  $2.00\n    \
         assets:cash\n\
         2026-01-03 C\n    \
         expenses:c  $3.00\n    \
         assets:cash\n"
    )
}

/// The same journal with the middle transaction removed — the only correct
/// result of deleting `B`.
fn expected_after(break_char: &str) -> String {
    format!(
        "; page{break_char}marker\n\
         2026-01-01 A\n    \
         expenses:a  $1.00\n    \
         assets:cash\n\
         2026-01-03 C\n    \
         expenses:c  $3.00\n    \
         assets:cash\n"
    )
}

/// Write `before`/`after` into `$LEDGELINE_DL1_DUMP` when set, so the fixture
/// can be fed to a real `hledger` outside the test process.
fn dump(name: &str, before: &str, after: &str) {
    let Ok(dir) = std::env::var("LEDGELINE_DL1_DUMP") else {
        return;
    };
    let dir = Path::new(&dir);
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join(format!("{name}-before.journal")), before);
    let _ = std::fs::write(dir.join(format!("{name}-after.journal")), after);
}

/// Deleting the middle transaction must remove exactly that transaction, for
/// every non-LF line break the file might contain.
#[test]
fn delete_addresses_the_right_lines_across_unicode_line_breaks() {
    // Every case is exercised before anything is asserted, so one failure does
    // not hide the other five (and every fixture reaches the dump directory).
    let mut problems: Vec<String> = Vec::new();
    for (name, break_char) in BREAKS {
        let scratch = Scratch::new(name);
        let path = scratch.path("main.journal");
        let before = journal_with(break_char);
        std::fs::write(&path, &before).expect("write journal");

        let mut editor = JournalEditor::open(&path).expect("open journal");
        assert_eq!(editor.transaction_count(), 3, "{name}: three transactions");

        // `B` is the second transaction in file order.
        let b = editor
            .journal()
            .transactions
            .iter()
            .find(|t| t.description == "B")
            .map(|t| t.index)
            .expect("transaction B present");
        assert_eq!(b, Tindex(2), "{name}: B is the second transaction");

        let a_before = editor
            .transaction_source(Tindex(1))
            .expect("A's source text");
        let c_before = editor
            .transaction_source(Tindex(3))
            .expect("C's source text");

        editor.delete_transaction(b).expect("delete B");
        editor.save().expect("save");

        let after = std::fs::read_to_string(&path).expect("read back");
        dump(name, &before, &after);

        if after != expected_after(break_char) {
            problems.push(format!(
                "{name}: deleting B removed the wrong lines\n  expected: {:?}\n  actual:   {after:?}",
                expected_after(break_char)
            ));
        }
        // The survivors keep their exact source text — the posting that used to
        // be destroyed belonged to `A`.
        if editor.transaction_source(Tindex(1)).as_deref() != Some(a_before.as_str()) {
            problems.push(format!("{name}: A's source text changed"));
        }
        if editor.transaction_source(Tindex(2)).as_deref() != Some(c_before.as_str()) {
            problems.push(format!("{name}: C's source text changed"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The same addressing bug reached every other edit. A header rewrite below a
/// form feed used to overwrite an unrelated line.
#[test]
fn header_rewrite_addresses_the_right_line_across_a_form_feed() {
    let scratch = Scratch::new("set-status");
    let path = scratch.path("main.journal");
    std::fs::write(&path, journal_with("\u{000C}")).expect("write journal");

    let mut editor = JournalEditor::open(&path).expect("open journal");
    editor
        .set_status(Tindex(3), ledgeline_core::model::Status::Cleared)
        .expect("mark C cleared");

    assert_eq!(
        editor.text(),
        "; page\u{000C}marker\n\
         2026-01-01 A\n    \
         expenses:a  $1.00\n    \
         assets:cash\n\
         2026-01-02 B\n    \
         expenses:b  $2.00\n    \
         assets:cash\n\
         2026-01-03 * C\n    \
         expenses:c  $3.00\n    \
         assets:cash\n",
        "only C's header line may change"
    );
}
