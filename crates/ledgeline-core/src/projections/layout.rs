//! A scenario file's text, cut into PIECES in file order (plan 31 C4, C7).
//!
//! Pure: this module reads lines and nothing else. It does not parse a rule,
//! and it does not know what a scenario is. [`super::serialize`] joins its
//! pieces to the journal parser's rules and to the model.
//!
//! # The pieces
//!
//! Everything after the header (and the blank lines that follow it, the
//! LEAD) is one piece or the blank lines after one (its GAP):
//!
//! | piece | the lines |
//! |---|---|
//! | `~` block | a column-1 `~` line and the indented, non-blank lines after it |
//! | `=` block | the same, from a column-1 `=` line |
//! | disabled block | a commented-out `~` or `=` block the caller confirmed (C7) |
//! | note / section | a run of column-1 comment lines; a section has a divider |
//! | block comment | `comment` through `end comment`, or to EOF |
//! | fixed | any other column-1 line, its indented lines, and the lines like it that follow |
//!
//! A note directly above a `~`, `=` or disabled block, with no blank line
//! between and no divider in it, is that block's ATTACHED comment and part of
//! its piece.
//!
//! # The extents agree with the parser's
//!
//! A `~` block's extent is [`crate::periodic::PeriodicDoc`]'s and an `=`
//! block's is [`crate::auto_edit::block_spans`]', both of which are the
//! parser's: a block ends at the first blank or column-1 line, and
//! `comment` … `end comment` is skipped whole (unterminated, it runs to EOF,
//! as hledger reads it). Tests assert both over every fixture.

use std::ops::Range;

/// The characters hledger reads as starting a top-level comment line
/// (`parse.rs`'s dispatch).
const COMMENT_CHARS: [char; 3] = [';', '#', '*'];

/// What one piece of a scenario file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawKind {
    /// A `~` block.
    Tilde,
    /// An `=` block.
    Auto,
    /// A commented-out block the caller confirmed as exactly one `~` rule.
    DisabledTilde,
    /// A commented-out block the caller confirmed as exactly one `=` rule.
    DisabledAuto,
    /// A run of comment lines with no divider.
    Note,
    /// A run of comment lines with a divider line.
    Section,
    /// `comment` … `end comment`.
    Block,
    /// Any other journal text.
    Fixed,
}

impl RawKind {
    /// Whether a comment run directly above this piece is attached to it.
    const fn takes_attached(self) -> bool {
        matches!(
            self,
            Self::Tilde | Self::Auto | Self::DisabledTilde | Self::DisabledAuto
        )
    }
}

/// What a commented-out candidate parsed as, alone (C7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confirmed {
    /// Exactly one `~` rule.
    Tilde,
    /// Exactly one `=` rule.
    Auto,
}

/// One piece, as byte ranges of the scanned text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawPiece {
    pub(crate) kind: RawKind,
    /// The whole piece: its attached comment, then its content. Every line
    /// with its terminator; the last line of a file may have none.
    pub(crate) span: Range<usize>,
    /// The attached comment: a prefix of `span`, empty (at `span.start`) for
    /// none.
    pub(crate) attached: Range<usize>,
    /// The blank lines after the piece, up to the next one.
    pub(crate) gap: Range<usize>,
    /// A disabled piece's comment prefix (`; `, `;`, `# ` …), which every
    /// line of its content carries.
    pub(crate) prefix: Option<String>,
}

#[cfg(test)]
impl RawPiece {
    /// The piece without its attached comment.
    pub(crate) fn content(&self) -> Range<usize> {
        self.attached.end..self.span.end
    }
}

/// A scanned file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scan {
    /// The blank lines between the header and the first piece.
    pub(crate) lead: Range<usize>,
    /// Every piece, in file order.
    pub(crate) pieces: Vec<RawPiece>,
}

/// One line of the text.
#[derive(Debug, Clone, Copy)]
struct Line {
    /// Its first byte.
    start: usize,
    /// Its last content byte, exclusive: the terminator (`\n` or `\r\n`) is
    /// not content.
    content_end: usize,
    /// Its terminator's end, exclusive.
    end: usize,
}

/// How a line reads at the top level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Blank,
    Indented,
    Comment,
    Tilde,
    Auto,
    CommentDirective,
    Other,
}

