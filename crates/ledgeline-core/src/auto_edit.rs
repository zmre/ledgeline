//! Rewriting one `= QUERY` auto-posting block in a journal file, and nothing
//! else (plan 30 amendment 22: Settings › Automatic postings edits the main
//! journal's rules).
//!
//! # The discipline
//!
//! [`crate::periodic`]'s, for a file of the same value: an edit rewrites bytes
//! only inside the span it names, and every other byte of the file comes out
//! the slice it went in as. The only spans touched are one block's whole
//! extent (a replace or a delete, the delete taking one now-redundant blank
//! line with it) or the end of the file (an append).
//!
//! Where each block is comes from the PARSER ([`AutoPostingRule::source_span`]),
//! never from a second scan here that could disagree with it about where a
//! block ends. And whether the result still says what was asked is the
//! caller's to prove, by re-parsing the whole journal with the edited text in
//! memory and comparing its rules with [`expected_raws`] — the same split
//! `budget_api` makes with [`crate::periodic::PeriodicDoc::verify`]: only the
//! caller knows which journal the file belongs to.

use std::ops::Range;

use crate::model::AutoPostingRule;

/// One change to one file's `=` blocks. `text` is a whole block, `=` line
/// through its last posting; a missing final newline is supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleEdit {
    /// Rewrite the file's `block`th rule (0-based, among this file's own).
    Replace {
        /// Its position among the file's rules.
        block: usize,
        /// The new block.
        text: String,
    },
    /// Delete the file's `block`th rule.
    Remove {
        /// Its position among the file's rules.
        block: usize,
    },
    /// Add a rule at the end of the file, after a blank line.
    Append {
        /// The new block.
        text: String,
    },
}

/// Why an edit cannot be made. Every case is the caller's: a stale index.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuleEditError {
    /// The file has no rule at that position (any more).
    #[error("there is no automatic posting rule {0} in this file")]
    NoSuchBlock(usize),
}

/// Each rule's whole extent in `text`, `=` line through its last line, as
/// byte ranges. `rules` are the ones the parser read from THIS file, in order.
#[must_use]
pub fn block_spans<'a>(
    text: &str,
    rules: impl IntoIterator<Item = &'a AutoPostingRule>,
) -> Vec<Range<usize>> {
    let mut starts: Vec<usize> = vec![0];
    starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    let offset = |line: u32| {
        let index = usize::try_from(line.saturating_sub(1)).unwrap_or(usize::MAX);
        starts
            .get(index)
            .copied()
            .unwrap_or(text.len())
            .min(text.len())
    };
    rules
        .into_iter()
        .map(|rule| {
            let (start, end) = &rule.source_span;
            offset(start.line)..offset(end.line)
        })
        .collect()
}

/// `text` with `edit` made, every byte outside the edited span unchanged.
///
/// # Errors
/// [`RuleEditError::NoSuchBlock`] for a position `spans` does not hold.
pub fn apply(text: &str, spans: &[Range<usize>], edit: &RuleEdit) -> Result<String, RuleEditError> {
    let span = |block: usize| {
        spans
            .get(block)
            .cloned()
            .ok_or(RuleEditError::NoSuchBlock(block))
    };
    Ok(match edit {
        RuleEdit::Replace { block, text: new } => {
            let span = span(*block)?;
            splice(text, span, &terminated(new))
        }
        RuleEdit::Remove { block } => {
            let mut span = span(*block)?;
            // A block between two blank lines (or a blank line and the end)
            // leaves two in a row; take the one above with it.
            let blank_above = text[..span.start].ends_with("\n\n") || &text[..span.start] == "\n";
            let blank_below = span.end == text.len() || text[span.end..].starts_with('\n');
            if blank_above && blank_below {
                span.start -= 1;
            }
            splice(text, span, "")
        }
        RuleEdit::Append { text: new } => {
            let mut out = text.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(&terminated(new));
            out
        }
    })
}

/// What the file's rules' `raw` texts must read back as after `edit`, given
/// what they were before.
///
/// # Errors
/// [`RuleEditError::NoSuchBlock`] for a position `before` does not hold.
pub fn expected_raws(before: &[String], edit: &RuleEdit) -> Result<Vec<String>, RuleEditError> {
    let mut out = before.to_vec();
    match edit {
        RuleEdit::Replace { block, text } => {
            *out.get_mut(*block)
                .ok_or(RuleEditError::NoSuchBlock(*block))? = terminated(text);
        }
        RuleEdit::Remove { block } => {
            if *block >= out.len() {
                return Err(RuleEditError::NoSuchBlock(*block));
            }
            out.remove(*block);
        }
        RuleEdit::Append { text } => out.push(terminated(text)),
    }
    Ok(out)
}

fn terminated(block: &str) -> String {
    if block.ends_with('\n') {
        block.to_string()
    } else {
        format!("{block}\n")
    }
}

