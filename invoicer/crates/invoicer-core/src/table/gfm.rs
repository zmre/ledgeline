//! GitHub-flavoured markdown pipe tables: found in a document, parsed with the
//! byte span of every cell, and written back aligned.
//!
//! This layer knows nothing about invoices. It is a lossless-enough model of a
//! pipe table for the bill-table engine to edit surgically:
//!
//! - every cell keeps its raw markdown text (escapes such as `\|` included),
//!   trimmed of the padding around it, and the byte span of that text in the
//!   source document;
//! - a table missing its delimiter (alignment) row still parses, and gets one
//!   when it is written;
//! - writing pads every cell to its column's width (by display width, so wide
//!   characters line up in a monospace editor), keeps the alignment markers,
//!   and never narrows a column the delimiter row already made wider
//!   ([`Layout::min_widths`]); the cell text itself is never touched;
//! - [`Table::edits`] turns a changed table into line-level splices against
//!   the source it was parsed from, verified by re-parsing.
//!
//! Tables inside fenced code blocks are not tables, and are skipped.

use std::ops::Range;
use thiserror::Error;
use unicode_width::UnicodeWidthStr;

/// A column's alignment, from its delimiter cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Alignment {
    /// `---`
    None,
    /// `:--`
    Left,
    /// `--:`
    Right,
    /// `:-:`
    Center,
}

/// A table's content: what a reader sees, with no layout.
///
/// Two tables with equal grids render identically in any GFM renderer; the
/// round-trip property is `parse(write(t)).grid == t.grid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    /// The header row's raw cell texts.
    pub header: Vec<String>,
    /// One per header cell.
    pub alignments: Vec<Alignment>,
    /// Body rows' raw cell texts. A row shorter than the header is padded
    /// with empty cells when parsed; a longer one keeps its extra cells.
    pub rows: Vec<Vec<String>>,
}

/// How a table was laid out in its source, so it can be written back in the
/// same shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Leading spaces before each line's first pipe (GFM allows up to three).
    pub indent: String,
    /// Lines end in `\r\n`.
    pub crlf: bool,
    /// The source had a delimiter row. When it did not, writing adds one.
    pub had_delimiter: bool,
    /// Per column, the content width the delimiter row allowed for (its
    /// dashes less the two padding spaces). Writing never narrows a column
    /// below this, which keeps a hand-widened column hand-widened.
    pub min_widths: Vec<usize>,
}

impl Layout {
    /// The layout of a table written fresh: no indent, `\n`, minimal widths.
    #[must_use]
    pub fn fresh() -> Self {
        Self {
            indent: String::new(),
            crlf: false,
            had_delimiter: true,
            min_widths: Vec::new(),
        }
    }
}

/// Where a parsed table sits in its source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMap {
    /// The table's bytes: from the header's first byte to the end of its last
    /// line, excluding that line's terminator.
    pub range: Range<usize>,
    /// The header's 1-based line number.
    pub first_line: usize,
    /// Each table line's bytes (terminator excluded): header, then the
    /// delimiter row when there was one, then body rows.
    pub lines: Vec<Range<usize>>,
    /// Body rows' cell spans: the trimmed cell text's bytes.
    pub cells: Vec<Vec<Range<usize>>>,
}

impl SourceMap {
    /// The 1-based line number of body row `row`.
    #[must_use]
    pub fn row_line(&self, row: usize, had_delimiter: bool) -> usize {
        self.first_line + 1 + usize::from(had_delimiter) + row
    }
}

/// A pipe table: its content, its layout, and (when parsed) its place in the
/// source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The content.
    pub grid: Grid,
    /// The layout to write it back in.
    pub layout: Layout,
    /// Where it was parsed from. `None` for a table built in memory. Kept
    /// unchanged by edits, because it describes the ORIGINAL source, which is
    /// what [`Table::edits`] splices into.
    pub source: Option<SourceMap>,
}

/// One splice into a source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The bytes replaced.
    pub range: Range<usize>,
    /// What replaces them.
    pub text: String,
}

/// Apply non-overlapping `edits` to `source`, in any order.
#[must_use]
pub fn apply_edits(source: &str, edits: &[Edit]) -> String {
    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|edit| edit.range.start);
    let (mut out, end) = sorted.iter().fold(
        (String::with_capacity(source.len()), 0usize),
        |(mut out, at), edit| {
            out.push_str(&source[at..edit.range.start]);
            out.push_str(&edit.text);
            (out, edit.range.end)
        },
    );
    out.push_str(&source[end..]);
    out
}

/// Why a table could not be written back.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WriteError {
    /// The table was built in memory and has no source to splice into.
    #[error("the table has no source text to edit")]
    NoSource,
    /// Re-parsing the written table did not give back the same table. This is
    /// a bug guard: it means a cell's text would have changed meaning.
    #[error("the rewritten table does not read back as the same table")]
    RoundTrip,
}

