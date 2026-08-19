//! Tests for the preferences store (`src/prefs.rs`) and hledger resolution
//! (`src/hledger.rs`), WP-11 lane A.
//!
//! # Why the modules are included by `#[path]`
//!
//! Both are private modules of the library (`mod prefs;` / `mod hledger;` in
//! `lib.rs`), which is correct — their public surface is the `/api/prefs` and
//! `/api/import/capabilities` routes, not a Rust API — but it means an
//! integration test crate cannot `use ledgeline_server::prefs`. Compiling the
//! sources into this test binary is the standard way out, and it keeps `lib.rs`
//! unchanged. `crate::prefs` inside `hledger.rs` resolves to the `prefs` module
//! declared below, because this file is the root of the test crate.
//!
//! # Env-var isolation: child processes, not a mutex
//!
//! `$LEDGELINE_CONFIG_DIR` and `$LEDGELINE_HLEDGER` are process-global, and
//! libtest runs tests on threads of ONE process. Two tests setting the same
//! variable race, and a test that sets one while another reads it corrupts a
//! result that will look like a logic bug. Worse, `std::env::set_var` is
//! `unsafe` in edition 2024 precisely because it is not thread-safe, and this
//! codebase does not use `unsafe`.
//!
//! So env-dependent tests never mutate this process's environment. They
//! re-execute THIS test binary as a child ([`run_child`]) with the variables set
//! for that child alone, naming a single `#[ignore]`d test to run. Each child
//! gets a pristine, private environment, so the tests are hermetic, order-
//! independent, and safe to run in parallel — and they exercise the real
//! `std::env::var_os` code path rather than a test-only seam around it.
//!
//! Everything that does NOT need the environment is driven through `load_from` /
//! `store_in`, which take the store path explicitly. That is the same shape
//! `recents.rs`'s own tests use.
//!
//! No test requires a real `hledger`: every resolution test points at a shell
//! stub written into a tempdir.

#[path = "../src/prefs.rs"]
mod prefs;

#[path = "../src/hledger.rs"]
mod hledger;

use hledger::{Hledger, HledgerError, MIN_HLEDGER, NO_CONF, Version};
use prefs::{Prefs, PrefsError};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The env var this test file uses to hand a child process its scratch dir,
/// since a child cannot be given a `TempDir` handle.
const CHILD_DIR_ENV: &str = "LEDGELINE_TEST_CHILD_DIR";

/// Write an executable `hledger` stub that prints `banner` on stdout, and return
/// its absolute path.
///
/// A stub rather than the real binary so the whole suite is hermetic: `cargo
/// test` must pass on a machine with no hledger installed, which is the standing
/// rule for this repo (`plans/11-enhanced-import.md`, "Definition of done").
fn write_stub(dir: &Path, name: &str, banner: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{banner}'\n"))
        .expect("write hledger stub");
    make_executable(&path);
    path
}

/// Write a non-executable regular file and return its path.
fn write_plain_file(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "not a program").expect("write plain file");
    path
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod the stub executable");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

/// Re-execute this test binary, running exactly the `#[ignore]`d test named
/// `test_name` with `env` applied to the child alone.
///
/// `--exact` pins the name, `--ignored` opts the child test in (it is ignored in
/// a normal run precisely so it only executes here), and `--test-threads=1`
/// keeps the child's output readable when it fails.
///
/// Asserts the child passed, printing its captured output on failure — otherwise
/// a broken child test reports only "exit code 101" with the real assertion
/// message thrown away.
fn run_child(test_name: &str, env: &[(&str, &Path)]) {
    let exe = std::env::current_exe().expect("locate this test binary");
    let mut command = Command::new(exe);
    command.args([test_name, "--exact", "--ignored", "--test-threads=1"]);
    // The child inherits our environment and then overrides these. Any var this
    // suite may have been handed is cleared so a child never sees a stale one.
    command.env_remove("LEDGELINE_CONFIG_DIR");
    command.env_remove("LEDGELINE_HLEDGER");
    command.env_remove(CHILD_DIR_ENV);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
        .output()
        .expect("re-run this test binary as a child");
    assert!(
        output.status.success(),
        "child test `{test_name}` failed\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// The scratch directory a child was given, or `None` when this test was invoked
/// directly (`cargo test -- --ignored`) rather than by [`run_child`].
///
/// Child tests return early in that case instead of failing: they are not
/// standalone tests, and a bare `--ignored` run should not report a failure for
/// a harness that was never set up.
fn child_dir() -> Option<PathBuf> {
    std::env::var_os(CHILD_DIR_ENV).map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// prefs: round-trip, defaults, forward compatibility
// ---------------------------------------------------------------------------

/// The basic contract: what goes in comes back out, including the three-valued
/// `git_autocommit` whose `None` is a distinct meaning ("decide by whether a
/// repo is present") and not merely an absent `false`.
#[test]
fn prefs_round_trip_through_the_store() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");
    let stub = write_stub(dir.path(), "hledger", "hledger 1.52, mac-aarch64");

    for autocommit in [None, Some(true), Some(false)] {
        let written = Prefs {
            hledger_path: Some(stub.clone()),
            git_autocommit: autocommit,
        };
        prefs::store_in(&store, &written).expect("store valid prefs");
        assert_eq!(
            prefs::load_from(&store),
            written,
            "git_autocommit {autocommit:?} must survive the round trip verbatim"
        );
    }
}

/// An absent store is the "never opened the settings screen" state, and must not
/// leave anything behind on disk.
#[test]
fn a_missing_store_reads_as_defaults() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");

    assert_eq!(prefs::load_from(&store), Prefs::default());
    assert_eq!(Prefs::default().hledger_path, None);
    assert_eq!(Prefs::default().git_autocommit, None);
    assert!(!store.exists(), "reading must not create the store");
    assert!(
        !store.with_extension("json.corrupt").exists(),
        "an absent store is not a corrupt one"
    );
}

/// The wire names are camelCase, because the SPA reads this same shape off
/// `/api/prefs`. A rename here silently breaks the settings form.
#[test]
fn the_stored_json_uses_camel_case_keys() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");
    let stub = write_stub(dir.path(), "hledger", "hledger 1.52, mac-aarch64");

    prefs::store_in(
        &store,
        &Prefs {
            hledger_path: Some(stub),
            git_autocommit: Some(false),
        },
    )
    .expect("store");

    let text = std::fs::read_to_string(&store).expect("read the store back");
    assert!(text.contains("\"hledgerPath\""), "got: {text}");
    assert!(text.contains("\"gitAutocommit\""), "got: {text}");
}

