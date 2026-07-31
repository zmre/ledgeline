//! Regression tests for the import-rules **discovery** guards
//! ([`ledgeline_core::rules::discover`], Imports step 5).
//!
//! Discovery decides which files the imports feature may look at, and therefore
//! which files a later `PUT` may overwrite. Every test below names the guard it
//! exercises and what goes wrong without it.
//!
//! | guard | without it |
//! |---|---|
//! | `symlink_metadata`, symlinks skipped | a link in the journal directory reads (and later WRITES) anywhere on disk |
//! | `file_type().is_file()` | a `read` on a FIFO named `x.rules` blocks forever and hangs the request |
//! | dot-directory / `SKIP_DIRS` skip | `.git/` and `node_modules/` become "your import rules" |
//! | `MAX_RULES_DEPTH` | a deep tree costs an unbounded walk |
//! | `MAX_SCAN_ENTRIES` | a journal in `$HOME` turns one scan into a full-disk walk |
//! | `MAX_RULES_BYTES` | a mis-named gigabyte is read into memory and parsed |
//! | relative-only warnings | a user-facing dialog becomes a filesystem existence oracle |
//! | `resolve` = string equality | a client-supplied id becomes a path, which is what every traversal bug is made of |
//! | `RulesPath` has no public constructor | "only write what discovery returned" is a convention instead of a type |
//!
//! Everything is built at test time in a scratch directory, in the style of
//! `include_security.rs`: a committed symlink (or FIFO, or 0700 directory) is a
//! portability trap, and half of these cases cannot be committed to git at all.

mod common;

use ledgeline_core::rules::{Discovery, discover};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Scratch-tree helpers (mirroring tests/include_security.rs)
// ---------------------------------------------------------------------------

/// A private scratch directory for one test, emptied first so reruns are clean.
/// Named per-test (and per-process) so the suite stays parallel-safe.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ledgeline_rules_security_{}_{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write `contents` to `dir/name` (creating parents) and return the path.
fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(&path, contents).expect("write fixture");
    path
}

/// A minimal, valid rules file — enough that `parsed` is true and the summary
/// counts are non-zero.
const RULES: &str = "\
skip 1
fields date, description, amount
account1 assets:bank:checking

if COFFEE
    account2 expenses:food:coffee
";

/// Write the scan's anchor: the journal whose directory becomes the scan root.
fn main_journal(dir: &Path) -> PathBuf {
    write(dir, "main.journal", "2026-01-01 x\n    a  $1.00\n    b\n")
}

/// The ids a scan of `main`'s directory returns, in order.
fn ids(discovery: &Discovery) -> Vec<String> {
    discovery.files.iter().map(|file| file.id.clone()).collect()
}

/// Every warning string a discovery produced — scan-level and per-file.
fn all_warnings(discovery: &Discovery) -> Vec<String> {
    discovery
        .warnings
        .iter()
        .chain(discovery.files.iter().flat_map(|file| &file.warnings))
        .map(|warning| warning.message.clone())
        .collect()
}

/// A sibling of the scratch directory, so nothing inside the scan root can
/// contain it by accident. This is where the "must never be reachable" files go.
fn out_of_tree(dir: &Path, name: &str) -> PathBuf {
    let outside = dir
        .parent()
        .expect("scratch dir has a parent")
        .join(format!("ledgeline_rules_outside_{}", std::process::id()));
    std::fs::create_dir_all(&outside).expect("create out-of-tree dir");
    let path = outside.join(name);
    std::fs::write(&path, RULES).expect("write out-of-tree rules");
    path
}

// ---------------------------------------------------------------------------
// The happy path — everything below is a departure from this
// ---------------------------------------------------------------------------

