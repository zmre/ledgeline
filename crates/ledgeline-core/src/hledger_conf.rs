//! `hledger.conf` — reading the `--alias` options it declares, and writing one
//! into it.
//!
//! # Why this module exists: two homes for one mapping
//!
//! An `alias` directive in a journal and an `--alias` option in a config file
//! look like the same thing and are not. Verified against hledger 1.52:
//!
//! | | applies when hledger READS the journal | applies to an `import`'s CSV |
//! | --- | --- | --- |
//! | `alias` directive in the journal | yes | **no** |
//! | `--alias` in `hledger.conf` | yes | **yes** |
//!
//! [`crate::aliases`] exists because of the first row's second column: Ledgeline
//! reads the journal's directives and hands them to hledger as `--alias`, which
//! is the only way they can reach a statement. This module exists because of the
//! consequence — a plain command-line `hledger import`, run by the same user over
//! the same CSV with the same rules file, gets no such help and writes different
//! account names. Verified: `PW Roth IRA - 3077:cash` from the terminal where
//! Ledgeline writes `assets:morganstanley:pw-roth-ira:cash`. Same inputs, two
//! journals.
//!
//! A config file closes that, because it applies to *every* hledger command. So
//! this module (a) reads one, so an import Ledgeline runs honours what the
//! user's terminal already honours, and (b) writes one, so the divergence has a
//! fix rather than only a warning.
//!
//! # We read a config file; we never delegate to one
//!
//! Note what this module does **not** do: pass `--conf`. Every invocation
//! `hledger.rs` builds carries `--no-conf`, because a config file can replace the
//! command hledger runs (its docs say so and the binary agrees), and because a
//! config may hold `--depth`, `-b`/`-e` or `--forecast`, any of which would
//! silently change output this crate parses and appends to a journal. Reading the
//! file ourselves and forwarding only the `--alias` values we understand is the
//! narrow version of the same benefit: the mapping travels, the rest does not.
//!
//! # The file format, as the binary actually reads it
//!
//! Each fact below was checked against hledger 1.52 rather than read off the
//! manual, because two of them are undocumented and one is a trap.
//!
//! * **Tokens are split on whitespace and QUOTES ARE NOT HONOURED.** Both
//!   `--alias="…"` and `--alias='…'` fail with a `parse error`; the quote
//!   character ends up inside the value. This is the trap, and [`conf_argument`]
//!   is the whole of this module's answer to it.
//! * `#` starts a comment, anywhere on a line. **`;` does not** — a leading `;`
//!   is read as the command word, which hijacks the invocation exactly as a bare
//!   `balance` would.
//! * **The FIRST token of the general section, if it does not begin with a dash,
//!   replaces the command.** Only the first: a bare word appearing after any
//!   option is passed through as an ordinary argument and hijacks nothing (both
//!   verified). A hijacked config file breaks every hledger command the user runs,
//!   so none of its options reach anything — which is why [`alias_arguments`]
//!   answers with none, and [`hijacks_command`] exists to say why.
//! * `[name]` opens a section. Everything before the first heading is the
//!   *general* section and applies to every command; a `[name]` section applies
//!   only to that command. An import therefore sees general + `[import]`, and
//!   [`Section::applies_to`] is that rule.
//! * The general section's options are applied **before** a command section's,
//!   and `--alias` options compose in order, so the first one to match an account
//!   wins.
//! * hledger searches for `hledger.conf` in the working directory and **every
//!   directory above it**, nearest first, then `$HOME/.hledger.conf`, then the
//!   XDG config dir — and uses exactly one file. [`locate`] implements only the
//!   upward walk, and only from the journal's own directory; see its docs.
//!
//! # Writing: the escaping rule
//!
//! Because there is no quoting and no escape for a space, an alias whose pattern
//! contains whitespace cannot be written literally. [`conf_argument`] converts it
//! or refuses; it never writes a mapping that does not match. See that function
//! for the two deliberate widenings and the four refusals.

