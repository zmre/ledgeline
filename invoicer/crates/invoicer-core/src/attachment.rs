//! What a customer's `contracts:` entry or an open PO's `attachment:` names
//! (ARCHITECTURE §3.4), read and written in this one place.
//!
//! An entry is one of three things:
//!
//! - a note, by wikilink: `"[[Customer X MSA 2026]]"`;
//! - a file in the books folder: `/customers/contracts/SOW 3.pdf` (`/` is
//!   the books folder), or a path relative to the customer note;
//! - a file outside the books folder: `~/Documents/MSA.pdf` under the
//!   user's home folder, else a `file:///Volumes/Share/MSA.pdf` URL,
//!   percent-encoded as a URL is.
//!
//! The books folder is what moves between machines, so a file outside it
//! may not be there on another one: `check` warns when it is missing.

use std::path::{Component, Path, PathBuf};

use crate::notes::{Site, resolve_relative, wikilink_target};
use ledgeline_core::vault::{Vault, VaultPath};
use serde::{Serialize, Serializer};
use thiserror::Error;

/// Where a file outside the books folder is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutsidePath {
    /// Under the user's home folder: the part after `~/`
    /// (`Documents/MSA.pdf`), `/`-separated, never climbing with `..`.
    Home(String),
    /// Anywhere else: an absolute path (a `file:///` URL, decoded).
    Absolute(PathBuf),
}

impl OutsidePath {
    /// The file's absolute path: a [`OutsidePath::Home`] path under `home`
    /// (`None` without one).
    #[must_use]
    pub fn expand(&self, home: Option<&Path>) -> Option<PathBuf> {
        match self {
            Self::Home(rest) => {
                home.map(|h| rest.split('/').fold(h.to_path_buf(), |p, s| p.join(s)))
            }
            Self::Absolute(path) => Some(path.clone()),
        }
    }

    /// Where it is, for people: `~/Documents/MSA.pdf` or
    /// `/Volumes/Share/MSA.pdf`.
    #[must_use]
    pub fn location(&self) -> String {
        match self {
            Self::Home(rest) => format!("~/{rest}"),
            Self::Absolute(path) => path.display().to_string(),
        }
    }

    /// The file's name (`MSA.pdf`).
    #[must_use]
    pub fn file_name(&self) -> String {
        match self {
            Self::Home(rest) => last_segment(rest),
            Self::Absolute(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
        }
    }
}

/// A `contracts:` entry or an open PO's `attachment:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// A note: `"[[Customer X MSA 2026]]"`, as written.
    Note {
        /// As written.
        link: String,
    },
    /// A file in the books folder: `/customers/contracts/SOW 3.pdf`, or
    /// relative to the customer note, as written.
    File {
        /// As written.
        path: String,
    },
    /// A file outside the books folder.
    Outside(OutsidePath),
}

/// Why an entry doesn't read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AttachmentError {
    /// Nothing written.
    #[error("it is empty")]
    Empty,
    /// More than one line.
    #[error("{0:?}: one line of text, please")]
    NotOneLine(String),
    /// A `file:` address that isn't a local file's.
    #[error("{0:?} isn't the address of a file on this computer")]
    BadFileUrl(String),
    /// `~/…` climbing out of the home folder with `..`.
    #[error("{0:?} climbs out of the home folder; name the file itself")]
    ClimbsOut(String),
}