#[test]
fn a_rules_file_in_the_tree_is_found_and_summarized() {
    let dir = scratch("happy");
    let main = main_journal(&dir);
    write(&dir, "checking.csv.rules", RULES);
    write(&dir, "import/2026/bank.rules", RULES);
    // A neighbour that is not a rules file, and one whose name is only the
    // suffix (a dotfile, which would strip to an empty label).
    write(&dir, "notes.txt", "x");
    write(&dir, ".rules", RULES);

    let found = discover(&main);
    assert!(!found.truncated);
    assert_eq!(
        ids(&found),
        vec!["checking.csv.rules", "import/2026/bank.rules"],
        "ids are relative, forward-slash, and sorted"
    );

    let checking = found.resolve("checking.csv.rules").expect("resolves");
    assert_eq!(checking.label, "checking", "`.csv.rules` strips as a unit");
    assert!(checking.parsed);
    assert_eq!(checking.size_bytes, RULES.len() as u64);
    assert_eq!(
        checking.account1.as_deref(),
        Some("assets:bank:checking"),
        "the summary comes from RulesDoc::settings"
    );
    assert_eq!(
        checking.account2, None,
        "a conditional account2 is not a setting"
    );
    assert_eq!(checking.if_block_count, 1);
    assert_eq!(checking.editable_block_count, 1);
    assert_eq!(checking.opaque_item_count, 0);
    assert!(checking.warnings.is_empty());
    assert!(
        checking.identity_unchanged(),
        "nothing moved between the scan and now"
    );
    assert_eq!(
        found
            .resolve("import/2026/bank.rules")
            .map(|f| f.label.clone()),
        Some("bank".to_string()),
        "a plain `.rules` strips too"
    );
}

#[test]
fn awkward_but_legal_file_names_are_handled_without_panicking() {
    // Names are attacker-chosen on this path — a rules file is discovered by its
    // NAME, and the suffix and label logic both slice by byte offset. A name
    // whose sixth-from-last byte is mid-code-point used to be a panic.
    let dir = scratch("names");
    let main = main_journal(&dir);
    for name in [
        "a€bcde",           // not a rules file, and the byte offset lands mid-`€`
        "café.rules",       // multi-byte before the suffix
        "日本語.csv.rules", // multi-byte throughout
        "UPPER.RULES",      // the suffix match is case-insensitive
        "spaced name.csv.rules",
        "-.rules",
    ] {
        write(&dir, name, RULES);
    }

    let found = discover(&main);
    assert_eq!(
        ids(&found),
        vec![
            "-.rules",
            "UPPER.RULES",
            "café.rules",
            "spaced name.csv.rules",
            "日本語.csv.rules",
        ],
        "a€bcde is not a rules file; everything else is"
    );
    assert_eq!(
        found.resolve("日本語.csv.rules").map(|f| f.label.clone()),
        Some("日本語".to_string())
    );
    assert_eq!(
        found.resolve("UPPER.RULES").map(|f| f.label.clone()),
        Some("UPPER".to_string())
    );
}

#[test]
fn the_revision_is_the_fingerprint_of_the_raw_bytes() {
    // The revision is what a later save uses as an `If-Match`, so it has to be
    // the fingerprint of the bytes on disk and nothing else.
    let dir = scratch("revision");
    let main = main_journal(&dir);
    write(&dir, "a.rules", RULES);

    let before = discover(&main);
    let revision = before.resolve("a.rules").expect("found").revision.clone();
    assert_eq!(
        revision,
        ledgeline_core::Fingerprint::of_bytes(RULES.as_bytes()).token()
    );

    write(&dir, "a.rules", &format!("{RULES}# one more line\n"));
    let after = discover(&main);
    assert_ne!(
        after.resolve("a.rules").expect("found").revision,
        revision,
        "an edited file must not keep its revision"
    );
}

#[test]
fn identity_unchanged_refuses_a_name_that_became_a_different_file() {
    // The scan proved the path was a regular file; that proof expires the moment
    // the scan ends. This is the check a write path runs immediately before
    // writing.
    let dir = scratch("identity");
    let main = main_journal(&dir);
    let path = write(&dir, "a.rules", RULES);

    let found = discover(&main);
    let file = found.resolve("a.rules").expect("found");
    assert!(file.identity_unchanged());

    // Replaced, not edited: same name, new inode.
    std::fs::remove_file(&path).expect("remove");
    write(&dir, "a.rules", RULES);
    #[cfg(unix)]
    assert!(
        !file.identity_unchanged(),
        "a new inode behind the same name must not pass"
    );

    // ...and a name that became a symlink fails everywhere, inode or not.
    std::fs::remove_file(&path).expect("remove again");
    #[cfg(unix)]
    {
        let target = write(&dir, "real.rules", RULES);
        std::os::unix::fs::symlink(&target, &path).expect("symlink");
        assert!(
            !file.identity_unchanged(),
            "a symlink is not a regular file"
        );
    }
}

#[test]
fn root_label_is_the_directory_name_and_not_the_path() {
    let dir = scratch("root_label");
    let main = main_journal(&dir);
    let found = discover(&main);
    let label = found.root_label();
    assert!(
        label.starts_with("ledgeline_rules_security_"),
        "expected the final component, got {label}"
    );
    assert!(!label.contains('/'), "a heading is not a path: {label}");
}

