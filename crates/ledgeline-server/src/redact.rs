//! The one path redactor: rewrite the absolute paths *we* know about out of
//! text that is about to reach a client.
//!
//! hledger, git and the journal parser all name files in their diagnostics —
//! `imported 3 new transactions from bank.csv to /Users/me/finances/main.journal`
//! is verbatim real output — and those diagnostics are the single most useful
//! thing on the screen when something goes wrong, so withholding them is not an
//! option. The trade every route in this crate makes instead: pass the words
//! through, redact the prefixes we know.
//!
//! Used by the import and QuickBooks-journal routes (subprocess output), the
//! edit routes (SEC-15: parse errors rendered with the editor's absolute
//! `source_name`) and `git.rs` (hook and identity complaints).
//!
//! # Rules
//!
//! - **Longest-first**, so `/root/import/bank.csv` becomes `import/bank.csv`
//!   rather than being half-rewritten by the root's own entry.
//! - **Both spellings** of every path — as written and canonicalized — because
//!   they differ routinely: on macOS `/tmp` is a symlink to `/private/tmp`, and a
//!   subprocess may report either.
//! - **Whole path components only.** A swap fires only where the match ends at a
//!   component boundary, so hiding `/u/fin` never rewrites `/u/finances2/x` into
//!   `.ances2/x`.

use std::path::{Path, PathBuf};

use crate::AppState;
use crate::error::AppError;

/// An ordered set of `(absolute spelling, replacement)` swaps.
#[derive(Debug, Default, Clone)]
pub(crate) struct Redactor {
    swaps: Vec<(String, String)>,
}

