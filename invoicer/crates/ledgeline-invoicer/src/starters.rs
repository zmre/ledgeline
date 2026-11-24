//! The starter templates (invoicer PLAN 3b, ARCHITECTURE §3.5): two MBR page
//! templates shipped in the binary (`templates/` beside this crate's
//! `Cargo.toml`) and copied into the books' `templates-dir` (default
//! `invoices/tpl/`) the first time an invoice uses one. From then on they are
//! the user's files: a copy never replaces a file that is already there.
//!
//! "Uses one" means `new` (and, from Phase 4, the editor) writing an invoice
//! whose `template:` is a starter's path in that folder: named with
//! `--template`, by the customer's `invoice_template`, or picked because the
//! books have no template at all, when the default starter ([`DEFAULT`]) is
//! used. The copy is made once the invoice is written (a dry run copies
//! nothing). A preview with a starter that isn't copied yet renders the
//! shipped text and writes nothing (`POST /api/preview`).
//!
//! It also means choosing one for an existing invoice (`set
//! template=…`, the app's "Use this template") and naming one by hand: an
//! invoice whose `template:` is a starter's path that isn't there yet is
//! what the user plainly meant, so the app copies the starter in when it
//! opens the books and when the watcher sees such a file
//! ([`install_referenced`]). Without the copy, MBR's page for the invoice
//! is a "template was not found" error (PLAN, Phase 3 feedback F9).

#[cfg(feature = "server")]
use invoicer_core::session::Session;
use ledgeline_core::vault::{Vault, VaultError, VaultPath};

/// A template shipped in the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Starter {
    /// Its file name in the templates folder.
    pub file: &'static str,
    /// Its display name (the `{# invoicer: name=… #}` line says the same).
    pub name: &'static str,
    /// Its Tera source.
    pub source: &'static str,
}

/// The starters, the default first.
pub(crate) const STARTERS: [Starter; 2] = [
    Starter {
        file: "classic.html",
        name: "Classic",
        source: include_str!("../templates/classic.html"),
    },
    Starter {
        file: "modern.html",
        name: "Modern",
        source: include_str!("../templates/modern.html"),
    },
];

/// The starter a books folder with no template gets.
pub(crate) const DEFAULT: &Starter = &STARTERS[0];

/// A starter's file in the templates folder `dir` (`templates-dir`).
///
/// # Errors
/// When `dir` isn't a path inside the books folder.
pub(crate) fn path_in(dir: &str, starter: &Starter) -> Result<VaultPath, VaultError> {
    VaultPath::new(dir.trim().trim_matches('/'))
        .and_then(|d| d.child(starter.file))
        .map_err(|e| {
            VaultError::from_io(
                dir,
                std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()),
            )
        })
}

/// The `template:` value naming `starter` in `dir`: `/invoices/tpl/classic.html`.
pub(crate) fn template_value(dir: &str, starter: &Starter) -> Option<String> {
    path_in(dir, starter).ok().map(|p| format!("/{p}"))
}

/// The starter a `template:` value names, when it is a starter's path in
/// `dir` (`/invoices/tpl/modern.html`; the leading `/` is optional).
pub(crate) fn named(dir: &str, template: &str) -> Option<&'static Starter> {
    let wanted = template.trim().trim_start_matches('/');
    STARTERS
        .iter()
        .find(|s| path_in(dir, s).ok().is_some_and(|p| p.as_str() == wanted))
}

/// What [`install`] did with a starter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Installed {
    /// The file.
    pub path: VaultPath,
    /// `true` when it was copied now; `false` when a file was already there
    /// (left exactly as it was).
    pub written: bool,
}