/// Forward compatibility, both directions: a file written by a LATER version
/// carries fields we have never heard of, and one written by an EARLIER version
/// is missing fields we now expect. Neither may fail to load — that is what
/// `#[serde(default)]` plus serde's tolerance of unknown fields buys, and it is
/// worth a test because adding `deny_unknown_fields` later would look harmless.
#[test]
fn unknown_and_missing_fields_both_load() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");

    // A newer version's file: an unknown scalar, an unknown object, and a key
    // we do know.
    std::fs::write(
        &store,
        r#"{
          "hledgerPath": null,
          "gitAutocommit": true,
          "futureSetting": "some value",
          "futureObject": { "nested": [1, 2, 3] }
        }"#,
    )
    .expect("write a forward-compatible store");

    assert_eq!(
        prefs::load_from(&store),
        Prefs {
            hledger_path: None,
            git_autocommit: Some(true),
        },
        "unknown fields must be ignored, not rejected"
    );
    assert!(
        !store.with_extension("json.corrupt").exists(),
        "a readable file with extra fields is not corrupt"
    );

    // An older version's file: no keys at all.
    std::fs::write(&store, "{}").expect("write an empty object");
    assert_eq!(prefs::load_from(&store), Prefs::default());
}

// ---------------------------------------------------------------------------
// prefs: the corrupt-store rule
// ---------------------------------------------------------------------------

/// The rule `recents.rs` established and this store inherits: a file we cannot
/// parse is content we FAILED TO READ, not content that is not there. It is
/// moved aside byte-for-byte and defaults are returned, so a settings file is
/// never destroyed by a parse bug.
#[test]
fn a_corrupt_store_is_moved_aside_on_load_and_its_bytes_survive() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");
    let garbage = "{ not valid json ]";
    std::fs::write(&store, garbage).expect("write garbage");

    assert_eq!(
        prefs::load_from(&store),
        Prefs::default(),
        "an unreadable store reads as defaults"
    );

    let aside = store.with_extension("json.corrupt");
    assert_eq!(
        std::fs::read_to_string(&aside).expect("the unreadable bytes are kept"),
        garbage,
        "the original bytes must survive in the moved-aside file"
    );
    assert!(!store.exists(), "and the corrupt store itself is gone");
}

/// The same rule on the WRITE path. `store_in` replaces the file wholesale, so
/// a caller that stores without loading first would otherwise be the one path
/// that silently destroys an unparseable file.
#[test]
fn storing_over_a_corrupt_file_preserves_it_too() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");
    let garbage = "\u{feff}not json at all";
    std::fs::write(&store, garbage).expect("write garbage");

    prefs::store_in(
        &store,
        &Prefs {
            hledger_path: None,
            git_autocommit: Some(true),
        },
    )
    .expect("store over a corrupt file");

    assert_eq!(
        std::fs::read_to_string(store.with_extension("json.corrupt"))
            .expect("the unreadable bytes are kept"),
        garbage
    );
    assert_eq!(
        prefs::load_from(&store).git_autocommit,
        Some(true),
        "and the store recovers rather than staying broken"
    );
}

/// JSON that parses but is the wrong SHAPE (an array, a bare string, a number)
/// is corrupt too — `serde_json::from_str::<Prefs>` is what draws the line, and
/// each of these must take the preserve-and-recover path rather than panicking
/// or reading as defaults-with-the-file-still-there.
#[test]
fn well_formed_json_of_the_wrong_shape_is_also_corrupt() {
    for (index, garbage) in ["[1, 2, 3]", "\"a string\"", "42", "null"]
        .into_iter()
        .enumerate()
    {
        let dir = TempDir::new().expect("temp dir");
        let store = dir.path().join("prefs.json");
        std::fs::write(&store, garbage).expect("write");

        assert_eq!(prefs::load_from(&store), Prefs::default(), "case {index}");
        assert_eq!(
            std::fs::read_to_string(store.with_extension("json.corrupt")).expect("kept"),
            garbage,
            "case {index}: {garbage} must be preserved, not overwritten"
        );
    }
}

// ---------------------------------------------------------------------------
// prefs: store-time validation of hledger_path
// ---------------------------------------------------------------------------