/// One source line: its byte range without the terminator, and whether that
/// terminator was `\r\n`.
#[derive(Debug, Clone)]
struct Line {
    range: Range<usize>,
    crlf: bool,
}

fn lines_of(source: &str) -> impl Iterator<Item = Line> + '_ {
    source.split_inclusive('\n').scan(0usize, |at, piece| {
        let start = *at;
        *at += piece.len();
        let body = piece.strip_suffix('\n').unwrap_or(piece);
        let crlf = body.ends_with('\r');
        Some(Line {
            range: start..start + body.len() - usize::from(crlf),
            crlf,
        })
    })
}

/// Up to three spaces of indent, then a pipe.
fn table_line_indent(text: &str) -> Option<usize> {
    let indent = text.len() - text.trim_start_matches(' ').len();
    (indent <= 3 && text[indent..].starts_with('|')).then_some(indent)
}

/// A code-fence opener or closer: up to three spaces, then three or more of
/// one fence character. Returns the character and the run length.
fn fence(text: &str) -> Option<(char, usize)> {
    let trimmed = text.trim_start_matches(' ');
    if text.len() - trimmed.len() > 3 {
        return None;
    }
    let first = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = trimmed.chars().take_while(|c| *c == first).count();
    (run >= 3).then_some((first, run))
}

/// Split one table line into cells: `(trimmed raw text, its byte span)`.
/// `offset` is the line's position in the source.
fn split_cells(text: &str, offset: usize) -> Vec<(String, Range<usize>)> {
    let indent = text.len() - text.trim_start_matches(' ').len();
    let body_start = indent + 1; // past the leading pipe
    let bytes = text.as_bytes();
    // Unescaped pipe positions after the leading one.
    let mut pipes = Vec::new();
    let mut escaped = false;
    for (i, b) in bytes.iter().enumerate().skip(body_start) {
        match (escaped, *b) {
            (true, _) => escaped = false,
            (false, b'\\') => escaped = true,
            (false, b'|') => pipes.push(i),
            _ => {}
        }
    }
    let trailing_pipe = text.trim_end().len() == pipes.last().map_or(0, |p| p + 1);
    let ends = pipes
        .iter()
        .copied()
        .chain((!trailing_pipe).then_some(text.len()));
    let starts = std::iter::once(body_start).chain(pipes.iter().map(|p| p + 1));
    starts
        .zip(ends)
        .map(|(start, end)| {
            let raw = &text[start..end];
            let lead = raw.len() - raw.trim_start_matches([' ', '\t']).len();
            let content = raw.trim_matches([' ', '\t']);
            let from = offset + start + lead;
            (content.to_string(), from..from + content.len())
        })
        .collect()
}

/// A delimiter cell's alignment, or `None` when it isn't one.
fn delimiter_alignment(cell: &str) -> Option<Alignment> {
    let left = cell.starts_with(':');
    let right = cell.len() > 1 && cell.ends_with(':');
    cell.get(usize::from(left)..cell.len() - usize::from(right))
        .filter(|inner| !inner.is_empty() && inner.bytes().all(|b| b == b'-'))?;
    Some(match (left, right) {
        (false, false) => Alignment::None,
        (true, false) => Alignment::Left,
        (false, true) => Alignment::Right,
        (true, true) => Alignment::Center,
    })
}

/// Every pipe table in `source`, in document order, skipping fenced code.
///
/// A run of consecutive pipe-led lines is a table when its second line is a
/// delimiter row matching the header's cell count, or — the hand-written
/// "missing alignment row" case — when the run has at least two lines.
#[must_use]
pub fn tables(source: &str) -> Vec<Table> {
    let lines: Vec<Line> = lines_of(source).collect();
    let mut found = Vec::new();
    let mut open_fence: Option<(char, usize)> = None;
    let mut i = 0;
    while i < lines.len() {
        let text = &source[lines[i].range.clone()];
        if let Some((ch, run)) = fence(text) {
            open_fence = match open_fence {
                None => Some((ch, run)),
                Some((open, need))
                    if open == ch && run >= need && text.trim().chars().all(|c| c == ch) =>
                {
                    None
                }
                still => still,
            };
            i += 1;
            continue;
        }
        if open_fence.is_some() || table_line_indent(text).is_none() {
            i += 1;
            continue;
        }
        let run_end = (i..lines.len())
            .find(|&j| table_line_indent(&source[lines[j].range.clone()]).is_none())
            .unwrap_or(lines.len());
        if let Some(table) = table_from_run(source, &lines[i..run_end], i + 1) {
            found.push(table);
        }
        i = run_end;
    }
    found
}

