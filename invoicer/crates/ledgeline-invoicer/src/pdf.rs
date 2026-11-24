//! PDF export, the parts with no webview (invoicer PLAN 3c, ARCHITECTURE
//! §5.3): where a PDF goes, what it is named, how earlier exports are kept,
//! and the `sent_hash` mirror. The CLI's `pdf` and `POST
//! /api/invoices/{num}/pdf` both plan with [`plan`], print through MBR's
//! export (the `gui` feature's event loop), then file with [`complete`].
//!
//! **Where a PDF goes** ([`Destination`]):
//!
//! | The invoice           | Goes to                                                     |
//! |-----------------------|-------------------------------------------------------------|
//! | sent                  | **kept**: `pdf-dir/<the markdown's name>.pdf` (I12)         |
//! | draft or void         | a temporary file (the system's temp folder), for a look or a share |
//! | `pdf -o PATH`         | that file: a copy, nothing kept and no mirror written       |
//!
//! **Earlier exports are kept** (I12): a kept export over an existing file
//! renames that file first to `<name> <its export date>.pdf` (` (2)`, ` (3)`
//! … on a second export the same day), so `pdf-dir` is an audit trail of
//! what was sent. The new PDF is printed to a hidden file beside it and
//! moved into place only once it is a verified PDF: a failed export leaves
//! the folder as it was.
//!
//! **The mirror.** A kept export records the invoice's hash as `sent_hash`
//! ([`invoicer_core::sent`]); [`plan`] reports whether the file changed
//! since, which is the "differs from what was sent" warning.
//!
//! In a CLI-only build nothing prints, so only the tests and `NEEDS_GUI`
//! use this; it stays in every build so the naming rules are tested there.
#![cfg_attr(not(feature = "server"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use invoicer_core::doc::{Invoice, Status};
use invoicer_core::sent::{self, Recorded};
use invoicer_core::session::Session;
use invoicer_core::settings::Settings;

use crate::tera_scan;
use ledgeline_core::vault::{Vault, VaultPath};
use serde::{Deserialize, Serialize};

/// The folder under the system's temporary folder that draft PDFs (and
/// anything else not kept) go to.
const TEMP_FOLDER: &str = "ledgeline-invoicer";

/// What a template includes when it signals its own readiness for paper
/// (MBR-CHANGES M5). A template without it is printed on `load` plus fonts.
const EXPORT_READY_PARTIAL: &str = "_export_ready";

/// The shipped base's block holding the export-ready partial.
const READY_BLOCK: &str = "ready";

/// The message for a build that can't print (no `gui` feature).
pub(crate) const NEEDS_GUI: &str = "PDF export needs the desktop build: it prints through the \
system's webview, and this ledgeline-invoicer was built without the `gui` feature";

/// Where an export's PDF goes. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Destination {
    /// A sent invoice's PDF, kept in `pdf-dir`.
    Kept,
    /// A draft's (or a void's), in the temporary folder.
    Temp,
    /// Where `-o` said.
    Copy,
}

/// Why an export can't be planned or filed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PdfError {
    /// No such invoice.
    NotFound(String),
    /// The request can't be met (a bad name, a missing folder).
    Refused(String),
    /// Something failed on the way (the disk, the print system).
    Failed(String),
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) | Self::Refused(m) | Self::Failed(m) => f.write_str(m),
        }
    }
}

/// One export, planned: what to print and where it goes.
#[derive(Debug, Clone)]
pub(crate) struct ExportPlan {
    /// The invoice's number.
    pub number: String,
    /// The markdown file.
    pub document: VaultPath,
    pub destination: Destination,
    /// Where the PDF ends up.
    pub path: PathBuf,
    /// Where the exporter prints it: a hidden file beside [`Self::path`]
    /// when kept, else the path itself.
    pub staging: PathBuf,
    /// The file's [`sent::hash`] as read now.
    pub hash: String,
    /// Whether it changed since its PDF was last kept (`None`: never kept).
    pub changed_since_sent: Option<bool>,
    /// MBR's page for the invoice (`/invoices/A%20B/`).
    pub page: String,
    /// Print on `load` plus fonts: the template doesn't include MBR's
    /// export-ready partial.
    pub ready_on_load: bool,
}