fn lines_of(text: &str, from: usize) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = from;
    while start < text.len() {
        let rest = &text[start..];
        let (content, full) = match rest.find('\n') {
            Some(at) => (at - usize::from(rest[..at].ends_with('\r')), at + 1),
            None => (rest.len(), rest.len()),
        };
        out.push(Line {
            start,
            content_end: start + content,
            end: start + full,
        });
        start += full;
    }
    out
}

fn classify(content: &str) -> Class {
    if content.trim().is_empty() {
        Class::Blank
    } else if content.starts_with([' ', '\t']) {
        Class::Indented
    } else if content.starts_with(COMMENT_CHARS) {
        Class::Comment
    } else if content.starts_with('~') {
        Class::Tilde
    } else if content.starts_with('=') {
        Class::Auto
    } else if content.split_whitespace().next() == Some("comment") {
        Class::CommentDirective
    } else {
        Class::Other
    }
}

/// Whether a comment line is a DIVIDER (C4.4): with its whitespace removed,
/// three or more characters, every one of them `- = _ ~ * # ;`.
///
/// `;;;;;;;;` and `; ---------` are dividers. A lone `;` or `;;` is not: it is
/// how a multi-paragraph note spaces itself, and reading it as a divider would
/// detach an attached comment from its rule (plan 31 amendment, C1 #2).
pub(crate) fn is_divider(content: &str) -> bool {
    let marks: Vec<char> = content.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && marks.iter().all(|c| "-=_~*#;".contains(*c))
}

/// A unit before attachment: a line range of the text and what it is.
struct Unit {
    kind: RawKind,
    lines: Range<usize>,
    prefix: Option<String>,
}

/// Cut `text` after `header_end` into pieces.
///
/// `confirm` is handed each commented-out CANDIDATE's stripped text (C7: the
/// comment prefix removed from every line, `\n` line ends) and answers what it
/// parses as alone, or `None` for prose. Candidates are offered in file order.
pub(crate) fn scan(
    text: &str,
    header_end: usize,
    confirm: &mut dyn FnMut(&str) -> Option<Confirmed>,
) -> Scan {
    let lines = lines_of(text, header_end);
    let content = |at: usize| &text[lines[at].start..lines[at].content_end];
    let class = |at: usize| classify(content(at));
    let n = lines.len();

    let mut at = 0;
    while at < n && class(at) == Class::Blank {
        at += 1;
    }
    let lead = header_end..lines.get(at).map_or(text.len(), |line| line.start);

    // Units, each with the number of blank lines after it.
    let mut units: Vec<(Unit, Range<usize>)> = Vec::new();
    while at < n {
        let start = at;
        let mut found: Vec<Unit> = Vec::new();
        match class(at) {
            Class::Tilde | Class::Auto => {
                at += 1;
                while at < n && class(at) == Class::Indented {
                    at += 1;
                }
                found.push(Unit {
                    kind: if class(start) == Class::Tilde {
                        RawKind::Tilde
                    } else {
                        RawKind::Auto
                    },
                    lines: start..at,
                    prefix: None,
                });
            }
            Class::CommentDirective => {
                at += 1;
                while at < n && content(at).trim() != "end comment" {
                    at += 1;
                }
                at = (at + 1).min(n);
                found.push(Unit {
                    kind: RawKind::Block,
                    lines: start..at,
                    prefix: None,
                });
            }
            Class::Comment => {
                while at < n && class(at) == Class::Comment {
                    at += 1;
                }
                found.extend(carve(&lines, text, start..at, confirm));
            }
            Class::Indented | Class::Other => {
                at += 1;
                while at < n && matches!(class(at), Class::Indented | Class::Other) {
                    at += 1;
                }
                found.push(Unit {
                    kind: RawKind::Fixed,
                    lines: start..at,
                    prefix: None,
                });
            }
            Class::Blank => unreachable!("blank lines are consumed as gaps"),
        }
        let gap_start = at;
        while at < n && class(at) == Class::Blank {
            at += 1;
        }
        // Units carved from one comment run follow each other with no gap.
        let last = found.len().saturating_sub(1);
        for (index, unit) in found.into_iter().enumerate() {
            let gap = if index == last {
                gap_start..at
            } else {
                unit.lines.end..unit.lines.end
            };
            units.push((unit, gap));
        }
    }

    // Attachment (C4.5): a prose run with no divider, directly above a block
    // that takes one, becomes part of that block's piece.
    let bytes = |range: &Range<usize>| -> Range<usize> {
        let start = lines.get(range.start).map_or(text.len(), |line| line.start);
        match range.end.checked_sub(1).and_then(|last| lines.get(last)) {
            Some(last) if !range.is_empty() => start..last.end,
            _ => start..start,
        }
    };
    let mut pieces: Vec<RawPiece> = Vec::new();
    let mut index = 0;
    while index < units.len() {
        let (unit, gap) = &units[index];
        let span = bytes(&unit.lines);
        if unit.kind == RawKind::Note
            && gap.is_empty()
            && let Some((next, next_gap)) = units.get(index + 1)
            && next.kind.takes_attached()
        {
            pieces.push(RawPiece {
                kind: next.kind,
                span: span.start..bytes(&next.lines).end,
                attached: span,
                gap: bytes(next_gap),
                prefix: next.prefix.clone(),
            });
            index += 2;
            continue;
        }
        pieces.push(RawPiece {
            kind: unit.kind,
            attached: span.start..span.start,
            span,
            gap: bytes(gap),
            prefix: unit.prefix.clone(),
        });
        index += 1;
    }
    Scan { lead, pieces }
}