// ---------------------------------------------------------------------------
// Symlinks — refused outright, unlike `include`
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn a_rules_file_outside_the_root_reached_by_a_symlink_is_not_found() {
    // The escape that matters: a link inside the tree pointing out of it. A
    // later PUT resolving this id would write outside the journal directory.
    let dir = scratch("symlink_out");
    let main = main_journal(&dir);
    let secret = out_of_tree(&dir, "secret.rules");
    std::os::unix::fs::symlink(&secret, dir.join("link.rules")).expect("symlink");

    let found = discover(&main);
    assert!(found.files.is_empty(), "{:?}", ids(&found));
    assert!(found.resolve("link.rules").is_none());

    let warnings = all_warnings(&found);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("link.rules") && w.contains("symbolic link")),
        "the skip must be reported: {warnings:?}"
    );
    assert!(
        !warnings
            .iter()
            .any(|w| w.contains(&secret.display().to_string())),
        "the link's target must not be disclosed: {warnings:?}"
    );
}

#[test]
#[cfg(unix)]
fn a_symlinked_directory_inside_the_root_is_not_descended() {
    // A link that stays *inside* the tree is refused too. Following it would
    // list the same file under two ids and reintroduce cycles and depth games.
    let dir = scratch("symlink_dir");
    let main = main_journal(&dir);
    write(&dir, "real/inner.rules", RULES);
    std::os::unix::fs::symlink(dir.join("real"), dir.join("mirror")).expect("symlink dir");

    let found = discover(&main);
    assert_eq!(
        ids(&found),
        vec!["real/inner.rules"],
        "the linked directory must not produce a second copy"
    );
    assert!(
        all_warnings(&found).iter().any(|w| w.contains("mirror")),
        "the skipped link is reported"
    );
}

#[test]
#[cfg(unix)]
fn a_symlinked_directory_pointing_out_of_the_root_is_not_descended() {
    let dir = scratch("symlink_dir_out");
    let main = main_journal(&dir);
    let secret = out_of_tree(&dir, "secret.rules");
    let outside = secret.parent().expect("out-of-tree dir");
    std::os::unix::fs::symlink(outside, dir.join("elsewhere")).expect("symlink dir");

    let found = discover(&main);
    assert!(found.files.is_empty(), "{:?}", ids(&found));
}

// ---------------------------------------------------------------------------
// Directories that are never scanned
// ---------------------------------------------------------------------------

#[test]
fn dot_directories_and_skip_dirs_are_not_scanned() {
    let dir = scratch("skips");
    let main = main_journal(&dir);
    write(&dir, ".git/hidden.rules", RULES);
    write(&dir, ".config/also-hidden.rules", RULES);
    write(&dir, "node_modules/dep.rules", RULES);
    write(&dir, "target/build.rules", RULES);
    write(&dir, "keep.rules", RULES);

    let found = discover(&main);
    assert_eq!(ids(&found), vec!["keep.rules"]);
    assert!(found.resolve(".git/hidden.rules").is_none());
    assert!(found.resolve("node_modules/dep.rules").is_none());
    assert!(
        found.warnings.is_empty(),
        "a policy skip is not a problem to report: {:?}",
        found.warnings
    );
}

#[test]
fn nesting_past_the_depth_cap_is_not_found() {
    // MAX_RULES_DEPTH is 8, counting the root as 0. `a/b/.../h/deep.rules` is
    // the deepest admitted; one more level is not.
    let dir = scratch("depth");
    let main = main_journal(&dir);
    let at_cap = "a/b/c/d/e/f/g/h";
    write(&dir, &format!("{at_cap}/deep.rules"), RULES);
    write(&dir, &format!("{at_cap}/i/too-deep.rules"), RULES);

    let found = discover(&main);
    assert_eq!(ids(&found), vec![format!("{at_cap}/deep.rules")]);
    assert!(
        found
            .resolve(&format!("{at_cap}/i/too-deep.rules"))
            .is_none()
    );
    assert!(
        found.truncated,
        "a depth skip means the list is incomplete, and the user must be told"
    );
}

// ---------------------------------------------------------------------------
// Caps
// ---------------------------------------------------------------------------