fn splice(text: &str, span: Range<usize>, with: &str) -> String {
    let mut out = String::with_capacity(text.len() + with.len());
    out.push_str(&text[..span.start]);
    out.push_str(with);
    out.push_str(&text[span.end..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;

    const JOURNAL: &str = "; my books\n\
        account revenue:sales\n\
        \n\
        = revenue:sales\n    \
            expenses:commission   *-0.03   ; aligned by hand\n    \
            assets:checking       *0.03\n\
        \n\
        = expenses:rent\n    \
            (budget:rent)  *1\n\
        \n\
        2026-01-05 sale\n    \
            revenue:sales  $-100\n    \
            assets:checking\n";

    fn rules(text: &str) -> Vec<AutoPostingRule> {
        parse_journal(text, "t.journal").expect("parses").auto_rules
    }

    fn edited(edit: &RuleEdit) -> String {
        let spans = block_spans(JOURNAL, &rules(JOURNAL));
        apply(JOURNAL, &spans, edit).expect("applies")
    }

    /// Each span holds exactly the rule's own text.
    #[test]
    fn spans_are_the_parser_s_blocks() {
        let rules = rules(JOURNAL);
        let spans = block_spans(JOURNAL, &rules);
        for (rule, span) in rules.iter().zip(&spans) {
            assert_eq!(&JOURNAL[span.clone()], rule.raw);
        }
    }

    #[test]
    fn a_replace_touches_its_block_alone() {
        let out = edited(&RuleEdit::Replace {
            block: 1,
            text: "= ^expenses:rent(:|$)\n    (budget:rent)  *2".to_string(),
        });
        assert_eq!(
            out,
            JOURNAL.replace(
                "= expenses:rent\n    (budget:rent)  *1\n",
                "= ^expenses:rent(:|$)\n    (budget:rent)  *2\n"
            )
        );
        // The hand alignment and comment of the other rule are untouched.
        assert!(out.contains("expenses:commission   *-0.03   ; aligned by hand\n"));
    }

    #[test]
    fn a_remove_takes_one_blank_line_with_it() {
        let out = edited(&RuleEdit::Remove { block: 0 });
        assert_eq!(
            out,
            "; my books\naccount revenue:sales\n\n= expenses:rent\n    (budget:rent)  *1\n\n\
             2026-01-05 sale\n    revenue:sales  $-100\n    assets:checking\n"
        );
        assert_eq!(rules(&out).len(), 1);
    }

    #[test]
    fn a_remove_at_the_end_of_a_file_leaves_no_trailing_blank() {
        let text = "2026-01-05 x\n    a  1\n    b\n\n= a\n    (c)  *1\n";
        let spans = block_spans(text, &rules(text));
        let out = apply(text, &spans, &RuleEdit::Remove { block: 0 }).expect("applies");
        assert_eq!(out, "2026-01-05 x\n    a  1\n    b\n");
    }

    #[test]
    fn an_append_goes_after_a_blank_line() {
        for (text, want) in [
            ("", "= a\n    (c)  *1\n"),
            ("x\n", "x\n\n= a\n    (c)  *1\n"),
            ("x", "x\n\n= a\n    (c)  *1\n"),
            ("x\n\n", "x\n\n= a\n    (c)  *1\n"),
        ] {
            let out = apply(
                text,
                &[],
                &RuleEdit::Append {
                    text: "= a\n    (c)  *1\n".to_string(),
                },
            )
            .expect("applies");
            assert_eq!(out, want, "{text:?}");
        }
    }

    #[test]
    fn a_stale_position_is_refused() {
        let spans = block_spans(JOURNAL, &rules(JOURNAL));
        assert_eq!(
            apply(JOURNAL, &spans, &RuleEdit::Remove { block: 2 }),
            Err(RuleEditError::NoSuchBlock(2))
        );
        assert_eq!(
            expected_raws(&[], &RuleEdit::Remove { block: 0 }),
            Err(RuleEditError::NoSuchBlock(0))
        );
    }

    /// Every edit reads back as [`expected_raws`] says.
    #[test]
    fn every_edit_reads_back_as_expected() {
        let before: Vec<String> = rules(JOURNAL).into_iter().map(|rule| rule.raw).collect();
        for edit in [
            RuleEdit::Replace {
                block: 0,
                text: "= desc:x\n    (y)  *1\n".to_string(),
            },
            RuleEdit::Remove { block: 1 },
            RuleEdit::Append {
                text: "= desc:x\n    (y)  *1".to_string(),
            },
        ] {
            let got: Vec<String> = rules(&edited(&edit))
                .into_iter()
                .map(|rule| rule.raw)
                .collect();
            assert_eq!(
                got,
                expected_raws(&before, &edit).expect("in range"),
                "{edit:?}"
            );
        }
    }
}
