//! The bill table (ARCHITECTURE §3.3): the first table in an invoice whose
//! header has an amount column.
//!
//! - [`gfm`] parses and writes pipe tables with cell spans.
//! - [`words`] holds the recognized words (`[invoicer.words]`).
//! - [`columns`] maps header text to column roles.
//! - [`money`] reads and writes amounts in the journal's commodity style.
//! - [`books`] parses the books column into an account or a [`Deferral`].
//! - This module classifies rows, does the positional arithmetic, rewrites
//!   computed amounts, and checks hand-written ones.
//!
//! [`Deferral`]: ledgeline_core::accrual::invoice::Deferral

pub mod books;
pub mod columns;
pub mod gfm;
pub mod money;
pub mod words;

use books::{BooksEntry, BooksError};
use columns::{Columns, HeaderProblem, Role};
use gfm::{Edit, Table, WriteError};
use ledgeline_core::Dec;
use money::{MoneyStyle, NumberError, Percent, is_blank, parse_quantity, plain, unbold};
use std::fmt;
use thiserror::Error;
use words::Words;

/// A body row's position: its 1-based number in the table, its 1-based line
/// in the document, and its description (for messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowRef {
    /// 0-based index into the table's body rows.
    pub index: usize,
    /// 1-based line in the document.
    pub line: usize,
    /// The description cell, as written.
    pub description: String,
}

impl fmt::Display for RowRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = plain(&self.description);
        if description.is_empty() {
            write!(f, "row {} (line {})", self.index + 1, self.line)
        } else {
            write!(
                f,
                "row {} \u{201c}{description}\u{201d} (line {})",
                self.index + 1,
                self.line
            )
        }
    }
}

/// What is wrong with one row.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RowProblem {
    /// More cells than the header has columns.
    #[error("it has {found} cells but the header has {expected} columns")]
    TooManyCells {
        /// Cells in the row.
        found: usize,
        /// Columns in the header.
        expected: usize,
    },
    /// A number cell that doesn't read.
    #[error(transparent)]
    Number(#[from] NumberError),
    /// A books cell that doesn't parse.
    #[error(transparent)]
    Books(#[from] BooksError),
    /// A percentage rate with a quantity.
    #[error("a percentage rate makes it an adjustment, which takes no quantity")]
    PercentWithQuantity,
    /// An adjustment with no item or total above it.
    #[error("it is an adjustment, but there is no item or total above it to apply to")]
    NothingToAdjust,
    /// An item with neither `qty × rate` nor a written amount.
    #[error("it has no amount: give it a quantity and a rate, or write the amount")]
    MissingAmount,
}

/// Why a bill table can't be read, computed or written.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TableError {
    /// The header names its columns ambiguously.
    #[error("the bill table's header (line {line}): {problem}")]
    Header {
        /// The header's line.
        line: usize,
        /// What is wrong.
        problem: HeaderProblem,
    },
    /// One row is wrong.
    #[error("{at}: {problem}")]
    Row {
        /// The row.
        at: RowRef,
        /// What is wrong.
        problem: RowProblem,
    },
    /// A cell position outside the table.
    #[error("there is no cell at row {row}, column {col}")]
    NoSuchCell {
        /// 0-based row.
        row: usize,
        /// 0-based column.
        col: usize,
    },
    /// Writing back failed.
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// A catalog key, from the item (SKU) column.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemCode(pub String);

/// How a row is read (ARCHITECTURE §3.3, "Row kinds").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Every cell blank or `&nbsp;`. No amount.
    Spacer,
    /// A bold total word. The running total of every non-total row above.
    Total,
    /// No quantity and a percentage rate: that percentage of the nearest
    /// item or total above, skipping other adjustments.
    Adjustment(Percent),
    /// No quantity, no rate, and an explicitly signed amount: as written.
    FixedAdjustment,
    /// Everything else: `qty × rate`, or as written.
    Item,
}

impl RowKind {
    /// An adjustment of either kind.
    #[must_use]
    pub fn is_adjustment(self) -> bool {
        matches!(self, Self::Adjustment(_) | Self::FixedAdjustment)
    }