/// The last `/`-separated segment of `path` that isn't empty, else `path`.
fn last_segment(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

impl Attachment {
    /// The entry `text` names.
    ///
    /// # Errors
    /// [`AttachmentError`]: empty, several lines, a `file:` URL that isn't a
    /// local file's, or `~/` climbing out with `..`.
    pub fn parse(text: &str) -> Result<Self, AttachmentError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(AttachmentError::Empty);
        }
        if text.chars().any(char::is_control) {
            return Err(AttachmentError::NotOneLine(text.to_string()));
        }
        if wikilink_target(text).is_some() {
            return Ok(Self::Note {
                link: text.to_string(),
            });
        }
        if let Some(rest) = text.strip_prefix("~/") {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() {
                return Err(AttachmentError::ClimbsOut(text.to_string()));
            }
            if rest.split('/').any(|s| s == "..") {
                return Err(AttachmentError::ClimbsOut(text.to_string()));
            }
            return Ok(Self::Outside(OutsidePath::Home(rest.to_string())));
        }
        if text
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
        {
            let bad = || AttachmentError::BadFileUrl(text.to_string());
            let url = url::Url::parse(text).map_err(|_| bad())?;
            let path = url.to_file_path().map_err(|()| bad())?;
            if !path.is_absolute() {
                return Err(bad());
            }
            return Ok(Self::Outside(OutsidePath::Absolute(path)));
        }
        Ok(Self::File {
            path: text.to_string(),
        })
    }

    /// [`Attachment::parse`] for reading a note: an entry that doesn't read
    /// is taken as a file in the books, so `check` reports it missing
    /// rather than dropping it unseen. `None` for an empty one.
    #[must_use]
    pub fn read(text: &str) -> Option<Self> {
        match Self::parse(text) {
            Ok(entry) => Some(entry),
            Err(AttachmentError::Empty) => None,
            Err(_) => Some(Self::File {
                path: text.trim().to_string(),
            }),
        }
    }

    /// The file outside the books folder at `file` (absolute): `~/…` when
    /// it is under `home`, else a `file:///` URL.
    #[must_use]
    pub fn outside(file: &Path, home: Option<&Path>) -> Self {
        let under_home = home
            .and_then(|h| file.strip_prefix(h).ok())
            .and_then(|rest| {
                rest.components()
                    .map(|c| match c {
                        Component::Normal(s) => s.to_str(),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .filter(|parts| !parts.is_empty())
            .map(|parts| parts.join("/"));
        Self::Outside(under_home.map_or_else(
            || OutsidePath::Absolute(file.to_path_buf()),
            OutsidePath::Home,
        ))
    }

    /// The note or file at `path` in the books: `[[Title]]` for a note
    /// whose title links to it and nothing else, otherwise the `/`-rooted
    /// path (a path is relative to the books folder when it starts with
    /// `/`).
    #[must_use]
    pub fn in_books(site: &Site, path: &VaultPath) -> Self {
        if let Some(note) = site.note(path) {
            let link = format!("[[{}]]", note.title);
            let safe = crate::customer_edit::link_safe_title(&note.title, "contract").is_ok();
            if safe
                && site
                    .resolve_wikilink(&VaultPath::root(), &link)
                    .is_some_and(|n| n.path == *path)
            {
                return Self::Note { link };
            }
        }
        Self::File {
            path: format!("/{path}"),
        }
    }

    /// What the note writes for it.
    #[must_use]
    pub fn entry(&self) -> String {
        match self {
            Self::Note { link } => link.clone(),
            Self::File { path } => path.clone(),
            Self::Outside(OutsidePath::Home(rest)) => format!("~/{rest}"),
            Self::Outside(OutsidePath::Absolute(path)) => url::Url::from_file_path(path)
                .map_or_else(|()| path.display().to_string(), String::from),
        }
    }

    /// What to call it from the entry alone: a link's target (`[[MSA|the
    /// MSA]]` → `MSA`), or the file's name (`/c/SOW 3.pdf` → `SOW 3.pdf`).
    /// Never the raw `[[…]]` or path.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Note { link } => wikilink_target(link).unwrap_or(link.as_str()).to_string(),
            Self::File { path } => last_segment(path),
            Self::Outside(outside) => outside.file_name(),
        }
    }

    /// Whether it is outside the books folder.
    #[must_use]
    pub fn is_outside(&self) -> bool {
        matches!(self, Self::Outside(_))
    }

    /// Where it is, seen from the customer note at `from`; `None` when it
    /// doesn't exist.
    #[must_use]
    pub fn resolve(
        &self,
        from: &VaultPath,
        site: &Site,
        vault: &dyn Vault,
        home: Option<&Path>,
    ) -> Option<AttachmentTarget> {
        match self {
            Self::Note { link } => site
                .resolve_wikilink(from, link)
                .map(|note| AttachmentTarget::Note(note.path.clone())),
            Self::File { path } => resolve_relative(from, path.trim())
                .filter(|p| vault.is_file(p))
                .map(AttachmentTarget::File),
            Self::Outside(outside) => outside
                .expand(home)
                .filter(|p| p.is_file())
                .map(AttachmentTarget::Outside),
        }
    }

    /// The entry as the customer page shows it, seen from the note at
    /// `from`.
    #[must_use]
    pub fn view(
        &self,
        from: &VaultPath,
        site: &Site,
        vault: &dyn Vault,
        home: Option<&Path>,
    ) -> AttachmentView {
        let target = self.resolve(from, site, vault, home);
        let name = match &target {
            Some(AttachmentTarget::Note(p)) => site
                .note(p)
                .map_or_else(|| self.name(), |n| n.title.clone()),
            _ => self.name(),
        };
        AttachmentView {
            entry: self.clone(),
            found: target.map(|t| match t {
                AttachmentTarget::Note(p) | AttachmentTarget::File(p) => p.to_string(),
                AttachmentTarget::Outside(p) => p.display().to_string(),
            }),
            name,
        }
    }
}

/// How an entry goes over the wire: `{kind: "note", link}`,
/// `{kind: "file", path}` or `{kind: "outside", path, location}`, where an
/// outside file's `path` is the entry as written.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Wire<'a> {
    Note { link: &'a str },
    File { path: &'a str },
    Outside { path: String, location: String },
}

impl Serialize for Attachment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Note { link } => Wire::Note { link },
            Self::File { path } => Wire::File { path },
            Self::Outside(outside) => Wire::Outside {
                path: self.entry(),
                location: outside.location(),
            },
        }
        .serialize(serializer)
    }
}

/// Where an entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentTarget {
    /// The note it links.
    Note(VaultPath),
    /// The file in the books it names.
    File(VaultPath),
    /// The file outside the books it names (absolute).
    Outside(PathBuf),
}

/// An entry and where it is: what the customer page lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttachmentView {
    /// As written.
    #[serde(flatten)]
    pub entry: Attachment,
    /// The note or file in the books (its path), or the file outside them
    /// (absolute); `None` when it doesn't exist.
    pub found: Option<String>,
    /// What to call it: the note's title, else the link's target, or the
    /// file's name.
    pub name: String,
}

/// The user's home folder, where `~/` entries start.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    std::env::home_dir().filter(|h| h.is_absolute())
}

#[cfg(test)]
mod tests;
