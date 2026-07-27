//! Regression tests for the `include` guards (CLEANUP.md SEC-4 and SEC-6).
//!
//! SEC-4: an `include` cycle recursed forever and overflowed the stack, which is
//! a `SIGABRT` — not a catchable panic. Because the live-reload watcher reparses
//! on any write into a watched directory, any local process could kill the app
//! by appending one line to a journal.
//!
//! SEC-6: `include` accepted absolute paths, `..` traversal and symlinks out of
//! the tree. Whatever the target failed to parse as was quoted back in the parse
//! error (which reaches stderr and the GUI error dialog), making a hostile
//! journal a one-line-at-a-time local file-read oracle; anything that *did*
//! parse was absorbed into the journal and served over HTTP.
//!
//! Each case below was compared against real `hledger 1.52` (mac-aarch64). Where
//! we deliberately differ, the test says so and why.
//!
//! | case                          | hledger 1.52                 | ledgeline |
//! |-------------------------------|------------------------------|-----------|
//! | self-include                  | accepted, silently ignored   | rejected (divergence) |
//! | A -> B -> A / A -> B -> C -> A | "forms a cycle" error       | rejected (parity) |
//! | diamond (D via B and via C)   | accepted, parsed twice       | accepted, parsed twice (parity) |
//! | same file twice, one parent   | accepted, parsed twice       | accepted, parsed twice (parity) |
//! | chain 100 deep                | accepted                     | rejected past 20 (divergence) |
//! | absolute / `..` / symlink out | accepted, reads the file     | rejected (divergence) |
//! | fan-out bomb, depth 19        | hangs (>60s)                 | rejected in ~30ms (divergence) |
//! | missing include target        | error                        | error (parity) |

use ledgeline_core::parse::parse_journal_with_overrides;
use ledgeline_core::parse_journal;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A private scratch directory for one test, emptied first so reruns are clean.
/// Named per-test (and per-process) so the suite stays parallel-safe.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ledgeline_include_security_{}_{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write `contents` to `dir/name` and return the path.
fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(&path, contents).expect("write fixture");
    path
}

/// Parse `main` (read from disk) and return the error message, asserting it failed.
fn parse_err(main: &Path) -> String {
    let text = std::fs::read_to_string(main).expect("read main journal");
    match parse_journal(&text, &main.to_string_lossy()) {
        Ok(journal) => panic!(
            "expected {} to be rejected, but it parsed {} transactions",
            main.display(),
            journal.transactions.len()
        ),
        Err(e) => e.to_string(),
    }
}

/// Parse `main` (read from disk), asserting it succeeded.
fn parse_ok(main: &Path) -> ledgeline_core::Journal {
    let text = std::fs::read_to_string(main).expect("read main journal");
    parse_journal(&text, &main.to_string_lossy())
        .unwrap_or_else(|e| panic!("expected {} to parse, got: {e}", main.display()))
}

const TXN: &str = "2026-01-01 txn\n    expenses:a   $1.00\n    assets:b\n";

// ---------------------------------------------------------------------------
// SEC-4: cycles
// ---------------------------------------------------------------------------

#[test]
fn self_include_is_rejected_instead_of_overflowing_the_stack() {
    // The CLEANUP.md repro verbatim: one file that includes itself. Before the
    // fix this recursed until `fatal runtime error: stack overflow, aborting`.
    //
    // DIVERGENCE: hledger 1.52 accepts this, silently ignoring the self-include
    // — inconsistent with the hard error it raises for any longer cycle. We
    // reject it: silently dropping a directive hides a real authoring mistake,
    // and a uniform rule is the one that is easy to reason about for a guard
    // whose failure mode is an uncatchable process abort.
    let dir = scratch("self");
    let main = write(&dir, "loop.journal", "include loop.journal\n");
    let err = parse_err(&main);
    assert!(err.contains("forms a cycle"), "{err}");
    assert!(err.contains("loop.journal"), "{err}");
}

#[test]
fn self_include_is_rejected_even_below_the_main_file() {
    // The cycle check must use the whole ancestor chain, not just the main file.
    let dir = scratch("self_nested");
    write(&dir, "mid.journal", "include mid.journal\n");
    let main = write(&dir, "main.journal", "include mid.journal\n");
    let err = parse_err(&main);
    assert!(err.contains("forms a cycle"), "{err}");
    assert!(err.contains("mid.journal"), "{err}");
}

