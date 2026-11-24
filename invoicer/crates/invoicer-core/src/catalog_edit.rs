//! Editing the catalog (invoicer PLAN 4c, ARCHITECTURE §3.6): a catalog
//! note's table as a grid, and the note a grid makes. One library call each,
//! shared by the API and, later, the CLI.
//!
//! - [`list`] reads every `type: catalog` note afresh: its table as a
//!   [`GridForm`] (the table's own headers, raw markdown cells, as the invoice
//!   editor's grid, E2), the note's content hash and its problems.
//! - [`apply`] makes the note's text from a grid: **only the table's bytes
//!   change**. The frontmatter, the prose around the table, other tables and
//!   the line endings stay byte for byte; an unchanged grid is the file as
//!   it is. A changed table is written aligned, keeping the table's
//!   alignments, indent and hand-widened columns when its header is the same.
//! - [`save`] writes it, if the file is still the bytes the caller was shown
//!   (the content hash, as customers, K2), conditional on that revision.
//! - [`create`] writes an empty catalog at `[invoicer] catalog` when there is
//!   none.
//!
//! Validation ([`check_grid`]) refuses what `check` would report: a row
//! without a code or a description, a code used twice (case-insensitively),
//! a code the CLI's `--from-catalog CODE:QTY` couldn't name (white space, `:`
//! or `|`), a rate that isn't money in the books' style, and a books cell
//! that doesn't read (an account, or a deferral). Rows left wholly blank are
//! dropped.

use crate::catalog::{self, catalog_table};
use crate::customer_edit::content_hash;
use crate::doc::Money;
use crate::editor::{ColumnForm, GridForm, role_name};
use crate::index::DocumentIndex;
use crate::notes::{Note, Stamp};
use crate::problem::Problem;
use crate::table::books::BooksEntry;
use crate::table::columns::{Columns, Role};
use crate::table::gfm::{self, Grid, Table, escape_cell};
use crate::table::money::{is_blank, plain};
use crate::table::words::Words;
use ledgeline_core::vault::{Fingerprint, Revision, Vault, VaultError, VaultPath};
use serde::Serialize;
use thiserror::Error;

/// The longest catalog code accepted.
const MAX_CODE: usize = 40;

/// One catalog note, as the editor edits it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogForm {
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// The file's content hash: what a save names.
    pub hash: String,
    /// Its table; `None` when the note has none the invoicer reads (edit it
    /// by hand; `problems` says why).
    pub table: Option<GridForm>,
    /// What `check` reports about it.
    pub problems: Vec<Problem>,
}

/// A cell a save refuses: the form's row (0-based, blank rows counted) and
/// column, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellProblem {
    /// The row in the form.
    pub row: usize,
    /// The column, when one cell is at fault.
    pub column: Option<usize>,
    /// Why.
    pub message: String,
}

/// What a [`save`] or a [`create`] wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogSaved {
    /// The note.
    pub path: VaultPath,
    /// Its content hash now: what the next save names.
    pub hash: String,
    /// Whether the file was written (`false`: the grid was the file).
    pub written: bool,
    /// How many items it holds now.
    pub items: usize,
}

/// Why a catalog edit refused. Nothing is written when one is returned.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// Cells that don't read.
    #[error("{}", describe(.0))]
    Invalid(Vec<CellProblem>),
    /// The grid's header, or the note's table, can't be edited.
    #[error("{0}")]
    Table(String),
    /// No note there.
    #[error("{0}: no such note")]
    NotFound(VaultPath),
    /// The note isn't `type: catalog`.
    #[error("{0} isn't a catalog note (`type: catalog`)")]
    NotACatalog(VaultPath),
    /// The file isn't what the caller was shown: read it again.
    #[error("{path} changed on disk since it was opened; reload it to see the change")]
    Changed {
        /// The note.
        path: VaultPath,
        /// Its hash now.
        current: String,
    },
    /// The file is already there.
    #[error("{0} already exists")]
    Exists(VaultPath),
    /// The file isn't UTF-8 text.
    #[error("{0} isn't UTF-8 text")]
    NotText(VaultPath),
    /// The file can't be read or written.
    #[error(transparent)]
    Vault(#[from] VaultError),
}