    /// A row an adjustment can apply to.
    #[must_use]
    pub fn is_adjustable(self) -> bool {
        matches!(self, Self::Item | Self::Total)
    }
}

/// The amount cell as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// Blank or `&nbsp;`.
    Blank,
    /// A readable amount.
    Value(Dec),
    /// Text that isn't an amount.
    Unreadable(NumberError),
}

/// One body row, classified and computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatedRow {
    /// Where it is.
    pub at: RowRef,
    /// How it reads.
    pub kind: RowKind,
    /// Its amount, rounded to the commodity's precision when computed.
    /// `None` for spacers.
    pub amount: Option<Dec>,
    /// Whether the amount is computed (totals, percentage adjustments, and
    /// items with both a quantity and a rate) rather than as written.
    pub computed: bool,
    /// For an adjustment: the index of the item or total row it applies to.
    pub applies_to: Option<usize>,
    /// The books cell.
    pub books: Option<BooksEntry>,
    /// The item (catalog) code.
    pub item: Option<ItemCode>,
    /// The amount cell as written.
    pub written: Written,
}

/// A whole table, computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    /// Every body row, in order.
    pub rows: Vec<EvaluatedRow>,
    /// The last total row's index, when there is a total row.
    pub final_total: Option<usize>,
    /// The amount due: the last total row's amount, or the sum of every
    /// non-total row when there is no total row.
    pub total: Dec,
}

/// One amount cell rewritten by [`BillTable::recompute`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellChange {
    /// The row.
    pub at: RowRef,
    /// 0-based column.
    pub col: usize,
    /// The text before.
    pub before: String,
    /// The text after.
    pub after: String,
}

/// A written amount that disagrees with the table's arithmetic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discrepancy {
    /// The row.
    pub at: RowRef,
    /// The amount as written (empty when blank).
    pub written: String,
    /// What the arithmetic gives, as the table would write it.
    pub expected: String,
}

impl fmt::Display for Discrepancy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if is_blank(&self.written) {
            write!(
                f,
                "{}: the amount is blank; it should be {}",
                self.at, self.expected
            )
        } else {
            write!(
                f,
                "{}: the amount says {}, but the arithmetic gives {}",
                self.at, self.written, self.expected
            )
        }
    }
}

/// The result of [`BillTable::recompute`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recalc {
    /// The table with every computed amount written.
    pub table: BillTable,
    /// The computation.
    pub evaluation: Evaluation,
    /// The cells that changed. Empty when the table already agreed.
    pub changes: Vec<CellChange>,
}

/// A bill table: a pipe table with its columns recognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillTable {
    table: Table,
    columns: Columns,
}

/// Running state of [`BillTable::evaluate`]'s fold.
struct Running {
    rows: Vec<EvaluatedRow>,
    sum: Dec,
}

impl BillTable {
    /// The bill table among `markdown`'s tables: the first whose header has
    /// an amount column. `Ok(None)` when there is none.
    ///
    /// # Errors
    /// [`TableError`] when that table's header is ambiguous or a row has more
    /// cells than the header.
    pub fn find(markdown: &str, words: &Words) -> Result<Option<Self>, TableError> {
        gfm::tables(markdown)
            .into_iter()
            .map(|table| Self::from_table(table, words))
            .find(|found| !matches!(found, Ok(None)))
            .unwrap_or(Ok(None))
    }