/// The whole point of validating at store time: a bad path must be refused HERE,
/// where the user just typed it and the form can say why, rather than persisted
/// and re-surfacing three screens later as "could not run hledger".
///
/// Every rejection case, and each must leave the filesystem untouched.
#[test]
fn an_unusable_hledger_path_is_rejected_and_nothing_is_written() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");

    let missing = dir.path().join("no-such-binary");
    let a_directory = dir.path().join("subdir");
    std::fs::create_dir(&a_directory).expect("create dir");
    let not_executable = write_plain_file(dir.path(), "hledger.txt");
    let relative = PathBuf::from("hledger");

    // The execute-bit case only exists on unix; `cfg!` rather than `#[cfg]` so
    // `not_executable` is used on every platform and the list stays one
    // expression.
    let unix_only: Vec<(&str, PathBuf, &str)> = if cfg!(unix) {
        vec![(
            "a file without the execute bit",
            not_executable,
            "is not executable",
        )]
    } else {
        Vec::new()
    };
    let cases: Vec<(&str, PathBuf, &str)> = vec![
        ("a path that does not exist", missing, "does not exist"),
        ("a directory", a_directory, "is not a regular file"),
        ("a relative path", relative, "must be an absolute path"),
    ]
    .into_iter()
    .chain(unix_only)
    .collect();

    for (label, path, expected_reason) in cases {
        let attempted = Prefs {
            hledger_path: Some(path),
            git_autocommit: None,
        };
        let error = prefs::store_in(&store, &attempted)
            .expect_err(&format!("{label} must be rejected, not persisted"));
        match error {
            PrefsError::InvalidHledgerPath { reason } => {
                assert_eq!(reason, expected_reason, "{label}");
            }
            other => panic!("{label}: expected InvalidHledgerPath, got {other:?}"),
        }
        assert!(
            !store.exists(),
            "{label}: a rejected value must not create the store"
        );
    }
}

/// The rejection message must not carry the path. `/api/prefs` renders
/// `PrefsError` through `Display`, and `tests/error_surface.rs` pins that no
/// `/api/*` body discloses an absolute path.
#[test]
fn a_rejection_message_never_names_the_path() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");
    let missing = dir.path().join("definitely-not-here");

    let error = prefs::store_in(
        &store,
        &Prefs {
            hledger_path: Some(missing.clone()),
            git_autocommit: None,
        },
    )
    .expect_err("rejected");

    let rendered = error.to_string();
    assert!(
        !rendered.contains(&*missing.to_string_lossy()),
        "the error body must not disclose a path: {rendered}"
    );
    assert!(!rendered.contains(&*dir.path().to_string_lossy()));
    assert_eq!(rendered, "the hledger path does not exist");
}

/// A rejected write must not clobber settings that are already there. The
/// validation happens before anything touches the filesystem, and this is what
/// pins that ordering.
#[test]
fn a_rejected_write_leaves_existing_settings_intact() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");
    let stub = write_stub(dir.path(), "hledger", "hledger 1.52, mac-aarch64");

    let good = Prefs {
        hledger_path: Some(stub),
        git_autocommit: Some(true),
    };
    prefs::store_in(&store, &good).expect("store good prefs");

    let error = prefs::store_in(
        &store,
        &Prefs {
            hledger_path: Some(dir.path().join("gone")),
            git_autocommit: Some(false),
        },
    );
    assert!(error.is_err());
    assert_eq!(
        prefs::load_from(&store),
        good,
        "the previous settings must survive a rejected write untouched"
    );
}

/// `hledger_path: None` is always valid — clearing the setting is how a user
/// goes back to automatic discovery, and it must not be caught by the validator.
#[test]
fn clearing_the_hledger_path_is_always_valid() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");
    let stub = write_stub(dir.path(), "hledger", "hledger 1.52, mac-aarch64");

    prefs::store_in(
        &store,
        &Prefs {
            hledger_path: Some(stub),
            git_autocommit: None,
        },
    )
    .expect("store");
    prefs::store_in(&store, &Prefs::default()).expect("clearing must be accepted");
    assert_eq!(prefs::load_from(&store), Prefs::default());
}

// ---------------------------------------------------------------------------
// prefs: on-disk hygiene
// ---------------------------------------------------------------------------

/// Written through `ledgeline_core::edit::atomic_write`, like `recents.rs`: the
/// store lands complete and owner-only, and no temp file is left behind. A
/// crash mid-write must leave the previous settings intact rather than a
/// truncated file that would read back as corrupt.
#[test]
fn the_store_is_written_atomically_and_owner_only() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("prefs.json");

    prefs::store_in(
        &store,
        &Prefs {
            hledger_path: None,
            git_autocommit: Some(true),
        },
    )
    .expect("store");

    let leftovers: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .expect("read config dir")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().contains(".ledgeline-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&store)
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode & !0o600,
            0,
            "a freshly created prefs store must not be wider than 0600, got {mode:o}"
        );
    }
}

/// A config directory that does not exist yet is created — this is the very
/// first run of a fresh install.
#[test]
fn storing_creates_a_missing_config_directory() {
    let dir = TempDir::new().expect("temp dir");
    let store = dir.path().join("nested").join("deeper").join("prefs.json");

    prefs::store_in(
        &store,
        &Prefs {
            hledger_path: None,
            git_autocommit: Some(false),
        },
    )
    .expect("store into a not-yet-existing directory");

    assert_eq!(prefs::load_from(&store).git_autocommit, Some(false));
}

// ---------------------------------------------------------------------------
// prefs: $LEDGELINE_CONFIG_DIR (child process — see the module docs)
// ---------------------------------------------------------------------------