use std::path::{Path, PathBuf};

/// The file name hledger looks for in a directory.
pub const CONF_NAME: &str = "hledger.conf";

/// The command section an import's options may come from, besides the general
/// one.
pub const IMPORT_COMMAND: &str = "import";

/// Largest config file this module will read.
///
/// A config file is a handful of option lines. The cap exists so a `hledger.conf`
/// that is secretly a disk image cannot be pulled into memory by a page load, in
/// the same spirit as the rules scan's own budgets.
pub const MAX_CONF_BYTES: u64 = 256 * 1024;

/// How many directories above the journal's own [`locate`] will look.
///
/// Deep enough for any real layout, finite so a pathological symlinked tree
/// cannot make the walk unbounded.
pub const MAX_ANCESTORS: usize = 64;

/// How many `--alias` options this module will take from one config file.
///
/// The same bound, and the same reasoning, as [`crate::aliases::MAX_FORWARDED`]:
/// it limits an `argv`, not a user.
pub const MAX_CONF_ALIASES: usize = 200;

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Which part of a config file an option was declared in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    /// Before the first `[heading]`. Applies to every command.
    General,
    /// Inside `[name]`. Applies to `name` only.
    Command(String),
}

impl Section {
    /// Does an option in this section reach `command`?
    #[must_use]
    pub fn applies_to(&self, command: &str) -> bool {
        match self {
            Self::General => true,
            Self::Command(name) => name.eq_ignore_ascii_case(command),
        }
    }
}

/// One option read out of a config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfOption {
    /// Where it was declared.
    pub section: Section,
    /// The flag, with its leading dashes (`--alias`).
    pub name: String,
    /// Its value, when it had one.
    pub value: Option<String>,
}

/// Every option a config file declares, in file order, each tagged with its
/// section.
///
/// Deliberately tolerant: a line this module does not understand contributes
/// nothing and stops nothing. The result is used for exactly one thing — finding
/// `--alias` values — and refusing to read a file because of an option we have no
/// opinion about would turn an unrelated setting into a broken import.
#[must_use]
pub fn parse(text: &str) -> Vec<ConfOption> {
    text.lines()
        .scan(Section::General, |section, line| {
            // `#` comments run to end of line, wherever they start. `;` does NOT
            // start one — see the module docs.
            let content = line.split('#').next().unwrap_or_default().trim();
            if let Some(name) = heading(content) {
                *section = Section::Command(name);
                return Some(Vec::new());
            }
            Some(options(content, section))
        })
        .flatten()
        .collect()
}

/// The section name if `line` is a `[heading]`, trimmed.
fn heading(line: &str) -> Option<String> {
    line.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(|name| name.trim().to_string())
}

/// The options on one already-decommented line.
///
/// Whitespace-separated, exactly as hledger splits it, and both spellings of a
/// valued option are read: `--alias=X=Y` (one token, and the only one this module
/// ever *writes*) and `--alias X=Y` (two tokens, which a hand-written file may
/// well use). A token that is not a flag is a positional — the command word, or
/// an option's value already consumed — and is skipped.
fn options(line: &str, section: &Section) -> Vec<ConfOption> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        let token = tokens[at];
        at += 1;
        if !token.starts_with('-') {
            continue;
        }
        let (name, value) = match token.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => {
                // A following token that is not itself a flag is this one's
                // value. A flag with no value (`--debug`) simply has none.
                let next = tokens.get(at).filter(|next| !next.starts_with('-'));
                if next.is_some() {
                    at += 1;
                }
                (token, next.map(|value| (*value).to_string()))
            }
        };
        out.push(ConfOption {
            section: section.clone(),
            name: name.to_string(),
            value,
        });
    }
    out
}

