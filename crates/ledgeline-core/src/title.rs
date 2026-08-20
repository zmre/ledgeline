//! "What am I looking at?" — the display title for the open journal.
//!
//! A journal file has no title field. hledger has no notion of one either. But a
//! user with books for a household, a consultancy and a rental property open in
//! three windows needs each window to say which is which, and a bare
//! `2026.journal` in a title bar says nothing at all.
//!
//! So the title is DERIVED, from the journal itself, in a fixed order:
//!
//! 1. **The main file's leading comment.** People already write one:
//!    `; Acme Books`, `; ===== Personal ledger =====`. It is the closest thing a
//!    journal has to a self-declared name, and it is the author's own words
//!    rather than our guess, so it wins outright. [`Journal::leading_comment`]
//!    carries it; everything below decides whether it reads as a title.
//! 2. **The containing directory's name.** `~/finance/2026.journal` → `finance`.
//!    Books live in a folder named for what they are far more often than files
//!    are named for it — the file is usually a year or `main`.
//! 3. **Nothing.** `None`, and the caller shows whatever it would show for an
//!    unnamed journal. A wrong title is worse than no title.
//!
//! # Why the comment is filtered rather than trusted
//!
//! Most leading comments are NOT titles. `fixtures/sample.journal` opens with
//! `; Ledgeline sample journal — hand-authored fixture (WP-09 Phase A).` — an
//! eight-word sentence describing the file, which as a window title is noise. A
//! separator rule (`; ==========`) is not a title either, and neither is a
//! paragraph of licence text. The four acceptance rules in [`reads_as_a_title`]
//! exist to let the first kind through and hold the rest back, and every one of
//! them fails SAFE: a rejected comment falls to the folder name, never to a
//! garbled title.
//!
//! # A folder name, not a filename
//!
//! [`crate::journals`] makes a point of never inspecting a filename, because
//! every naming rule it could apply fails on some real layout. Rule 2 here is
//! not that: it never DECIDES anything from the name — it only displays it, and
//! only after the journal declined to name itself. Nothing in the engine
//! branches on the result.

use crate::model::Journal;
use std::path::Path;

/// Characters that decorate a header comment rather than say anything:
/// `; ===== Acme Books =====`, `; --- Acme Books ---`, `; ~~~ Acme Books ~~~`.
/// Stripped from both ends of a candidate before it is judged, which is also
/// what reduces a bare rule (`; ==========`) to nothing and rejects it.
const DECORATION: [char; 6] = ['=', '-', '*', '_', '#', '~'];

/// The longest accepted title, in CHARACTERS (not bytes — a title is displayed,
/// and `Société Générale` is 16 characters wherever it is rendered). Past this a
/// comment is prose about the file, not a name for it.
const MAX_TITLE_CHARS: usize = 60;

/// The most whitespace-separated words an accepted title may have. Real journal
/// names are one to three words (`Acme Books`, `Personal ledger 2026`); a
/// sentence describing the file is longer, and this is the cheapest rule that
/// separates them.
const MAX_TITLE_WORDS: usize = 5;

/// The display title for `journal`, or `None` when none could be derived.
///
/// Pure: everything it reads was captured when the journal was parsed. See the
/// module docs for the derivation order and why it is that order.
#[must_use]
pub fn journal_title(journal: &Journal) -> Option<String> {
    journal
        .leading_comment
        .as_deref()
        .and_then(title_from_comment)
        .or_else(|| folder_name(journal))
}