/// `$LEDGELINE_CONFIG_DIR` redirects the whole store, exactly as it does for
/// `recents.rs`. Driven in a child process so this suite never mutates its own
/// environment and never touches the real user config.
#[test]
fn the_config_dir_env_var_redirects_the_store() {
    let dir = TempDir::new().expect("temp dir");
    run_child(
        "child_config_dir_env_var_is_honoured",
        &[
            ("LEDGELINE_CONFIG_DIR", dir.path()),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );

    // Asserted from the PARENT as well, so the child cannot pass by asserting
    // nothing: the file really is where the env var said, under that exact name.
    let store = dir.path().join("prefs.json");
    assert!(
        store.is_file(),
        "the child's `store()` must have written {}",
        store.display()
    );
    assert_eq!(
        prefs::load_from(&store),
        Prefs {
            hledger_path: None,
            git_autocommit: Some(false),
        }
    );
}

/// Runs only as a child of [`the_config_dir_env_var_redirects_the_store`], with
/// `$LEDGELINE_CONFIG_DIR` set for this process alone.
#[test]
#[ignore = "child process: driven by the_config_dir_env_var_redirects_the_store"]
fn child_config_dir_env_var_is_honoured() {
    let Some(dir) = child_dir() else { return };

    // The env-reading `load`/`store`, not the `_in` variants: this is the one
    // test that exercises the real environment path end to end.
    assert_eq!(
        prefs::load(),
        Prefs::default(),
        "a fresh config dir has no settings"
    );

    let written = Prefs {
        hledger_path: None,
        git_autocommit: Some(false),
    };
    prefs::store(&written).expect("store through the env-resolved path");
    assert_eq!(
        prefs::load(),
        written,
        "and load reads back what store wrote"
    );
    assert!(
        dir.join("prefs.json").is_file(),
        "the store must live directly in $LEDGELINE_CONFIG_DIR"
    );
}

// ---------------------------------------------------------------------------
// hledger: version parsing
// ---------------------------------------------------------------------------

/// Table-driven over real-shaped `hledger --version` output.
///
/// The tail after the version varies by build (platform triple, patch level, a
/// git-describe suffix), so the parser reads only the leading dotted-numeric run
/// of the second token. Every accepted shape here was taken from a real banner
/// or a real build convention.
#[test]
fn version_parses_every_real_banner_shape() {
    let cases = [
        // The exact string from hledger 1.52 on this machine.
        ("hledger 1.52, mac-aarch64", 1, 52),
        ("hledger 1.40, linux-x86_64", 1, 40),
        ("hledger 1.52, linux-aarch64\n", 1, 52),
        // A patch component: kept out of the comparison, not a parse failure.
        ("hledger 1.32.3, mac-aarch64", 1, 32),
        ("hledger 1.40.1", 1, 40),
        // A dev build's git-describe suffix.
        ("hledger 1.42.1-g8f3a2b1-20260115, linux-x86_64", 1, 42),
        // Two components only.
        ("hledger 1.5", 1, 5),
        // Trailing text with no comma.
        ("hledger 1.52 mac-aarch64", 1, 52),
        // Multi-line output: only the first non-empty line is read.
        ("hledger 1.52, mac-aarch64\nsomething else entirely", 1, 52),
        ("\n\nhledger 1.52, mac-aarch64", 1, 52),
        // Leading/trailing whitespace.
        ("  hledger 1.52, mac-aarch64  \n", 1, 52),
        // A `v` prefix hledger does not currently emit, tolerated anyway:
        // rejecting a real binary costs the user their import.
        ("hledger v1.52, mac-aarch64", 1, 52),
        // Multi-digit minor, and a major bump.
        ("hledger 1.100, x", 1, 100),
        ("hledger 2.0, mac-aarch64", 2, 0),
        // A missing minor reads as .0 rather than failing.
        ("hledger 2", 2, 0),
    ];
    for (banner, major, minor) in cases {
        assert_eq!(
            Version::parse(banner),
            Some(Version { major, minor }),
            "failed to parse: {banner:?}"
        );
    }
}

/// Output that is not an hledger banner must not parse into some plausible-
/// looking version — a wrong number here is a version gate that passes for the
/// wrong binary.
#[test]
fn version_rejects_anything_that_is_not_an_hledger_banner() {
    for banner in [
        "",
        "\n\n",
        "hledger",
        "hledger ",
        // A different program that happens to print a version.
        "ledger 3.3.2",
        "hledger-ui 1.52, mac-aarch64",
        "git version 2.51.0",
        // The right program name but no number.
        "hledger unknown",
        "hledger , mac-aarch64",
        // Not a leading token.
        "this is hledger 1.52",
    ] {
        assert_eq!(Version::parse(banner), None, "must not parse: {banner:?}");
    }
}

/// Versions compare as NUMBERS, not strings. 1.9 vs 1.40 is the case that
/// matters: a lexicographic comparison calls 1.9 the newer one and lets exactly
/// the ancient distro package this gate exists to catch straight through.
#[test]
fn versions_compare_numerically_not_lexicographically() {
    let v = |major, minor| Version { major, minor };

    assert!(v(1, 9) < v(1, 40), "1.9 is OLDER than 1.40");
    assert!(v(1, 9) < MIN_HLEDGER);
    assert!(v(1, 39) < MIN_HLEDGER);
    assert!(v(1, 40) >= MIN_HLEDGER, "1.40 is the floor, inclusive");
    assert!(v(1, 52) >= MIN_HLEDGER);
    assert!(v(2, 0) > v(1, 100));
    assert_eq!(MIN_HLEDGER, v(1, 40), "the floor is the --rules rename");
}

/// The `Display` used in the `TooOld` sentence the banner shows.
#[test]
fn version_displays_as_major_dot_minor() {
    assert_eq!(
        Version {
            major: 1,
            minor: 52
        }
        .to_string(),
        "1.52"
    );
    assert_eq!(MIN_HLEDGER.to_string(), "1.40");
    assert_eq!(
        HledgerError::TooOld {
            found: Version {
                major: 1,
                minor: 34
            },
            min: MIN_HLEDGER,
        }
        .to_string(),
        "hledger 1.34 is older than 1.40"
    );
}

// ---------------------------------------------------------------------------
// hledger: resolution (child processes — $LEDGELINE_HLEDGER is process-global)
// ---------------------------------------------------------------------------

/// `$LEDGELINE_HLEDGER` naming a current-enough stub resolves to exactly that
/// path, and reports the version the stub printed.
#[test]
fn resolve_honours_the_env_var() {
    let dir = TempDir::new().expect("temp dir");
    write_stub(dir.path(), "hledger", "hledger 1.52, mac-aarch64");
    run_child(
        "child_resolve_uses_the_env_var",
        &[
            ("LEDGELINE_HLEDGER", &dir.path().join("hledger")),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );
}

#[test]
#[ignore = "child process: driven by resolve_honours_the_env_var"]
fn child_resolve_uses_the_env_var() {
    let Some(dir) = child_dir() else { return };
    let expected = dir.join("hledger");

    let resolved = Hledger::resolve(&Prefs::default()).expect("the stub must resolve");
    assert_eq!(resolved.path(), expected);
    assert_eq!(
        resolved.version(),
        Version {
            major: 1,
            minor: 52
        }
    );
}

/// The documented precedence: `prefs.hledger_path` beats `$LEDGELINE_HLEDGER`.
/// Both stubs are valid and current, and they report DIFFERENT versions, so the
/// assertion identifies which one actually ran rather than merely that something
/// did.
#[test]
fn a_preference_outranks_the_env_var() {
    let dir = TempDir::new().expect("temp dir");
    write_stub(dir.path(), "from-prefs", "hledger 1.52, mac-aarch64");
    write_stub(dir.path(), "from-env", "hledger 1.44, mac-aarch64");
    run_child(
        "child_preference_outranks_the_env_var",
        &[
            ("LEDGELINE_HLEDGER", &dir.path().join("from-env")),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );
}

#[test]
#[ignore = "child process: driven by a_preference_outranks_the_env_var"]
fn child_preference_outranks_the_env_var() {
    let Some(dir) = child_dir() else { return };

    // Sanity: with no preference, the env var wins and we get 1.44.
    let from_env = Hledger::resolve(&Prefs::default()).expect("env stub resolves");
    assert_eq!(from_env.path(), dir.join("from-env"));
    assert_eq!(from_env.version().minor, 44);

    // With a preference set, it takes precedence — and the version proves it.
    let preferred = Hledger::resolve(&Prefs {
        hledger_path: Some(dir.join("from-prefs")),
        git_autocommit: None,
    })
    .expect("preferred stub resolves");
    assert_eq!(preferred.path(), dir.join("from-prefs"));
    assert_eq!(
        preferred.version().minor,
        52,
        "the preference must win over $LEDGELINE_HLEDGER"
    );
}

/// A binary that runs and reports a version below the floor is refused with
/// `TooOld` carrying both numbers — not a cryptic `--rules` usage dump halfway
/// through an import.
#[test]
fn an_old_hledger_is_refused_as_too_old() {
    let dir = TempDir::new().expect("temp dir");
    write_stub(dir.path(), "hledger", "hledger 1.39, mac-aarch64");
    run_child(
        "child_an_old_stub_is_too_old",
        &[
            ("LEDGELINE_HLEDGER", &dir.path().join("hledger")),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );
}

#[test]
#[ignore = "child process: driven by an_old_hledger_is_refused_as_too_old"]
fn child_an_old_stub_is_too_old() {
    let Some(_dir) = child_dir() else { return };

    match Hledger::resolve(&Prefs::default()) {
        Err(HledgerError::TooOld { found, min }) => {
            assert_eq!(
                found,
                Version {
                    major: 1,
                    minor: 39
                }
            );
            assert_eq!(min, MIN_HLEDGER);
        }
        other => panic!("expected TooOld, got {other:?}"),
    }
}

/// `TooOld` is TERMINAL — it does not fall through to the next candidate. A
/// preference pointing at an old binary must be reported, not silently replaced
/// by a newer one found further down the list, or "I set the path and it used a
/// different one" becomes unexplainable.
#[test]
fn too_old_does_not_fall_through_to_a_newer_candidate() {
    let dir = TempDir::new().expect("temp dir");
    write_stub(dir.path(), "old", "hledger 1.30, mac-aarch64");
    write_stub(dir.path(), "new", "hledger 1.52, mac-aarch64");
    run_child(
        "child_too_old_is_terminal",
        &[
            ("LEDGELINE_HLEDGER", &dir.path().join("new")),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );
}

#[test]
#[ignore = "child process: driven by too_old_does_not_fall_through_to_a_newer_candidate"]
fn child_too_old_is_terminal() {
    let Some(dir) = child_dir() else { return };

    let error = Hledger::resolve(&Prefs {
        hledger_path: Some(dir.join("old")),
        git_autocommit: None,
    })
    .expect_err("an old preference must be reported, not skipped");

    assert!(
        matches!(error, HledgerError::TooOld { found, .. } if found.minor == 30),
        "expected TooOld(1.30), got {error:?}"
    );
}

/// A preference whose binary has been DELETED (a Nix garbage-collect will do it)
/// falls through to the next candidate rather than failing the import. That is
/// the opposite disposition to `TooOld`, and the distinction is deliberate: a
/// missing file is not an answer, a version number is.
#[test]
fn a_stale_preference_falls_through_to_the_next_candidate() {
    let dir = TempDir::new().expect("temp dir");
    write_stub(dir.path(), "hledger", "hledger 1.52, mac-aarch64");
    run_child(
        "child_stale_preference_falls_through",
        &[
            ("LEDGELINE_HLEDGER", &dir.path().join("hledger")),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );
}

#[test]
#[ignore = "child process: driven by a_stale_preference_falls_through_to_the_next_candidate"]
fn child_stale_preference_falls_through() {
    let Some(dir) = child_dir() else { return };

    for stale in [
        dir.join("deleted-by-a-gc"), // missing
        dir.clone(),                 // a directory, which `exists()` would accept
        PathBuf::from("hledger"),    // relative
    ] {
        let resolved = Hledger::resolve(&Prefs {
            hledger_path: Some(stale.clone()),
            git_autocommit: None,
        })
        .unwrap_or_else(|error| panic!("{stale:?} should fall through, got {error:?}"));
        assert_eq!(
            resolved.path(),
            dir.join("hledger"),
            "{stale:?} must fall through to $LEDGELINE_HLEDGER"
        );
    }
}

/// With no preference, no env var, and a `$PATH` that contains no `hledger`, the
/// answer is `NotFound` — the actionable banner's case.
///
/// The child's `$PATH` is pointed at an empty directory, so the result does not
/// depend on whether the machine running the suite happens to have hledger
/// installed. That is what keeps this test hermetic in both directions: it
/// passes on a developer laptop with hledger on `$PATH` and in CI without.
#[test]
fn nothing_anywhere_reports_not_found() {
    let dir = TempDir::new().expect("temp dir");
    let empty = dir.path().join("empty-path-entry");
    std::fs::create_dir(&empty).expect("create an empty PATH entry");

    run_child(
        "child_nothing_is_not_found",
        &[("PATH", &empty), (CHILD_DIR_ENV, dir.path())],
    );
}

#[test]
#[ignore = "child process: driven by nothing_anywhere_reports_not_found"]
fn child_nothing_is_not_found() {
    if child_dir().is_none() {
        return;
    }
    assert_eq!(
        Hledger::resolve(&Prefs::default()),
        Err(HledgerError::NotFound),
        "no preference, no env var and an empty $PATH is NotFound"
    );
}

/// A binary that runs but does not answer like hledger is `Unrunnable`, and that
/// too is terminal: a program called `hledger` that prints something else is a
/// misconfiguration to report, not one to paper over.
#[test]
fn a_binary_that_is_not_hledger_is_unrunnable() {
    let dir = TempDir::new().expect("temp dir");
    write_stub(dir.path(), "hledger", "ledger 3.3.2");
    run_child(
        "child_not_hledger_is_unrunnable",
        &[
            ("LEDGELINE_HLEDGER", &dir.path().join("hledger")),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );
}

#[test]
#[ignore = "child process: driven by a_binary_that_is_not_hledger_is_unrunnable"]
fn child_not_hledger_is_unrunnable() {
    if child_dir().is_none() {
        return;
    }
    assert_eq!(
        Hledger::resolve(&Prefs::default()),
        Err(HledgerError::Unrunnable)
    );
}

// ---------------------------------------------------------------------------
// hledger: the invocation helper
// ---------------------------------------------------------------------------

/// The invariant the whole preview feature rests on: stdout and stderr are
/// captured SEPARATELY. `hledger import --dry-run` writes the proposed
/// transactions to stdout and its `would import N new transactions` status line
/// to stderr, and merging them would put a status line in the middle of journal
/// text.
#[test]
fn stdout_and_stderr_are_captured_separately() {
    let dir = TempDir::new().expect("temp dir");
    let script = dir.path().join("hledger");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *--version*) printf 'hledger 1.52, mac-aarch64\\n' ;;\n\
         *) printf '2026-01-01 Opening\\n    assets:bank  $1.00\\n'\n\
            printf 'would import 1 new transactions from data.csv:\\n' >&2\n\
            exit 0 ;;\n\
         esac\n",
    )
    .expect("write the two-stream stub");
    make_executable(&script);

    let hledger = resolve_stub(&script);
    let output = hledger
        .invoke(["import", "--dry-run"])
        .run()
        .expect("the stub runs");

    assert!(output.success());
    assert_eq!(
        output.stdout_lossy(),
        "2026-01-01 Opening\n    assets:bank  $1.00\n",
        "stdout must be the journal text alone"
    );
    assert_eq!(
        output.stderr_lossy(),
        "would import 1 new transactions from data.csv:\n",
        "and the status line must be on stderr alone"
    );
}

/// Arguments go through as a `Vec<OsString>` — no shell, so no quoting, no
/// word-splitting and no substitution. Every one of these would be mangled if
/// the call went through `sh -c`.
#[test]
fn arguments_are_passed_verbatim_with_no_shell() {
    let dir = TempDir::new().expect("temp dir");
    let script = dir.path().join("hledger");
    // Echoes each argument on its own line, so word-splitting is visible.
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *--version*) printf 'hledger 1.52, x\\n' ;;\n\
         *) for a in \"$@\"; do printf '%s\\n' \"$a\"; done ;;\n\
         esac\n",
    )
    .expect("write the echo stub");
    make_executable(&script);

    let hledger = resolve_stub(&script);
    let hostile = [
        "a file with spaces.csv",
        "$(rm -rf /)",
        "`whoami`",
        "quote\"and'quote",
        "semi;colon && pipe | redirect > /tmp/x",
        "wild*card?glob[abc]",
        "trailing\\backslash",
        "--rules",
    ];
    let output = hledger.invoke(hostile).run().expect("the stub runs");

    // `--no-conf` first, then the caller's arguments byte for byte. This is the
    // BEHAVIOURAL half of the config-file guard — the flag is observed arriving
    // at a real process, where `import_api`'s lint only inspects the vector —
    // and it belongs in this test because "the flag is added" and "nothing else
    // is touched" are the same assertion made once.
    let expected: Vec<&str> = std::iter::once(NO_CONF).chain(hostile).collect();
    assert_eq!(
        output.stdout_lossy().lines().collect::<Vec<_>>(),
        expected,
        "every argument must arrive byte-for-byte as it was passed, behind --no-conf"
    );
}

/// A binary too old to know `--no-conf` is still reported as **too old**, not as
/// unrunnable.
///
/// `--no-conf` arrived in hledger 1.40 — the same release that introduced config
/// files and the same release [`MIN_HLEDGER`] is set to — so against a 1.39 the
/// flag is an unrecognised option and the version probe would learn nothing. The
/// probe therefore retries once without it, and this pins that: the user of an
/// ancient hledger gets a number to act on instead of "could not run hledger"
/// about a binary that is sitting right there.
#[test]
fn a_binary_that_predates_the_flag_still_reports_its_version() {
    let dir = TempDir::new().expect("temp dir");
    let script = dir.path().join("hledger");
    // Exactly how a pre-1.40 hledger behaves: an unknown flag is a usage error
    // on stderr and a non-zero exit, with nothing version-shaped on stdout.
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *--no-conf*) printf 'hledger: Unknown flag: --no-conf\\n' >&2; exit 1 ;;\n\
         *--version*) printf 'hledger 1.39\\n' ;;\n\
         esac\n",
    )
    .expect("write the ancient stub");
    make_executable(&script);

    let error = Hledger::resolve(&Prefs {
        hledger_path: Some(script.clone()),
        git_autocommit: None,
    })
    .expect_err("1.39 is below the floor");
    assert!(
        matches!(
            error,
            HledgerError::TooOld {
                found: Version {
                    major: 1,
                    minor: 39
                },
                ..
            }
        ),
        "{error}"
    );
}

