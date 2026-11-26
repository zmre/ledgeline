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
//!
//! **The base** (PLAN Q1, option B): the starters are small children of
//! one shipped base, [`BASE`] (`_invoice_base.v1.html`), which holds the
//! page around the sheet. Copying a starter in copies the base beside it
//! first (create-new, like the starter), so the starter never lands
//! without what it extends. The base is versioned in its name because the
//! user owns the copy: a fix ships as `v2`, and children of `v1` keep it.
//! A preview through a starter that isn't copied in, in books that don't
//! have the base either, renders the starter folded into the shipped base
//! ([`preview_source`]): MBR reads a template's siblings from disk only.

use crate::tera_scan::{blocks, outermost};
use invoicer_core::edit::{self, SetOutcome, WriteError};
use invoicer_core::frontmatter::FieldEdit;
use invoicer_core::session::Session;
use ledgeline_core::vault::{Vault, VaultError, VaultPath};
use serde_json::Value;

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

/// The shipped base the starters extend (PLAN Q1): never listed as a
/// template (its name starts with `_`), copied in with a starter.
pub(crate) const BASE: Starter = Starter {
    file: "_invoice_base.v1.html",
    name: "Invoice base, version 1",
    source: include_str!("../templates/_invoice_base.v1.html"),
};

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
    /// The base ([`BASE`]) when it was copied in now, with the starter.
    pub base: Option<VaultPath>,
}

impl Installed {
    /// Every file copied in now: the base first, then the starter.
    pub(crate) fn written_files(&self) -> Vec<VaultPath> {
        self.base
            .iter()
            .cloned()
            .chain(self.written.then(|| self.path.clone()))
            .collect()
    }
}

/// Copy `starter` into `dir` unless a file is already there, with the base
/// it extends ([`BASE`]) first when the starter is copied now. Never
/// overwrites: each write is a create-new, so even a file that appears
/// between the check and the write is kept. A starter already there is
/// left alone, and so is a missing base beside it: the user's file may no
/// longer need one.
///
/// # Errors
/// When the folder can't be made or a file can't be written.
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
            base: None,
        });
    }
    let base_path = path_in(dir, &BASE)?;
    let base = create_new(vault, &base_path, BASE.source)?.then_some(base_path);
    let written = create_new(vault, &path, starter.source)?;
    Ok(Installed {
        path,
        written,
        base,
    })
}

/// Write `source` at `path` unless a file is there: `true` when written.
fn create_new(vault: &dyn Vault, path: &VaultPath, source: &str) -> Result<bool, VaultError> {
    if vault.stat(path)?.is_some() {
        return Ok(false);
    }
    if let Some(parent) = path.parent().filter(|p| !p.is_root()) {
        vault.create_dir_all(&parent)?;
    }
    match vault.create_new(path, source.as_bytes()) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

/// The text to preview `starter` with, when it isn't copied into `dir`
/// yet: its shipped source when the base it extends is in the books
/// (MBR reads it from disk beside the template, as it will once the
/// starter is copied in), else the starter folded into the shipped base
/// ([`fold_into_base`]), since MBR can't be handed a sibling that isn't
/// on disk.
#[cfg_attr(
    not(feature = "server"),
    allow(
        dead_code,
        reason = "only the app's preview folds a starter into the base"
    )
)]
pub(crate) fn preview_source(vault: &dyn Vault, dir: &str, starter: &Starter) -> String {
    let base_on_disk = path_in(dir, &BASE)
        .ok()
        .and_then(|p| vault.stat(&p).ok().flatten())
        .is_some();
    if base_on_disk || !extends_base(starter.source) {
        return starter.source.to_string();
    }
    fold_into_base(starter.source, BASE.source).unwrap_or_else(|| starter.source.to_string())
}

