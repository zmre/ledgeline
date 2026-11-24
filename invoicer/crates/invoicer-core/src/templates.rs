//! Templates (ARCHITECTURE §3.5): any non-ignored `.html` file whose Tera
//! source mentions `invoice_num` or extends an invoice base
//! (`{% extends "_invoice_base.v1.html" %}`, PLAN Q1), and whose name
//! doesn't start with `_`: a `_` file is a base or a partial for templates
//! to extend or include, never a template of its own. A Tera comment on the
//! first line may name it and the document types it is for:
//!
//! ```text
//! {# invoicer: name="MyCo default" kinds="invoice,estimate" #}
//! ```
//!
//! Without one, the name is the file stem and the kind is `invoice`.
//! Invoices name a template by MBR's `template:` path, `/`-rooted.

use crate::doc::DocType;
use crate::notes::{Site, stem};
use ledgeline_core::vault::{Vault, VaultPath};
use serde::Serialize;

/// Templates larger than this aren't read.
const MAX_TEMPLATE_BYTES: u64 = 1024 * 1024;

/// What a template's `{# invoicer: … #}` line says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TemplateHeader {
    /// `name="…"`.
    pub name: Option<String>,
    /// `kinds="…"`, split on commas.
    pub kinds: Vec<String>,
}

/// Read the `{# invoicer: key="value" … #}` comment on the first line.
#[must_use]
pub fn header(source: &str) -> Option<TemplateHeader> {
    let first = source.trim_start_matches('\u{feff}').lines().next()?.trim();
    let inner = first.strip_prefix("{#")?.strip_suffix("#}")?.trim();
    let attrs = inner.strip_prefix("invoicer:")?;
    let mut found = TemplateHeader::default();
    let mut rest = attrs.trim_start();
    while let Some((key, after)) = rest.split_once('=') {
        let after = after.trim_start().strip_prefix('"')?;
        let end = after.find('"')?;
        let value = &after[..end];
        match key.trim() {
            "name" => found.name = Some(value.trim().to_string()).filter(|v| !v.is_empty()),
            "kinds" => {
                found.kinds = value
                    .split(',')
                    .map(|k| k.trim().to_lowercase())
                    .filter(|k| !k.is_empty())
                    .collect();
            }
            _ => {}
        }
        rest = after[end + 1..].trim_start();
    }
    Some(found)
}

/// The prefix of the invoicer's shipped base templates' file names
/// (`_invoice_base.v1.html`).
pub const BASE_PREFIX: &str = "_invoice_base";

/// Whether an HTML file is an invoice template, by its source: it mentions
/// `invoice_num`, or it extends an invoice base (a child that only
/// restyles needn't mention the number).
#[must_use]
pub fn is_template(source: &str) -> bool {
    source.contains("invoice_num") || extends_invoice_base(source)
}

/// Whether `source` has `{% extends "_invoice_base…" %}`.
fn extends_invoice_base(source: &str) -> bool {
    source.match_indices("extends").any(|(at, _)| {
        let before = source[..at].trim_end().trim_end_matches('-');
        let after = source[at + "extends".len()..].trim_start();
        before.ends_with("{%")
            && after
                .strip_prefix(['"', '\''])
                .is_some_and(|name| name.starts_with(BASE_PREFIX))
    })
}

/// Whether the file at `path` is kept for templates to extend or include
/// (its name starts with `_`), so never a template itself.
#[must_use]
pub fn is_partial(path: &VaultPath) -> bool {
    path.file_name().is_some_and(|name| name.starts_with('_'))
}

/// A template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Template {
    /// The file.
    pub path: VaultPath,
    /// Its display name.
    pub name: String,
    /// The document types it is for (`invoice` when unsaid).
    pub kinds: Vec<String>,
}

impl Template {
    /// A template from its file, or `None` when it isn't one (a `_` file,
    /// or a source that isn't a template's).
    #[must_use]
    pub fn from_source(path: VaultPath, source: &str) -> Option<Self> {
        if is_partial(&path) || !is_template(source) {
            return None;
        }
        let head = header(source).unwrap_or_default();
        Some(Self {
            name: head.name.unwrap_or_else(|| stem(&path).to_string()),
            kinds: if head.kinds.is_empty() {
                vec![DocType::Invoice.as_str().to_string()]
            } else {
                head.kinds
            },
            path,
        })
    }

    /// The `template:` value naming it: `/invoices/tpl/x.html`.
    #[must_use]
    pub fn template_path(&self) -> String {
        format!("/{}", self.path.as_str())
    }

    /// Whether it is for `kind`.
    #[must_use]
    pub fn supports(&self, kind: DocType) -> bool {
        self.kinds.iter().any(|k| k == kind.as_str())
    }
}