/// The units one comment run is made of: its confirmed disabled blocks, and
/// the prose before, between and after them, each a note or a section.
fn carve(
    lines: &[Line],
    text: &str,
    run: Range<usize>,
    confirm: &mut dyn FnMut(&str) -> Option<Confirmed>,
) -> Vec<Unit> {
    let content = |at: usize| &text[lines[at].start..lines[at].content_end];
    let prose = |range: Range<usize>| Unit {
        kind: if range.clone().any(|at| is_divider(content(at))) {
            RawKind::Section
        } else {
            RawKind::Note
        },
        lines: range,
        prefix: None,
    };
    let mut out = Vec::new();
    let mut prose_start = run.start;
    let mut at = run.start;
    while at < run.end {
        if let Some((prefix, end, stripped)) = candidate(lines, text, at, run.end)
            && let Some(confirmed) = confirm(&stripped)
        {
            if prose_start < at {
                out.push(prose(prose_start..at));
            }
            out.push(Unit {
                kind: match confirmed {
                    Confirmed::Tilde => RawKind::DisabledTilde,
                    Confirmed::Auto => RawKind::DisabledAuto,
                },
                lines: at..end,
                prefix: Some(prefix),
            });
            at = end;
            prose_start = end;
            continue;
        }
        at += 1;
    }
    if prose_start < run.end {
        out.push(prose(prose_start..run.end));
    }
    out
}

/// A comment line's prefix: its comment character and at most one space.
fn prefix_of(content: &str) -> Option<&str> {
    let first = content
        .chars()
        .next()
        .filter(|c| COMMENT_CHARS.contains(c))?;
    let len = first.len_utf8() + usize::from(content[first.len_utf8()..].starts_with(' '));
    Some(&content[..len])
}