/// Whether `source` extends the shipped base.
#[cfg_attr(
    not(feature = "server"),
    allow(
        dead_code,
        reason = "only the app's preview folds a starter into the base"
    )
)]
fn extends_base(source: &str) -> bool {
    crate::tera_scan::extends(source) == Some(BASE.file)
}

/// `child` (a template that extends `base`) as one template: `base`'s
/// text with each block `child` defines put in place of `base`'s own. The
/// same page Tera renders from the two, for the shipped starters, which
/// define only top-level blocks and don't call `super()` (a test renders
/// both ways). `None` when `child` defines a block `base` lacks, calls
/// `super()`, or the tags don't pair.
#[cfg_attr(
    not(feature = "server"),
    allow(
        dead_code,
        reason = "only the app's preview folds a starter into the base"
    )
)]
pub(crate) fn fold_into_base(child: &str, base: &str) -> Option<String> {
    if child.contains("super()") {
        return None;
    }
    let mut out = base.to_string();
    for block in outermost(blocks(child)?) {
        let body = &child[block.body.clone()];
        let target = blocks(&out)?.into_iter().find(|b| b.name == block.name)?;
        out.replace_range(target.body, body);
    }
    Some(out)
}

/// Copy `starter` in once a document that uses it is written, and index
/// the copies so its `unknown-template` warning goes at once. The one step
/// `new`, `set template=…`, and the app's New, Use template and save all
/// take after their write (RA-5).
///
/// `Ok(Some(path))` when copied now, `Ok(None)` when a file was already
/// there.
///
/// # Errors
/// The warning to show: the document is written either way, so a failed
/// copy is never an error a retry would answer by writing it again (RA-7).
/// The app copies it in when it next opens the books.
pub(crate) fn copy_in(
    session: &mut Session,
    starter: &Starter,
) -> Result<Option<VaultPath>, String> {
    let dir = session.settings().templates_dir.clone();
    let installed = install(session.vault(), &dir, starter).map_err(|error| {
        format!(
            "the {} starter template wasn't copied into {dir}: {error}; the invoice names it, \
             so copy it in (or choose another) before printing",
            starter.name
        )
    })?;
    let (index, vault) = session.parts_mut();
    index.refresh(vault, &installed.written_files());
    Ok(installed.written.then_some(installed.path))
}

/// [`copy_in`] for the starter a `template:` value names; `Ok(None)` when it
/// names none.
///
/// # Errors
/// As [`copy_in`].
pub(crate) fn copy_in_named(
    session: &mut Session,
    template: &str,
) -> Result<Option<VaultPath>, String> {
    let dir = session.settings().templates_dir.clone();
    match named(&dir, template) {
        Some(starter) => copy_in(session, starter),
        None => Ok(None),
    }
}

/// What [`set`] did.
#[derive(Debug)]
pub(crate) struct Set {
    /// The frontmatter write.
    pub outcome: SetOutcome,
    /// The starter copied in now, when a `template:` it sets names one.
    pub copied: Option<VaultPath>,
    /// Why a starter wasn't copied ([`copy_in`]).
    pub warnings: Vec<String>,
}

/// `set`: write `edits` to the document at `path`, then [`copy_in`] any
/// starter a `template:` among them names. The CLI's `set` and the app's
/// Use template.
///
/// # Errors
/// [`WriteError`] from the write (nothing copied).
pub(crate) fn set(
    session: &mut Session,
    path: &VaultPath,
    edits: &[FieldEdit],
) -> Result<Set, WriteError> {
    let (index, vault) = session.parts_mut();
    let outcome = edit::set(index, vault, path, edits)?;
    let mut done = Set {
        outcome,
        copied: None,
        warnings: Vec::new(),
    };
    for change in edits {
        if let FieldEdit::Set(key, Value::String(template)) = change
            && key == "template"
        {
            match copy_in_named(session, template) {
                Ok(Some(path)) => done.copied = Some(path),
                Ok(None) => {}
                Err(warning) => done.warnings.push(warning),
            }
        }
    }
    Ok(done)
}