/// The command word this config file forces on every hledger invocation, if it
/// forces one.
///
/// hledger's own manual: "If the first word in a config file's top (general)
/// section does not begin with a dash (eg: print), it is treated as the command
/// argument (overriding any argument on the command line)." Verified, including
/// the narrowness of it — a bare word after any option is an ordinary argument
/// and overrides nothing.
///
/// A config in this state makes **every** hledger command fail or run as
/// something else, so its options never reach anything. Worth naming rather than
/// silently reading around, because a user whose terminal hledger stopped working
/// is owed the reason.
#[must_use]
pub fn hijacks_command(text: &str) -> Option<String> {
    parse_tokens(text)
        .into_iter()
        .find(|(section, _)| *section == Section::General)
        .map(|(_, token)| token)
        .filter(|token| !token.starts_with('-'))
}

/// Every whitespace-separated token, decommented, tagged with its section.
fn parse_tokens(text: &str) -> Vec<(Section, String)> {
    text.lines()
        .scan(Section::General, |section, line| {
            let content = line.split('#').next().unwrap_or_default().trim();
            if let Some(name) = heading(content) {
                *section = Section::Command(name);
                return Some(Vec::new());
            }
            Some(
                content
                    .split_whitespace()
                    .map(|token| (section.clone(), token.to_string()))
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

/// The `--alias` values a config file would give `command`, in the order hledger
/// applies them.
///
/// General-section options first, then the command section's, because that is the
/// order hledger assembles them and `--alias` options compose in order — the
/// first one to match an account is the one that rewrites it.
///
/// **None at all when [`hijacks_command`] answers**: a config that replaces the
/// command breaks the invocation outright, so nothing in it applies to anything.
#[must_use]
pub fn alias_arguments(text: &str, command: &str) -> Vec<String> {
    if hijacks_command(text).is_some() {
        return Vec::new();
    }
    let parsed = parse(text);
    let pick = |wanted: &Section| -> Vec<String> {
        parsed
            .iter()
            .filter(|option| &option.section == wanted && option.name == "--alias")
            .filter_map(|option| option.value.clone())
            .filter(|value| !value.is_empty())
            .collect()
    };
    pick(&Section::General)
        .into_iter()
        .chain(pick(&Section::Command(command.to_string())))
        .take(MAX_CONF_ALIASES)
        .collect()
}

/// The nearest `hledger.conf` at or above `directory`, if there is one.
///
/// # Two deliberate differences from what hledger itself does
///
/// 1. **The walk starts at the JOURNAL's directory**, not at this process's
///    working directory. hledger searches from wherever it was launched, and
///    where a user launches hledger for these books is beside these books. A
///    server process's working directory is an accident of how it was started and
///    describes nothing about the user's ledger.
/// 2. **`$HOME/.hledger.conf` and the XDG config dir are NOT consulted**, though
///    hledger falls back to both. Ledgeline reads inside the tree the user pointed
///    it at and nowhere else — the same posture the `include` guard takes — and a
///    desktop app quietly picking up a home-directory dotfile to decide what
///    account names to write into a ledger is not a thing to do without being
///    asked. The cost is stated where it is felt: the divergence notice says which
///    directories were searched, so a user whose config lives in `$HOME` can see
///    why it was not counted.
///
/// Only a regular file counts. A `hledger.conf` that is a directory or a FIFO is
/// skipped rather than opened, because opening the second one never returns.
#[must_use]
pub fn locate(directory: &Path) -> Option<PathBuf> {
    directory
        .ancestors()
        .take(MAX_ANCESTORS)
        .map(|dir| dir.join(CONF_NAME))
        .find(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_CONF_BYTES)
        })
}

/// Read a config file located by [`locate`].
///
/// Lossy rather than strict UTF-8: a config file is read to find `--alias`
/// values, and refusing the whole file — and with it the account mapping an
/// import depends on — over one stray byte in a comment would be the wrong trade.
///
/// # Errors
/// Whatever [`std::fs::read`] reports.
pub fn read(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Why an alias cannot be written into a config file.
///
/// Every variant is a case where the value *could* be written and would then
/// mean something other than it says. A refusal is reported to the user beside
/// the alias it belongs to; none of them is a silent drop, because a mapping that
/// looks installed and does not match is worse than one that is visibly missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfRefusal {
    /// A `#` on either side. It starts a comment in a config file, so everything
    /// after it would be discarded.
    Comment,
    /// Whitespace in the replacement. The pattern's whitespace has a workaround
    /// (`.` matches a space); an account NAME has none — a config file has no
    /// quoting and no escape, so the name would simply be cut short.
    ReplacementWhitespace,
    /// A backslash in the replacement of an alias that must be converted to a
    /// regular expression, where `\1` means a capture group rather than a
    /// literal backslash.
    ReplacementBackslash,
    /// A bracket expression in a regular-expression pattern that also contains
    /// whitespace. Inside `[...]` a `.` is an ordinary dot, so the substitution
    /// this module performs would change what the pattern matches.
    PatternBracket,
    /// A backslash in a regular-expression pattern that also contains whitespace.
    /// Substituting next to an escape somebody else wrote is a guess.
    PatternBackslash,
    /// A `/` in a plain pattern that must be converted to a regular expression,
    /// where `/` ends the pattern and would have to be re-escaped.
    PatternSlash,
}

impl ConfRefusal {
    /// A sentence completing "this alias cannot be added to your config file
    /// because …", written for the person reading the screen.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Comment => {
                "it contains a `#`, which starts a comment in a config file — everything after it \
                 would be thrown away"
            }
            Self::ReplacementWhitespace => {
                "the account it maps to contains a space, and a config file has no quoting and no \
                 escape for one, so hledger would read only the first word"
            }
            Self::ReplacementBackslash => {
                "the account it maps to contains a backslash, which would become a capture-group \
                 reference once this alias is written as a regular expression"
            }
            Self::PatternBracket => {
                "its pattern contains both a space and a `[...]` group, and inside such a group a \
                 `.` is an ordinary dot rather than the any-character stand-in this rewrite needs"
            }
            Self::PatternBackslash => {
                "its pattern contains both a space and a backslash escape, and Ledgeline will not \
                 rewrite next to an escape it did not write"
            }
            Self::PatternSlash => {
                "its pattern contains both a space and a `/`, and the `/` would have to be \
                 re-escaped to survive being written as a regular expression"
            }
        }
    }

    /// The machine-readable half, for the wire. Spelled out rather than derived
    /// from `Debug`, so a rename here cannot silently change a wire value.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::ReplacementWhitespace => "replacementWhitespace",
            Self::ReplacementBackslash => "replacementBackslash",
            Self::PatternBracket => "patternBracket",
            Self::PatternBackslash => "patternBackslash",
            Self::PatternSlash => "patternSlash",
        }
    }
}