#[test]
fn the_entry_cap_stops_the_walk_and_reports_truncation() {
    // MAX_SCAN_ENTRIES is the bound that stops a journal in `$HOME` from turning
    // one scan into a full-disk walk. 20,050 plain files exhaust it before the
    // (alphabetically last) rules file is ever examined.
    let dir = scratch("entry_cap");
    let main = main_journal(&dir);
    for i in 0..20_050u32 {
        std::fs::write(dir.join(format!("f{i:05}.txt")), "").expect("write filler");
    }
    write(&dir, "zz-late.rules", RULES);

    let start = std::time::Instant::now();
    let found = discover(&main);
    let elapsed = start.elapsed();

    assert!(found.truncated, "the cap must be surfaced, not hidden");
    assert!(
        found.resolve("zz-late.rules").is_none(),
        "the walk stopped at the cap"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "the scan took {elapsed:?}; the entry cap must bound the work"
    );
}

#[test]
fn the_file_cap_returns_at_most_max_rules_files() {
    // MAX_RULES_FILES is 200. 201 files means a truncated list of exactly 200.
    let dir = scratch("file_cap");
    let main = main_journal(&dir);
    for i in 0..201u32 {
        write(&dir, &format!("r{i:04}.rules"), RULES);
    }
    let found = discover(&main);
    assert_eq!(found.files.len(), 200);
    assert!(found.truncated);
}

#[test]
fn an_over_size_file_is_listed_but_not_parsed() {
    // MAX_RULES_BYTES is 1 MiB. Listing it (rather than hiding it) is the point:
    // a file the user can see and cannot edit beats a file that is silently
    // absent from their own directory.
    let dir = scratch("oversize");
    let main = main_journal(&dir);
    let huge = vec![b'#'; (1 << 20) + 1];
    std::fs::write(dir.join("huge.rules"), &huge).expect("write huge");

    let found = discover(&main);
    let file = found.resolve("huge.rules").expect("still listed");
    assert!(!file.parsed);
    assert_eq!(file.size_bytes, huge.len() as u64);
    assert_eq!(file.if_block_count, 0);
    assert_eq!(file.account1, None);
    assert!(
        file.warnings
            .iter()
            .any(|w| w.message.contains("larger than")),
        "{:?}",
        file.warnings
    );
    assert_ne!(
        file.revision,
        ledgeline_core::Fingerprint::of_bytes(&huge).token(),
        "an unread file must not claim a content revision"
    );
}

#[test]
fn a_non_utf8_file_is_listed_but_not_parsed() {
    let dir = scratch("not_utf8");
    let main = main_journal(&dir);
    // Latin-1 `£` in a comment: enough to make the file undecodable.
    let bytes = b"# caf\xe9 rules\nfields date, description, amount\n";
    std::fs::write(dir.join("latin1.rules"), bytes).expect("write latin1");

    let found = discover(&main);
    let file = found.resolve("latin1.rules").expect("still listed");
    assert!(!file.parsed);
    assert!(
        file.warnings
            .iter()
            .any(|w| w.message.contains("not valid UTF-8")),
        "{:?}",
        file.warnings
    );
    assert_eq!(
        file.revision,
        ledgeline_core::Fingerprint::of_bytes(bytes).token(),
        "the fingerprint is over the RAW bytes, so it covers a file we cannot decode"
    );
}

// ---------------------------------------------------------------------------
// Things that are not regular files
// ---------------------------------------------------------------------------

#[test]
fn a_directory_named_like_a_rules_file_is_not_listed() {
    let dir = scratch("dir_named_rules");
    let main = main_journal(&dir);
    std::fs::create_dir_all(dir.join("x.rules")).expect("create dir");
    write(&dir, "x.rules/inner.rules", RULES);

    let found = discover(&main);
    assert_eq!(
        ids(&found),
        vec!["x.rules/inner.rules"],
        "the directory is descended into, never listed as a file"
    );
    assert!(found.resolve("x.rules").is_none());
}

#[test]
#[cfg(unix)]
fn a_fifo_named_like_a_rules_file_is_skipped_and_the_scan_returns() {
    // THE reason step 3 requires `is_file()`. A `read` on a FIFO with no writer
    // blocks forever, so a scan that treated one as a rules file would not fail
    // the request — it would hang it, and hold the handler thread for good.
    //
    // The assertion that matters is therefore not "not found" but "returned at
    // all", so the scan runs on its own thread against a wall clock.
    let dir = scratch("fifo");
    let main = main_journal(&dir);
    let fifo = dir.join("x.rules");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !made {
        eprintln!("skipping: mkfifo is unavailable on this platform");
        return;
    }
    write(&dir, "real.rules", RULES);

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let found = discover(&main);
        let _ = sender.send(ids(&found));
    });
    let found = receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the scan must RETURN; a `read` on the FIFO would block forever");

    assert_eq!(found, vec!["real.rules"], "the FIFO must not be listed");
}