    /// `table` as a bill table, or `Ok(None)` when it has no amount column.
    /// A table missing its delimiter row gets one aligned by role (numbers
    /// right, the rest left) for when it is written.
    ///
    /// # Errors
    /// As [`BillTable::find`].
    pub fn from_table(table: Table, words: &Words) -> Result<Option<Self>, TableError> {
        let header_line = table.source.as_ref().map_or(1, |map| map.first_line);
        let Some(columns) = Columns::recognize(&table.grid.header, words).map_err(|problem| {
            TableError::Header {
                line: header_line,
                problem,
            }
        })?
        else {
            return Ok(None);
        };
        let width = table.grid.header.len();
        let bill = Self {
            columns: columns.clone(),
            table: table.clone(),
        };
        if let Some((index, row)) = table
            .grid
            .rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.iter().skip(width).any(|cell| !is_blank(cell)))
        {
            return Err(bill.row_error(
                index,
                RowProblem::TooManyCells {
                    found: row.len(),
                    expected: width,
                },
            ));
        }
        let alignments = if table.layout.had_delimiter {
            table.grid.alignments
        } else {
            columns
                .roles()
                .iter()
                .map(|r| r.default_alignment())
                .collect()
        };
        let rows = table
            .grid
            .rows
            .into_iter()
            .map(|row| row.into_iter().take(width).collect())
            .collect();
        Ok(Some(Self {
            table: Table {
                grid: gfm::Grid {
                    alignments,
                    rows,
                    ..table.grid
                },
                ..table
            },
            columns,
        }))
    }

    /// The underlying pipe table.
    #[must_use]
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// The recognized columns.
    #[must_use]
    pub fn columns(&self) -> &Columns {
        &self.columns
    }

    /// The table, written aligned.
    #[must_use]
    pub fn render(&self) -> String {
        self.table.render()
    }

    /// Where body row `index` is.
    #[must_use]
    pub fn row_ref(&self, index: usize) -> RowRef {
        let line = self.table.source.as_ref().map_or(index + 3, |map| {
            map.row_line(index, self.table.layout.had_delimiter)
        });
        RowRef {
            index,
            line,
            description: self.cell(index, Role::Description).to_string(),
        }
    }

    fn row_error(&self, index: usize, problem: RowProblem) -> TableError {
        TableError::Row {
            at: self.row_ref(index),
            problem,
        }
    }

    /// Body row `row`'s cell in the `role` column; empty when the table has
    /// no such column.
    #[must_use]
    pub fn cell(&self, row: usize, role: Role) -> &str {
        self.columns
            .find(role)
            .and_then(|col| self.table.grid.rows.get(row)?.get(col))
            .map_or("", String::as_str)
    }

    /// The commodity the table's money is written in: the symbol on the
    /// first rate or amount cell that has one.
    #[must_use]
    pub fn commodity(&self) -> Option<String> {
        let money_cols: Vec<usize> = [Role::Rate, Role::Amount]
            .iter()
            .filter_map(|role| self.columns.find(*role))
            .collect();
        self.table.grid.rows.iter().find_map(|row| {
            money_cols.iter().find_map(|col| {
                let text = plain(row.get(*col)?);
                if text.ends_with('%') {
                    return None;
                }
                let symbolic = |c: char| {
                    !(c.is_ascii_digit() || c.is_whitespace() || "-+.,'_\u{2212}".contains(c))
                };
                let body = text.trim_start_matches(['-', '+', '\u{2212}']);
                let lead: String = body.chars().take_while(|c| symbolic(*c)).collect();
                let trail: String = {
                    let rev: String = body.chars().rev().take_while(|c| symbolic(*c)).collect();
                    rev.chars().rev().collect()
                };
                [lead, trail]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .map(|s| s.trim().to_string())
            })
        })
    }

    /// Classify and compute every row.
    ///
    /// # Errors
    /// [`TableError::Row`] for the first row that can't be read or computed.
    pub fn evaluate(&self, words: &Words, money: &MoneyStyle) -> Result<Evaluation, TableError> {
        let start = Running {
            rows: Vec::with_capacity(self.table.grid.rows.len()),
            sum: Dec::zero(),
        };
        let done = (0..self.table.grid.rows.len()).try_fold(start, |running, index| {
            let row = self
                .evaluate_row(index, &running.rows, running.sum, words, money)
                .map_err(|problem| self.row_error(index, problem))?;
            let sum = match (row.kind, row.amount) {
                (RowKind::Total | RowKind::Spacer, _) | (_, None) => running.sum,
                (_, Some(amount)) => running
                    .sum
                    .add(amount)
                    .map_err(|_| self.row_error(index, NumberError::Overflow.into()))?,
            };
            let mut rows = running.rows;
            rows.push(row);
            Ok::<_, TableError>(Running { rows, sum })
        })?;
        let final_total = done.rows.iter().rposition(|row| row.kind == RowKind::Total);
        let total = final_total
            .and_then(|i| done.rows[i].amount)
            .unwrap_or(done.sum);
        Ok(Evaluation {
            rows: done.rows,
            final_total,
            total,
        })
    }

    fn evaluate_row(
        &self,
        index: usize,
        above: &[EvaluatedRow],
        running: Dec,
        words: &Words,
        money: &MoneyStyle,
    ) -> Result<EvaluatedRow, RowProblem> {
        let cells = &self.table.grid.rows[index];
        let get = |role| self.cell(index, role);
        let mark = money.decimal_mark();
        let amount_text = get(Role::Amount);
        let parsed = (!is_blank(amount_text)).then(|| money.parse(amount_text));
        let signed = matches!(parsed, Some(Ok(w)) if w.signed);
        let written = match parsed {
            None => Written::Blank,
            Some(Ok(w)) => Written::Value(w.value),
            Some(Err(error)) => Written::Unreadable(error),
        };
        let books = BooksEntry::parse(get(Role::Books))?;
        let item = Some(plain(get(Role::Item)))
            .filter(|code| !code.is_empty())
            .map(|code| ItemCode(code.to_string()));
        let row = |kind, amount, computed, applies_to| EvaluatedRow {
            at: self.row_ref(index),
            kind,
            amount,
            computed,
            applies_to,
            books: books.clone(),
            item: item.clone(),
            written: written.clone(),
        };

        if cells.iter().all(|cell| is_blank(cell)) {
            return Ok(row(RowKind::Spacer, None, false, None));
        }
        let is_total = cells.iter().enumerate().any(|(col, cell)| {
            !matches!(self.columns.role(col), Role::Amount | Role::Books)
                && unbold(cell.trim()).is_some_and(|label| words.is_total_label(label))
        });
        if is_total {
            return Ok(row(RowKind::Total, Some(running), true, None));
        }

        let quantity = Some(get(Role::Quantity))
            .filter(|text| !is_blank(text))
            .map(|text| parse_quantity(text, mark))
            .transpose()?;
        let rate_text = get(Role::Rate);
        let percent = Percent::parse(rate_text, mark).transpose()?;
        let target = above.iter().rposition(|r| r.kind.is_adjustable());
        let written_value = || match &written {
            Written::Value(value) => Ok(*value),
            Written::Blank => Err(RowProblem::MissingAmount),
            Written::Unreadable(error) => Err(error.clone().into()),
        };

        match (quantity, percent) {
            (None, Some(percent)) => {
                let target = target.ok_or(RowProblem::NothingToAdjust)?;
                let base = above[target].amount.unwrap_or_else(Dec::zero);
                let amount = base
                    .mul(percent.fraction())
                    .map_err(NumberError::from)
                    .and_then(|value| money.round(value))?;
                Ok(row(
                    RowKind::Adjustment(percent),
                    Some(amount),
                    true,
                    Some(target),
                ))
            }
            (Some(_), Some(_)) => Err(RowProblem::PercentWithQuantity),
            (None, None) if is_blank(rate_text) && signed => Ok(row(
                RowKind::FixedAdjustment,
                Some(written_value()?),
                false,
                target,
            )),
            (Some(quantity), None) if !is_blank(rate_text) => {
                let rate = money.parse(rate_text)?.value;
                let amount = quantity
                    .mul(rate)
                    .map_err(NumberError::from)
                    .and_then(|value| money.round(value))?;
                Ok(row(RowKind::Item, Some(amount), true, None))
            }
            (_, None) => {
                // A rate with no quantity must still be money.
                if !is_blank(rate_text) {
                    money.parse(rate_text)?;
                }
                Ok(row(RowKind::Item, Some(written_value()?), false, None))
            }
        }
    }

    /// Every computed amount, written where the table disagrees with it.
    /// Totals are written bold. A cell whose value already matches keeps its
    /// text (`$8,000` stays `$8,000` under a two-place style).
    ///
    /// # Errors
    /// As [`BillTable::evaluate`].
    pub fn recompute(&self, words: &Words, money: &MoneyStyle) -> Result<Recalc, TableError> {
        let evaluation = self.evaluate(words, money)?;
        let col = self.columns.find(Role::Amount).unwrap_or(0);
        let changes: Vec<CellChange> = evaluation
            .rows
            .iter()
            .filter(|row| row.computed)
            .filter_map(|row| {
                let expected = row.amount?;
                if row.written == Written::Value(expected) {
                    return None;
                }
                let text = money.format(expected);
                let after = if row.kind == RowKind::Total {
                    format!("**{text}**")
                } else {
                    text
                };
                Some(CellChange {
                    at: row.at.clone(),
                    col,
                    before: self.cell(row.at.index, Role::Amount).to_string(),
                    after,
                })
            })
            .collect();
        let table = self.with_cells(
            &changes
                .iter()
                .map(|c| (c.at.index, c.col, c.after.as_str()))
                .collect::<Vec<_>>(),
        )?;
        Ok(Recalc {
            table,
            evaluation,
            changes,
        })
    }

    /// Report every written amount that disagrees with the arithmetic,
    /// changing nothing.
    ///
    /// # Errors
    /// As [`BillTable::evaluate`].
    pub fn check(&self, words: &Words, money: &MoneyStyle) -> Result<Vec<Discrepancy>, TableError> {
        Ok(self
            .recompute(words, money)?
            .changes
            .into_iter()
            .map(|change| Discrepancy {
                at: change.at,
                written: change.before,
                expected: change.after,
            })
            .collect())
    }

    /// The table with body cell `(row, col)` set to `text`.
    ///
    /// # Errors
    /// [`TableError::NoSuchCell`] outside the table.
    pub fn with_cell(&self, row: usize, col: usize, text: &str) -> Result<Self, TableError> {
        self.with_cells(&[(row, col, text)])
    }

    /// The table with several body cells set, in one pass.
    ///
    /// # Errors
    /// [`TableError::NoSuchCell`] outside the table.
    pub fn with_cells(&self, cells: &[(usize, usize, &str)]) -> Result<Self, TableError> {
        let grid = &self.table.grid;
        if let Some((row, col, _)) = cells
            .iter()
            .find(|(row, col, _)| *row >= grid.rows.len() || *col >= grid.header.len())
        {
            return Err(TableError::NoSuchCell {
                row: *row,
                col: *col,
            });
        }
        let rows = grid
            .rows
            .iter()
            .enumerate()
            .map(|(r, row)| {
                row.iter()
                    .enumerate()
                    .map(|(c, cell)| {
                        cells
                            .iter()
                            .rev()
                            .find(|(row, col, _)| *row == r && *col == c)
                            .map_or_else(|| cell.clone(), |(_, _, text)| gfm::escape_cell(text))
                    })
                    .collect()
            })
            .collect();
        Ok(Self {
            table: Table {
                grid: gfm::Grid {
                    rows,
                    ..grid.clone()
                },
                ..self.table.clone()
            },
            columns: self.columns.clone(),
        })
    }

    /// Splices turning the source this table was parsed from into this
    /// table, verified by re-parsing. Empty when nothing changes.
    ///
    /// # Errors
    /// [`TableError::Write`].
    pub fn edits(&self, source: &str) -> Result<Vec<Edit>, TableError> {
        Ok(self.table.edits(source)?)
    }
}

/// Recompute the bill table in `markdown`. `Ok(None)` when there is no bill
/// table or every amount already agrees; otherwise the new document, with
/// only the table's changed lines rewritten.
///
/// # Errors
/// [`TableError`] from finding, computing or writing the table.
pub fn recalc_markdown(
    markdown: &str,
    words: &Words,
    money: &MoneyStyle,
) -> Result<Option<String>, TableError> {
    let Some(table) = BillTable::find(markdown, words)? else {
        return Ok(None);
    };
    let recalc = table.recompute(words, money)?;
    if recalc.changes.is_empty() {
        return Ok(None);
    }
    let edits = recalc.table.edits(markdown)?;
    Ok(Some(gfm::apply_edits(markdown, &edits)))
}
