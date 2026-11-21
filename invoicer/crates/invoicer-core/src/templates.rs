//! Templates (ARCHITECTURE §3.5): any non-ignored `.html` file whose Tera
//! source mentions `invoice_num`. A Tera comment on the first line may name
//! it and the document types it is for:
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

/// Whether an HTML file is an invoice template.
#[must_use]
pub fn is_template(source: &str) -> bool {
    source.contains("invoice_num")
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
    /// A template from its file, or `None` when it isn't one.
    #[must_use]
    pub fn from_source(path: VaultPath, source: &str) -> Option<Self> {
        if !is_template(source) {
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
/// can't be read are skipped.
#[must_use]
pub fn discover(vault: &dyn Vault, site: &Site) -> Vec<Template> {
    site.others()
        .iter()
        .filter(|other| {
            other
                .path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("html"))
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
}