/// The BARE filename of the journal's main file — `2026.journal`, never a path.
///
/// Deliberately not the path: this is shown next to the title, and a client has
/// no business being told where on disk the user keeps their books (the same
/// rule the rules-file and journal-target ids hold to). `None` when the journal
/// has no recorded source file, or when its source name has no filename
/// component at all.
pub(crate) fn main_file_name(journal: &Journal) -> Option<String> {
    main_path(journal)?
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// The main journal file: the first entry of [`Journal::source_files`], which
/// the parser records before any `include`.
fn main_path(journal: &Journal) -> Option<&Path> {
    journal
        .source_files
        .first()
        .map(std::path::PathBuf::as_path)
}

/// A leading comment as a title, or `None` if it does not read as one.
///
/// The marker is already gone (the parser strips it); what is left is stripped
/// of decoration at both ends, re-trimmed, and then judged by
/// [`reads_as_a_title`].
fn title_from_comment(comment: &str) -> Option<String> {
    let candidate = comment.trim_matches(DECORATION).trim();
    reads_as_a_title(candidate).then(|| candidate.to_string())
}

/// Does `candidate` read as a journal's name rather than as prose about it?
///
/// All four must hold:
/// * **At least one word.** Which also rules out the empty string — a bare
///   separator rule reduces to exactly that once its decoration is stripped.
/// * **At least one alphanumeric character.** `; ***` and `; . . .` survive the
///   decoration strip in part but name nothing.
/// * **At most [`MAX_TITLE_WORDS`] words.** A sentence is a description.
/// * **At most [`MAX_TITLE_CHARS`] characters.** So is a very long single token.
fn reads_as_a_title(candidate: &str) -> bool {
    let words = candidate.split_whitespace().count();
    (1..=MAX_TITLE_WORDS).contains(&words)
        && candidate.chars().any(char::is_alphanumeric)
        && candidate.chars().count() <= MAX_TITLE_CHARS
}

/// The name of the directory holding the main journal file.
///
/// `None` when there is no directory to name: a bare relative filename has an
/// empty parent, and a file at the filesystem root has a parent with no name.
fn folder_name(journal: &Journal) -> Option<String> {
    main_path(journal)?
        .parent()?
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;

    /// A path that does not exist, so `resolve_source_file` leaves it exactly as
    /// written and these tests never depend on the machine they run on.
    const MAIN: &str = "/books/2026.journal";

    /// The whole chain: parse `text` as the main file at `source_name`, then
    /// derive its title. Going through the parser (rather than hand-building a
    /// `Journal`) is deliberate — the capture and the derivation are one
    /// contract, and a test that skipped the capture would not notice it break.
    fn titled(text: &str, source_name: &str) -> Option<String> {
        let journal = parse_journal(text, source_name).expect("journal parses");
        journal_title(&journal)
    }

    #[test]
    fn a_short_leading_comment_is_the_title() {
        assert_eq!(
            titled("; Acme Books\n", MAIN).as_deref(),
            Some("Acme Books")
        );
        // `#` and `*` are comment markers too, and a comment may be indented.
        assert_eq!(
            titled("# Acme Books\n", MAIN).as_deref(),
            Some("Acme Books")
        );
        assert_eq!(
            titled("   ;   Acme Books   \n", MAIN).as_deref(),
            Some("Acme Books")
        );
        // A repeated marker run is still just a marker.
        assert_eq!(
            titled(";;;; Acme Books\n", MAIN).as_deref(),
            Some("Acme Books")
        );
    }

    #[test]
    fn decoration_around_the_name_is_stripped() {
        for line in [
            "; ===== Acme Books =====",
            "; --- Acme Books ---",
            "; ~~~ Acme Books ~~~",
            "; ___Acme Books___",
            "; ***** Acme Books *****",
        ] {
            assert_eq!(
                titled(&format!("{line}\n"), MAIN).as_deref(),
                Some("Acme Books"),
                "{line}"
            );
        }
    }

    #[test]
    fn a_separator_rule_names_nothing_and_falls_back_to_the_folder() {
        // Each of these reduces to the empty string, or to symbols with no
        // alphanumeric character in them. Either way it is not a name.
        for line in ["; ==========", "; ----------", "; ***", "; . . .", ";"] {
            assert_eq!(
                titled(&format!("{line}\n"), MAIN).as_deref(),
                Some("books"),
                "{line}"
            );
        }
    }

    #[test]
    fn a_sentence_about_the_file_is_not_a_title() {
        // Five words is the limit and is accepted.
        assert_eq!(
            titled("; One Two Three Four Five\n", MAIN).as_deref(),
            Some("One Two Three Four Five")
        );
        // Six is a description. This is the shape `fixtures/sample.journal`
        // opens with, and the reason the rule exists.
        assert_eq!(
            titled("; One Two Three Four Five Six\n", MAIN).as_deref(),
            Some("books")
        );
        assert_eq!(
            titled(
                "; Ledgeline sample journal — hand-authored fixture (WP-09 Phase A).\n",
                MAIN
            )
            .as_deref(),
            Some("books")
        );
    }

    #[test]
    fn a_title_may_be_sixty_characters_but_not_sixty_one() {
        let sixty = "A".repeat(MAX_TITLE_CHARS);
        assert_eq!(
            titled(&format!("; {sixty}\n"), MAIN).as_deref(),
            Some(sixty.as_str())
        );
        let too_long = "A".repeat(MAX_TITLE_CHARS + 1);
        assert_eq!(
            titled(&format!("; {too_long}\n"), MAIN).as_deref(),
            Some("books"),
            "one character over the limit must fall back, not truncate"
        );
    }

    #[test]
    fn a_journal_that_opens_with_anything_but_a_comment_uses_its_folder() {
        assert_eq!(
            titled(
                "2026-01-01 t\n    expenses:x   $1.00\n    assets:bank\n",
                MAIN
            )
            .as_deref(),
            Some("books")
        );
        // A directive counts as "anything but a comment" too.
        assert_eq!(
            titled("account assets:bank\n", MAIN).as_deref(),
            Some("books")
        );
        // And so does an empty file.
        assert_eq!(titled("", MAIN).as_deref(), Some("books"));
    }

    #[test]
    fn blank_leading_lines_are_skipped_to_reach_the_comment() {
        assert_eq!(
            titled("\n\n   \n\t\n; Acme Books\n", MAIN).as_deref(),
            Some("Acme Books"),
            "the FIRST NON-EMPTY line is the one that counts"
        );
    }

    #[test]
    fn a_path_with_no_containing_directory_has_no_title() {
        // A bare relative name has an empty parent, so rule 2 has nothing to
        // report and the answer is honestly `None`.
        assert_eq!(
            titled("2026-01-01 t\n    a   $1\n    b\n", "2026.journal"),
            None
        );
        // The filesystem root is a parent with no name of its own.
        assert_eq!(titled("account assets:bank\n", "/2026.journal"), None);
    }

    #[test]
    fn a_non_ascii_name_is_a_name() {
        assert_eq!(
            titled("; Société Générale\n", MAIN).as_deref(),
            Some("Société Générale")
        );
        // CJK is alphanumeric and whitespace-free: one word, and a real title.
        assert_eq!(titled("; 会計帳簿\n", MAIN).as_deref(), Some("会計帳簿"));
    }

    #[test]
    fn the_file_is_the_bare_name_and_never_a_path() {
        let journal = parse_journal("", MAIN).expect("journal parses");
        assert_eq!(main_file_name(&journal).as_deref(), Some("2026.journal"));
        // Even when the title falls back or is absent, the filename stands on
        // its own — the two fields are independently nullable.
        let rootless = parse_journal("", "2026.journal").expect("journal parses");
        assert_eq!(journal_title(&rootless), None);
        assert_eq!(main_file_name(&rootless).as_deref(), Some("2026.journal"));
    }
}
