//! `ledgeline export` and the File → Export menu (plan 39): write the open
//! journal as QIF (one file per commodity) or as posting-row CSV.
//!
//! The formats themselves are `ledgeline_core::export`'s — pure functions
//! from a parsed journal to text. This module owns only what touches the
//! filesystem: reading the journal, refusing to overwrite, and writing each
//! file whole (a temporary sibling renamed into place, so an interrupted
//! export never leaves half a file under the real name). The CLI and the
//! desktop menu both call [`export_qif_to_dir`] / [`export_csv_to_file`], so
//! the two cannot write different bytes.
//!
//! No hledger subprocess is involved, so neither `hledger.conf` nor the
//! bundled hledger version matters here.

use ledgeline_core::ParseError;
use ledgeline_core::export::csv::{CsvExport, export_csv};
use ledgeline_core::export::qif::{QifOptions, TransferPlacement, export_qif};
use ledgeline_core::export::{LossReport, load_journal};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Arguments of `ledgeline export`.
#[derive(clap::Args, Debug, Clone)]
#[command(group(
    clap::ArgGroup::new("format").required(true).args(["qif", "csv"])
))]
pub struct CliExport {
    /// Write QIF into this directory (created if missing): one file per
    /// commodity, named after the journal (`main-dollar.qif`, `main-EUR.qif`).
    /// What QIF cannot carry is reported on stdout.
    #[arg(long, value_name = "OUT_DIR")]
    qif: Option<PathBuf>,

    /// Write one row per posting, in hledger's `print -O csv` columns, to this
    /// file (`-` for stdout).
    #[arg(long, value_name = "OUT.csv")]
    csv: Option<PathBuf>,

    /// The journal to export. Its `include`d files are exported with it.
    #[arg(short = 'j', long)]
    journal: PathBuf,

    /// Replace output files that already exist.
    #[arg(long)]
    force: bool,

    /// QIF only: also write each transfer as its own record in the other
    /// account, as Quicken does. By default a transfer is written once and the
    /// importing app creates the other side.
    #[arg(long, conflicts_with = "csv")]
    mirror_transfers: bool,
}

/// Why an export did not happen. Every variant names the file involved.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// The journal could not be read or parsed.
    #[error("reading {path}: {source}")]
    Journal { path: String, source: ParseError },
    /// Output files exist and `force` was not given.
    #[error(
        "refusing to overwrite {}; pass --force to replace {}",
        .0.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "),
        if .0.len() == 1 { "it" } else { "them" }
    )]
    Exists(Vec<PathBuf>),
    /// Writing failed.
    #[error("writing {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    /// Arithmetic overflow summing a transaction (not reachable with real
    /// amounts).
    #[error("exporting: {0}")]
    Decimal(#[from] ledgeline_core::DecError),
}

/// What a QIF export wrote.
#[derive(Debug, Clone)]
pub struct QifWritten {
    /// The files written, in [`LossReport::files`] order.
    pub files: Vec<PathBuf>,
    /// What was dropped or reshaped.
    pub loss: LossReport,
}

fn read(journal: &Path) -> Result<ledgeline_core::Journal, ExportError> {
    load_journal(journal).map_err(|source| ExportError::Journal {
        path: journal.display().to_string(),
        source,
    })
}

/// The QIF files would be named after this: the journal's file stem.
fn stem(journal: &Path) -> String {
    journal.file_stem().map_or_else(
        || "ledgeline".to_string(),
        |s| s.to_string_lossy().into_owned(),
    )
}

/// The paths a QIF export of `journal` into `dir` would write, without writing
/// anything — what the GUI checks before asking whether to replace them.
///
/// # Errors
/// As [`export_qif_to_dir`], short of writing.
pub fn qif_targets(journal: &Path, dir: &Path) -> Result<Vec<PathBuf>, ExportError> {
    let parsed = read(journal)?;
    let out = export_qif(&parsed, &qif_options(journal, TransferPlacement::Once))?;
    Ok(out.files.iter().map(|f| dir.join(&f.file_name)).collect())
}

fn qif_options(journal: &Path, transfers: TransferPlacement) -> QifOptions {
    QifOptions {
        name: stem(journal),
        transfers,
    }
}

/// Export `journal` as QIF into `dir` (created if missing).
///
/// # Errors
/// [`ExportError::Exists`] when any target exists and `force` is false —
/// checked for every file before any is written — and the read/write errors.
pub fn export_qif_to_dir(
    journal: &Path,
    dir: &Path,
    force: bool,
    transfers: TransferPlacement,
) -> Result<QifWritten, ExportError> {
    let parsed = read(journal)?;
    let out = export_qif(&parsed, &qif_options(journal, transfers))?;
    let targets: Vec<(PathBuf, &str)> = out
        .files
        .iter()
        .map(|file| (dir.join(&file.file_name), file.text.as_str()))
        .collect();
    refuse_existing(targets.iter().map(|(path, _)| path.as_path()), force)?;
    std::fs::create_dir_all(dir).map_err(|source| ExportError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    for (path, text) in &targets {
        write_whole(path, text)?;
    }
    Ok(QifWritten {
        files: targets.into_iter().map(|(path, _)| path).collect(),
        loss: out.loss,
    })
}

/// Export `journal` as posting-row CSV to `path`.
///
/// # Errors
/// [`ExportError::Exists`] when `path` exists and `force` is false, and the
/// read/write errors.
pub fn export_csv_to_file(
    journal: &Path,
    path: &Path,
    force: bool,
) -> Result<CsvExport, ExportError> {
    let parsed = read(journal)?;
    refuse_existing(std::iter::once(path), force)?;
    let out = export_csv(&parsed);
    write_whole(path, &out.text)?;
    Ok(out)
}

fn refuse_existing<'a>(
    paths: impl Iterator<Item = &'a Path>,
    force: bool,
) -> Result<(), ExportError> {
    if force {
        return Ok(());
    }
    let existing: Vec<PathBuf> = paths
        .filter(|path| path.exists())
        .map(Path::to_path_buf)
        .collect();
    if existing.is_empty() {
        Ok(())
    } else {
        Err(ExportError::Exists(existing))
    }
}