fn describe(problems: &[CellProblem]) -> String {
    problems
        .iter()
        .map(|p| format!("row {}: {}", p.row + 1, p.message))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The form of one catalog note's text.
#[must_use]
pub fn form_of(path: VaultPath, source: &str, words: &Words, money: &Money) -> CatalogForm {
    let note = note_of(&path, source);
    let (_, problems) = catalog::Catalog::parse(path.clone(), source, words, money);
    let table = catalog_table(source, words).map(|(table, columns)| GridForm {
        columns: table
            .grid
            .header
            .iter()
            .enumerate()
            .map(|(col, header)| ColumnForm {
                header: header.clone(),
                role: role_name(columns.role(col)).to_string(),
            })
            .collect(),
        rows: table
            .grid
            .rows
            .iter()
            .map(|row| {
                (0..table.grid.header.len())
                    .map(|col| row.get(col).cloned().unwrap_or_default())
                    .collect()
            })
            .collect(),
    });
    CatalogForm {
        title: note.title,
        hash: content_hash(source.as_bytes()),
        path,
        table,
        problems,
    }
}

/// Every catalog note, read now (not as the index last saw it), by path.
#[must_use]
pub fn list(index: &DocumentIndex, vault: &dyn Vault) -> Vec<CatalogForm> {
    let words = &index.settings().words;
    index
        .site()
        .notes_of_type("catalog")
        .filter_map(|note| {
            let source = vault.read_to_string(&note.path).ok()?;
            Some(form_of(note.path.clone(), &source, words, index.money()))
        })
        .collect()
}

fn note_of(path: &VaultPath, source: &str) -> Note {
    Note::from_source(
        path.clone(),
        source,
        Stamp {
            modified: None,
            size: source.len() as u64,
            fingerprint: Fingerprint::of_bytes(source.as_bytes()),
        },
    )
}

/// Whether `code` is one the CLI's `CODE[:QTY]` grammar can name.
fn code_problem(code: &str) -> Option<String> {
    if code.chars().any(char::is_whitespace) {
        return Some(format!("the code {code:?} can't hold a space"));
    }
    if let Some(c) = code.chars().find(|c| matches!(c, ':' | '|' | '\\')) {
        return Some(format!(
            "the code {code} can't hold {c:?} (`--from-catalog CODE:QTY` reads it)"
        ));
    }
    if code.chars().count() > MAX_CODE {
        return Some(format!("the code is longer than {MAX_CODE} characters"));
    }
    None
}

/// A grid that reads: what [`check_grid`] answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedGrid {
    /// The header cells.
    pub header: Vec<String>,
    /// Their roles.
    pub columns: Columns,
    /// The body rows, one cell per column.
    pub rows: Vec<Vec<String>>,
}