/// `arg` and `args` extend the same list in order — the shape the import lane
/// builds a call with (fixed flags, then a path it computed).
#[test]
fn arg_and_args_append_in_order() {
    let dir = TempDir::new().expect("temp dir");
    let script = dir.path().join("hledger");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *--version*) printf 'hledger 1.52, x\\n' ;;\n\
         *) for a in \"$@\"; do printf '%s\\n' \"$a\"; done ;;\n\
         esac\n",
    )
    .expect("write stub");
    make_executable(&script);

    let hledger = resolve_stub(&script);
    let output = hledger
        .invoke(["import"])
        .arg("--dry-run")
        .args(["--rules", "/tmp/bank.rules"])
        .arg(Path::new("/tmp/data.csv"))
        .run()
        .expect("runs");

    assert_eq!(
        output.stdout_lossy().lines().collect::<Vec<_>>(),
        [
            // Prepended by `Invocation::argv`, ahead of the subcommand — a
            // config file's own injected command word would otherwise sit in
            // front of it.
            NO_CONF,
            "import",
            "--dry-run",
            "--rules",
            "/tmp/bank.rules",
            "/tmp/data.csv"
        ]
    );
}

/// stdin is how the balance check is done: hledger's balance ASSERTIONS do not
/// aggregate across two `-f` flags, so combined verification is
/// `cat A B | hledger -f- check`. This is that pipe, without a shell.
#[test]
fn stdin_is_delivered_to_the_child() {
    let dir = TempDir::new().expect("temp dir");
    let script = dir.path().join("hledger");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *--version*) printf 'hledger 1.52, x\\n' ;;\n\
         *) cat ;;\n\
         esac\n",
    )
    .expect("write the cat stub");
    make_executable(&script);

    let hledger = resolve_stub(&script);
    let journal = "2026-01-01 A\n    assets:bank  $1.00\n    equity  $-1.00\n";
    let output = hledger
        .invoke(["-f-", "check"])
        .stdin(journal.as_bytes().to_vec())
        .run()
        .expect("runs");

    assert_eq!(output.stdout_lossy(), journal);
}