impl Redactor {
    /// The redaction for the journal `state` currently serves: every source file
    /// (main plus every `include`) mapped to its bare file name, and the main
    /// file's directory mapped away entirely.
    ///
    /// The directory matters on its own: a *newly* added `include` naming a file
    /// that does not exist is not in `source_files` (that parse never
    /// succeeded), and `ParseError::Include` renders its absolute path.
    ///
    /// The main file is registered under the spelling it was OPENED with too
    /// (`Journal::source_name`), not just the resolved one in `source_files`:
    /// parse errors render that spelling, and the "canonical" twin of an already
    /// canonical path is itself — so a journal opened through a symlink (or
    /// macOS's `/tmp`) would otherwise leak its directory.
    pub(crate) fn for_journal(state: &AppState) -> Self {
        let opened = PathBuf::from(&state.snapshot().journal.source_name);
        let sources = state.source_files();
        let mains = sources.first().into_iter().chain(Some(&opened));
        let files = sources
            .iter()
            .chain(Some(&opened))
            .fold(Self::default(), |redactor, path| {
                let handle = path.file_name().map_or_else(
                    || "the journal".to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                redactor.hide(path, &handle)
            });
        mains
            .filter_map(|main| main.parent())
            .fold(files, Self::hide_prefix)
    }

    /// Replace every occurrence of `path` with `handle`.
    pub(crate) fn hide(mut self, path: &Path, handle: &str) -> Self {
        for spelling in spellings(path) {
            if !self.swaps.iter().any(|(from, _)| *from == spelling) {
                self.swaps.push((spelling, handle.to_string()));
            }
        }
        self
    }

    /// Replace a DIRECTORY prefix: `dir/x` becomes `x`, and a bare mention of the
    /// directory becomes `.`.
    pub(crate) fn hide_prefix(mut self, dir: &Path) -> Self {
        for prefix in spellings(dir) {
            self.swaps.push((format!("{prefix}/"), String::new()));
            self.swaps.push((prefix, ".".to_string()));
        }
        self
    }

    /// `text` with every known path rewritten.
    pub(crate) fn apply(&self, text: &str) -> String {
        let mut swaps: Vec<&(String, String)> = self.swaps.iter().collect();
        swaps.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        swaps.iter().fold(text.to_string(), |redacted, (from, to)| {
            replace_components(&redacted, from, to)
        })
    }

    /// `error` with its message redacted, keeping the variant — and therefore
    /// the status the SPA switches on — exactly as it was.
    ///
    /// The match is deliberately exhaustive rather than a catch-all: a new
    /// [`AppError`] variant must fail to compile here so somebody decides
    /// whether it can carry a path, instead of silently leaking one.
    pub(crate) fn redact_error(&self, error: AppError) -> AppError {
        match error {
            AppError::BadRequest(message) => AppError::BadRequest(self.apply(&message)),
            AppError::NotFound(message) => AppError::NotFound(self.apply(&message)),
            AppError::Conflict(message) => AppError::Conflict(self.apply(&message)),
            AppError::EditingDisabled(message) => AppError::EditingDisabled(self.apply(&message)),
            AppError::Unavailable(message) => AppError::Unavailable(self.apply(&message)),
            AppError::Internal(message) => AppError::Internal(self.apply(&message)),
        }
    }
}

/// A path as written and, when it differs, as canonicalized. Never empty
/// strings: an empty `from` would match everywhere.
fn spellings(path: &Path) -> impl Iterator<Item = String> {
    let written = path.to_string_lossy().into_owned();
    let canonical = std::fs::canonicalize(path)
        .map(|resolved| resolved.to_string_lossy().into_owned())
        .ok()
        .filter(|resolved| *resolved != written);
    std::iter::once(written)
        .chain(canonical)
        .filter(|spelling| !spelling.is_empty())
}

/// `text` with each occurrence of `from` that ends on a path-component boundary
/// replaced by `to`. Occurrences that run on into a longer component — `/u/fin`
/// inside `/u/finances2` — are left alone.
fn replace_components(text: &str, from: &str, to: &str) -> String {
    let (mut out, last) = text.match_indices(from).fold(
        (String::with_capacity(text.len()), 0),
        |(mut out, last), (start, _)| {
            let end = start + from.len();
            // Overlapping or mid-component matches are skipped.
            if start < last || !ends_component(from, &text[end..]) {
                return (out, last);
            }
            out.push_str(&text[last..start]);
            out.push_str(to);
            (out, end)
        },
    );
    out.push_str(&text[last..]);
    out
}

/// Does a match of `from`, followed by `rest`, end on a component boundary?
///
/// A `from` that already ends in a separator always does. Otherwise the next
/// character must not continue a file name: a letter, digit, `_`, `-`, or a `.`
/// that is itself followed by one of those (`main.journal.bak`) continues it; a
/// separator, whitespace, punctuation such as `:` `'` `"` `)`, or a
/// sentence-ending `.` does not.
fn ends_component(from: &str, rest: &str) -> bool {
    let continues = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    if from.ends_with('/') {
        return true;
    }
    let mut chars = rest.chars();
    match chars.next() {
        None => true,
        Some('.') => !chars.next().is_some_and(continues),
        Some(c) => !continues(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Redaction is longest-first, so a file inside the root is rewritten to its
    /// own handle rather than being half-rewritten by the root's entry.
    #[test]
    fn redaction_prefers_the_most_specific_path() {
        let redactor = Redactor::default()
            .hide(Path::new("/books/2026/2026.journal"), "2026/2026.journal")
            .hide_prefix(Path::new("/books"));
        assert_eq!(
            redactor.apply("imported 3 transactions to /books/2026/2026.journal"),
            "imported 3 transactions to 2026/2026.journal"
        );
        assert_eq!(
            redactor.apply("could not read /books/other.csv"),
            "could not read other.csv"
        );
        assert_eq!(redactor.apply("in /books"), "in .");
    }

    /// The shared bug both copies had: a prefix swap ignored path boundaries, so
    /// hiding `/u/fin` turned a sibling directory into `.ances2/x`.
    #[test]
    fn a_prefix_only_matches_whole_path_components() {
        let redactor = Redactor::default().hide_prefix(Path::new("/u/fin"));
        assert_eq!(
            redactor.apply("could not read /u/finances2/x"),
            "could not read /u/finances2/x"
        );
        assert_eq!(
            redactor.apply("/u/fin-old and /u/fin_2"),
            "/u/fin-old and /u/fin_2"
        );
        assert_eq!(redactor.apply("/u/fin.bak/x"), "/u/fin.bak/x");
        // Every real boundary still redacts.
        assert_eq!(redactor.apply("/u/fin/main.journal"), "main.journal");
        assert_eq!(
            redactor.apply("not a git repository: /u/fin"),
            "not a git repository: ."
        );
        assert_eq!(redactor.apply("in '/u/fin': denied"), "in '.': denied");
        assert_eq!(redactor.apply("(/u/fin)"), "(.)");
        assert_eq!(redactor.apply("dir /u/fin."), "dir ..");
        assert_eq!(redactor.apply("/u/fin\n/u/finx"), ".\n/u/finx");
    }

    /// A file handle respects the boundary too, and a longer sibling falls
    /// through to the directory's swap instead of leaking.
    #[test]
    fn a_file_swap_does_not_eat_into_a_longer_name() {
        let redactor = Redactor::default()
            .hide(Path::new("/u/fin/main.journal"), "main.journal")
            .hide_prefix(Path::new("/u/fin"));
        assert_eq!(
            redactor.apply("/u/fin/main.journal:3: bad"),
            "main.journal:3: bad"
        );
        assert_eq!(
            redactor.apply("/u/fin/main.journal.bak"),
            "main.journal.bak"
        );
        assert_eq!(redactor.apply("/u/fin/main.journals/x"), "main.journals/x");
    }

    /// Both spellings of a symlinked path are hidden, and an empty path never
    /// becomes a match-everything swap.
    #[test]
    fn both_spellings_are_hidden_and_empty_is_ignored() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let real = dir.path().join("real");
        std::fs::create_dir(&real)?;
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link)?;
        let file = link.join("main.journal");
        std::fs::write(&file, "")?;

        let redactor = Redactor::default()
            .hide(&file, "main.journal")
            .hide(Path::new(""), "nothing");
        let canonical = std::fs::canonicalize(&file)?;
        let text = format!("{} and {}", file.display(), canonical.display());
        assert_eq!(redactor.apply(&text), "main.journal and main.journal");
        assert_eq!(redactor.apply("untouched"), "untouched");
        Ok(())
    }

    /// The swaps `edit_api::PathRedaction::for_journal` built before it was
    /// folded into this module, reproduced verbatim as the parity oracle.
    fn legacy_path_redaction(sources: &[std::path::PathBuf]) -> Vec<(String, String)> {
        let files = sources.iter().flat_map(|path| {
            let handle = path.file_name().map_or_else(
                || "the journal".to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            spellings(path).map(move |spelling| (spelling, handle.clone()))
        });
        let dirs = sources
            .iter()
            .take(1)
            .filter_map(|path| path.parent())
            .flat_map(|dir| {
                spellings(dir).flat_map(|spelling| {
                    [
                        (format!("{spelling}/"), String::new()),
                        (spelling, ".".to_string()),
                    ]
                })
            });
        files
            .chain(dirs)
            .filter(|(from, _)| !from.is_empty())
            .collect()
    }

    fn sorted(mut swaps: Vec<(String, String)>) -> Vec<(String, String)> {
        swaps.sort();
        swaps.dedup();
        swaps
    }

    /// A two-file journal (`main.journal` including `2026/year.journal`) under
    /// `dir/books`, and the main file's path.
    fn two_file_journal(dir: &Path) -> std::io::Result<PathBuf> {
        let books = dir.join("books");
        std::fs::create_dir_all(books.join("2026"))?;
        std::fs::write(books.join("main.journal"), "include 2026/year.journal\n")?;
        std::fs::write(
            books.join("2026/year.journal"),
            "2026-01-01 x\n    a  1\n    b\n",
        )?;
        Ok(books.join("main.journal"))
    }

    /// Opened by its canonical path, `for_journal` registers exactly the swaps
    /// the edit path's old `PathRedaction` did — main and included files to bare
    /// names, the main file's directory away.
    #[test]
    fn for_journal_matches_the_old_edit_path_redaction() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let main = two_file_journal(&std::fs::canonicalize(dir.path())?)?;
        let state = AppState::from_journal_path(&main)?;
        assert_eq!(state.source_files().len(), 2, "the include was followed");

        let redactor = Redactor::for_journal(&state);
        assert_eq!(
            sorted(redactor.swaps.clone()),
            sorted(legacy_path_redaction(&state.source_files()))
        );

        let books = main.parent().ok_or("no parent")?;
        let message = format!(
            "{}:3: bad; see {} and {}/missing.journal in {}",
            main.display(),
            books.join("2026/year.journal").display(),
            books.display(),
            books.display()
        );
        let redacted = redactor.redact_error(AppError::BadRequest(message));
        assert!(
            matches!(
                redacted,
                AppError::BadRequest(ref m)
                    if m == "main.journal:3: bad; see year.journal and missing.journal in ."
            ),
            "{redacted:?}"
        );
        Ok(())
    }

    /// Opened through a symlink, the error names the spelling it was opened
    /// with, which `source_files` (all resolved) never contains. The old
    /// redaction leaked that directory; `for_journal` keeps every old swap and
    /// adds the opened spelling.
    #[test]
    fn for_journal_hides_the_spelling_the_journal_was_opened_with()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        two_file_journal(dir.path())?;
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("books"), &link)?;
        let main = link.join("main.journal");
        let state = AppState::from_journal_path(&main)?;

        let redactor = Redactor::for_journal(&state);
        let legacy = legacy_path_redaction(&state.source_files());
        assert!(legacy.iter().all(|swap| redactor.swaps.contains(swap)));

        let message = format!(
            "{}:3: bad; {}/missing.journal in {}",
            main.display(),
            link.display(),
            link.display()
        );
        let legacy_text = Redactor { swaps: legacy }.apply(&message);
        assert!(
            legacy_text.contains(&link.display().to_string()),
            "{legacy_text}"
        );
        assert_eq!(
            redactor.apply(&message),
            "main.journal:3: bad; missing.journal in ."
        );
        Ok(())
    }

    /// `redact_error` keeps the variant and rewrites only the message.
    #[test]
    fn redact_error_keeps_the_variant() {
        let redactor = Redactor::default().hide_prefix(Path::new("/u/fin"));
        let redacted = redactor.redact_error(AppError::BadRequest("/u/fin/a.journal:1: x".into()));
        assert!(matches!(redacted, AppError::BadRequest(ref m) if m == "a.journal:1: x"));
        let redacted = redactor.redact_error(AppError::Internal("in /u/fin".into()));
        assert!(matches!(redacted, AppError::Internal(ref m) if m == "in ."));
    }
}