/// The header and body a grid makes, every cell as a table holds it (one
/// line, pipes escaped), rows left wholly blank dropped; or every cell that
/// doesn't read.
///
/// # Errors
/// [`CatalogError::Table`] for a header the catalog can't have,
/// [`CatalogError::Invalid`] for cells.
pub fn check_grid(
    grid: &GridForm,
    words: &Words,
    money: &Money,
) -> Result<CheckedGrid, CatalogError> {
    let header: Vec<String> = grid
        .columns
        .iter()
        .map(|c| escape_cell(c.header.trim()))
        .collect();
    if header.is_empty() {
        return Err(CatalogError::Table("the table has no columns".into()));
    }
    let columns = Columns::recognize_any(&header, words)
        .map_err(|problem| CatalogError::Table(problem.to_string()))?;
    let Some(item_col) = columns.find(Role::Item) else {
        return Err(CatalogError::Table(format!(
            "the table needs a `{}` column",
            words.item.first()
        )));
    };
    let width = header.len();
    let style = money.style(None);
    let mut problems = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(grid.rows.len());
    let mut codes: Vec<String> = Vec::new();
    for (at, row) in grid.rows.iter().enumerate() {
        if row.iter().all(|c| is_blank(c)) {
            continue;
        }
        if row.iter().skip(width).any(|c| !c.trim().is_empty()) {
            problems.push(CellProblem {
                row: at,
                column: None,
                message: format!("{} cells, but the table has {width} columns", row.len()),
            });
            continue;
        }
        let cells: Vec<String> = (0..width)
            .map(|col| row.get(col).map_or_else(String::new, |c| escape_cell(c)))
            .collect();
        let cell = |role: Role| -> Option<(usize, &str)> {
            columns
                .find(role)
                .map(|col| (col, plain(&cells[col])))
                .filter(|(_, text)| !is_blank(text))
        };
        let mut problem = |column: usize, message: String| {
            problems.push(CellProblem {
                row: at,
                column: Some(column),
                message,
            });
        };
        match cell(Role::Item) {
            None => problem(item_col, "a row needs a code".into()),
            Some((col, code)) => {
                if let Some(why) = code_problem(code) {
                    problem(col, why);
                } else if codes.iter().any(|c| c.eq_ignore_ascii_case(code)) {
                    problem(col, format!("the code {code} is used twice"));
                } else {
                    codes.push(code.to_string());
                }
            }
        }
        if cell(Role::Description).is_none() {
            let col = columns.find(Role::Description).unwrap_or(item_col);
            problem(col, "a row needs a description".into());
        }
        if let Some((col, rate)) = cell(Role::Rate)
            && let Err(e) = style.parse(rate)
        {
            problem(col, format!("the rate {rate}: {e}"));
        }
        if let Some((col, books)) = cell(Role::Books)
            && let Err(e) = BooksEntry::parse(books)
        {
            problem(col, e.to_string());
        }
        rows.push(cells);
    }
    if problems.is_empty() {
        Ok(CheckedGrid {
            header,
            columns,
            rows,
        })
    } else {
        Err(CatalogError::Invalid(problems))
    }
}

/// The text `grid` makes of the catalog note `source`: its table replaced,
/// everything else as it is (see the module docs). Only reads.
///
/// # Errors
/// As [`check_grid`]; [`CatalogError::Table`] when the note has no catalog
/// table to replace.
pub fn apply(
    source: &str,
    grid: &GridForm,
    words: &Words,
    money: &Money,
) -> Result<String, CatalogError> {
    let Some((written, _)) = catalog_table(source, words) else {
        return Err(CatalogError::Table(format!(
            "the note has no table with `{}` and `{}` columns to edit; add one by hand",
            words.item.first(),
            words.description.first()
        )));
    };
    let Some(range) = written.source.as_ref().map(|s| s.range.clone()) else {
        return Err(CatalogError::Table(
            "the table has no place in the note".into(),
        ));
    };
    let CheckedGrid {
        header,
        columns,
        rows,
    } = check_grid(grid, words, money)?;
    let same_header = written.grid.header == header;
    let width = written.grid.header.len();
    let padded = |row: &Vec<String>| -> Vec<String> {
        (0..width.max(row.len()))
            .map(|c| row.get(c).cloned().unwrap_or_default())
            .collect()
    };
    if same_header
        && written.grid.rows.len() == rows.len()
        && written
            .grid
            .rows
            .iter()
            .zip(&rows)
            .all(|(a, b)| padded(a) == padded(b))
    {
        return Ok(source.to_string());
    }
    let (alignments, layout) = if same_header {
        (written.grid.alignments.clone(), written.layout.clone())
    } else {
        let mut layout = gfm::Layout::fresh();
        layout.indent.clone_from(&written.layout.indent);
        layout.crlf = written.layout.crlf;
        (
            columns
                .roles()
                .iter()
                .map(|r| r.default_alignment())
                .collect(),
            layout,
        )
    };
    let table = Table {
        grid: Grid {
            header,
            alignments,
            rows,
        },
        layout,
        source: None,
    };
    let mut out = String::with_capacity(source.len() + 64);
    out.push_str(&source[..range.start]);
    out.push_str(&table.render());
    out.push_str(&source[range.end..]);
    Ok(out)
}