/// The `--alias` **value** for one alias, in a form a config file can hold.
///
/// # The problem
///
/// hledger's config parser splits on whitespace and ignores quotes, so
/// `--alias="/^PW Roth IRA - 3077/=assets:x"` is a parse error and so is the
/// unquoted spelling. Both verified. There is no escape for a space.
///
/// # The rule, in order
///
/// 1. **No whitespace anywhere ⇒ write it verbatim**, in whichever form the user
///    declared. This is the common case and it is provably identical to what
///    Ledgeline forwards on the command line, because it is the same string.
/// 2. **Whitespace in the replacement ⇒ refuse.** An account name is matched
///    literally; there is no wildcard to stand in for its space.
/// 3. **Whitespace in a REGEX pattern ⇒ substitute.** Each whitespace character
///    becomes `.`, which matches it. Refused when the pattern also holds `[` or
///    `\`, where the substitution could mean something else.
/// 4. **Whitespace in a PLAIN pattern ⇒ convert to a regex.** A plain alias
///    matches the whole account name or a prefix ending at a `:` (verified: it
///    rewrites `a` and `a:sub` and leaves `abc` alone), which is exactly
///    `/^PATTERN($|:)/` with `\1` carrying the boundary into the replacement.
///    Regex metacharacters in the original are escaped first, then its whitespace
///    becomes `.`.
///
/// # Two widenings, both deliberate and both shown to the user
///
/// A `.` matches *any* character, not only a space, and hledger's regex aliases
/// are **case-insensitive** where plain ones are not (both verified). So a
/// converted alias matches everything the original did and can match a little
/// more. That is the trade for being expressible at all, and it is why the
/// resulting line is displayed before it is written rather than after.
///
/// # Errors
/// [`ConfRefusal`], one variant per case above.
pub fn conf_argument(pattern: &str, replacement: &str, regex: bool) -> Result<String, ConfRefusal> {
    if pattern.contains('#') || replacement.contains('#') {
        return Err(ConfRefusal::Comment);
    }
    if replacement.chars().any(char::is_whitespace) {
        return Err(ConfRefusal::ReplacementWhitespace);
    }
    if !pattern.chars().any(char::is_whitespace) {
        // Rule 1: the same bytes we put on the command line.
        let rendered = if regex {
            format!("/{pattern}/")
        } else {
            pattern.to_string()
        };
        return Ok(format!("{rendered}={replacement}"));
    }
    if regex {
        if pattern.contains('[') {
            return Err(ConfRefusal::PatternBracket);
        }
        if pattern.contains('\\') {
            return Err(ConfRefusal::PatternBackslash);
        }
        return Ok(format!("/{}/={replacement}", dotted(pattern)));
    }
    if pattern.contains('/') {
        return Err(ConfRefusal::PatternSlash);
    }
    if replacement.contains('\\') {
        return Err(ConfRefusal::ReplacementBackslash);
    }
    Ok(format!(
        "/^{}($|:)/={replacement}\\1",
        dotted(&escaped(pattern))
    ))
}

