//! The time accounts file (`time-accounts.journal`, F-accounts): its layout,
//! and edits to it that keep that layout.
//!
//! hledger ignores an *indented* `account` directive (fact 8), so the tree
//! is shown by order and spacing instead: one block per top-level branch,
//! separated by a blank line, parents before children, each block sorted by
//! segment, and the tag comments aligned in one column (the file's own, or
//! [`COMMENT_COLUMN`]).
//!
//! [`AccountsDoc`] edits it line by line: [`AccountsDoc::declare`] inserts a
//! line where the layout puts it, [`AccountsDoc::set_tag`] rewrites one
//! line's comment (through [`super::with_tag`], so a note and other tags
//! survive), and [`AccountsDoc::rename`] renames an account and its
//! descendants. Every other byte is kept, and each result is verified by
//! re-parse: the declarations must be the old ones with exactly the asked-for
//! change.

use super::book::{SeedAccount, TimeConfig};
use super::model::is_under;
use super::{TagRole, TimeError, check_account, is_tag_name, line_spans};
use crate::parse::{parse_tags, split_account_name};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Where tag comments start when the file has none to copy (1-based column
/// of the `;`, as in F-accounts).
pub const COMMENT_COLUMN: usize = 51;

/// One `account` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Index of the line in the file.
    pub line: usize,
    /// The account.
    pub name: String,
    /// The comment after `;`, trimmed (empty when none).
    pub comment: String,
    /// Its tags.
    pub tags: Vec<(String, String)>,
    start: usize,
    end: usize,
}

/// A parsed accounts file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountsDoc {
    text: String,
    declarations: Vec<Declaration>,
    newline: &'static str,
}