/// A payload larger than one pipe buffer (~64 KiB) must not deadlock. This is
/// the case a naive `wait()`-then-`read()` hangs on, and it is exactly the size
/// an `import --dry-run` of a few hundred transactions produces — so it is the
/// realistic case, not an exotic one.
#[test]
fn a_payload_larger_than_a_pipe_buffer_does_not_deadlock() {
    let dir = TempDir::new().expect("temp dir");
    let script = dir.path().join("hledger");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *--version*) printf 'hledger 1.52, x\\n' ;;\n\
         *) cat; printf 'done\\n' >&2 ;;\n\
         esac\n",
    )
    .expect("write stub");
    make_executable(&script);

    let hledger = resolve_stub(&script);
    // 1 MiB in and 1 MiB back out, both far past any pipe buffer.
    let payload = "2026-01-01 A description that is a realistic length\n".repeat(20_000);
    let output = hledger
        .invoke(["-f-", "print"])
        .stdin(payload.as_bytes().to_vec())
        .timeout(std::time::Duration::from_secs(30))
        .run()
        .expect("runs without deadlocking");

    assert_eq!(output.stdout.len(), payload.len());
    assert_eq!(output.stderr_lossy(), "done\n");
}

/// A child that never exits is killed at the deadline rather than hanging the
/// GUI forever. `TimedOut` names the budget so the banner can say what happened.
#[test]
fn a_hung_child_is_killed_at_the_timeout() {
    let dir = TempDir::new().expect("temp dir");
    let script = dir.path().join("hledger");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *--version*) printf 'hledger 1.52, x\\n' ;;\n\
         *) sleep 120 ;;\n\
         esac\n",
    )
    .expect("write the hanging stub");
    make_executable(&script);

    let hledger = resolve_stub(&script);
    let budget = std::time::Duration::from_millis(250);
    let started = std::time::Instant::now();
    let error = hledger
        .invoke(["import"])
        .timeout(budget)
        .run()
        .expect_err("a hung child must not return output");
    let elapsed = started.elapsed();

    assert_eq!(error, HledgerError::TimedOut { after: budget });
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "the call must return at the deadline, not when the child would have \
         finished; took {elapsed:?}"
    );
}

