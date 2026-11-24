//! The product and service catalog (ARCHITECTURE §3.6): a `type: catalog`
//! note whose body is one table keyed by a `Code` (or `SKU`) column, with the
//! bill table's grammar and books column.
//!
//! Choosing a catalog item copies it into an invoice row, resolved account
//! included, so a later catalog change never rewrites history.

use crate::doc::Money;
use crate::doc::masked_body;
use crate::frontmatter::locate;
use crate::notes::Site;
use crate::problem::{Problem, ProblemCode};
use crate::table::ItemCode;
use crate::table::books::BooksEntry;
use crate::table::columns::{Columns, Role};
use crate::table::gfm;
use crate::table::money::{is_blank, plain};
use crate::table::words::Words;
use ledgeline_core::Dec;
use ledgeline_core::vault::{Vault, VaultPath};
use serde::Serialize;

/// One catalog row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogItem {
    /// The key (`ARCH`).
    pub code: String,
    /// The description an invoice row gets.
    pub description: String,
    /// The unit (`h`, `ea`).
    pub unit: Option<String>,
    /// The rate as written (`$800`).
    pub rate_text: Option<String>,
    /// The rate.
    #[serde(skip)]
    pub rate: Option<Dec>,
    /// The books cell as written.
    pub books_text: Option<String>,
    /// The books cell, read.
    #[serde(skip)]
    pub books: Option<BooksEntry>,
    /// Its 1-based line.
    pub line: usize,
}

/// One catalog note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Catalog {
    /// The note.
    pub path: VaultPath,
    /// Its rows.
    pub items: Vec<CatalogItem>,
}

impl Catalog {
    /// Read a catalog note's table, with a problem for each row that doesn't
    /// read and each duplicate code.
    #[must_use]
    pub fn parse(
        path: VaultPath,
        source: &str,
        words: &Words,
        money: &Money,
    ) -> (Self, Vec<Problem>) {
        let mut problems = Vec::new();
        let Some((table, columns)) = catalog_table(source, words) else {
            let problem = Problem::error(
                ProblemCode::Catalog,
                format!(
                    "the catalog has no table with `{}` and `{}` columns",
                    words.item.first(),
                    words.description.first()
                ),
            )
            .in_file(&path);
            return (
                Self {
                    path,
                    items: Vec::new(),
                },
                vec![problem],
            );
        };
        let style = money.style(None);
        let first_line = table.source.as_ref().map_or(1, |m| m.first_line);
        let had_delimiter = table.layout.had_delimiter;
        let cell = |row: &[String], role: Role| -> Option<String> {
            columns
                .find(role)
                .and_then(|col| row.get(col))
                .map(|text| plain(text).to_string())
                .filter(|text| !is_blank(text))
        };
        let mut items: Vec<CatalogItem> = Vec::new();
        for (i, row) in table.grid.rows.iter().enumerate() {
            let line = first_line + 1 + usize::from(had_delimiter) + i;
            let Some(code) = cell(row, Role::Item) else {
                if row.iter().any(|c| !is_blank(c)) {
                    problems.push(
                        Problem::warning(ProblemCode::Catalog, "a catalog row has no code")
                            .at_line(Some(line)),
                    );
                }
                continue;
            };
            if items
                .iter()
                .any(|item| item.code.eq_ignore_ascii_case(&code))
            {
                problems.push(
                    Problem::error(
                        ProblemCode::Catalog,
                        format!("the code {code} is used twice"),
                    )
                    .at_line(Some(line)),
                );
                continue;
            }
            let rate_text = cell(row, Role::Rate);
            let rate = match rate_text.as_deref().map(|t| style.parse(t)) {
                None => None,
                Some(Ok(written)) => Some(written.value),
                Some(Err(e)) => {
                    problems.push(
                        Problem::error(ProblemCode::Catalog, format!("{code}: {e}"))
                            .at_line(Some(line)),
                    );
                    None
                }
            };
            let books_text = cell(row, Role::Books);
            let books = match BooksEntry::parse(books_text.as_deref().unwrap_or("")) {
                Ok(books) => books,
                Err(e) => {
                    problems.push(
                        Problem::error(ProblemCode::Catalog, format!("{code}: {e}"))
                            .at_line(Some(line)),
                    );
                    None
                }
            };
            items.push(CatalogItem {
                description: cell(row, Role::Description).unwrap_or_default(),
                unit: cell(row, Role::Unit),
                rate_text,
                rate,
                books_text,
                books,
                line,
                code,
            });
        }
        let problems = problems.into_iter().map(|p| p.in_file(&path)).collect();
        (Self { path, items }, problems)
    }