/// Write `text` to a hidden sibling of `path`, then rename it into place.
fn write_whole(path: &Path, text: &str) -> Result<(), ExportError> {
    let io = |source| ExportError::Io {
        path: path.display().to_string(),
        source,
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = path.with_file_name(format!(".{name}.ledgeline-export"));
    std::fs::write(&temp, text).map_err(io)?;
    std::fs::rename(&temp, path).map_err(|source| {
        let _ = std::fs::remove_file(&temp);
        io(source)
    })
}

/// Run `ledgeline export`: write the files and print what happened.
///
/// QIF prints the loss report to stdout. CSV prints a one-line summary to
/// stdout, or — when writing the CSV itself to stdout with `--csv -` — to
/// stderr, so the data stream stays clean.
///
/// # Errors
/// The [`ExportError`] as a sentence for `main` to print.
pub fn run_cli_export(args: &CliExport) -> Result<(), String> {
    let journal = &args.journal;
    if let Some(dir) = &args.qif {
        let transfers = if args.mirror_transfers {
            TransferPlacement::Mirrored
        } else {
            TransferPlacement::Once
        };
        let written = export_qif_to_dir(journal, dir, args.force, transfers)
            .map_err(|error| error.to_string())?;
        print!("{}", written.loss.render());
        for file in &written.files {
            println!("wrote {}", file.display());
        }
        return Ok(());
    }
    let Some(path) = &args.csv else {
        // clap's required "format" group makes this unreachable.
        return Err("pass --qif OUT_DIR or --csv OUT.csv".to_string());
    };
    if path.as_os_str() == "-" {
        let out = export_csv(&read(journal).map_err(|e| e.to_string())?);
        std::io::stdout()
            .write_all(out.text.as_bytes())
            .map_err(|error| format!("writing stdout: {error}"))?;
        eprintln!("{}", csv_summary(&out, "stdout"));
        return Ok(());
    }
    let out = export_csv_to_file(journal, path, args.force).map_err(|e| e.to_string())?;
    println!("{}", csv_summary(&out, &path.display().to_string()));
    Ok(())
}

/// `wrote 12 rows (5 transactions) to out.csv`.
#[must_use]
pub fn csv_summary(out: &CsvExport, destination: &str) -> String {
    format!(
        "wrote {} row{} ({} transaction{}) to {destination}",
        out.rows,
        if out.rows == 1 { "" } else { "s" },
        out.transactions,
        if out.transactions == 1 { "" } else { "s" },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOURNAL: &str = "account assets:bank  ; type: C\naccount expenses  ; type: X\n\n\
                           2026-01-02 Shop\n    expenses:food  $5\n    assets:bank\n";

    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = dir.path().join("books.journal");
        std::fs::write(&journal, JOURNAL).expect("write journal");
        (dir, journal)
    }

    #[test]
    fn qif_writes_one_file_named_after_the_journal_and_refuses_to_overwrite() {
        let (dir, journal) = scratch();
        let out_dir = dir.path().join("out");
        let written =
            export_qif_to_dir(&journal, &out_dir, false, TransferPlacement::Once).expect("exports");
        assert_eq!(written.files, [out_dir.join("books-dollar.qif")]);
        assert_eq!(
            qif_targets(&journal, &out_dir).expect("targets"),
            written.files
        );
        let error = export_qif_to_dir(&journal, &out_dir, false, TransferPlacement::Once)
            .expect_err("refuses");
        assert!(matches!(error, ExportError::Exists(ref paths) if paths == &written.files));
        assert!(error.to_string().contains("--force"), "{error}");
        export_qif_to_dir(&journal, &out_dir, true, TransferPlacement::Once).expect("forced");
        // No temporary file is left behind.
        let names: Vec<String> = std::fs::read_dir(&out_dir)
            .expect("dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["books-dollar.qif"]);
    }

    #[test]
    fn csv_refuses_to_overwrite_unless_forced() {
        let (dir, journal) = scratch();
        let path = dir.path().join("out.csv");
        let out = export_csv_to_file(&journal, &path, false).expect("exports");
        assert_eq!((out.transactions, out.rows), (1, 2));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), out.text);
        assert!(matches!(
            export_csv_to_file(&journal, &path, false),
            Err(ExportError::Exists(_))
        ));
        export_csv_to_file(&journal, &path, true).expect("forced");
        assert_eq!(csv_summary(&out, "x"), "wrote 2 rows (1 transaction) to x");
    }

    #[test]
    fn a_missing_journal_is_named_in_the_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope.journal");
        let error =
            export_csv_to_file(&missing, &dir.path().join("o.csv"), false).expect_err("missing");
        assert!(error.to_string().contains("nope.journal"), "{error}");
    }
}