#[test]
fn mutual_cycle_is_rejected_from_either_entry_point() {
    // A -> B -> A. hledger 1.52 agrees, from either entry point:
    //   "This included file forms a cycle: /private/tmp/.../a.journal"
    let dir = scratch("mutual");
    let a = write(&dir, "a.journal", "include b.journal\n");
    let b = write(&dir, "b.journal", "include a.journal\n");
    for entry in [&a, &b] {
        let err = parse_err(entry);
        assert!(err.contains("forms a cycle"), "{entry:?}: {err}");
    }
}

#[test]
fn three_file_cycle_is_rejected() {
    // A -> B -> C -> A; hledger 1.52 rejects this too.
    let dir = scratch("three_cycle");
    write(&dir, "y.journal", "include z.journal\n");
    write(&dir, "z.journal", "include x.journal\n");
    let x = write(&dir, "x.journal", "include y.journal\n");
    let err = parse_err(&x);
    assert!(err.contains("forms a cycle"), "{err}");
    // The error names the file that closes the loop, and the line that did it.
    assert!(err.contains("x.journal"), "{err}");
    assert!(err.contains("include x.journal"), "{err}");
}

#[test]
fn cycle_through_a_relative_path_spelling_is_still_caught() {
    // `./a.journal` and `sub/../a.journal` are the same file as `a.journal`;
    // detection is on the canonical path, not the spelling.
    let dir = scratch("spelling");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let a = write(&dir, "a.journal", "include ./sub/../b.journal\n");
    write(&dir, "b.journal", "include ./a.journal\n");
    let err = parse_err(&a);
    assert!(err.contains("forms a cycle"), "{err}");
}

// ---------------------------------------------------------------------------
// SEC-4: legitimate include shapes must keep working
// ---------------------------------------------------------------------------

#[test]
fn diamond_include_is_allowed_and_parses_the_shared_file_twice() {
    // top -> {b, c}, and both b and c -> d. `d` is NOT an ancestor of itself on
    // either branch, so this is legal. Verified against hledger 1.52, which
    // prints d's transaction TWICE — a stack-based cycle check reproduces that;
    // a global visited set would silently drop the second copy and change the
    // user's totals.
    let dir = scratch("diamond");
    write(&dir, "d.journal", TXN);
    write(&dir, "b.journal", "include d.journal\n");
    write(&dir, "c.journal", "include d.journal\n");
    let top = write(
        &dir,
        "top.journal",
        "include b.journal\ninclude c.journal\n",
    );
    let journal = parse_ok(&top);
    assert_eq!(
        journal.transactions.len(),
        2,
        "diamond include must parse the shared file once per branch, like hledger"
    );
    // ...but the watch set still lists each file once.
    assert_eq!(journal.source_files.len(), 4);
}

#[test]
fn same_file_included_twice_from_one_parent_is_allowed() {
    // hledger 1.52 parses the file twice here as well.
    let dir = scratch("twice");
    write(&dir, "sub.journal", TXN);
    let main = write(
        &dir,
        "main.journal",
        "include sub.journal\ninclude sub.journal\n",
    );
    assert_eq!(parse_ok(&main).transactions.len(), 2);
}

#[test]
fn a_chain_within_the_depth_cap_still_parses() {
    // 19 nested includes: under the cap, so unchanged behaviour.
    let dir = scratch("deep_ok");
    for i in 0..19 {
        write(
            &dir,
            &format!("c{i}.journal"),
            &format!("include c{}.journal\n", i + 1),
        );
    }
    write(&dir, "c19.journal", TXN);
    let main = dir.join("c0.journal");
    assert_eq!(parse_ok(&main).transactions.len(), 1);
}

#[test]
fn includes_into_subdirectories_still_parse() {
    // The confinement root is the MAIN journal's directory, so the common
    // `include 2026/jan.journal` layout is unaffected.
    let dir = scratch("subdir");
    write(&dir, "2026/jan.journal", TXN);
    write(&dir, "2026/all.journal", "include jan.journal\n");
    let main = write(&dir, "main.journal", "include 2026/all.journal\n");
    assert_eq!(parse_ok(&main).transactions.len(), 1);
}

// ---------------------------------------------------------------------------
// SEC-4: depth and fan-out budgets
// ---------------------------------------------------------------------------