/// Copy `starter` into `dir` unless a file is already there. Never
/// overwrites: the write is a create-new, so even a file that appears
/// between the check and the write is kept.
///
/// # Errors
/// When the folder can't be made or the file can't be written.
pub(crate) fn install(
    vault: &dyn Vault,
    dir: &str,
    starter: &Starter,
) -> Result<Installed, VaultError> {
    let path = path_in(dir, starter)?;
    if vault.stat(&path)?.is_some() {
        return Ok(Installed {
            path,
            written: false,
        });
    }
    if let Some(parent) = path.parent().filter(|p| !p.is_root()) {
        vault.create_dir_all(&parent)?;
    }
    match vault.create_new(&path, starter.source.as_bytes()) {
        Ok(_) => Ok(Installed {
            path,
            written: true,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(Installed {
            path,
            written: false,
        }),
        Err(error) => Err(error),
    }
}

/// Copy in the starter `template` names (a `template:` value), if it names
/// one: [`install`] for it, or `Ok(None)` when it names no starter.
///
/// # Errors
/// As [`install`].
pub(crate) fn install_named(
    vault: &dyn Vault,
    dir: &str,
    template: &str,
) -> Result<Option<Installed>, VaultError> {
    named(dir, template)
        .map(|starter| install(vault, dir, starter))
        .transpose()
}

/// Copy in every starter the documents at `paths` (or all documents) name
/// by `template:` and that isn't in the books yet, then index the copies so
/// the documents' `unknown-template` warning goes at once. Returns the
/// files copied now and, for each copy that failed, the file and why.
#[cfg(feature = "server")]
pub(crate) fn install_referenced(
    session: &mut Session,
    paths: Option<&[VaultPath]>,
) -> (Vec<VaultPath>, Vec<(String, String)>) {
    let dir = session.settings().templates_dir.clone();
    let mut wanted: Vec<&'static Starter> = Vec::new();
    for doc in session.index().docs() {
        if paths.is_some_and(|only| !only.contains(&doc.path)) {
            continue;
        }
        if let Some(starter) = doc.header.template.as_deref().and_then(|t| named(&dir, t))
            && !wanted.contains(&starter)
        {
            wanted.push(starter);
        }
    }
    let mut copied = Vec::new();
    let mut failed = Vec::new();
    for starter in wanted {
        match install(session.vault(), &dir, starter) {
            Ok(done) if done.written => copied.push(done.path),
            Ok(_) => {}
            Err(error) => failed.push((starter.file.to_string(), error.to_string())),
        }
    }
    if !copied.is_empty() {
        let (index, vault) = session.parts_mut();
        index.refresh(vault, &copied);
    }
    (copied, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicer_core::templates::{self, Template};
    use ledgeline_core::vault::MemVault;

    #[test]
    fn every_starter_is_a_template_with_the_house_rules() {
        for starter in &STARTERS {
            let path = VaultPath::new(&format!("invoices/tpl/{}", starter.file)).unwrap();
            let template = Template::from_source(path, starter.source)
                .unwrap_or_else(|| panic!("{} mentions invoice_num", starter.file));
            assert_eq!(template.name, starter.name);
            assert!(template.supports(invoicer_core::doc::DocType::Invoice));
            let head = templates::header(starter.source).expect("an invoicer: line");
            assert_eq!(head.name.as_deref(), Some(starter.name));
            // The books column rule, exactly as ARCHITECTURE §3.3 gives it.
            assert!(
                starter.source.contains(
                    "table:has(thead th:last-child > em:only-child) :is(th, td):last-child { display: none; }"
                ),
                "{}: the books-column rule",
                starter.file
            );
            // Print CSS and MBR's export-ready partial (MBR-CHANGES M5).
            assert!(starter.source.contains("@page {"), "{}", starter.file);
            assert!(starter.source.contains("@media print"), "{}", starter.file);
            assert!(
                starter
                    .source
                    .contains(r#"{% include "/.mbr/_export_ready.html" %}"#),
                "{}",
                starter.file
            );
            // MBR's chrome when MBR serves the page, and the sheet alone where
            // scripts are off (the app's frame) or on paper.
            for part in [
                "{%- set chrome = server_mode | default(value=false) -%}",
                r#"{% include "/.mbr/_nav.html" %}"#,
                r#"{% include "/.mbr/_scripts.html" %}"#,
                "body.invoice-page > :not(main) { display: none !important; }",
                r#"<main class="invoice-main">"#,
            ] {
                assert!(starter.source.contains(part), "{}: {part}", starter.file);
            }
            // No remote fonts or scripts: an invoice renders offline.
            assert!(!starter.source.contains("http"), "{}", starter.file);
        }
    }

    #[test]
    fn a_starter_is_named_by_its_path_in_the_templates_folder() {
        assert_eq!(
            template_value("invoices/tpl", DEFAULT).as_deref(),
            Some("/invoices/tpl/classic.html")
        );
        assert_eq!(
            template_value("/docs/tpl/", &STARTERS[1]).as_deref(),
            Some("/docs/tpl/modern.html")
        );
        assert_eq!(
            named("invoices/tpl", "/invoices/tpl/modern.html").map(|s| s.name),
            Some("Modern")
        );
        assert_eq!(
            named("invoices/tpl", "invoices/tpl/classic.html").map(|s| s.name),
            Some("Classic")
        );
        assert!(named("invoices/tpl", "/elsewhere/classic.html").is_none());
        assert!(named("invoices/tpl", "/invoices/tpl/mine.html").is_none());
        assert!(path_in("../outside", DEFAULT).is_err());
    }

    #[test]
    fn install_copies_once_and_never_overwrites() {
        let vault = MemVault::new().with_file("invoices/tpl/modern.html", "mine");
        let first = install(&vault, "invoices/tpl", DEFAULT).expect("installs");
        assert!(first.written);
        assert_eq!(first.path.as_str(), "invoices/tpl/classic.html");
        assert_eq!(
            vault.read_to_string(&first.path).unwrap(),
            DEFAULT.source,
            "the shipped text, byte for byte"
        );

        // The user's edit survives a second use.
        vault
            .write_atomic(&first.path, b"edited by hand")
            .expect("edits");
        let again = install(&vault, "invoices/tpl", DEFAULT).expect("runs");
        assert!(!again.written);
        assert_eq!(vault.read_to_string(&first.path).unwrap(), "edited by hand");

        // A file already at a starter's path is the user's, not ours.
        let modern = install(&vault, "invoices/tpl", &STARTERS[1]).expect("runs");
        assert!(!modern.written);
        assert_eq!(vault.read_to_string(&modern.path).unwrap(), "mine");
    }
}