/// Plan the export of `target` (a number or a path in the books folder).
/// `output` is `-o`; `temp` asks for a temporary file even for a sent
/// invoice (a share of the PDF, a look at it).
///
/// Makes the folder a kept PDF goes to; writes nothing else.
///
/// # Errors
/// [`PdfError`].
pub(crate) fn plan(
    session: &Session,
    target: &str,
    output: Option<&Path>,
    temp: bool,
) -> Result<ExportPlan, PdfError> {
    let doc = session
        .find(target)
        .map_err(|e| PdfError::NotFound(e.to_string()))?;
    let number = doc
        .number()
        .ok_or_else(|| PdfError::Refused(format!("{} has no `invoice_num`", doc.path)))?
        .to_string();
    let source = session
        .vault()
        .read(&doc.path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| PdfError::Failed(format!("{} can't be read as text", doc.path)))?;
    let invoice = Invoice::parse(source.as_str(), &session.settings().words);
    let status = invoice.header().status;
    let pdf_dir = local_dir(session, &session.settings().pdf_dir)?;
    let name = pdf_name(&doc.path)?;
    let (destination, path) = match output {
        Some(given) => (Destination::Copy, given.to_path_buf()),
        None if status == Some(Status::Sent) && !temp => (Destination::Kept, pdf_dir.join(&name)),
        None => (Destination::Temp, temp_dir()?.join(&name)),
    };
    let staging = match destination {
        Destination::Kept => {
            std::fs::create_dir_all(&pdf_dir)
                .map_err(|e| PdfError::Failed(format!("making {}: {e}", pdf_dir.display())))?;
            staging_path(&path)
        }
        Destination::Temp | Destination::Copy => path.clone(),
    };
    let ready_on_load = invoice
        .header()
        .template
        .as_deref()
        .is_some_and(|template| !signals_ready(session, template));
    Ok(ExportPlan {
        number,
        document: doc.path.clone(),
        destination,
        path,
        staging,
        hash: sent::hash(&source),
        changed_since_sent: sent::changed_since_sent(&invoice),
        page: page_url(&doc.path),
        ready_on_load,
    })
}

/// A folder named in the settings (`invoices/pdf`), on disk.
fn local_dir(session: &Session, dir: &str) -> Result<PathBuf, PdfError> {
    let path = Settings::vault_path(dir)
        .map_err(|e| PdfError::Refused(format!("pdf-dir = {dir:?}: {e}")))?;
    Ok(session.local_path(&path))
}

/// Whether the template at `template` (`/invoices/tpl/x.html`) includes
/// MBR's export-ready partial, through what it extends: a child of the
/// shipped base (PLAN Q1) signals when the base's `ready` block does and
/// the child doesn't replace it with one that doesn't (the fixture's
/// template empties it). Unreadable counts as no: printing on load beats
/// waiting out the timeout.
fn signals_ready(session: &Session, template: &str) -> bool {
    let Ok(path) = VaultPath::new(template.trim_start_matches('/')) else {
        return false;
    };
    signals_ready_at(session.vault(), &path, 0)
}

/// [`signals_ready`] for the template file `path`, `depth` parents up.
fn signals_ready_at(vault: &dyn Vault, path: &VaultPath, depth: usize) -> bool {
    // Tera's chains are short; a loop of `extends` is Tera's error to report.
    const MAX_DEPTH: usize = 8;
    let Ok(bytes) = vault.read(path) else {
        return false;
    };
    let source = String::from_utf8_lossy(&bytes);
    if let Some(ready) = tera_scan::block_body(&source, READY_BLOCK) {
        return ready.contains(EXPORT_READY_PARTIAL);
    }
    match tera_scan::extends(&source) {
        // MBR registers a template's siblings by file name: the parent is
        // beside it.
        Some(parent) if depth < MAX_DEPTH => path
            .parent()
            .unwrap_or_default()
            .child(parent)
            .is_ok_and(|parent| signals_ready_at(vault, &parent, depth + 1)),
        Some(_) => false,
        None => source.contains(EXPORT_READY_PARTIAL),
    }
}

/// The PDF's file name: the markdown's, with `.pdf` (ARCHITECTURE §5.3:
/// `2026-11-17 Customer X Invoice 105.pdf`).
///
/// # Errors
/// A file with no name to give.
pub(crate) fn pdf_name(document: &VaultPath) -> Result<String, PdfError> {
    Path::new(document.as_str())
        .file_stem()
        .map(|stem| format!("{}.pdf", stem.to_string_lossy()))
        .filter(|name| name != ".pdf")
        .ok_or_else(|| PdfError::Refused(format!("{document} has no file name")))
}