impl AccountsDoc {
    /// Parse the file's `account` lines (everything else is kept, unread).
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let declarations = line_spans(text)
            .into_iter()
            .enumerate()
            .filter_map(|(line, (start, end, _))| {
                let body = text[start..end].strip_prefix("account")?;
                if !body.starts_with([' ', '\t']) {
                    return None;
                }
                let (name, comment, _) = split_account_name(body.trim_start())?;
                // `split_account_name` hands back the comment after its `;`.
                let comment = comment.map(str::trim).unwrap_or("").to_string();
                Some(Declaration {
                    line,
                    tags: parse_tags(&comment),
                    name,
                    comment,
                    start,
                    end,
                })
            })
            .collect();
        Self {
            text: text.to_string(),
            declarations,
            newline: if text.contains("\r\n") { "\r\n" } else { "\n" },
        }
    }

    /// The text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The declarations, in file order.
    #[must_use]
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }

    /// The declaration of `name` (the last, as hledger's last wins).
    #[must_use]
    pub fn declaration(&self, name: &str) -> Option<&Declaration> {
        self.declarations.iter().rev().find(|d| d.name == name)
    }

    /// The comment column this file uses: the most common `;` column among
    /// its declarations, else [`COMMENT_COLUMN`].
    #[must_use]
    pub fn comment_column(&self) -> usize {
        let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
        for decl in &self.declarations {
            if let Some(at) = self.text[decl.start..decl.end].find(';') {
                *counts
                    .entry(self.text[decl.start..decl.start + at].chars().count() + 1)
                    .or_default() += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|(column, count)| (*count, *column))
            .map_or(COMMENT_COLUMN, |(column, _)| column)
    }

    /// Declare `name` with `tags`, at its place in the layout.
    ///
    /// # Errors
    /// [`TimeError::Invalid`] for a bad name or tag, or when it is already
    /// declared; [`TimeError::Verify`].
    pub fn declare(&self, name: &str, tags: &[(String, String)]) -> Result<String, TimeError> {
        check_account(name)?;
        check_tags(tags)?;
        if self.declaration(name).is_some() {
            return Err(TimeError::Invalid(format!("{name} is already declared")));
        }
        let line = render_line(name, &render_tags(tags), self.comment_column());
        let text = self.insert(name, &line);
        let mut want = self.expected();
        want.push((name.to_string(), sorted(tags.to_vec())));
        self.verify(&text, want)?;
        Ok(text)
    }

    /// Set `tag` to `value` on `name` (or remove it, for `None`), declaring
    /// the account when it isn't and a value is given.
    ///
    /// # Errors
    /// As [`Self::declare`]; [`TimeError::NotFound`] removing a tag from an
    /// undeclared account.
    pub fn set_tag(&self, name: &str, tag: &str, value: Option<&str>) -> Result<String, TimeError> {
        if !is_tag_name(tag) {
            return Err(TimeError::Invalid(format!("`{tag}` is not a tag name")));
        }
        if let Some(value) = value {
            check_tag_value(value)?;
        }
        let Some(decl) = self.declaration(name) else {
            return match value {
                Some(value) => self.declare(name, &[(tag.to_string(), value.to_string())]),
                None => Err(TimeError::NotFound(format!("{name} is not declared"))),
            };
        };
        let comment = super::with_tag(&decl.comment, tag, value);
        let line = render_line(name, &comment, self.comment_column());
        let text = splice(&self.text, decl.start..decl.end, &line);
        let want = self
            .expected()
            .into_iter()
            .map(|(n, tags)| {
                if n != name {
                    return (n, tags);
                }
                let mut tags: Vec<(String, String)> =
                    tags.into_iter().filter(|(t, _)| t != tag).collect();
                if let Some(value) = value {
                    tags.push((tag.to_string(), value.to_string()));
                }
                (n, sorted(tags))
            })
            .collect();
        self.verify(&text, want)?;
        Ok(text)
    }

    /// Rename `old` and every account below it to `new` (`old:x` →
    /// `new:x`), each line re-aligned and kept where it is.
    ///
    /// # Errors
    /// [`TimeError::Invalid`] for a bad name, or when `new` (or a renamed
    /// descendant) is already declared; [`TimeError::Verify`].
    pub fn rename(&self, old: &str, new: &str) -> Result<String, TimeError> {
        check_account(new)?;
        let column = self.comment_column();
        let renamed = |name: &str| -> Option<String> {
            is_under(name, old).then(|| format!("{new}{}", &name[old.len()..]))
        };
        let mut text = self.text.clone();
        let mut edits: Vec<(&Declaration, String)> = self
            .declarations
            .iter()
            .filter_map(|decl| renamed(&decl.name).map(|to| (decl, to)))
            .collect();
        for (_, to) in &edits {
            if self
                .declaration(to)
                .is_some_and(|d| !is_under(&d.name, old))
            {
                return Err(TimeError::Invalid(format!("{to} is already declared")));
            }
        }
        edits.sort_by_key(|(decl, _)| std::cmp::Reverse(decl.start));
        for (decl, to) in &edits {
            text = splice(
                &text,
                decl.start..decl.end,
                &render_line(to, &decl.comment, column),
            );
        }
        let want = self
            .expected()
            .into_iter()
            .map(|(name, tags)| (renamed(&name).unwrap_or(name), tags))
            .collect();
        self.verify(&text, want)?;
        Ok(text)
    }

    fn expected(&self) -> Vec<(String, Vec<(String, String)>)> {
        self.declarations
            .iter()
            .map(|d| (d.name.clone(), sorted(d.tags.clone())))
            .collect()
    }

    fn verify(
        &self,
        text: &str,
        mut want: Vec<(String, Vec<(String, String)>)>,
    ) -> Result<(), TimeError> {
        let mut got: Vec<(String, Vec<(String, String)>)> = Self::parse(text)
            .declarations
            .into_iter()
            .map(|d| (d.name, sorted(d.tags)))
            .collect();
        got.sort();
        want.sort();
        if got == want {
            Ok(())
        } else {
            Err(TimeError::Verify(
                "the accounts file's declarations would not be what was asked".to_string(),
            ))
        }
    }

    /// The text with `line` inserted where `name` belongs.
    fn insert(&self, name: &str, line: &str) -> String {
        let nl = self.newline;
        let spans = line_spans(&self.text);
        let top = top_of(name);
        let block: Vec<&Declaration> = self
            .declarations
            .iter()
            .filter(|d| top_of(&d.name) == top)
            .collect();
        let line_start = |at: usize| spans.get(at).map_or(self.text.len(), |s| s.0);
        let line_next = |at: usize| spans.get(at).map_or(self.text.len(), |s| s.2);
        if !block.is_empty() {
            let at = match block
                .iter()
                .rev()
                .find(|d| by_segment(&d.name, name) == Ordering::Less)
            {
                Some(before) => line_next(before.line),
                None => line_start(block[0].line),
            };
            return insert_at(&self.text, at, &format!("{line}{nl}"), nl);
        }
        // A new block: before the first block whose branch sorts after it.
        match self
            .declarations
            .iter()
            .find(|d| by_segment(top_of(&d.name), top) == Ordering::Greater)
        {
            Some(next) => insert_at(
                &self.text,
                line_start(next.line),
                &format!("{line}{nl}{nl}"),
                nl,
            ),
            None => {
                let mut out = self.text.clone();
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push_str(nl);
                }
                // A blank line between blocks (and after a header).
                if !out.is_empty() && !out.ends_with(&format!("{nl}{nl}")) {
                    out.push_str(nl);
                }
                out.push_str(line);
                out.push_str(nl);
                out
            }
        }
    }
}