/// A non-zero exit is REPORTED, not turned into an `Err`: hledger exits non-zero
/// for a failed `check` or a rules error, and its stderr is good enough to show
/// the user verbatim. Swallowing it as an error would lose that text.
#[test]
fn a_non_zero_exit_is_reported_with_its_output() {
    let dir = TempDir::new().expect("temp dir");
    let script = dir.path().join("hledger");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *--version*) printf 'hledger 1.52, x\\n' ;;\n\
         *) printf 'hledger: Error: balance assertion failed\\n' >&2; exit 1 ;;\n\
         esac\n",
    )
    .expect("write the failing stub");
    make_executable(&script);

    let hledger = resolve_stub(&script);
    let output = hledger.invoke(["check"]).run().expect("runs to completion");

    assert!(!output.success());
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stderr_lossy().contains("balance assertion failed"),
        "hledger's own diagnostic must survive: {:?}",
        output.stderr_lossy()
    );
}

/// The child gets no stdin unless we supply some, so nothing can block reading a
/// terminal a windowed app does not have.
#[test]
fn the_child_never_inherits_our_stdin() {
    let dir = TempDir::new().expect("temp dir");
    let script = dir.path().join("hledger");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *--version*) printf 'hledger 1.52, x\\n' ;;\n\
         *) cat; printf 'eof\\n' ;;\n\
         esac\n",
    )
    .expect("write stub");
    make_executable(&script);

    let hledger = resolve_stub(&script);
    let output = hledger
        .invoke(["print"])
        .timeout(std::time::Duration::from_secs(10))
        .run()
        .expect("a child with no stdin must reach EOF immediately");

    assert_eq!(output.stdout_lossy(), "eof\n");
}