fn table_from_run(source: &str, run: &[Line], first_line: usize) -> Option<Table> {
    let header_line = run.first()?;
    let header_text = &source[header_line.range.clone()];
    let header: Vec<(String, Range<usize>)> = split_cells(header_text, header_line.range.start);
    let delimiter: Option<Vec<(Alignment, usize)>> = run.get(1).and_then(|line| {
        let cells = split_cells(&source[line.range.clone()], line.range.start);
        (cells.len() == header.len())
            .then(|| {
                cells
                    .iter()
                    .map(|(text, _)| delimiter_alignment(text).map(|align| (align, text.len())))
                    .collect::<Option<Vec<_>>>()
            })
            .flatten()
    });
    let had_delimiter = delimiter.is_some();
    if !had_delimiter && run.len() < 2 {
        return None;
    }
    let body = &run[1 + usize::from(had_delimiter)..];
    let (rows, cells): (Vec<Vec<String>>, Vec<Vec<Range<usize>>>) = body
        .iter()
        .map(|line| {
            let mut parsed = split_cells(&source[line.range.clone()], line.range.start);
            let pad_at = line.range.end;
            while parsed.len() < header.len() {
                parsed.push((String::new(), pad_at..pad_at));
            }
            parsed.into_iter().unzip()
        })
        .unzip();
    let (alignments, min_widths) = delimiter.map_or_else(
        || (vec![Alignment::None; header.len()], Vec::new()),
        |cells| {
            cells
                .into_iter()
                // `:--` spans the cell's padding too, so its content width is
                // the run's length less the two padding spaces.
                .map(|(align, len)| (align, len.saturating_sub(2)))
                .unzip()
        },
    );
    let last = run.last()?;
    Some(Table {
        grid: Grid {
            header: header.into_iter().map(|(text, _)| text).collect(),
            alignments,
            rows,
        },
        layout: Layout {
            indent: " ".repeat(header_text.len() - header_text.trim_start_matches(' ').len()),
            crlf: header_line.crlf,
            had_delimiter,
            min_widths,
        },
        source: Some(SourceMap {
            range: header_line.range.start..last.range.end,
            first_line,
            lines: run.iter().map(|line| line.range.clone()).collect(),
            cells,
        }),
    })
}