/// Every template among the site's HTML files, sorted by path. Files that
/// can't be read are skipped, and so are `_` files ([`is_partial`]), unread.
#[must_use]
pub fn discover(vault: &dyn Vault, site: &Site) -> Vec<Template> {
    site.others()
        .iter()
        .filter(|other| {
            other
                .path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("html"))
                && !is_partial(&other.path)
        })
        .filter_map(|other| {
            let (bytes, _) = vault
                .read_pinned(&other.path, Some(MAX_TEMPLATE_BYTES))
                .ok()?;
            let source = String::from_utf8(bytes).ok()?;
            Template::from_source(other.path.clone(), &source)
        })
        .collect()
}

/// The template a `template:` value names (`/`-rooted, as MBR reads it; a
/// value without the `/` is read from the root too).
#[must_use]
pub fn find<'a>(templates: &'a [Template], template: &str) -> Option<&'a Template> {
    let wanted = template.trim().trim_start_matches('/');
    templates.iter().find(|t| t.path.as_str() == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::SiteConfig;
    use ledgeline_core::vault::MemVault;

    #[test]
    fn the_header_names_and_kinds() {
        let head =
            header("{# invoicer: name=\"MyCo default\" kinds=\"invoice, Estimate\" #}\n<html>")
                .expect("header");
        assert_eq!(head.name.as_deref(), Some("MyCo default"));
        assert_eq!(head.kinds, ["invoice", "estimate"]);
        assert_eq!(header("<html>{# invoicer: name=\"x\" #}"), None);
        assert_eq!(header("{# other comment #}"), None);
        assert_eq!(header("{# invoicer: name=\"unclosed #}"), None);
    }

    #[test]
    fn discovery_needs_invoice_num() {
        let vault = MemVault::new()
            .with_file(
                "invoices/tpl/myco.html",
                "{# invoicer: name=\"MyCo\" #}\n{{ frontmatter.invoice_num }}",
            )
            .with_file(
                "invoices/tpl/plain.html",
                "<p>{{ frontmatter.invoice_num }}</p>",
            )
            .with_file("site/page.html", "<p>not a template</p>")
            .with_file("templates/ignored.html", "{{ frontmatter.invoice_num }}");
        let site = Site::open(&vault, SiteConfig::default());
        let all = discover(&vault, &site);
        let names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["MyCo", "plain"]);
        assert!(all[1].supports(DocType::Invoice));
        assert!(!all[1].supports(DocType::Estimate));
        assert_eq!(all[0].template_path(), "/invoices/tpl/myco.html");
        assert_eq!(
            find(&all, "/invoices/tpl/plain.html").map(|t| t.name.as_str()),
            Some("plain")
        );
        assert!(find(&all, "/invoices/tpl/missing.html").is_none());
    }

    /// PLAN Q1: a base (`_…`) is never a template, even though it mentions
    /// `invoice_num`; a child that only restyles is one, though it doesn't.
    #[test]
    fn discovery_skips_bases_and_finds_their_children() {
        let vault = MemVault::new()
            .with_file(
                "invoices/tpl/_invoice_base.v1.html",
                "<h1>{{ frontmatter.invoice_num }}</h1>{% block theme %}{% endblock theme %}",
            )
            .with_file("invoices/tpl/_lines.html", "{{ frontmatter.invoice_num }}")
            .with_file(
                "invoices/tpl/mine.html",
                "{# invoicer: name=\"Mine\" #}\n{%- extends \"_invoice_base.v1.html\" -%}\n\
                 {% block theme %}.sheet { color: red; }{% endblock theme %}",
            )
            .with_file(
                "invoices/tpl/other.html",
                "{% extends \"somewhere_else.html\" %}{% block x %}{% endblock x %}",
            )
            .with_file(
                "invoices/tpl/whole.html",
                "<p>{{ frontmatter.invoice_num }}</p>",
            );
        let site = Site::open(&vault, SiteConfig::default());
        let paths: Vec<String> = discover(&vault, &site)
            .iter()
            .map(Template::template_path)
            .collect();
        assert_eq!(
            paths,
            ["/invoices/tpl/mine.html", "/invoices/tpl/whole.html"]
        );
        let base = VaultPath::new("invoices/tpl/_invoice_base.v1.html").unwrap();
        assert!(is_partial(&base));
        assert!(Template::from_source(base, "{{ frontmatter.invoice_num }}").is_none());
        assert!(is_template("{% extends '_invoice_base.v2.html' %}"));
        assert!(!is_template("extends \"_invoice_base.v1.html\""));
    }
}