/// A binary that is momentarily **busy** is waited for, not reported missing.
///
/// Linux refuses to `exec` a file that any process holds open for writing
/// (`ETXTBSY`), and a spawn failure resolves to `NotFound` — so without the retry
/// in `spawn_retrying_while_busy` the answer is "hledger was not found" about the
/// binary the user configured, which sends them looking for an installation that
/// is right there.
///
/// This is not hypothetical, and it is why the retry exists: every stub in this
/// file is written and then immediately run from a test binary running tests on
/// every core, and `fork` carries a write descriptor another thread had open at
/// that instant into the window. It surfaced as
/// `a_binary_that_predates_the_flag_still_reports_its_version` failing on a
/// 4-core Ubuntu CI runner — and only there, because the same binary on a 32-core
/// machine rarely loses the race.
///
/// Holding the descriptor open is the deterministic form of that race. macOS does
/// not implement `ETXTBSY` — a spawn against an open write descriptor simply
/// succeeds — so there is nothing to pin there and this test is Linux-only rather
/// than platform-agnostic with an assertion that proves nothing.
#[cfg(target_os = "linux")]
#[test]
fn a_binary_that_is_still_open_for_writing_is_waited_for() {
    let dir = TempDir::new().expect("temp dir");
    let script = write_stub(dir.path(), "hledger", "hledger 1.52, x");

    let held = std::fs::OpenOptions::new()
        .write(true)
        .open(&script)
        .expect("hold the stub open for writing");
    // Released well inside the retry budget, so the first spawn must fail with
    // ETXTBSY and a later one must succeed. Without the retry, `resolve` answers
    // the first failure.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(held);
    });

    let hledger = Hledger::resolve(&Prefs {
        hledger_path: Some(script.clone()),
        git_autocommit: None,
    })
    .expect("a momentarily busy binary must be waited for, not called missing");
    assert_eq!(hledger.path(), script);
}

/// Resolve a stub by absolute path through the ordinary preference route, so the
/// invocation tests exercise the same `Hledger` a real caller would hold.
fn resolve_stub(path: &Path) -> Hledger {
    Hledger::resolve(&Prefs {
        hledger_path: Some(path.to_path_buf()),
        git_autocommit: None,
    })
    .expect("the stub resolves")
}