fn insert_at(text: &str, at: usize, what: &str, nl: &str) -> String {
    let mut out = String::with_capacity(text.len() + what.len() + 2);
    out.push_str(&text[..at]);
    if at > 0 && !out.ends_with('\n') {
        out.push_str(nl);
    }
    out.push_str(what);
    out.push_str(&text[at..]);
    out
}

fn splice(text: &str, range: std::ops::Range<usize>, with: &str) -> String {
    format!("{}{with}{}", &text[..range.start], &text[range.end..])
}

fn top_of(name: &str) -> &str {
    name.split(':').next().unwrap_or(name)
}

/// Order account names segment by segment, so a parent sorts right before
/// its children (`a:b` < `a:b:c` < `a:b-x`).
#[must_use]
pub fn by_segment(a: &str, b: &str) -> Ordering {
    a.split(':').cmp(b.split(':'))
}

fn sorted(mut tags: Vec<(String, String)>) -> Vec<(String, String)> {
    tags.sort();
    tags
}

fn render_tags(tags: &[(String, String)]) -> String {
    tags.iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `account NAME`, then (when there is one) the comment with its `;` at
/// `column`, or two spaces after a name too long for it.
fn render_line(name: &str, comment: &str, column: usize) -> String {
    let head = format!("account {name}");
    if comment.is_empty() {
        return head;
    }
    let width = head.chars().count();
    let pad = column.saturating_sub(1).saturating_sub(width).max(2);
    format!("{head}{}; {comment}", " ".repeat(pad))
}

fn check_tags(tags: &[(String, String)]) -> Result<(), TimeError> {
    for (name, value) in tags {
        if !is_tag_name(name) {
            return Err(TimeError::Invalid(format!("`{name}` is not a tag name")));
        }
        check_tag_value(value)?;
    }
    Ok(())
}

pub(crate) fn check_tag_value(value: &str) -> Result<(), TimeError> {
    if value.contains([',', ';', '\n', '\r']) || value.chars().any(char::is_control) {
        return Err(TimeError::Invalid(format!(
            "`{value}` can't be a tag value: `,` or `;` would end it"
        )));
    }
    Ok(())
}

/// A new accounts file (F-accounts): the header, then `seed` laid out.
///
/// # Errors
/// [`TimeError::Invalid`] for a bad name or tag in the seed.
pub fn new_accounts_text(seed: &[SeedAccount], config: &TimeConfig) -> Result<String, TimeError> {
    let names = |role: TagRole| config.tags.name(role).to_string();
    let header = format!(
        "; Time accounts. Tags: {} (links to an Invoicer customer), {}\n\
         ; (yes/no, nearest ancestor wins), {} (catalog code to bill as),\n\
         ; {} (hide from pickers). Edit freely.\n\n",
        names(TagRole::Customer),
        names(TagRole::Billable),
        names(TagRole::Item),
        names(TagRole::Archived),
    );
    let mut seed: Vec<&SeedAccount> = seed.iter().collect();
    seed.sort_by(|a, b| by_segment(&a.name, &b.name));
    let mut out = header;
    let mut last_top: Option<&str> = None;
    for account in seed {
        check_account(&account.name)?;
        check_tags(&account.tags)?;
        let top = top_of(&account.name);
        if last_top.is_some_and(|last| last != top) {
            out.push('\n');
        }
        last_top = Some(top);
        out.push_str(&render_line(
            &account.name,
            &render_tags(&account.tags),
            COMMENT_COLUMN,
        ));
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "; header\n\naccount clients                                   ; billable: yes\naccount clients:acme                              ; customer: Acme Corp, item: ARCH\naccount clients:acme:travel                       ; billable: no\n\naccount internal                                  ; billable: no\naccount internal:admin\n";

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn new_file_matches_the_f_accounts_layout() {
        let seed = vec![
            SeedAccount {
                name: "internal:admin".into(),
                tags: vec![],
            },
            SeedAccount {
                name: "clients".into(),
                tags: tags(&[("billable", "yes")]),
            },
            SeedAccount {
                name: "internal".into(),
                tags: tags(&[("billable", "no")]),
            },
            SeedAccount {
                name: "clients:acme".into(),
                tags: tags(&[("customer", "Acme Corp"), ("item", "ARCH")]),
            },
        ];
        let text = new_accounts_text(&seed, &TimeConfig::default()).expect("text");
        assert!(
            text.ends_with(
                "account clients                                   ; billable: yes\n\
             account clients:acme                              ; customer: Acme Corp, item: ARCH\n\
             \n\
             account internal                                  ; billable: no\n\
             account internal:admin\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn declare_places_lines_in_their_block() {
        let doc = AccountsDoc::parse(FILE);
        assert_eq!(doc.comment_column(), 51);
        let text = doc.declare("clients:acme:advisory", &[]).expect("declare");
        assert!(text.contains("ARCH\naccount clients:acme:advisory\naccount clients:acme:travel"));
        let text = doc
            .declare("clients:globex", &tags(&[("customer", "Globex")]))
            .expect("declare");
        assert!(text.contains("billable: no\naccount clients:globex                            ; customer: Globex\n\naccount internal"), "{text}");
        let text = doc.declare("projects:x", &[]).expect("new block");
        assert!(
            text.ends_with("account internal:admin\n\naccount projects:x\n"),
            "{text}"
        );
        let text = doc.declare("admin", &[]).expect("first block");
        assert!(
            text.contains("; header\n\naccount admin\n\naccount clients "),
            "{text}"
        );
        assert!(doc.declare("clients", &[]).is_err());
        assert!(doc.declare("2026-11-01 x", &[]).is_err());
        assert!(doc.declare("a  b", &[]).is_err());
    }

    #[test]
    fn set_tag_and_rename_keep_everything_else() {
        let doc = AccountsDoc::parse(FILE);
        let text = doc
            .set_tag("clients:acme", "item", Some("TAX"))
            .expect("set");
        assert!(text.contains(
            "account clients:acme                              ; customer: Acme Corp, item: TAX\n"
        ));
        assert_eq!(text.len(), FILE.len() - 1);
        let text = doc
            .set_tag("internal:admin", "archived", Some("yes"))
            .expect("set");
        assert!(text.contains("account internal:admin                            ; archived: yes"));
        let text = doc
            .set_tag("clients:acme:travel", "billable", None)
            .expect("rm");
        assert!(text.contains("account clients:acme:travel\n"));
        let text = doc
            .set_tag("clients:new", "billable", Some("yes"))
            .expect("declares");
        assert!(text.contains("account clients:new "));
        let text = doc
            .rename("clients:acme", "clients:acme-corp")
            .expect("rename");
        assert!(text.contains("account clients:acme-corp                         ; customer: Acme Corp, item: ARCH\naccount clients:acme-corp:travel                  ; billable: no\n"), "{text}");
        assert!(doc.rename("clients:acme", "internal").is_err());
    }
}