#[test]
fn a_chain_past_the_depth_cap_is_rejected() {
    let dir = scratch("deep_bad");
    for i in 0..30 {
        write(
            &dir,
            &format!("c{i}.journal"),
            &format!("include c{}.journal\n", i + 1),
        );
    }
    write(&dir, "c30.journal", TXN);
    let err = parse_err(&dir.join("c0.journal"));
    assert!(
        err.contains("include nesting deeper than 20 levels"),
        "{err}"
    );
    // It stopped AT the cap, not after walking the whole chain.
    assert!(err.contains("c20.journal"), "{err}");
}

#[test]
fn an_include_bomb_fails_fast_instead_of_hanging() {
    // 30 files each including two children: 2^30 parses without a guard. Depth
    // detection trips first and aborts the whole parse, so it costs ~20 reads.
    let dir = scratch("bomb");
    for i in 0..30 {
        write(
            &dir,
            &format!("b{i}.journal"),
            &format!("include b{0}.journal\ninclude b{0}.journal\n", i + 1),
        );
    }
    write(&dir, "b30.journal", TXN);

    let start = std::time::Instant::now();
    let err = parse_err(&dir.join("b0.journal"));
    let elapsed = start.elapsed();
    assert!(err.contains("include nesting deeper than"), "{err}");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "include bomb took {elapsed:?}; it must fail fast"
    );
}

#[test]
fn a_wide_bomb_under_the_depth_cap_is_stopped_by_the_file_budget() {
    // Depth 19 stays under the depth cap but still fans out to 2^19 (~524k)
    // parses. hledger 1.52 does not finish this in 60 seconds; the total-file
    // budget stops it in milliseconds.
    let dir = scratch("wide_bomb");
    for i in 0..19 {
        write(
            &dir,
            &format!("b{i}.journal"),
            &format!("include b{0}.journal\ninclude b{0}.journal\n", i + 1),
        );
    }
    write(&dir, "b19.journal", TXN);

    let start = std::time::Instant::now();
    let err = parse_err(&dir.join("b0.journal"));
    let elapsed = start.elapsed();
    assert!(err.contains("more than 1000 included files"), "{err}");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "wide include bomb took {elapsed:?}; it must fail fast"
    );
}

// ---------------------------------------------------------------------------
// SEC-6: directory confinement
// ---------------------------------------------------------------------------

/// The marker line every escape fixture below points at. If it ever shows up in
/// an error message, the read oracle is back.
const SECRET_LINE: &str = "this-line-must-never-be-echoed";

/// Build an out-of-tree file containing [`SECRET_LINE`] as unparseable content,
/// and return its path. It lives in a SIBLING of the journal directory, so no
/// containment check can be satisfied by accident.
fn out_of_tree_secret(dir: &Path) -> PathBuf {
    let outside = dir
        .parent()
        .expect("scratch dir has a parent")
        .join(format!(
            "ledgeline_include_security_outside_{}",
            std::process::id()
        ));
    std::fs::create_dir_all(&outside).expect("create out-of-tree dir");
    let path = outside.join("secret.journal");
    std::fs::write(&path, format!("{SECRET_LINE}\n")).expect("write secret");
    path
}

/// Assert `err` is a confinement rejection that discloses nothing about the
/// target file: it must name the path AS WRITTEN and never quote the contents.
fn assert_confined(err: &str, as_written: &str) {
    assert!(
        err.contains("resolves outside the journal directory"),
        "expected a confinement error, got: {err}"
    );
    assert!(
        err.contains(as_written),
        "error should quote the include as written: {err}"
    );
    assert!(
        !err.contains(SECRET_LINE),
        "SEC-6 regression: the target file's contents leaked into the error: {err}"
    );
}

#[test]
fn absolute_path_escape_is_rejected_without_echoing_the_file() {
    // CLEANUP.md's `include /etc/passwd`, made hermetic. Before the fix this
    // echoed a line of the target back to stderr and the GUI error dialog.
    // DIVERGENCE: hledger 1.52 follows absolute includes anywhere.
    let dir = scratch("abs_escape");
    let secret = out_of_tree_secret(&dir);
    let main = write(
        &dir,
        "main.journal",
        &format!("include {}\n", secret.display()),
    );
    let err = parse_err(&main);
    assert_confined(&err, &secret.display().to_string());
}