/// The catalog note at `path`, if the file is still the bytes `expected`
/// names.
fn read_expected(
    vault: &dyn Vault,
    path: &VaultPath,
    expected: &str,
) -> Result<(String, Revision), CatalogError> {
    let (bytes, revision) = vault.read_pinned(path, None).map_err(|e| {
        if e.is_not_found() {
            CatalogError::NotFound(path.clone())
        } else {
            CatalogError::Vault(e)
        }
    })?;
    let source = String::from_utf8(bytes).map_err(|_| CatalogError::NotText(path.clone()))?;
    if !note_of(path, &source).is_type("catalog") {
        return Err(CatalogError::NotACatalog(path.clone()));
    }
    let current = revision.fingerprint.token();
    if current != expected {
        return Err(CatalogError::Changed {
            path: path.clone(),
            current,
        });
    }
    Ok((source, revision))
}

fn items_in(text: &str, path: &VaultPath, words: &Words, money: &Money) -> usize {
    catalog::Catalog::parse(path.clone(), text, words, money)
        .0
        .items
        .len()
}

/// Save `grid` as the table of the catalog note at `path`, if the file is
/// still the bytes `expected` (a content hash) names. Re-indexed.
///
/// # Errors
/// [`CatalogError::Changed`] (an outside edit), [`CatalogError::NotFound`],
/// [`CatalogError::NotACatalog`], as [`apply`], or a vault error.
pub fn save(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    expected: &str,
    grid: &GridForm,
) -> Result<CatalogSaved, CatalogError> {
    let (source, revision) = read_expected(vault, path, expected)?;
    let words = index.settings().words.clone();
    let text = apply(&source, grid, &words, index.money())?;
    let written = text != source;
    if written {
        vault
            .replace(path, text.as_bytes(), &revision)
            .map_err(|e| match e {
                VaultError::Changed { .. } => CatalogError::Changed {
                    path: path.clone(),
                    current: vault
                        .read(path)
                        .map(|b| content_hash(&b))
                        .unwrap_or_default(),
                },
                other => CatalogError::Vault(other),
            })?;
        index.refresh(vault, std::slice::from_ref(path));
    }
    Ok(CatalogSaved {
        path: path.clone(),
        hash: content_hash(text.as_bytes()),
        written,
        items: items_in(&text, path, &words, index.money()),
    })
}

/// The text of a new, empty catalog: `title: Catalog`, `type: catalog`, and
/// a table with the code, description, unit and rate words and the books
/// column (`*Account*`).
#[must_use]
pub fn empty_catalog(words: &Words) -> String {
    let table = Table {
        grid: Grid {
            header: vec![
                words.item.first().to_string(),
                words.description.first().to_string(),
                words.unit.first().to_string(),
                words.rate.first().to_string(),
                "*Account*".to_string(),
            ],
            alignments: [
                Role::Item,
                Role::Description,
                Role::Unit,
                Role::Rate,
                Role::Books,
            ]
            .iter()
            .map(|r| r.default_alignment())
            .collect(),
            rows: Vec::new(),
        },
        layout: gfm::Layout::fresh(),
        source: None,
    };
    format!(
        "---\ntitle: Catalog\ntype: catalog\n---\n\n{}\n",
        table.render()
    )
}

/// Create the catalog note `[invoicer] catalog` names (`invoices/catalog.md`
/// by default), empty ([`empty_catalog`]). It never replaces a file.
///
/// # Errors
/// [`CatalogError::Exists`], [`CatalogError::Table`] for a setting that
/// isn't a markdown path in the books, or a vault error.
pub fn create(index: &mut DocumentIndex, vault: &dyn Vault) -> Result<CatalogSaved, CatalogError> {
    let name = index
        .settings()
        .catalog
        .trim()
        .trim_start_matches('/')
        .to_string();
    let path = VaultPath::new(&name)
        .ok()
        .filter(|p| index.site().config().is_markdown(p))
        .ok_or_else(|| {
            CatalogError::Table(format!(
                "[invoicer] catalog = {name:?} isn't a markdown file in the books folder"
            ))
        })?;
    let text = empty_catalog(&index.settings().words);
    if let Some(parent) = path.parent() {
        vault.create_dir_all(&parent)?;
    }
    vault
        .create_new_shared(&path, text.as_bytes())
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                CatalogError::Exists(path.clone())
            } else {
                CatalogError::Vault(e)
            }
        })?;
    index.refresh(vault, std::slice::from_ref(&path));
    Ok(CatalogSaved {
        hash: content_hash(text.as_bytes()),
        written: true,
        items: 0,
        path,
    })
}

#[cfg(test)]
mod tests;