/// Every whitespace character replaced by `.`, the regex any-character atom.
///
/// One `.` per character, never one per run: `.` matches exactly one character,
/// so collapsing two spaces into one dot would produce a pattern that no longer
/// matches the name it came from.
fn dotted(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_whitespace() { '.' } else { c })
        .collect()
}

/// Every POSIX-ERE metacharacter escaped, so a literal pattern stays literal.
///
/// hledger's alias regexes are Haskell `regex-tdfa`, i.e. POSIX ERE. The set
/// below is that dialect's specials plus `/`, which ends an alias pattern, and
/// `\` itself. Whitespace is deliberately NOT escaped here — [`dotted`] runs
/// afterwards and turns it into the one metacharacter this conversion wants.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '.' | '^'
                | '$'
                | '*'
                | '+'
                | '?'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '|'
                | '\\'
                | '/'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// One complete config-file line for an alias value.
#[must_use]
pub fn conf_line(argument: &str) -> String {
    format!("--alias={argument}")
}

/// Where a new general-section option has to go, as a byte offset into `text`.
///
/// **Not the end of the file**, and that distinction is load-bearing. Everything
/// after the first `[heading]` belongs to that command's section, so a line
/// appended at EOF of a file ending in `[balance]` would be a balance-only option
/// — present, plausible, and never applied to an import. So the insertion point
/// is immediately before the first heading, or the end of the file when there is
/// none.
#[must_use]
pub fn general_section_end(text: &str) -> usize {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.split('#').next().unwrap_or_default().trim();
        if heading(content).is_some() {
            return offset;
        }
        offset += line.len();
    }
    text.len()
}