#[test]
fn dotdot_escape_is_rejected_without_echoing_the_file() {
    // CLEANUP.md's `include ../../../etc/hosts`, made hermetic.
    let dir = scratch("dotdot_escape");
    let secret = out_of_tree_secret(&dir);
    let relative = format!(
        "../{}/secret.journal",
        secret
            .parent()
            .and_then(Path::file_name)
            .expect("out-of-tree dir name")
            .to_string_lossy()
    );
    let main = write(&dir, "main.journal", &format!("include {relative}\n"));
    let err = parse_err(&main);
    assert_confined(&err, &relative);
}

#[test]
#[cfg(unix)]
fn symlink_escape_is_rejected_without_echoing_the_file() {
    // A symlink inside the tree pointing out of it. Confinement is checked on
    // the CANONICAL path, so the link is resolved before the test — a lexical
    // check alone would wave this through. hledger 1.52 follows it.
    let dir = scratch("symlink_escape");
    let secret = out_of_tree_secret(&dir);
    let link = dir.join("link.journal");
    std::os::unix::fs::symlink(&secret, &link).expect("create symlink");
    let main = write(&dir, "main.journal", "include link.journal\n");
    let err = parse_err(&main);
    assert_confined(&err, "link.journal");
    // The symlink's TARGET is not disclosed either: only what the journal wrote.
    assert!(
        !err.contains(&secret.display().to_string()),
        "the symlink target path should not be disclosed: {err}"
    );
}

#[test]
fn an_escaping_include_is_rejected_even_when_its_contents_would_parse() {
    // The other half of SEC-6: a file that parses cleanly was silently absorbed
    // into the journal and then served over HTTP. Confinement must not depend on
    // the target failing to parse.
    let dir = scratch("parseable_escape");
    let outside = out_of_tree_secret(&dir);
    std::fs::write(&outside, TXN).expect("overwrite with valid journal content");
    let main = write(
        &dir,
        "main.journal",
        &format!("include {}\n", outside.display()),
    );
    let err = parse_err(&main);
    assert!(
        err.contains("resolves outside the journal directory"),
        "{err}"
    );
}

#[test]
fn confinement_also_applies_to_the_editor_override_reparse_path() {
    // `parse_journal_with_overrides` is the editor's reparse-to-validate entry
    // point; it must not be a way around the guard.
    let dir = scratch("overrides_escape");
    let secret = out_of_tree_secret(&dir);
    let main = write(
        &dir,
        "main.journal",
        &format!("include {}\n", secret.display()),
    );
    let overrides: HashMap<PathBuf, String> = HashMap::new();
    let err = parse_journal_with_overrides(&main.to_string_lossy(), &overrides)
        .expect_err("escaping include must be rejected on the overrides path too")
        .to_string();
    assert_confined(&err, &secret.display().to_string());
}

#[test]
fn cycles_are_caught_on_the_editor_override_reparse_path() {
    let dir = scratch("overrides_cycle");
    write(&dir, "b.journal", "include a.journal\n");
    let a = write(&dir, "a.journal", "include b.journal\n");
    let overrides: HashMap<PathBuf, String> = HashMap::new();
    let err = parse_journal_with_overrides(&a.to_string_lossy(), &overrides)
        .expect_err("cycle must be rejected on the overrides path too")
        .to_string();
    assert!(err.contains("forms a cycle"), "{err}");
}

// ---------------------------------------------------------------------------
// Unchanged behaviour
// ---------------------------------------------------------------------------

#[test]
fn a_missing_include_still_reports_a_read_error_not_a_confinement_error() {
    // A typo inside the tree must keep its old, useful diagnostic. hledger 1.52
    // likewise errors here ("No files were matched by: nope.journal").
    let dir = scratch("missing");
    let main = write(&dir, "main.journal", "include nope.journal\n");
    let err = parse_err(&main);
    assert!(
        !err.contains("resolves outside the journal directory"),
        "a missing sibling is not an escape: {err}"
    );
    assert!(err.contains("nope.journal"), "{err}");
}

#[test]
fn an_empty_include_target_is_still_a_malformed_directive() {
    let dir = scratch("empty_target");
    let main = write(&dir, "main.journal", "include\n");
    let err = parse_err(&main);
    assert!(err.contains("malformed directive"), "{err}");
}
