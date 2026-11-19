//! `ledgeline.toml`, the per-ledger settings file beside the journal, read and
//! rewritten the way an untrusted file is: capped at [`MAX_FILE_BYTES`], never
//! through a symlink, and written only over the exact revision that was read
//! (retrying when another writer got there first).
//!
//! Shared by the server's settings and bank sync writers and by plan 45's
//! `[books]` table ([`crate::accrual::books_config`]), so a program built on
//! this crate edits the file exactly as Ledgeline does. Every writer edits
//! through `toml_edit`, so comments, key order and other tables come back
//! byte for byte.

use crate::vault::{Revision, Vault, VaultError, VaultPath};
use toml_edit::DocumentMut;

/// The file's name, beside the main journal.
pub const LEDGER_FILE: &str = "ledgeline.toml";

/// The largest `ledgeline.toml` that is read; a larger one is unusable.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;

/// How many times [`edit`] re-reads and re-applies after losing a race.
pub const WRITE_ATTEMPTS: usize = 3;

/// The file as read.
#[derive(Debug)]
pub enum LedgerFile {
    /// Nothing is there.
    Missing,
    /// A parsed document and the revision of exactly those bytes.
    Present {
        /// The document.
        document: Box<DocumentMut>,
        /// What a write must still find there.
        revision: Revision,
    },
    /// Present but not something to read or overwrite; the phrase says why
    /// ("is not valid TOML").
    Unusable(&'static str),
}

/// Where `ledgeline.toml` is for the journal whose main file is `main`.
///
/// # Errors
/// [`VaultError`] only if the name can't be a path segment, which it always
/// can.
pub fn ledger_path(main: &VaultPath) -> Result<VaultPath, VaultError> {
    Ok(main.parent().unwrap_or_default().child(LEDGER_FILE)?)
}

/// Read `path` capped and never through a symlink.
#[must_use]
pub fn read(vault: &dyn Vault, path: &VaultPath) -> LedgerFile {
    match vault.read_pinned(path, Some(MAX_FILE_BYTES)) {
        Ok((bytes, revision)) => String::from_utf8(bytes)
            .ok()
            .and_then(|text| text.parse::<DocumentMut>().ok())
            .map_or(LedgerFile::Unusable("is not valid TOML"), |document| {
                LedgerFile::Present {
                    document: Box::new(document),
                    revision,
                }
            }),
        Err(error) if error.is_not_found() => LedgerFile::Missing,
        Err(error) if error.kind() == std::io::ErrorKind::FileTooLarge => {
            LedgerFile::Unusable("is larger than 64 KiB")
        }
        Err(error) if error.is_not_regular_file() => {
            LedgerFile::Unusable("is not a regular file (a symbolic link is never followed)")
        }
        Err(VaultError::NotDownloaded { .. }) => LedgerFile::Unusable("is not downloaded yet"),
        Err(_) => LedgerFile::Unusable("could not be read"),
    }
}

/// What [`edit`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOutcome {
    /// The edit changed nothing, so nothing was written.
    Unchanged,
    /// The file was rewritten.
    Written,
    /// The file was created.
    Created,
}

/// Why [`edit`] wrote nothing.
#[derive(Debug, thiserror::Error)]
pub enum LedgerEditError<E> {
    /// The caller's edit refused.
    #[error("{0}")]
    Edit(E),
    /// The file is there but unusable; the phrase says why.
    #[error("{LEDGER_FILE} {0}, so it will not be written")]
    Unusable(&'static str),
    /// Every attempt lost a race with another writer.
    #[error("{LEDGER_FILE} kept changing while it was being saved; nothing was written")]
    KeptChanging,
    /// The write itself failed.
    #[error("could not save {LEDGER_FILE} ({0}); nothing was written")]
    Write(std::io::ErrorKind),
}

/// Rewrite `path` with `edit` (old text → new text), creating it — `header`
/// first, since `toml_edit` keeps a comment-only document as trailing text —
/// when it is missing. Writes only over the revision just read, re-reading
/// and re-applying up to [`WRITE_ATTEMPTS`] times when another writer got
/// there first. An unusable file is never written.
///
/// # Errors
/// See [`LedgerEditError`].
pub fn edit<E>(
    vault: &dyn Vault,
    path: &VaultPath,
    header: &str,
    edit: impl Fn(&str) -> Result<String, E>,
) -> Result<EditOutcome, LedgerEditError<E>> {
    for _ in 0..WRITE_ATTEMPTS {
        match read(vault, path) {
            LedgerFile::Missing => {
                let text = format!("{header}{}", edit("").map_err(LedgerEditError::Edit)?);
                match vault.create_new_shared(path, text.as_bytes()) {
                    Ok(_) => return Ok(EditOutcome::Created),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(LedgerEditError::Write(error.kind())),
                }
            }
            LedgerFile::Unusable(reason) => return Err(LedgerEditError::Unusable(reason)),
            LedgerFile::Present { document, revision } => {
                let before = document.to_string();
                let after = edit(&before).map_err(LedgerEditError::Edit)?;
                if after == before {
                    return Ok(EditOutcome::Unchanged);
                }
                match vault.replace(path, after.as_bytes(), &revision) {
                    Ok(_) => return Ok(EditOutcome::Written),
                    Err(VaultError::Changed { .. }) => {}
                    Err(error) => return Err(LedgerEditError::Write(error.kind())),
                }
            }
        }
    }
    Err(LedgerEditError::KeptChanging)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::MemVault;

    #[test]
    fn edit_creates_then_rewrites_and_refuses_bad_toml() {
        let vault = MemVault::with_root("/books");
        let path = VaultPath::new("ledgeline.toml").expect("path");
        let add = |text: &str| -> Result<String, ()> { Ok(format!("{text}x = 1\n")) };
        assert_eq!(
            edit(&vault, &path, "# h\n", add).expect("creates"),
            EditOutcome::Created
        );
        let add_y = |text: &str| -> Result<String, ()> { Ok(format!("{text}y = 2\n")) };
        assert_eq!(
            edit(&vault, &path, "# h\n", add_y).expect("writes"),
            EditOutcome::Written
        );
        let same = |text: &str| -> Result<String, ()> { Ok(text.to_string()) };
        assert_eq!(
            edit(&vault, &path, "", same).expect("no-op"),
            EditOutcome::Unchanged
        );
        let LedgerFile::Present { document, .. } = read(&vault, &path) else {
            panic!("present");
        };
        assert_eq!(document.to_string(), "# h\nx = 1\ny = 2\n");
        vault.write_atomic(&path, b"not [ toml").expect("writes");
        assert!(matches!(
            edit(&vault, &path, "", add),
            Err(LedgerEditError::Unusable("is not valid TOML"))
        ));
    }
}