// ---------------------------------------------------------------------------
// Resolution is string equality, never path arithmetic
// ---------------------------------------------------------------------------

#[test]
fn resolve_rejects_traversal_absolute_and_wrong_case_ids() {
    let dir = scratch("resolve");
    let main = main_journal(&dir);
    let secret = out_of_tree(&dir, "escape.rules");
    write(&dir, "checking.rules", RULES);

    let found = discover(&main);
    assert!(found.resolve("checking.rules").is_some());

    // Traversal: `root.join(id)` would reach the sibling directory. String
    // equality cannot, because no scan ever produces an id with a `..`.
    assert!(
        found
            .resolve("../ledgeline_rules_outside/escape.rules")
            .is_none()
    );
    assert!(found.resolve("./checking.rules").is_none());
    assert!(found.resolve("import/../checking.rules").is_none());
    assert!(found.resolve(&format!("../{}", secret.display())).is_none());

    // Absolute paths, in both spellings.
    assert!(found.resolve("/etc/passwd").is_none());
    assert!(found.resolve(&secret.display().to_string()).is_none());
    assert!(
        found
            .resolve(&dir.join("checking.rules").display().to_string())
            .is_none()
    );

    // Wrong case. On a case-INSENSITIVE filesystem (macOS's default) this id
    // names a real, openable file, and it still misses. That is deliberate and
    // is not a bug to "fix": a case-folding lookup would be path arithmetic
    // wearing a different hat.
    assert!(
        found.resolve("Checking.rules").is_none(),
        "resolution is exact string equality"
    );
    assert!(found.resolve("CHECKING.RULES").is_none());
    assert!(found.resolve("").is_none());
}

#[test]
fn a_stale_id_from_an_earlier_scan_simply_misses() {
    // The contract is scan-resolve-write inside one request. A file that is gone
    // must not resolve against a fresh scan.
    let dir = scratch("stale");
    let main = main_journal(&dir);
    write(&dir, "gone.rules", RULES);
    assert!(discover(&main).resolve("gone.rules").is_some());

    std::fs::remove_file(dir.join("gone.rules")).expect("remove");
    assert!(discover(&main).resolve("gone.rules").is_none());
}

// ---------------------------------------------------------------------------
// Disclosure
// ---------------------------------------------------------------------------

#[test]
fn no_warning_discloses_an_absolute_path() {
    // These strings reach a user-facing dialog verbatim. A dialog that echoes a
    // resolved path is a filesystem existence oracle, which is the same SEC-6
    // failure `parse.rs` documents for `include` diagnostics.
    let dir = scratch("disclosure");
    let main = main_journal(&dir);
    let secret = out_of_tree(&dir, "secret.rules");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&secret, dir.join("link.rules")).expect("symlink");
        std::os::unix::fs::symlink(secret.parent().expect("parent"), dir.join("linkdir"))
            .expect("symlink dir");
    }
    std::fs::write(dir.join("huge.rules"), vec![b'#'; (1 << 20) + 1]).expect("write huge");
    std::fs::write(dir.join("latin1.rules"), b"# caf\xe9\n").expect("write latin1");
    write(&dir, "fine.rules", RULES);

    let found = discover(&main);
    let warnings = all_warnings(&found);
    assert!(!warnings.is_empty(), "this tree must produce warnings");

    let root = dir.canonicalize().expect("canonical scratch root");
    for message in &warnings {
        assert!(
            !message.starts_with('/'),
            "warning starts with an absolute path: {message}"
        );
        for disclosed in [
            root.display().to_string(),
            dir.display().to_string(),
            secret.display().to_string(),
        ] {
            assert!(
                !message.contains(&disclosed),
                "warning discloses {disclosed}: {message}"
            );
        }
    }

    // The ids are relative too — they are the other half of the same response.
    for file in &found.files {
        assert!(!file.id.starts_with('/'), "id is absolute: {}", file.id);
        assert!(!file.id.contains(".."), "id traverses: {}", file.id);
    }
    // ...and so is the heading.
    assert!(!found.root_label().contains('/'));
}