/// Copy in every starter the documents at `paths` (or all documents) name
/// by `template:` and that isn't in the books yet, then index the copies so
/// the documents' `unknown-template` warning goes at once. Returns the
/// files copied now (a starter's base too, [`install`]) and, for each copy
/// that failed, the file and why.
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
            Ok(done) => copied.extend(done.written_files()),
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
    fn every_starter_is_a_small_child_of_the_base() {
        for starter in &STARTERS {
            let path = VaultPath::new(&format!("invoices/tpl/{}", starter.file)).unwrap();
            let template = Template::from_source(path, starter.source)
                .unwrap_or_else(|| panic!("{} is a template", starter.file));
            assert_eq!(template.name, starter.name);
            assert!(template.supports(invoicer_core::doc::DocType::Invoice));
            let head = templates::header(starter.source).expect("an invoicer: line");
            assert_eq!(head.name.as_deref(), Some(starter.name));
            // The `extends` line right after the first (Tera allows only one
            // comment before it).
            assert_eq!(
                starter.source.lines().nth(1),
                Some(r#"{% extends "_invoice_base.v1.html" %}"#),
                "{}",
                starter.file
            );
            assert!(extends_base(starter.source), "{}", starter.file);
            // What the base owns isn't copied into the children.
            for owned in [
                "_nav.html",
                "<noscript>",
                "books column: last column",
                "<main",
            ] {
                assert!(!starter.source.contains(owned), "{}: {owned}", starter.file);
            }
            // Print CSS of its own, and no remote fonts or scripts: an
            // invoice renders offline.
            assert!(starter.source.contains("@page {"), "{}", starter.file);
            assert!(!starter.source.contains("http"), "{}", starter.file);
            // The preview's fold (no base in the books) works on it.
            assert!(
                fold_into_base(starter.source, BASE.source).is_some(),
                "{}",
                starter.file
            );
        }
    }

    #[test]
    fn the_base_has_the_house_rules_and_is_no_template() {
        let base = BASE.source;
        // The books column rule, exactly as ARCHITECTURE §3.3 gives it.
        assert!(base.contains(
            "table:has(thead th:last-child > em:only-child) :is(th, td):last-child { display: none; }"
        ));
        // Print CSS and MBR's export-ready partial (MBR-CHANGES M5).
        assert!(base.contains("@page {"));
        assert!(base.contains("@media print"));
        assert!(base.contains(r#"{% include "/.mbr/_export_ready.html" %}"#));
        // MBR's chrome when MBR serves the page, and the sheet alone where
        // scripts are off (the app's frame) or on paper.
        for part in [
            "{%- set chrome = server_mode | default(value=false) -%}",
            r#"{% include "/.mbr/_nav.html" %}"#,
            r#"{% include "/.mbr/_scripts.html" %}"#,
            "body.invoice-page > :not(main) { display: none !important; }",
            r#"<main class="invoice-main">"#,
        ] {
            assert!(base.contains(part), "{part}");
        }
        assert!(!base.contains("http"));
        // Every block the base documents is one it has.
        let names: Vec<&str> = blocks(base).unwrap().iter().map(|b| b.name).collect();
        assert_eq!(
            names,
            [
                "settings", "ready", "head", "theme", "header", "parties", "details", "body",
                "footer"
            ]
        );
        for name in &names {
            assert!(base.contains(&format!("- `{name}`:")), "{name} documented");
        }
        // Not a template: its name starts with `_`.
        let path = VaultPath::new(&format!("invoices/tpl/{}", BASE.file)).unwrap();
        assert!(templates::is_partial(&path));
        assert!(Template::from_source(path, base).is_none());
        assert!(templates::header(base).is_none());
    }

    /// The fixtures' copies are the shipped base, byte for byte, so the
    /// fixture template (a child) shows what a user's books show.
    #[test]
    fn the_fixtures_have_the_shipped_base() {
        for books in ["accrual", "cash"] {
            let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures")
                .join(books)
                .join("invoices/tpl")
                .join(BASE.file);
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                BASE.source,
                "{}: copy templates/{} over it",
                file.display(),
                BASE.file
            );
        }
    }

    #[test]
    fn folding_puts_the_childs_blocks_in_the_base() {
        let base = "{# {% block theme %} in a comment #}<head>{% block theme %}BASE-THEME{% endblock theme %}</head>\
                    {%- block header -%}<h1>{% block title %}T{% endblock %}</h1>{%- endblock header -%}\
                    {% raw %}{% block body %}{% endraw %}{% block body %}B{% endblock body %}";
        let child = "{# invoicer: name=\"x\" #}\n{% extends \"_invoice_base.v1.html\" %}\n\
                     {% block theme %}MINE{% endblock theme %}\n{% block title %}Mine{% endblock title %}";
        assert_eq!(
            fold_into_base(child, base).as_deref(),
            Some(
                "{# {% block theme %} in a comment #}<head>{% block theme %}MINE{% endblock theme %}</head>\
                 {%- block header -%}<h1>{% block title %}Mine{% endblock %}</h1>{%- endblock header -%}\
                 {% raw %}{% block body %}{% endraw %}{% block body %}B{% endblock body %}"
            )
        );
        // What it can't fold: a block the base lacks, `super()`, unpaired tags.
        assert!(fold_into_base("{% block nope %}x{% endblock %}", base).is_none());
        assert!(fold_into_base("{% block theme %}{{ super() }}{% endblock %}", base).is_none());
        assert!(fold_into_base("{% block theme %}x", base).is_none());
        assert!(fold_into_base("{% block theme %}x{% endblock other %}", base).is_none());
    }

    #[test]
    fn a_preview_folds_only_when_the_books_lack_the_base() {
        let vault = MemVault::new();
        let folded = preview_source(&vault, "invoices/tpl", DEFAULT);
        assert!(!extends_base(&folded), "folded");
        assert!(folded.starts_with("{#-\n  Ledgeline Invoicer's base"));
        assert!(folded.contains("class=\"letterhead\""));
        assert!(folded.contains("<main class=\"invoice-main\">"));
        assert!(
            vault
                .list_dir(&VaultPath::root(), None)
                .unwrap_or_default()
                .is_empty(),
            "writes nothing"
        );

        let vault = MemVault::new().with_file("invoices/tpl/_invoice_base.v1.html", "mine");
        assert_eq!(
            preview_source(&vault, "invoices/tpl", DEFAULT),
            DEFAULT.source
        );
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
        // The base it extends comes with it, first.
        let base = VaultPath::new("invoices/tpl/_invoice_base.v1.html").unwrap();
        assert_eq!(first.base.as_ref(), Some(&base));
        assert_eq!(
            first
                .written_files()
                .iter()
                .map(VaultPath::as_str)
                .collect::<Vec<_>>(),
            [
                "invoices/tpl/_invoice_base.v1.html",
                "invoices/tpl/classic.html"
            ]
        );
        assert_eq!(vault.read_to_string(&base).unwrap(), BASE.source);

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

        // The user's base is theirs too: a starter copied later keeps it.
        vault.write_atomic(&base, b"my base").expect("edits");
        vault.remove_file(&first.path).expect("removes");
        let later = install(&vault, "invoices/tpl", DEFAULT).expect("runs");
        assert!(later.written);
        assert_eq!(later.base, None);
        assert_eq!(vault.read_to_string(&base).unwrap(), "my base");

        // A starter already there doesn't bring back a base the user removed.
        vault.remove_file(&base).expect("removes");
        let kept = install(&vault, "invoices/tpl", DEFAULT).expect("runs");
        assert!(!kept.written && kept.base.is_none());
        assert!(vault.stat(&base).unwrap().is_none());
    }
}
