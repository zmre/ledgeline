//! A light scan of Tera template text, without parsing it: its `{% … %}`
//! tags outside comments and `{% raw %}`, its `{% block %}`s, and what it
//! `{% extends %}`. For the shipped base (PLAN Q1): the app folds a starter
//! into it for a preview and sees whether a child prints with MBR's
//! export-ready signal; [`crate::template_edit`] reads which blocks a
//! template overrides (PLAN 4d). Tera itself stays the judge of what
//! renders.

/// A `{% … %}` tag in a template: where it is and its words.
pub struct Tag<'a> {
    /// Where `{%` is.
    pub start: usize,
    /// Just after `%}`.
    pub end: usize,
    /// The words between, without `-` trim marks.
    pub inner: &'a str,
}

impl<'a> Tag<'a> {
    /// The tag's words (`["block", "theme"]`).
    #[must_use]
    pub fn words(&self) -> Vec<&'a str> {
        self.inner.split_whitespace().collect()
    }
}

/// The `{% … %}` tags of `source`, outside `{# … #}` comments and
/// `{% raw %}` sections, in order.
pub fn tags(source: &str) -> impl Iterator<Item = Tag<'_>> {
    let mut at = 0;
    let mut raw = false;
    std::iter::from_fn(move || {
        loop {
            let rest = &source[at..];
            let open = rest.find("{%");
            let comment = rest.find("{#");
            match (open, comment) {
                (_, Some(c)) if open.is_none_or(|o| c < o) => {
                    let close = rest[c..].find("#}")?;
                    at += c + close + 2;
                }
                (Some(o), _) => {
                    let close = rest[o..].find("%}")?;
                    let start = at + o;
                    let end = start + close + 2;
                    let inner = source[start + 2..end - 2].trim_matches('-').trim();
                    at = end;
                    let first = inner.split_whitespace().next().unwrap_or_default();
                    if raw {
                        raw = first != "endraw";
                        continue;
                    }
                    if first == "raw" {
                        raw = true;
                        continue;
                    }
                    return Some(Tag { start, end, inner });
                }
                (None, _) => return None,
            }
        }
    })
}

/// A `{% block NAME %}…{% endblock %}` pair: its name and its body's span.
pub struct Block<'a> {
    /// The block's name.
    pub name: &'a str,
    /// Its body, between the tags.
    pub body: std::ops::Range<usize>,
}

/// Every block of `source`, by position; `None` when the block tags don't
/// pair.
#[must_use]
pub fn blocks(source: &str) -> Option<Vec<Block<'_>>> {
    let mut open: Vec<(&str, usize)> = Vec::new();
    let mut found = Vec::new();
    for tag in tags(source) {
        let words = tag.words();
        match words.first().copied() {
            Some("block") => open.push((words.get(1)?, tag.end)),
            Some("endblock") => {
                let (name, body_start) = open.pop()?;
                if words.get(1).is_some_and(|w| *w != name) {
                    return None;
                }
                found.push(Block {
                    name,
                    body: body_start..tag.start,
                });
            }
            _ => {}
        }
    }
    if !open.is_empty() {
        return None;
    }
    found.sort_by_key(|b| b.body.start);
    Some(found)
}

/// The blocks not inside another (sorted by position): a block inside
/// another is carried with it.
#[must_use]
pub fn outermost(sorted: Vec<Block<'_>>) -> Vec<Block<'_>> {
    let mut outer: Vec<Block<'_>> = Vec::new();
    for block in sorted {
        if outer.last().is_none_or(|o| block.body.start >= o.body.end) {
            outer.push(block);
        }
    }
    outer
}

/// The template `source` extends (`{% extends "x.html" %}`), if any.
#[must_use]
pub fn extends(source: &str) -> Option<&str> {
    tags(source).find_map(|tag| {
        let name = tag.inner.strip_prefix("extends")?.trim();
        let quote = name
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '\'' | '`'))?;
        name[1..].strip_suffix(quote)
    })
}

/// The body of the block `name` in `source`, if it has one.
#[must_use]
pub fn block_body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let found = blocks(source)?;
    let block = found.iter().find(|b| b.name == name)?;
    Some(&source[block.body.clone()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extends_and_block_bodies() {
        let child = "{# invoicer: name=\"x\" #}\n{%- extends \"_invoice_base.v1.html\" -%}\n\
                     {# {% block ready %}not this{% endblock %} #}{% block ready %}{% endblock ready %}";
        assert_eq!(extends(child), Some("_invoice_base.v1.html"));
        assert_eq!(block_body(child, "ready"), Some(""));
        assert_eq!(block_body(child, "theme"), None);
        assert_eq!(extends("{% block x %}{% endblock %}"), None);
        assert_eq!(extends("{% extends 'a.html' %}"), Some("a.html"));
    }
}