#[test]
#[cfg(unix)]
fn an_unreadable_directory_is_a_warning_and_not_a_failure() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("unreadable");
    let main = main_journal(&dir);
    write(&dir, "fine.rules", RULES);
    let locked = dir.join("locked");
    std::fs::create_dir_all(&locked).expect("create dir");
    write(&locked, "inside.rules", RULES);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    // Probe WHILE it is locked: root can read a 0000 directory, and a suite that
    // checked afterwards would silently skip itself everywhere.
    let really_locked = std::fs::read_dir(&locked).is_err();

    let found = discover(&main);
    // Restore before asserting, so a failure does not leave an undeletable dir.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("chmod back");

    if !really_locked {
        eprintln!("skipping: running as a user that can read a 0000 directory");
        return;
    }
    assert_eq!(
        ids(&found),
        vec!["fine.rules"],
        "one unreadable directory must not cost the whole listing"
    );
    assert!(
        found.warnings.iter().any(|w| w.message.contains("locked")),
        "{:?}",
        found.warnings
    );
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn ordering_is_deterministic_across_runs() {
    // `read_dir` order is filesystem- and run-dependent. A list that reshuffles
    // between two identical requests makes every later diff and every UI
    // selection unstable.
    let dir = scratch("ordering");
    let main = main_journal(&dir);
    for name in [
        "zebra.rules",
        "alpha.csv.rules",
        "import/2026/b.rules",
        "import/2025/a.rules",
        "middle.rules",
    ] {
        write(&dir, name, RULES);
    }

    let first = ids(&discover(&main));
    assert_eq!(
        first,
        vec![
            "alpha.csv.rules",
            "import/2025/a.rules",
            "import/2026/b.rules",
            "middle.rules",
            "zebra.rules",
        ],
        "sorted by id"
    );
    for _ in 0..3 {
        assert_eq!(ids(&discover(&main)), first);
    }
}

// ---------------------------------------------------------------------------
// The committed discovery fixture
// ---------------------------------------------------------------------------

#[test]
fn the_committed_tree_fixture_finds_exactly_one_rules_file() {
    // `fixtures/rules/tree/` is the shape a real journal has: the journal at the
    // top, its rules under `import/YYYY/`, and two decoys that must never be
    // offered for editing.
    //
    // The decoy for the dot-directory rule is `.hidden/` rather than `.git/`
    // because git refuses to track any path with a `.git` component, so a
    // committed `.git/hidden.rules` cannot exist. The `.git` case itself is
    // covered by `dot_directories_and_skip_dirs_are_not_scanned` above, which
    // builds it at test time.
    let main = common::fixtures_dir().join("rules/tree/main.journal");
    let found = discover(&main);

    assert!(!found.truncated);
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    assert_eq!(ids(&found), vec!["import/2026/bank.csv.rules"]);
    assert!(found.resolve("node_modules/dep.rules").is_none());
    assert!(found.resolve(".hidden/hidden.rules").is_none());
    assert_eq!(found.root_label(), "tree");

    let bank = found.resolve("import/2026/bank.csv.rules").expect("found");
    assert_eq!(bank.label, "bank");
    assert!(bank.parsed);
    assert!(bank.warnings.is_empty());
    assert_eq!(bank.account1.as_deref(), Some("assets:bank:checking"));
    assert_eq!(bank.account2.as_deref(), Some("expenses:unknown"));
    // Four conditional constructs: three editable blocks (`COFFEE`,
    // `LANDLORD`, and the two-matcher OR list) plus the conditional TABLE,
    // which is counted here and is also the file's one opaque item. The file
    // carries that table because it doubles as the rules-API document golden,
    // which needs an `opaque` item to describe.
    assert_eq!(bank.if_block_count, 4);
    assert_eq!(bank.editable_block_count, 3);
    assert_eq!(bank.opaque_item_count, 1);
    assert!(bank.path().as_path().is_absolute());
    assert!(bank.identity_unchanged());
}

#[test]
fn a_journal_with_no_rules_beside_it_discovers_nothing_and_says_nothing() {
    // The other committed journal tree: `fixtures/` itself has no `*.rules`
    // directly beside `sample.journal`, and an empty list is not an error.
    let main = common::fixtures_dir().join("sample.journal");
    let found = discover(&main);
    assert!(
        found.files.iter().all(|file| file.id.ends_with(".rules")),
        "only rules files are ever listed"
    );
    for file in &found.files {
        assert!(!file.id.starts_with('/'));
    }
}