/// MBR's URL path for a note: the path without its extension, each segment
/// percent-encoded as `encodeURIComponent` does, with a trailing slash. The
/// SPA's `pageUrl` (`invoicer/web/src/lib/invoices.ts`) is the same rule.
pub(crate) fn page_url(document: &VaultPath) -> String {
    let path = document.as_str().trim_start_matches('/');
    let lower = path.to_ascii_lowercase();
    let bare = [".markdown", ".md"]
        .iter()
        .find(|ext| lower.ends_with(*ext))
        .map_or(path, |ext| &path[..path.len() - ext.len()]);
    let encoded: Vec<String> = bare.split('/').map(encode_component).collect();
    format!("/{}/", encoded.join("/"))
}

/// `encodeURIComponent`: everything but `A–Z a–z 0–9 - _ . ! ~ * ' ( )` as
/// UTF-8 `%XX`.
fn encode_component(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            );
        if keep {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The temporary folder (made, private to the user on Unix).
///
/// # Errors
/// When it can't be made.
pub(crate) fn temp_dir() -> Result<PathBuf, PdfError> {
    let dir = std::env::temp_dir().join(TEMP_FOLDER);
    let made = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    let made = made.and_then(|()| {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
    });
    made.map_err(|e| PdfError::Failed(format!("making {}: {e}", dir.display())))?;
    Ok(dir)
}

/// Distinguishes staging files made in one process.
static STAGING: AtomicU64 = AtomicU64::new(0);

/// A hidden file beside `path` to print into: hidden, so neither the
/// invoicer's scan nor MBR's lists it, and on the same filesystem, so the
/// final move is a rename.
fn staging_path(path: &Path) -> PathBuf {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = STAGING.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.export-{}-{n}.pdf", std::process::id()))
}

/// The name an earlier export is kept under: `<stem> <date>.pdf`, then
/// `<stem> <date> (2).pdf`, … the first that `taken` says is free.
pub(crate) fn kept_name(path: &Path, date: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    (1..)
        .map(|n| {
            let name = if n == 1 {
                format!("{stem} {date}.pdf")
            } else {
                format!("{stem} {date} ({n}).pdf")
            };
            path.with_file_name(name)
        })
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| path.to_path_buf())
}

/// Whether `file` is one of `stem`'s exports: `<stem>.pdf`, or a kept
/// earlier one (`<stem> YYYY-MM-DD.pdf`, `<stem> YYYY-MM-DD (n).pdf`).
pub(crate) fn is_export_of(file: &str, stem: &str) -> bool {
    let Some(rest) = file.strip_prefix(stem) else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(".pdf") else {
        return false;
    };
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = rest.strip_prefix(' ') else {
        return false;
    };
    let (date, count) = rest.split_at(rest.len().min(10));
    let dated = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });
    dated
        && (count.is_empty()
            || count
                .strip_prefix(" (")
                .and_then(|c| c.strip_suffix(')'))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
}

/// The local date `path` was last written, `YYYY-MM-DD`.
fn written_on(path: &Path) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        chrono::DateTime::<chrono::Local>::from(modified)
            .format("%Y-%m-%d")
            .to_string(),
    )
}

/// Move a printed PDF from `staging` to `path`, keeping whatever was there
/// as `<stem> <its date>.pdf` first. Returns where the earlier one went.
/// If the final move fails, the earlier file is put back.
///
/// # Errors
/// The renames.
pub(crate) fn file_kept(
    staging: &Path,
    path: &Path,
    today: &str,
) -> Result<Option<PathBuf>, PdfError> {
    let earlier = if path.exists() {
        let date = written_on(path).unwrap_or_else(|| today.to_string());
        let kept = kept_name(path, &date, Path::exists);
        std::fs::rename(path, &kept).map_err(|e| {
            PdfError::Failed(format!(
                "keeping {} as {}: {e}",
                path.display(),
                kept.display()
            ))
        })?;
        Some(kept)
    } else {
        None
    };
    if let Err(error) = std::fs::rename(staging, path) {
        if let Some(kept) = &earlier {
            let _ = std::fs::rename(kept, path);
        }
        let _ = std::fs::remove_file(staging);
        return Err(PdfError::Failed(format!(
            "moving the PDF to {}: {error}",
            path.display()
        )));
    }
    Ok(earlier)
}