/// Escape every unescaped pipe in `text` and fold line breaks to spaces, so
/// it can be a cell's raw text.
#[must_use]
pub fn escape_cell(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut escaped = false;
    for ch in text.trim().chars() {
        match (escaped, ch) {
            (true, _) => {
                out.push(ch);
                escaped = false;
            }
            (false, '\\') => {
                out.push(ch);
                escaped = true;
            }
            (false, '|') => out.push_str("\\|"),
            (false, '\r' | '\n') => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

impl Table {
    /// Render the table: every cell padded to its column's width, alignment
    /// markers kept, lines joined by the source's line ending (no trailing
    /// terminator).
    #[must_use]
    pub fn render(&self) -> String {
        self.render_lines()
            .join(if self.layout.crlf { "\r\n" } else { "\n" })
    }

    fn render_lines(&self) -> Vec<String> {
        let grid = &self.grid;
        let columns = grid
            .rows
            .iter()
            .map(Vec::len)
            .chain([grid.header.len()])
            .max()
            .unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|col| {
                std::iter::once(&grid.header)
                    .chain(grid.rows.iter())
                    .filter_map(|row| row.get(col))
                    .map(|cell| width(cell))
                    .chain(self.layout.min_widths.get(col).copied())
                    .max()
                    .unwrap_or(0)
                    .max(1)
            })
            .collect();
        let capacity = self.layout.indent.len() + 1 + widths.iter().map(|w| w + 3).sum::<usize>();
        // One allocation per line: this runs on every keystroke.
        let line = |cells: &[String]| -> String {
            cells.iter().zip(&widths).fold(
                {
                    let mut out = String::with_capacity(capacity + 16);
                    out.push_str(&self.layout.indent);
                    out.push('|');
                    out
                },
                |mut out, (cell, w)| {
                    out.push(' ');
                    out.push_str(cell);
                    out.extend(std::iter::repeat_n(' ', w - width(cell)));
                    out.push_str(" |");
                    out
                },
            )
        };
        let delimiter: String = grid
            .alignments
            .iter()
            .zip(&widths)
            .map(|(align, w)| {
                let dashes = |n: usize| "-".repeat(n);
                match align {
                    Alignment::None => format!("{}|", dashes(w + 2)),
                    Alignment::Left => format!(":{}|", dashes(w + 1)),
                    Alignment::Right => format!("{}:|", dashes(w + 1)),
                    Alignment::Center => format!(":{}:|", dashes(*w)),
                }
            })
            .collect();
        std::iter::once(line(&grid.header))
            .chain(std::iter::once(format!(
                "{}|{delimiter}",
                self.layout.indent
            )))
            .chain(grid.rows.iter().map(|row| line(row)))
            .collect()
    }

    /// The same table with body cell `(row, col)` set to `text` (escaped as a
    /// cell). Out-of-range positions return the table unchanged.
    #[must_use]
    pub fn with_cell(&self, row: usize, col: usize, text: &str) -> Self {
        let rows = self
            .grid
            .rows
            .iter()
            .enumerate()
            .map(|(r, cells)| {
                if r == row {
                    cells
                        .iter()
                        .enumerate()
                        .map(|(c, cell)| {
                            if c == col {
                                escape_cell(text)
                            } else {
                                cell.clone()
                            }
                        })
                        .collect()
                } else {
                    cells.clone()
                }
            })
            .collect();
        Self {
            grid: Grid {
                rows,
                ..self.grid.clone()
            },
            ..self.clone()
        }
    }

    /// Splices that turn the source this table was parsed from into the
    /// table as it is now. Only lines whose text changes are edited; when the
    /// line count changes (a delimiter row added), the whole table is one
    /// edit. Empty when nothing would change.
    ///
    /// # Errors
    /// [`WriteError::NoSource`] for an in-memory table;
    /// [`WriteError::RoundTrip`] when the rendered text would not read back
    /// as this table (a guard, never expected).
    pub fn edits(&self, source: &str) -> Result<Vec<Edit>, WriteError> {
        let map = self.source.as_ref().ok_or(WriteError::NoSource)?;
        let rendered = self.render_lines();
        let reparsed = tables(&rendered.join("\n"));
        if reparsed.len() != 1 || reparsed[0].grid != self.grid {
            return Err(WriteError::RoundTrip);
        }
        if rendered.len() != map.lines.len() {
            return Ok(vec![Edit {
                range: map.range.clone(),
                text: self.render(),
            }]);
        }
        Ok(map
            .lines
            .iter()
            .zip(rendered)
            .filter(|(range, text)| source.get((*range).clone()) != Some(text.as_str()))
            .map(|(range, text)| Edit {
                range: range.clone(),
                text,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_keep_escaped_pipes_and_their_spans() {
        let src = "intro\n\n| a | b \\| c |\n|---|---|\n| x |  y  |\n";
        let found = tables(src);
        assert_eq!(found.len(), 1);
        let table = &found[0];
        assert_eq!(table.grid.header, ["a", "b \\| c"]);
        assert_eq!(table.grid.rows, [["x", "y"]]);
        let map = table.source.as_ref().expect("parsed");
        assert_eq!(&src[map.cells[0][1].clone()], "y");
        assert_eq!(map.first_line, 3);
    }

    #[test]
    fn fenced_tables_are_not_tables() {
        let src = "```markdown\n| a |\n|---|\n| x |\n```\n\n| b |\n|---|\n";
        let found = tables(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].grid.header, ["b"]);
    }

    #[test]
    fn a_missing_delimiter_row_is_added_on_write() {
        let src = "| a | b |\n| x | y |\n";
        let table = &tables(src)[0];
        assert!(!table.layout.had_delimiter);
        assert_eq!(table.render(), "| a | b |\n|---|---|\n| x | y |");
        let edits = table.edits(src).expect("writes");
        assert_eq!(
            apply_edits(src, &edits),
            "| a | b |\n|---|---|\n| x | y |\n"
        );
    }

    #[test]
    fn short_rows_are_padded_and_missing_trailing_pipes_accepted() {
        let table = &tables("| a | b | c |\n|:--|--:|:-:|\n| x\n")[0];
        assert_eq!(table.grid.rows, [["x", "", ""]]);
        assert_eq!(
            table.grid.alignments,
            [Alignment::Left, Alignment::Right, Alignment::Center]
        );
    }

    #[test]
    fn an_unchanged_aligned_table_produces_no_edits() {
        let src = "| a   | bb |\n|:----|---:|\n| xyz | 1  |\n";
        let table = &tables(src)[0];
        assert_eq!(table.edits(src).expect("writes"), []);
    }

    #[test]
    fn crlf_is_kept() {
        let src = "| a |\r\n|---|\r\n| xx |\r\n";
        let table = &tables(src)[0];
        let out = apply_edits(src, &table.edits(src).expect("writes"));
        assert_eq!(out, "| a  |\r\n|----|\r\n| xx |\r\n");
    }

    #[test]
    fn escape_cell_escapes_only_bare_pipes() {
        assert_eq!(escape_cell("a|b \\| c\nd"), "a\\|b \\| c d");
    }
}