/// `text` with `arguments` added as `--alias` lines in its general section.
///
/// Pure, and every byte outside the inserted run comes out of `text` unchanged —
/// the same discipline `aliases::AliasDoc` holds to, for the same reason: this is
/// a file the user may well have written by hand.
///
/// Arguments already present are skipped, so pressing the button twice adds
/// nothing the second time. A missing final newline before the insertion point is
/// supplied, because without one the new option would be glued onto whatever line
/// was there and become part of its value.
#[must_use]
pub fn with_aliases(text: &str, arguments: &[String]) -> String {
    let existing = alias_arguments(text, IMPORT_COMMAND);
    let wanted: Vec<&String> = arguments
        .iter()
        .filter(|argument| !existing.contains(argument))
        .collect();
    if wanted.is_empty() {
        return text.to_string();
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let at = general_section_end(text);
    let head = &text[..at];
    let lead = if head.is_empty() || head.ends_with('\n') {
        String::new()
    } else {
        newline.to_string()
    };
    let body: String = wanted
        .iter()
        .map(|argument| format!("{}{newline}", conf_line(argument)))
        .collect();
    format!("{head}{lead}{body}{}", &text[at..])
}

/// The header a config file Ledgeline creates from scratch opens with.
///
/// Only for a file that did not exist. An existing config gets its option lines
/// and nothing else — a comment block inserted into somebody's hand-written file
/// is bytes they did not ask for.
#[must_use]
pub fn new_file_header() -> String {
    "# Written by Ledgeline so that `hledger import` run from a terminal maps the\n\
     # same account names the Ledgeline import screen does. An `alias` directive in\n\
     # a journal is NOT applied to an imported CSV; an --alias option here is.\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_and_import_sections_apply_and_others_do_not() {
        let text = "--alias=/^A/=general\n\
                    \n\
                    [balance]\n\
                    --alias=/^A/=balanceonly\n\
                    \n\
                    [import]\n\
                    --alias=/^A/=importonly\n";
        assert_eq!(
            alias_arguments(text, IMPORT_COMMAND),
            vec!["/^A/=general", "/^A/=importonly"]
        );
        // And the general one is FIRST, which is the order hledger applies them
        // and therefore the one that decides which alias wins.
        assert_eq!(alias_arguments(text, "balance")[1], "/^A/=balanceonly");
    }

    #[test]
    fn a_hash_is_a_comment_and_a_semicolon_is_not() {
        // Verified against the binary: `#` is the comment character, inline as
        // well as leading. `;` is not one — as the first token it is read as the
        // COMMAND, which is why the file below hijacks rather than comments.
        assert_eq!(
            alias_arguments(
                "# --alias=/^A/=nope\n--alias=/^A/=yes # trailing\n",
                IMPORT_COMMAND
            ),
            vec!["/^A/=yes"]
        );
        assert_eq!(hijacks_command("; --alias=/^A/=B\n").as_deref(), Some(";"));
        assert!(alias_arguments("; --alias=/^A/=B\n", IMPORT_COMMAND).is_empty());
    }

    #[test]
    fn only_the_first_token_of_the_general_section_hijacks() {
        // The single most dangerous thing a config file can do, and it is exactly
        // this narrow — verified against hledger 1.52 in both directions.
        assert_eq!(hijacks_command("balance\n").as_deref(), Some("balance"));
        assert_eq!(hijacks_command("--alias=a=b\nfoo\n"), None);
        // A bare word inside a COMMAND section is that section's argument and
        // overrides nothing in general.
        assert_eq!(hijacks_command("[import]\nfoo\n--alias=a=b\n"), None);
        assert_eq!(
            alias_arguments("[import]\nfoo\n--alias=a=b\n", IMPORT_COMMAND),
            vec!["a=b"]
        );
    }

    #[test]
    fn both_spellings_of_a_valued_option_are_read() {
        assert_eq!(
            alias_arguments("--alias=a=b\n--alias c=d\n", IMPORT_COMMAND),
            vec!["a=b", "c=d"]
        );
    }

    #[test]
    fn an_option_this_module_has_no_opinion_about_stops_nothing() {
        let text = "--depth 3\n--forecast\n--alias=a=b\n";
        assert_eq!(alias_arguments(text, IMPORT_COMMAND), vec!["a=b"]);
    }

    #[test]
    fn a_value_with_no_whitespace_is_written_verbatim_in_its_own_form() {
        assert_eq!(conf_argument("a:b", "c:d", false).unwrap(), "a:b=c:d");
        assert_eq!(conf_argument("^a.*", "c:d", true).unwrap(), "/^a.*/=c:d");
    }

    #[test]
    fn a_plain_pattern_with_spaces_becomes_the_equivalent_anchored_regex() {
        // The exact string this was all for, and the exact string a real
        // `hledger import` was verified to honour.
        assert_eq!(
            conf_argument(
                "PW Roth IRA - 3077",
                "assets:morganstanley:pw-roth-ira",
                false
            )
            .unwrap(),
            "/^PW.Roth.IRA.-.3077($|:)/=assets:morganstanley:pw-roth-ira\\1"
        );
    }

    #[test]
    fn metacharacters_are_escaped_before_whitespace_becomes_a_dot() {
        // A literal `.` in the bank's name must stay literal, or the mapping
        // silently matches accounts it was never meant to.
        assert_eq!(
            conf_argument("A.B C", "x", false).unwrap(),
            "/^A\\.B.C($|:)/=x\\1"
        );
        // Every space becomes its OWN dot: `.` matches exactly one character.
        assert_eq!(
            conf_argument("a  b", "x", false).unwrap(),
            "/^a..b($|:)/=x\\1"
        );
    }

    #[test]
    fn a_regex_pattern_with_spaces_only_has_its_spaces_substituted() {
        assert_eq!(
            conf_argument("^PW Roth (.+)$", "assets:\\1", true).unwrap(),
            "/^PW.Roth.(.+)$/=assets:\\1"
        );
    }

    #[test]
    fn every_inexpressible_alias_is_refused_by_name() {
        for (pattern, replacement, regex, expected) in [
            ("a", "b#c", false, ConfRefusal::Comment),
            ("a b", "c d", false, ConfRefusal::ReplacementWhitespace),
            ("a b", "c\\1", false, ConfRefusal::ReplacementBackslash),
            ("a [xy] b", "c", true, ConfRefusal::PatternBracket),
            ("a \\d b", "c", true, ConfRefusal::PatternBackslash),
            ("a/b c", "d", false, ConfRefusal::PatternSlash),
        ] {
            assert_eq!(
                conf_argument(pattern, replacement, regex),
                Err(expected),
                "{pattern:?} → {replacement:?}"
            );
        }
    }

    #[test]
    fn a_new_option_lands_in_the_general_section_not_at_eof() {
        // The trap this function exists for: appended at EOF, the option would
        // be inside `[balance]` and would never reach an import.
        let text = "--depth 3\n\n[balance]\n--no-total\n";
        assert_eq!(
            with_aliases(text, &["a=b".to_string()]),
            "--depth 3\n\n--alias=a=b\n[balance]\n--no-total\n"
        );
    }

    #[test]
    fn adding_the_same_alias_twice_adds_it_once() {
        let text = "--alias=a=b\n";
        assert_eq!(with_aliases(text, &["a=b".to_string()]), text);
    }

    #[test]
    fn a_file_with_no_final_newline_gets_one_before_the_new_option() {
        assert_eq!(
            with_aliases("--depth 3", &["a=b".to_string()]),
            "--depth 3\n--alias=a=b\n"
        );
    }

    #[test]
    fn a_crlf_file_stays_crlf() {
        assert_eq!(
            with_aliases("--depth 3\r\n", &["a=b".to_string()]),
            "--depth 3\r\n--alias=a=b\r\n"
        );
    }

    #[test]
    fn an_empty_file_is_written_without_a_leading_blank_line() {
        assert_eq!(with_aliases("", &["a=b".to_string()]), "--alias=a=b\n");
    }
}
