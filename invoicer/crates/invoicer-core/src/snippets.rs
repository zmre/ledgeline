//! Snippets (ARCHITECTURE §3.7): `type: snippet` notes whose body is copied
//! into an invoice (payment instructions, terms, thank-yous). Copied, never
//! included live: a sent invoice must not change when a snippet does.

use crate::frontmatter::locate;
use crate::notes::{Note, Site};
use ledgeline_core::vault::{Vault, VaultError, VaultPath};
use serde::Serialize;

/// A snippet note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snippet {
    /// The note.
    pub path: VaultPath,
    /// Its title (what `[[Net 30 ACH]]` names).
    pub title: String,
    /// `snippet_kind`: `payment`, `terms`, `note`, or anything else.
    pub kind: Option<String>,
}

impl Snippet {
    /// The snippet in `note`, when it is one.
    #[must_use]
    pub fn from_note(note: &Note) -> Option<Self> {
        note.is_type("snippet").then(|| Self {
            path: note.path.clone(),
            title: note.title.clone(),
            kind: note.str("snippet_kind").map(|k| k.trim().to_lowercase()),
        })
    }

    /// The markdown to copy: the body, without its frontmatter or the blank
    /// lines around it.
    ///
    /// # Errors
    /// [`VaultError`] when the note can't be read.
    pub fn body(&self, vault: &dyn Vault) -> Result<String, VaultError> {
        let source = vault.read_to_string(&self.path)?;
        Ok(body_of(&source))
    }
}

/// A note's body: everything after the frontmatter, blank lines trimmed at
/// both ends, ending in one newline (empty when there is nothing).
#[must_use]
pub fn body_of(source: &str) -> String {
    let start = locate(source).map_or(0, |b| b.body_start);
    let body = source[start..].trim_matches(['\n', '\r']);
    if body.trim().is_empty() {
        String::new()
    } else {
        format!("{body}\n")
    }
}

/// Every snippet, sorted by path.
#[must_use]
pub fn snippets(site: &Site) -> Vec<Snippet> {
    site.notes_of_type("snippet")
        .filter_map(Snippet::from_note)
        .collect()
}

/// The snippet a link (`[[Net 30 ACH]]`) names, seen from `from`.
#[must_use]
pub fn find(site: &Site, from: &VaultPath, link: &str) -> Option<Snippet> {
    site.resolve_wikilink(from, link)
        .and_then(Snippet::from_note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::SiteConfig;
    use ledgeline_core::vault::MemVault;

    #[test]
    fn snippets_are_found_and_their_bodies_copied() {
        let vault = MemVault::new()
            .with_file(
                "invoices/tpl/snippets/net-30.md",
                "---\ntitle: Net 30 ACH\ntype: snippet\nsnippet_kind: Payment\n---\n\n## Payment\n\nPay by ACH within 30 days.\n\n",
            )
            .with_file("notes/other.md", "---\ntype: note\n---\n");
        let site = Site::open(&vault, SiteConfig::default());
        let all = snippets(&site);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].kind.as_deref(), Some("payment"));
        let found = find(&site, &VaultPath::root(), "[[Net 30 ACH]]").expect("found");
        assert_eq!(
            found.body(&vault).expect("body"),
            "## Payment\n\nPay by ACH within 30 days.\n"
        );
        assert!(find(&site, &VaultPath::root(), "[[other]]").is_none());
        assert_eq!(body_of("---\na: 1\n---\n\n\n"), "");
        assert_eq!(body_of("no frontmatter"), "no frontmatter\n");
    }
}
