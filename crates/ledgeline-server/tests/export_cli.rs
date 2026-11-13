//! `ledgeline export` (plan 39) — the real binary, across the process
//! boundary: argument parsing, exit codes, stdout/stderr and the files it
//! writes.
//!
//! Hermetic: export runs no hledger, so nothing here needs it installed.
//! Every child gets its own `$LEDGELINE_CONFIG_DIR` and ignores the user's
//! `hledger.conf`, as `import_cli.rs` does, so no test reads the developer's
//! own settings.

mod common;

use common::fixtures_dir;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn export(config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ledgeline"))
        .arg("export")
        .args(args)
        .env("LEDGELINE_CONFIG_DIR", config)
        .env("LEDGELINE_IGNORE_USER_HLEDGER_CONF", "1")
        .output()
        .expect("the ledgeline binary runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn fixture(name: &str) -> String {
    fixtures_dir().join(name).to_string_lossy().into_owned()
}

#[test]
fn qif_writes_the_golden_files_and_prints_the_loss_report() {
    let scratch = TempDir::new().expect("tempdir");
    let out_dir = scratch.path().join("qif");
    let out_arg = out_dir.to_string_lossy().into_owned();
    let journal = fixture("export/multi-currency.journal");
    let output = export(scratch.path(), &["--qif", &out_arg, "-j", &journal]);
    assert!(output.status.success(), "{}", text(&output.stderr));

    let golden_dir = fixtures_dir().join("export/qif/multi-currency");
    let stdout = text(&output.stdout);
    let loss = std::fs::read_to_string(golden_dir.join("loss.txt")).expect("golden loss");
    assert!(stdout.starts_with(&loss), "{stdout}");
    for name in ["multi-currency-dollar.qif", "multi-currency-EUR.qif"] {
        let written = std::fs::read_to_string(out_dir.join(name)).expect("written");
        let golden = std::fs::read_to_string(golden_dir.join(name)).expect("golden");
        assert_eq!(written, golden, "{name}");
        assert!(stdout.contains(&format!("wrote {}", out_dir.join(name).display())));
    }
}

#[test]
fn an_existing_file_is_never_overwritten_without_force() {
    let scratch = TempDir::new().expect("tempdir");
    let out_dir = scratch.path().join("qif");
    std::fs::create_dir_all(&out_dir).expect("mkdir");
    let existing = out_dir.join("holding-dollar.qif");
    std::fs::write(&existing, "mine").expect("seed");
    let out_arg = out_dir.to_string_lossy().into_owned();
    let journal = fixture("export/holding.journal");

    let refused = export(scratch.path(), &["--qif", &out_arg, "-j", &journal]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        text(&refused.stderr).contains("--force"),
        "{}",
        text(&refused.stderr)
    );
    assert_eq!(std::fs::read_to_string(&existing).expect("read"), "mine");

    let forced = export(
        scratch.path(),
        &["--qif", &out_arg, "-j", &journal, "--force"],
    );
    assert!(forced.status.success(), "{}", text(&forced.stderr));
    assert_ne!(std::fs::read_to_string(&existing).expect("read"), "mine");

    let csv = scratch.path().join("out.csv");
    std::fs::write(&csv, "mine").expect("seed");
    let csv_arg = csv.to_string_lossy().into_owned();
    let refused = export(scratch.path(), &["--csv", &csv_arg, "-j", &journal]);
    assert_eq!(refused.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&csv).expect("read"), "mine");
}

#[test]
fn csv_matches_the_hledger_golden_on_disk_and_on_stdout() {
    let scratch = TempDir::new().expect("tempdir");
    let journal = fixture("export/splits-transfers.journal");
    let golden = std::fs::read_to_string(fixtures_dir().join("export/csv/splits-transfers.csv"))
        .expect("golden");

    let path = scratch.path().join("out.csv");
    let path_arg = path.to_string_lossy().into_owned();
    let output = export(scratch.path(), &["--csv", &path_arg, "-j", &journal]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(std::fs::read_to_string(&path).expect("written"), golden);
    assert_eq!(
        text(&output.stdout),
        format!("wrote 25 rows (9 transactions) to {}\n", path.display())
    );

    // `-` streams the CSV itself to stdout; the summary moves to stderr.
    let streamed = export(scratch.path(), &["--csv", "-", "-j", &journal]);
    assert!(streamed.status.success(), "{}", text(&streamed.stderr));
    assert_eq!(text(&streamed.stdout), golden);
    assert!(text(&streamed.stderr).contains("wrote 25 rows (9 transactions) to stdout"));
}

#[test]
fn arguments_are_validated() {
    let scratch = TempDir::new().expect("tempdir");
    let journal = fixture("sample.journal");
    // A format is required.
    assert_eq!(
        export(scratch.path(), &["-j", &journal]).status.code(),
        Some(2)
    );
    // Only one format at a time.
    assert_eq!(
        export(
            scratch.path(),
            &["--qif", "a", "--csv", "b.csv", "-j", &journal]
        )
        .status
        .code(),
        Some(2)
    );
    // Mirroring transfers means nothing to the CSV.
    assert_eq!(
        export(
            scratch.path(),
            &["--csv", "b.csv", "--mirror-transfers", "-j", &journal]
        )
        .status
        .code(),
        Some(2)
    );
    // A journal that does not exist is named.
    let missing = export(
        scratch.path(),
        &["--csv", "-", "-j", "/nonexistent/books.journal"],
    );
    assert_eq!(missing.status.code(), Some(1));
    assert!(text(&missing.stderr).contains("books.journal"));
}