/// The disabled-entry CANDIDATE starting at line `at` of a comment run ending
/// at `end` (C7): its prefix, the line after its last, and its stripped text.
///
/// A candidate starts at a line whose stripped text begins with `~` or `=`.
/// Its body is the following lines that carry the same prefix and whose
/// stripped text is indented and non-blank. It needs at least one body line
/// that is not itself a comment (a posting). A body line in a DIFFERENT
/// prefix makes it no candidate at all, so a mixed `;`/`#` run is never
/// sliced.
fn candidate(lines: &[Line], text: &str, at: usize, end: usize) -> Option<(String, usize, String)> {
    let content = |line: usize| &text[lines[line].start..lines[line].content_end];
    let header = content(at);
    let prefix = prefix_of(header)?;
    let head = &header[prefix.len()..];
    if !head.starts_with(['~', '=']) {
        return None;
    }
    let body_of = |line: &str, prefix: &str| -> Option<String> {
        let rest = line.strip_prefix(prefix)?;
        (rest.starts_with([' ', '\t']) && !rest.trim().is_empty()).then(|| rest.to_string())
    };
    let mut stripped = format!("{head}\n");
    let mut last = at + 1;
    let mut posting = false;
    while last < end {
        let Some(body) = body_of(content(last), prefix) else {
            break;
        };
        posting |= !body.trim_start().starts_with(';');
        stripped.push_str(&body);
        stripped.push('\n');
        last += 1;
    }
    // A line that would continue the body under its OWN prefix, in a
    // different one: a mixed run, never sliced.
    let mixed = last < end
        && prefix_of(content(last))
            .is_some_and(|other| other != prefix && body_of(content(last), other).is_some());
    (posting && !mixed).then(|| (prefix.to_string(), last, stripped))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A confirm that accepts every candidate, as the kind its first byte
    /// says. The real one parses.
    fn any(stripped: &str) -> Option<Confirmed> {
        if stripped.starts_with('~') {
            Some(Confirmed::Tilde)
        } else {
            Some(Confirmed::Auto)
        }
    }

    fn kinds(text: &str) -> Vec<(RawKind, String, String, String)> {
        let scan = scan(text, 0, &mut any);
        scan.pieces
            .iter()
            .map(|piece| {
                (
                    piece.kind,
                    text[piece.attached.clone()].to_string(),
                    text[piece.content()].to_string(),
                    text[piece.gap.clone()].to_string(),
                )
            })
            .collect()
    }

    /// Every byte after the header is the lead, a piece or a gap, in order.
    fn assert_tiles(text: &str, header_end: usize) {
        let scan = scan(text, header_end, &mut any);
        let mut at = scan.lead.end;
        assert_eq!(scan.lead.start, header_end);
        for piece in &scan.pieces {
            assert_eq!(piece.span.start, at, "{text:?}");
            assert_eq!(piece.gap.start, piece.span.end, "{text:?}");
            at = piece.gap.end;
        }
        assert_eq!(at, text.len(), "{text:?}");
    }

    #[test]
    fn the_pieces_tile_the_text() {
        for text in [
            "",
            "\n\n",
            "~ monthly\n    a  $1\n",
            "~ monthly\n    a  $1",
            "; x\n\n~ monthly\n    a  $1\n\n\n= a\n    (b)  *1\n",
            "account a\naccount b\n2026-01-01 x\n    a  1\n    b\n",
            "comment\n~ monthly\n    a  $1\n",
            "~ monthly\r\n    a  $1\r\n\r\n; note\r\n",
        ] {
            assert_tiles(text, 0);
        }
        assert_tiles("; projection: x\n\n~ monthly\n    a  $1\n", 16);
    }

    #[test]
    fn a_note_directly_above_a_rule_is_attached_and_a_gap_detaches_it() {
        let got = kinds("; about a\n~ monthly\n    a  $1\n\n; free\n\n~ monthly\n    b  $2\n");
        assert_eq!(got[0].0, RawKind::Tilde);
        assert_eq!(got[0].1, "; about a\n");
        assert_eq!(got[0].2, "~ monthly\n    a  $1\n");
        assert_eq!(got[1].0, RawKind::Note);
        assert_eq!(got[1].2, "; free\n");
        assert_eq!(got[2].1, "");
    }

    #[test]
    fn a_divider_run_directly_above_a_rule_is_a_section_and_not_attached() {
        let got = kinds(";;;;;;;;;;;;;;;;;;;;;;\n;; Operational expenses\n~ monthly\n    a  $1\n");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, RawKind::Section);
        assert_eq!(got[0].3, "", "no blank line between, and still a section");
        assert_eq!(got[1].1, "");
        for divider in [";;;;;;;;", "; ---------", "# ===", "* ~~~", ";;;"] {
            assert!(is_divider(divider), "{divider}");
        }
        for prose in [";", ";;", "; - a list item", "; = Totals =", "; note"] {
            assert!(!is_divider(prose), "{prose}");
        }
    }

    #[test]
    fn hash_and_star_runs_are_comments_too() {
        let got = kinds("# a hash note\n* an org heading\n\n~ monthly\n    a  $1\n");
        assert_eq!(got[0].0, RawKind::Note);
        assert_eq!(got[0].2, "# a hash note\n* an org heading\n");
        let got = kinds("# about a\n~ monthly\n    a  $1\n");
        assert_eq!(got[0].1, "# about a\n");
    }

    #[test]
    fn a_comment_block_is_one_piece_and_unterminated_runs_to_eof() {
        let got = kinds("comment\n~ monthly\n    a  $1\nend comment\n\n~ weekly\n    b  $1\n");
        assert_eq!(got[0].0, RawKind::Block);
        assert_eq!(got[0].2, "comment\n~ monthly\n    a  $1\nend comment\n");
        assert_eq!(got[1].0, RawKind::Tilde);
        let got = kinds("comment\n~ monthly\n    a  $1\n\n~ weekly\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, RawKind::Block);
    }

    #[test]
    fn a_comment_between_a_rule_and_eof_and_a_missing_final_newline() {
        let got = kinds("~ monthly\n    a  $1\n; trailing");
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].0, RawKind::Note);
        assert_eq!(got[1].2, "; trailing");
    }

    #[test]
    fn crlf_lines_keep_their_terminators_inside_the_piece() {
        let got = kinds("; about\r\n~ monthly\r\n    a  $1\r\n\r\n");
        assert_eq!(got[0].1, "; about\r\n");
        assert_eq!(got[0].2, "~ monthly\r\n    a  $1\r\n");
        assert_eq!(got[0].3, "\r\n");
    }

    #[test]
    fn a_commented_out_block_is_a_candidate_and_its_note_attaches() {
        let text = "; the with alternative\n; ~ monthly from 2027-01-01  acme\n;     revenues:acme  $-40000\n;     assets:checking\n";
        let mut seen = Vec::new();
        let scan = scan(text, 0, &mut |stripped| {
            seen.push(stripped.to_string());
            Some(Confirmed::Tilde)
        });
        assert_eq!(
            seen,
            ["~ monthly from 2027-01-01  acme\n    revenues:acme  $-40000\n    assets:checking\n"]
        );
        assert_eq!(scan.pieces.len(), 1);
        let piece = &scan.pieces[0];
        assert_eq!(piece.kind, RawKind::DisabledTilde);
        assert_eq!(&text[piece.attached.clone()], "; the with alternative\n");
        assert_eq!(piece.prefix.as_deref(), Some("; "));
    }

    #[test]
    fn prose_look_alikes_are_not_candidates() {
        for text in [
            "; ~ monthly is how often\n",
            "; = Totals =\n",
            "; ~ monthly\n; not indented\n",
            "; ~ monthly\n;     ; only a comment\n",
            ";~ monthly\n#    a  $1\n",
        ] {
            let mut offered = false;
            let got = scan(text, 0, &mut |_| {
                offered = true;
                Some(Confirmed::Tilde)
            });
            assert!(!offered, "{text:?} was offered");
            assert!(
                got.pieces
                    .iter()
                    .all(|piece| matches!(piece.kind, RawKind::Note | RawKind::Section)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_mixed_prefix_run_is_never_sliced() {
        let mut offered = false;
        let got = scan("; ~ monthly\n;     a  $1\n#     b  $2\n", 0, &mut |_| {
            offered = true;
            Some(Confirmed::Tilde)
        });
        assert!(!offered);
        assert_eq!(got.pieces.len(), 1);
        assert_eq!(got.pieces[0].kind, RawKind::Note);
    }

    #[test]
    fn a_refused_candidate_stays_in_its_run() {
        let text = "; before\n; ~ monthly\n;     a  $1\n; after\n\n";
        let got = scan(text, 0, &mut |_| None);
        assert_eq!(got.pieces.len(), 1);
        assert_eq!(got.pieces[0].kind, RawKind::Note);
        assert_eq!(
            &text[got.pieces[0].span.clone()],
            "; before\n; ~ monthly\n;     a  $1\n; after\n"
        );
    }

    #[test]
    fn two_disabled_blocks_in_one_run_and_the_prose_between() {
        let text = "; ~ monthly\n;     a  $1\n; about b\n; = b\n;     (c)  *1\n; tail\n";
        let got = kinds(text);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].0, RawKind::DisabledTilde);
        assert_eq!(got[0].3, "");
        assert_eq!(got[1].0, RawKind::DisabledAuto);
        assert_eq!(got[1].1, "; about b\n");
        assert_eq!(got[2].0, RawKind::Note);
        assert_eq!(got[2].2, "; tail\n");
    }

    #[test]
    fn a_prefix_without_its_space_is_its_own_prefix() {
        let text = ";~ monthly\n;    a  $1\n";
        let scan = scan(text, 0, &mut any);
        assert_eq!(scan.pieces[0].kind, RawKind::DisabledTilde);
        assert_eq!(scan.pieces[0].prefix.as_deref(), Some(";"));
    }

    #[test]
    fn fixed_text_takes_its_indented_lines_and_its_neighbours() {
        let got =
            kinds("account a\n    ; a note on it\naccount b\n\n2026-01-01 x\n    a  1\n    b\n");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, RawKind::Fixed);
        assert_eq!(got[0].2, "account a\n    ; a note on it\naccount b\n");
        assert_eq!(got[1].2, "2026-01-01 x\n    a  1\n    b\n");
    }
}