    /// The item with `code`, case-insensitively.
    #[must_use]
    pub fn item(&self, code: &str) -> Option<&CatalogItem> {
        self.items
            .iter()
            .find(|item| item.code.eq_ignore_ascii_case(code.trim()))
    }

    /// The item an [`ItemCode`] from a bill table names.
    #[must_use]
    pub fn item_for(&self, code: &ItemCode) -> Option<&CatalogItem> {
        self.item(&code.0)
    }
}

/// A catalog note's table: the first in its body whose header has a code
/// column (and a description, as every table the invoicer reads), with its
/// columns. Its source map is into `source` itself.
#[must_use]
pub fn catalog_table(source: &str, words: &Words) -> Option<(gfm::Table, Columns)> {
    let masked = masked_body(source, locate(source).as_ref());
    gfm::tables(&masked).into_iter().find_map(|table| {
        match Columns::recognize_any(&table.grid.header, words) {
            Ok(columns) if columns.find(Role::Item).is_some() => Some((table, columns)),
            _ => None,
        }
    })
}

/// Every `type: catalog` note, read. Unreadable notes are problems.
#[must_use]
pub fn load(
    vault: &dyn Vault,
    site: &Site,
    words: &Words,
    money: &Money,
) -> (Vec<Catalog>, Vec<Problem>) {
    site.notes_of_type("catalog")
        .map(|note| match vault.read_to_string(&note.path) {
            Ok(source) => Catalog::parse(note.path.clone(), &source, words, money),
            Err(e) => (
                Catalog {
                    path: note.path.clone(),
                    items: Vec::new(),
                },
                vec![
                    Problem::error(ProblemCode::Catalog, format!("can't be read: {e}"))
                        .in_file(&note.path),
                ],
            ),
        })
        .fold(
            (Vec::new(), Vec::new()),
            |(mut catalogs, mut problems), (catalog, p)| {
                catalogs.push(catalog);
                problems.extend(p);
                (catalogs, problems)
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = "---
title: Catalog
type: catalog
---

| Code     | Description              | Unit | Rate   | *Account*                                                    |
|:---------|:-------------------------|:-----|-------:|:-------------------------------------------------------------|
| ARCH     | Senior architect hours   | h    | $800   | revenue:consulting                                           |
| JR       | Junior engineer          | h    | $200   | revenue:consulting:delivery                                  |
| SAAS-Y   | Platform subscription, 1 year | ea | $12,000 | liabilities:deferred-revenue → revenue:saas, 12 months |
";

    fn path() -> VaultPath {
        VaultPath::new("invoices/catalog.md").expect("path")
    }

    #[test]
    fn reads_the_spec_catalog() {
        let (catalog, problems) =
            Catalog::parse(path(), CATALOG, &Words::default(), &Money::default());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(catalog.items.len(), 3);
        let arch = catalog.item("arch").expect("ARCH");
        assert_eq!(arch.description, "Senior architect hours");
        assert_eq!(arch.unit.as_deref(), Some("h"));
        assert_eq!(arch.rate, Dec::parse("800", '.').ok());
        assert_eq!(arch.line, 8);
        let saas = catalog.item("SAAS-Y").expect("saas");
        assert!(saas.books.as_ref().and_then(BooksEntry::deferral).is_some());
        assert_eq!(
            catalog.item_for(&ItemCode("JR".into())).map(|i| i.line),
            Some(9)
        );
    }

    #[test]
    fn duplicates_and_bad_rows_are_problems() {
        let doc = format!(
            "{CATALOG}| arch | Again | h | $1 | revenue:x |\n| BAD | Rate | h | ten | |\n| | orphan | | | |\n"
        );
        let (catalog, problems) =
            Catalog::parse(path(), &doc, &Words::default(), &Money::default());
        assert_eq!(catalog.items.len(), 4);
        let lines: Vec<Option<usize>> = problems.iter().map(|p| p.line).collect();
        assert_eq!(lines, [Some(11), Some(12), Some(13)]);
        assert!(problems.iter().all(|p| p.file == Some(path())));
    }

    #[test]
    fn a_catalog_without_its_table_is_one_problem() {
        let (catalog, problems) = Catalog::parse(
            path(),
            "---\ntype: catalog\n---\n\n| Name | Rate |\n|---|---|\n",
            &Words::default(),
            &Money::default(),
        );
        assert!(catalog.items.is_empty());
        assert_eq!(problems.len(), 1);
    }
}