/// A finished export, as the CLI's `--json` and the API answer it.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Exported {
    pub number: String,
    /// The markdown file.
    pub document: VaultPath,
    pub destination: Destination,
    /// The PDF's file name.
    pub file: String,
    /// The PDF in the books folder (kept exports only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept_at: Option<VaultPath>,
    /// The earlier export, kept under its date (`invoices/pdf/… 2026-11-10.pdf`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earlier: Option<VaultPath>,
    /// The PDF on disk. The CLI prints it; the API leaves it out (it names
    /// files by the books folder, never by full path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub bytes: u64,
    /// Whether the invoice changed since its PDF was last kept, before this
    /// export (`None`: never kept). `true` is the "differs from what was
    /// sent" warning.
    pub changed_since_sent: Option<bool>,
    /// What became of `sent_hash` (kept exports only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_hash: Option<Recorded>,
    pub warnings: Vec<String>,
}

/// File a printed PDF as `plan` says (see the module docs): move a kept one
/// into `pdf-dir`, keeping the earlier one, and record `sent_hash`.
/// `bytes` and `warnings` come from the print.
///
/// # Errors
/// [`PdfError`] when the PDF can't be moved into place. A mirror that can't
/// be written is a warning: the PDF is there.
pub(crate) fn complete(
    session: &mut Session,
    plan: &ExportPlan,
    bytes: u64,
    mut warnings: Vec<String>,
    today: &str,
) -> Result<Exported, PdfError> {
    let mut earlier = None;
    let mut recorded = None;
    let mut kept_at = None;
    if plan.destination == Destination::Kept {
        earlier = file_kept(&plan.staging, &plan.path, today)?;
        kept_at = session.vault().vault_path(&plan.path).ok();
        let (index, vault) = session.parts_mut();
        match sent::record(index, vault, &plan.document, &plan.hash) {
            Ok(Recorded::Stale) => {
                warnings.push(format!(
                    "{} changed while the PDF was printing; `sent_hash` still names the last \
                     kept export, so the next export will warn",
                    plan.document
                ));
                recorded = Some(Recorded::Stale);
            }
            Ok(done) => recorded = Some(done),
            Err(error) => warnings.push(format!(
                "the PDF is kept, but `sent_hash` wasn't written: {error}"
            )),
        }
    }
    if plan.changed_since_sent == Some(true) {
        warnings.push(format!(
            "invoice {} changed since its PDF was last kept: this PDF differs from what was sent",
            plan.number
        ));
    }
    Ok(Exported {
        number: plan.number.clone(),
        document: plan.document.clone(),
        destination: plan.destination,
        file: plan
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        earlier: earlier.and_then(|p| session.vault().vault_path(&p).ok()),
        kept_at,
        path: Some(plan.path.display().to_string()),
        bytes,
        changed_since_sent: plan.changed_since_sent,
        sent_hash: recorded,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(path: &str) -> VaultPath {
        VaultPath::new(path).expect("vault path")
    }

    /// P7 with the base (PLAN Q1): a child signals readiness through the
    /// base it extends, unless it empties the `ready` block, as the
    /// fixture's template does.
    #[test]
    fn the_export_ready_signal_is_read_through_the_base() {
        use crate::starters::{BASE, STARTERS};
        let vault = ledgeline_core::vault::MemVault::new()
            .with_file("tpl/_invoice_base.v1.html", BASE.source)
            .with_file("tpl/classic.html", STARTERS[0].source)
            .with_file("tpl/modern.html", STARTERS[1].source)
            .with_file(
                "tpl/myco.html",
                "{% extends \"_invoice_base.v1.html\" %}{% block ready %}{% endblock ready %}",
            )
            .with_file("tpl/orphan.html", "{% extends \"_gone.html\" %}")
            .with_file("tpl/loop.html", "{% extends \"loop.html\" %}")
            .with_file(
                "tpl/whole.html",
                "<head>{% include \"/.mbr/_export_ready.html\" %}</head>",
            )
            .with_file("tpl/plain.html", "<p>{{ frontmatter.invoice_num }}</p>")
            .with_file("root.html", "{% extends \"tpl/x.html\" %}");
        let signals = |p: &str| signals_ready_at(&vault, &vp(p), 0);
        for yes in [
            "tpl/classic.html",
            "tpl/modern.html",
            "tpl/whole.html",
            "tpl/_invoice_base.v1.html",
        ] {
            assert!(signals(yes), "{yes}");
        }
        for no in [
            "tpl/myco.html",
            "tpl/orphan.html",
            "tpl/loop.html",
            "tpl/plain.html",
            "root.html",
            "tpl/none.html",
        ] {
            assert!(!signals(no), "{no}");
        }
    }

    #[test]
    fn a_pdf_is_named_like_its_markdown() {
        assert_eq!(
            pdf_name(&vp("invoices/2026-11-17 Customer X Invoice 105.md")).unwrap(),
            "2026-11-17 Customer X Invoice 105.pdf"
        );
        assert_eq!(pdf_name(&vp("x/105.markdown")).unwrap(), "105.pdf");
    }

    #[test]
    fn page_urls_match_the_spa() {
        // The same cases as invoicer/web/src/lib/invoices.test.ts.
        assert_eq!(
            page_url(&vp("invoices/2026-11-10 Customer X Invoice 105.md")),
            "/invoices/2026-11-10%20Customer%20X%20Invoice%20105/"
        );
        assert_eq!(page_url(&vp("a/b#c?.markdown")), "/a/b%23c%3F/");
        assert_eq!(page_url(&vp("x.MD")), "/x/");
        assert_eq!(page_url(&vp("Café (1).md")), "/Caf%C3%A9%20(1)/");
    }

    #[test]
    fn earlier_exports_are_kept_under_their_date() {
        let path = Path::new("/books/invoices/pdf/2026-11-17 X Invoice 105.pdf");
        let none = kept_name(path, "2026-11-20", |_| false);
        assert_eq!(
            none,
            Path::new("/books/invoices/pdf/2026-11-17 X Invoice 105 2026-11-20.pdf")
        );
        let taken = [
            PathBuf::from("/books/invoices/pdf/2026-11-17 X Invoice 105 2026-11-20.pdf"),
            PathBuf::from("/books/invoices/pdf/2026-11-17 X Invoice 105 2026-11-20 (2).pdf"),
        ];
        assert_eq!(
            kept_name(path, "2026-11-20", |p| taken.iter().any(|t| t == p)),
            Path::new("/books/invoices/pdf/2026-11-17 X Invoice 105 2026-11-20 (3).pdf")
        );
    }

    #[test]
    fn exports_of_an_invoice_are_recognized_by_name() {
        let stem = "2026-11-10 X Invoice 10";
        for yes in [
            "2026-11-10 X Invoice 10.pdf",
            "2026-11-10 X Invoice 10 2026-11-12.pdf",
            "2026-11-10 X Invoice 10 2026-11-12 (2).pdf",
        ] {
            assert!(is_export_of(yes, stem), "{yes}");
        }
        for no in [
            "2026-11-10 X Invoice 105.pdf",
            "2026-11-10 X Invoice 105 2026-11-12.pdf",
            "2026-11-10 X Invoice 10 copy.pdf",
            "2026-11-10 X Invoice 10 2026-11-12 (x).pdf",
            "2026-11-10 X Invoice 10 2026-11-12 ().pdf",
            "2026-11-10 X Invoice 10.md",
            ".2026-11-10 X Invoice 10.export-1-0.pdf",
        ] {
            assert!(!is_export_of(no, stem), "{no}");
        }
    }

    #[test]
    fn filing_keeps_the_earlier_pdf_and_replaces_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("105.pdf");
        std::fs::write(&path, b"%PDF-old").unwrap();
        let staging = staging_path(&path);
        assert!(
            staging
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".105.export-")
        );
        std::fs::write(&staging, b"%PDF-new").unwrap();
        let earlier = file_kept(&staging, &path, "2026-11-23")
            .expect("filed")
            .expect("an earlier one");
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-new");
        assert_eq!(std::fs::read(&earlier).unwrap(), b"%PDF-old");
        assert!(!staging.exists());
        let name = earlier.file_name().unwrap().to_string_lossy().into_owned();
        assert!(is_export_of(&name, "105"), "{name}");

        // A first export: nothing to keep.
        let fresh = dir.path().join("106.pdf");
        let staging = staging_path(&fresh);
        std::fs::write(&staging, b"%PDF-1").unwrap();
        assert_eq!(file_kept(&staging, &fresh, "2026-11-23").unwrap(), None);
        assert!(fresh.exists());
    }

    #[test]
    fn a_failed_move_puts_the_earlier_pdf_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("105.pdf");
        std::fs::write(&path, b"%PDF-old").unwrap();
        let missing = dir.path().join(".105.export-gone.pdf");
        let error = file_kept(&missing, &path, "2026-11-23").unwrap_err();
        assert!(matches!(error, PdfError::Failed(_)), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-old");
        let others: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(others.len(), 1, "only 105.pdf is left");
    }
}
