//! File → Export (plan 39): the submenu, and the native dialogs around one
//! export of the journal that is open.
//!
//! Kept out of `gui.rs` so the menu bar's own code only gains the lines that
//! place the submenu and route its clicks here. The export itself is
//! [`ledgeline::export_qif_to_dir`] / [`ledgeline::export_csv_to_file`] — the
//! same functions `ledgeline export` calls, so the menu and the command line
//! write the same bytes.
//!
//! Everything after the click runs on a background thread, the way the
//! File → Open picker does: the dialogs block, and so does the export of a
//! large journal.

use std::path::{Path, PathBuf};

use ledgeline::{ExportError, csv_summary, export_csv_to_file, export_qif_to_dir, qif_targets};
use ledgeline_core::export::qif::DEFAULT_TRANSFER_PLACEMENT;
use muda::{MenuId, MenuItem, Submenu};

const QIF_ID: &str = "export-qif";
const CSV_ID: &str = "export-csv";

/// The File → Export submenu.
pub(super) fn submenu() -> Submenu {
    let export = Submenu::new("&Export", true);
    let qif = MenuItem::with_id(QIF_ID, "&QIF…", true, None);
    let csv = MenuItem::with_id(CSV_ID, "&CSV (posting rows)…", true, None);
    super::log_menu("export menu", export.append_items(&[&qif, &csv]));
    export
}

/// Start the export `id` names for `journal`, if it is one of ours. Returns
/// whether it was.
pub(super) fn handle(id: &MenuId, journal: &Path) -> bool {
    let journal = journal.to_path_buf();
    if *id == QIF_ID {
        std::thread::spawn(move || export_qif(&journal));
        true
    } else if *id == CSV_ID {
        std::thread::spawn(move || export_csv(&journal));
        true
    } else {
        false
    }
}

fn export_qif(journal: &Path) {
    let Some(dir) = rfd::FileDialog::new()
        .set_title("Export QIF: choose a folder")
        .set_directory(journal.parent().unwrap_or(Path::new(".")))
        .pick_folder()
    else {
        return;
    };
    // The folder picker cannot confirm an overwrite the way a save panel
    // does, so ask before replacing an earlier export.
    let existing: Vec<PathBuf> = match qif_targets(journal, &dir) {
        Ok(targets) => targets.into_iter().filter(|path| path.exists()).collect(),
        Err(error) => return show_error(&error),
    };
    if !existing.is_empty() && !confirm_replace(&existing) {
        return;
    }
    match export_qif_to_dir(journal, &dir, true, DEFAULT_TRANSFER_PLACEMENT) {
        Ok(written) => show_info("QIF export finished", &written.loss.render()),
        Err(error) => show_error(&error),
    }
}

fn export_csv(journal: &Path) {
    let suggested = journal.file_stem().map_or_else(
        || "ledgeline.csv".to_string(),
        |stem| format!("{}.csv", stem.to_string_lossy()),
    );
    let Some(path) = rfd::FileDialog::new()
        .set_title("Export CSV (one row per posting)")
        .set_directory(journal.parent().unwrap_or(Path::new(".")))
        .set_file_name(suggested)
        .add_filter("CSV", &["csv"])
        .save_file()
    else {
        return;
    };
    // The save panel has already confirmed any overwrite.
    match export_csv_to_file(journal, &path, true) {
        Ok(out) => show_info(
            "CSV export finished",
            &csv_summary(&out, &path.display().to_string()),
        ),
        Err(error) => show_error(&error),
    }
}

fn confirm_replace(existing: &[PathBuf]) -> bool {
    let names: Vec<String> = existing
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Replace the earlier export?")
        .set_description(format!(
            "These files already exist and will be replaced:\n{}",
            names.join("\n")
        ))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

fn show_info(title: &str, text: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn show_error(error: &ExportError) {
    eprintln!("ledgeline: export failed: {error}");
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Export failed")
        .set_description(error.to_string())
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
