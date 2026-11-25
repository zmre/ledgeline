//! An invoice document (ARCHITECTURE §3.2): frontmatter fields, prose
//! before, the bill table, prose after, and any supporting tables.
//!
//! Parsing never fails: a document that doesn't read is still a document,
//! and [`Invoice::problems`] says what is wrong with it, with lines and
//! fix-its, so `check` can report every file at once. Prose is located, not
//! parsed: it is free markdown that only the renderer reads.

use crate::frontmatter::{self, Block, Frontmatter, FrontmatterError, KeyLine};
use crate::problem::{FixEdit, Problem, ProblemCode};
use crate::table::books::is_iso_date;
use crate::table::gfm::{self, Table, apply_edits};
use crate::table::money::MoneyStyle;
use crate::table::words::Words;
use crate::table::{BillTable, Evaluation, RowKind, TableError};
use ledgeline_core::{Dec, Journal};
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::ops::Range;

/// The document types (`type:`), fixed file-format words (§3.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocType {
    /// `invoice`.
    Invoice,
    /// `estimate` (later).
    Estimate,
    /// `credit-note` (later).
    CreditNote,
}

impl DocType {
    /// Every type, in the order `type:` values are listed.
    pub const ALL: [Self; 3] = [Self::Invoice, Self::Estimate, Self::CreditNote];

    /// The `type:` value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invoice => "invoice",
            Self::Estimate => "estimate",
            Self::CreditNote => "credit-note",
        }
    }

    /// Read a `type:` value, case-insensitively.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str().eq_ignore_ascii_case(text.trim()))
    }

    /// The statuses this type allows.
    #[must_use]
    pub fn statuses(self) -> &'static [Status] {
        match self {
            Self::Invoice | Self::CreditNote => &[Status::Draft, Status::Sent, Status::Void],
            Self::Estimate => &[Status::EstimateOpen, Status::EstimateClosed],
        }
    }
}

/// `status:`: the author's intent (I7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Never booked.
    Draft,
    /// Booked (accrual) or tracked (cash).
    Sent,
    /// Reversed.
    Void,
    /// An estimate not yet accepted or declined.
    EstimateOpen,
    /// An estimate done with.
    EstimateClosed,
}

impl Status {
    /// The `status:` value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Sent => "sent",
            Self::Void => "void",
            Self::EstimateOpen => "estimate-open",
            Self::EstimateClosed => "estimate-closed",
        }
    }

    /// Read a `status:` value, case-insensitively.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::Draft,
            Self::Sent,
            Self::Void,
            Self::EstimateOpen,
            Self::EstimateClosed,
        ]
        .into_iter()
        .find(|s| s.as_str().eq_ignore_ascii_case(text.trim()))
    }
}

/// A document number (`invoice_num`): the `invoice:` tag value in the books.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct InvoiceNumber(String);

impl InvoiceNumber {
    /// A number, or `None` when it can't be a journal tag value: empty, or
    /// with a comma or a line break (§3.2).
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        (!text.is_empty() && !text.contains([',', '\n', '\r'])).then(|| Self(text.to_string()))
    }

    /// The number as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InvoiceNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One payment in the `paid:` mirror.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaidPart {
    /// When.
    pub date: String,
    /// How much, as written (`$1,500`).
    pub amount: String,
}

/// The `paid:` mirror (I7): `no`, `partial <date> <amount>, …`, or the date
/// it was paid in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Paid {
    /// Nothing paid.
    No,
    /// Part paid, by these payments.
    Partial {
        /// The payments.
        parts: Vec<PaidPart>,
    },
    /// Paid in full on this date.
    Full {
        /// The date.
        date: String,
    },
}

impl Paid {
    /// Read the mirror's grammar, or `None` when it doesn't follow it.
    #[must_use]
    pub fn parse(value: &Value) -> Option<Self> {
        let text = match value {
            Value::Bool(false) => return Some(Self::No),
            Value::String(s) => s.trim(),
            _ => return None,
        };
        if text.eq_ignore_ascii_case("no") {
            return Some(Self::No);
        }
        if is_iso_date(text) {
            return Some(Self::Full {
                date: text.to_string(),
            });
        }
        let rest = text.strip_prefix("partial ")?.trim();
        // Amounts hold commas (`$1,500`), so parts split only at a comma
        // followed by a date.
        let mut starts: Vec<usize> = vec![0];
        starts.extend(rest.match_indices(", ").filter_map(|(at, sep)| {
            let next = at + sep.len();
            rest.get(next..next + 10)
                .is_some_and(is_iso_date)
                .then_some(next)
        }));
        let parts: Option<Vec<PaidPart>> = starts
            .iter()
            .enumerate()
            .map(|(i, &start)| {
                let end = starts.get(i + 1).map_or(rest.len(), |next| next - 2);
                let piece = rest[start..end].trim();
                let (date, amount) = piece.split_once(' ')?;
                (is_iso_date(date) && !amount.trim().is_empty()).then(|| PaidPart {
                    date: date.to_string(),
                    amount: amount.trim().to_string(),
                })
            })
            .collect();
        parts
            .filter(|p| !p.is_empty())
            .map(|parts| Self::Partial { parts })
    }
}

impl fmt::Display for Paid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::No => f.write_str("no"),
            Self::Full { date } => f.write_str(date),
            Self::Partial { parts } => {
                f.write_str("partial ")?;
                let text: Vec<String> = parts
                    .iter()
                    .map(|p| format!("{} {}", p.date, p.amount))
                    .collect();
                f.write_str(&text.join(", "))
            }
        }
    }
}

/// One side of an invoice: the link and the snapshot (`invoicee_*` or
/// `invoicer_*`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Party {
    /// `customer:` or `invoicer:`, as written (`[[Customer X]]`).
    pub link: Option<String>,
    /// `_name`.
    pub name: Option<String>,
    /// `_email`.
    pub email: Option<String>,
    /// `_phone`.
    pub phone: Option<String>,
    /// `_address`.
    pub address: Option<String>,
}

/// The frontmatter fields the invoicer reads, as written. A field of the
/// wrong shape reads as absent here; [`Invoice::problems`] names it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Header {
    /// `type`.
    pub doc_type: Option<DocType>,
    /// `invoice_num`, exactly as written (`0042` stays `0042`).
    pub number: Option<InvoiceNumber>,
    /// `date`.
    pub date: Option<String>,
    /// `status`.
    pub status: Option<Status>,
    /// `template`.
    pub template: Option<String>,
    /// The customer: `customer` and `invoicee_*`.
    pub invoicee: Party,
    /// Your company: `invoicer` and `invoicer_*`.
    pub invoicer: Party,
    /// `terms`, net days.
    pub terms: Option<i64>,
    /// `due_date`.
    pub due_date: Option<String>,
    /// `reference`.
    pub reference: Option<String>,
    /// `paid`.
    pub paid: Option<Paid>,
    /// `sent_date`.
    pub sent_date: Option<String>,
    /// `sent_via`.
    pub sent_via: Option<String>,
    /// `void_date`.
    pub void_date: Option<String>,
    /// `deposit_for`.
    pub deposit_for: Option<String>,
}

/// Where each part of the document is, as byte ranges.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Layout {
    /// The frontmatter block.
    pub frontmatter: Option<Range<usize>>,
    /// Prose between the frontmatter and the bill table (the whole body
    /// when there is no bill table).
    pub prose_before: Range<usize>,
    /// The bill table.
    pub bill_table: Option<Range<usize>>,
    /// Prose after the bill table (supporting tables included).
    pub prose_after: Range<usize>,
    /// Other tables, left alone.
    pub supporting_tables: Vec<Range<usize>>,
}

/// The journal's money styles, per commodity: what bill tables are read and
/// written in (`$8,000` matches how the books write `$`).
#[derive(Debug, Clone)]
pub struct Money {
    declared: Vec<MoneyStyle>,
    default_commodity: String,
}

impl Default for Money {
    fn default() -> Self {
        Self::fallback("$")
    }
}

impl Money {
    /// Every commodity the journal declares a style for; the first declared
    /// is the default for a table with no symbol.
    #[must_use]
    pub fn of_journal(journal: &Journal) -> Self {
        let declared: Vec<MoneyStyle> = journal
            .commodity_styles
            .iter()
            .map(|(c, _)| MoneyStyle::of_journal(journal, &c.0))
            .collect();
        let default_commodity = declared
            .first()
            .map_or_else(|| "$".to_string(), |s| s.commodity().to_string());
        Self {
            declared,
            default_commodity,
        }
    }

    /// No journal: [`MoneyStyle::fallback`] for every commodity.
    #[must_use]
    pub fn fallback(default_commodity: &str) -> Self {
        Self {
            declared: Vec::new(),
            default_commodity: default_commodity.to_string(),
        }
    }

    /// The style for `commodity` (the default when `None`).
    #[must_use]
    pub fn style(&self, commodity: Option<&str>) -> MoneyStyle {
        let commodity = commodity.unwrap_or(&self.default_commodity);
        self.declared
            .iter()
            .find(|s| s.commodity() == commodity)
            .cloned()
            .unwrap_or_else(|| MoneyStyle::fallback(commodity))
    }

    /// An amount written outside a table (a PO's `amount: $40,000`): its
    /// commodity and value, read in the default commodity's style first, then
    /// each declared one's. `None` when none reads it.
    #[must_use]
    pub fn read(&self, text: &str) -> Option<(String, ledgeline_core::Dec)> {
        std::iter::once(self.style(None))
            .chain(self.declared.iter().cloned())
            .find_map(|style| {
                style
                    .parse(text)
                    .ok()
                    .map(|m| (style.commodity().to_string(), m.value))
            })
    }

    /// The style a bill table is written in: its own symbol's.
    #[must_use]
    pub fn for_table(&self, table: &BillTable) -> MoneyStyle {
        self.style(table.commodity().as_deref())
    }
}

/// What the index keeps of a bill table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableSummary {
    /// The amount due: the last total.
    #[serde(skip)]
    pub total: Dec,
    /// The amount due, written as the table writes money.
    pub total_text: String,
    /// The commodity.
    pub commodity: String,
    /// Every account in the books column, in first-use order.
    pub accounts: Vec<String>,
    /// Body rows.
    pub rows: usize,
    /// Written amounts that disagree with the arithmetic.
    pub discrepancies: usize,
}

/// An invoice (or estimate, or credit note) document.
#[derive(Debug, Clone)]
pub struct Invoice {
    source: String,
    frontmatter: Frontmatter,
    frontmatter_error: Option<FrontmatterError>,
    keys: Vec<KeyLine>,
    header: Header,
    bill: Result<Option<BillTable>, TableError>,
    layout: Layout,
}

/// `source` with its frontmatter blanked (every byte but line breaks made a
/// space), so table scanning can't see YAML and offsets and lines hold.
pub(crate) fn masked_body<'a>(source: &'a str, block: Option<&Block>) -> std::borrow::Cow<'a, str> {
    match block {
        None => std::borrow::Cow::Borrowed(source),
        Some(block) => {
            let mut out = String::with_capacity(source.len());
            out.extend(
                source[..block.body_start]
                    .bytes()
                    .map(|b| if b == b'\n' { '\n' } else { ' ' }),
            );
            // Multi-byte characters blank to one space per byte, so the body
            // starts at the same offset.
            out.push_str(&source[block.body_start..]);
            std::borrow::Cow::Owned(out)
        }
    }
}

fn table_range(table: &Table) -> Option<Range<usize>> {
    table.source.as_ref().map(|map| map.range.clone())
}

/// A field as text: strings as they are, numbers as written in the source
/// when the parser normalized them (`0042`).
fn text_of(fields: &frontmatter::Fields, keys: &[KeyLine], key: &str) -> Option<String> {
    match fields.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(
            keys.iter()
                .find(|k| k.key == key && !k.multiline)
                .map(|k| k.raw_value.trim_matches(['"', '\'']).to_string())
                .filter(|raw| !raw.is_empty())
                .unwrap_or_else(|| n.to_string()),
        ),
        _ => None,
    }
}

fn header_of(fields: &frontmatter::Fields, keys: &[KeyLine]) -> Header {
    let text = |key: &str| text_of(fields, keys, key).filter(|s| !s.trim().is_empty());
    let party = |link: &str, prefix: &str| Party {
        link: text(link),
        name: text(&format!("{prefix}_name")),
        email: text(&format!("{prefix}_email")),
        phone: text(&format!("{prefix}_phone")),
        address: text(&format!("{prefix}_address")),
    };
    Header {
        doc_type: text("type").as_deref().and_then(DocType::parse),
        number: text("invoice_num").as_deref().and_then(InvoiceNumber::new),
        date: text("date"),
        status: text("status").as_deref().and_then(Status::parse),
        template: text("template"),
        invoicee: party("customer", "invoicee"),
        invoicer: party("invoicer", "invoicer"),
        terms: fields.get("terms").and_then(Value::as_i64),
        due_date: text("due_date"),
        reference: text("reference"),
        paid: fields.get("paid").and_then(Paid::parse),
        sent_date: text("sent_date"),
        sent_via: text("sent_via"),
        void_date: text("void_date"),
        deposit_for: text("deposit_for"),
    }
}

impl Invoice {
    /// Read a document. Never fails; see [`Invoice::problems`].
    #[must_use]
    pub fn parse(source: impl Into<String>, words: &Words) -> Self {
        let source = source.into();
        let (frontmatter, frontmatter_error) = match Frontmatter::parse(&source) {
            Ok(fm) => (fm, None),
            Err(e) => {
                // Keep the block's place so the body is still found.
                let block = frontmatter::locate(&source);
                (
                    Frontmatter {
                        block,
                        ..Frontmatter::default()
                    },
                    Some(e),
                )
            }
        };
        let keys = frontmatter::key_lines(&source).unwrap_or_default();
        let header = header_of(&frontmatter.fields, &keys);
        let body_start = frontmatter.body_start();
        let masked = masked_body(&source, frontmatter.block.as_ref());
        let tables = gfm::tables(&masked);
        let bill_at = tables
            .iter()
            .position(|table| !matches!(BillTable::from_table(table.clone(), words), Ok(None)));
        let bill = bill_at.map_or(Ok(None), |at| {
            BillTable::from_table(tables[at].clone(), words)
        });
        let bill_range = bill_at.and_then(|at| table_range(&tables[at]));
        let supporting_tables = tables
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != bill_at)
            .filter_map(|(_, t)| table_range(t))
            .collect();
        let layout = Layout {
            frontmatter: frontmatter.block.as_ref().map(|b| b.block.clone()),
            prose_before: body_start..bill_range.as_ref().map_or(source.len(), |r| r.start),
            prose_after: bill_range.as_ref().map_or(source.len(), |r| r.end)..source.len(),
            bill_table: bill_range,
            supporting_tables,
        };
        Self {
            source,
            frontmatter,
            frontmatter_error,
            keys,
            header,
            bill,
            layout,
        }
    }

    /// The text it was read from.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Every frontmatter field.
    #[must_use]
    pub fn frontmatter(&self) -> &Frontmatter {
        &self.frontmatter
    }

    /// Why the frontmatter didn't read, if it didn't.
    #[must_use]
    pub fn frontmatter_error(&self) -> Option<&FrontmatterError> {
        self.frontmatter_error.as_ref()
    }

    /// The fields the invoicer reads.
    #[must_use]
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The bill table: `Ok(None)` when there is none.
    ///
    /// # Errors
    /// The [`TableError`] that stops it being read.
    pub fn bill_table(&self) -> Result<Option<&BillTable>, &TableError> {
        self.bill.as_ref().map(Option::as_ref)
    }

    /// Where the parts are.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The prose above the bill table.
    #[must_use]
    pub fn prose_before(&self) -> &str {
        &self.source[self.layout.prose_before.clone()]
    }

    /// The prose below the bill table.
    #[must_use]
    pub fn prose_after(&self) -> &str {
        &self.source[self.layout.prose_after.clone()]
    }

    /// The supporting tables' text.
    pub fn supporting_tables(&self) -> impl Iterator<Item = &str> {
        self.layout
            .supporting_tables
            .iter()
            .map(|r| &self.source[r.clone()])
    }

    /// A top-level key's 1-based line.
    #[must_use]
    pub fn line_of(&self, key: &str) -> Option<usize> {
        self.keys.iter().find(|k| k.key == key).map(|k| k.line)
    }

    /// The bill table computed.
    ///
    /// # Errors
    /// [`TableError`] when there is no readable table or a row can't be
    /// computed.
    pub fn evaluate(&self, words: &Words, money: &Money) -> Result<Option<Evaluation>, TableError> {
        match &self.bill {
            Err(e) => Err(e.clone()),
            Ok(None) => Ok(None),
            Ok(Some(table)) => table.evaluate(words, &money.for_table(table)).map(Some),
        }
    }

    /// What the index keeps of the bill table, or `None` without a readable
    /// one.
    #[must_use]
    pub fn summary(&self, words: &Words, money: &Money) -> Option<TableSummary> {
        let table = self.bill.as_ref().ok()?.as_ref()?;
        let style = money.for_table(table);
        let recalc = table.recompute(words, &style).ok()?;
        let mut accounts: Vec<String> = Vec::new();
        for row in &recalc.evaluation.rows {
            if let Some(books) = &row.books {
                let name = books.account().as_str();
                if !accounts.iter().any(|a| a == name) {
                    accounts.push(name.to_string());
                }
            }
        }
        Some(TableSummary {
            total: recalc.evaluation.total,
            total_text: style.format(recalc.evaluation.total),
            commodity: style.commodity().to_string(),
            accounts,
            rows: recalc.evaluation.rows.len(),
            discrepancies: recalc.changes.len(),
        })
    }

    /// The document with every computed amount rewritten (`recalc`), or
    /// `Ok(None)` when it already agrees. Only the table's changed lines
    /// change.
    ///
    /// # Errors
    /// [`TableError`].
    pub fn recalc(&self, words: &Words, money: &Money) -> Result<Option<String>, TableError> {
        let Some(table) = self.bill.clone()? else {
            return Ok(None);
        };
        let recalc = table.recompute(words, &money.for_table(&table))?;
        if recalc.changes.is_empty() {
            return Ok(None);
        }
        let edits = recalc.table.edits(&self.source)?;
        Ok(Some(apply_edits(&self.source, &edits)))
    }

    /// What is wrong with this document on its own (no other file
    /// consulted): fields, dates, statuses, the table and its arithmetic.
    #[must_use]
    pub fn problems(&self, words: &Words, money: &Money) -> Vec<Problem> {
        if let Some(e) = &self.frontmatter_error {
            return vec![
                Problem::error(ProblemCode::Frontmatter, e.to_string()).at_line(Some(e.line())),
            ];
        }
        let mut out = self.field_problems();
        out.extend(self.table_problems(words, money));
        out
    }

    fn field_problems(&self) -> Vec<Problem> {
        let fields = &self.frontmatter.fields;
        let h = &self.header;
        let line = |key: &str| self.line_of(key);
        let present = |key: &str| fields.get(key).is_some_and(|v| !v.is_null());
        let mut out = Vec::new();

        let required_errors = [
            "type",
            "invoice_num",
            "date",
            "status",
            "template",
            "customer",
            "invoicee_name",
            "invoicer",
            "invoicer_name",
        ];
        let required_warnings = [
            "invoicee_email",
            "invoicee_address",
            "invoicer_email",
            "invoicer_address",
        ];
        out.extend(
            required_errors
                .iter()
                .filter(|k| !present(k))
                .map(|k| Problem::error(ProblemCode::MissingField, format!("`{k}` is missing"))),
        );
        out.extend(
            required_warnings
                .iter()
                .filter(|k| !present(k))
                .map(|k| Problem::warning(ProblemCode::MissingField, format!("`{k}` is missing"))),
        );

        // Shapes: these are text, never lists or maps.
        let scalar_keys = [
            "invoice_num",
            "date",
            "status",
            "template",
            "customer",
            "reference",
            "due_date",
            "sent_date",
            "void_date",
            "invoicee_name",
            "invoicee_email",
            "invoicee_phone",
            "invoicee_address",
            "invoicer",
            "invoicer_name",
            "invoicer_email",
            "invoicer_phone",
            "invoicer_address",
        ];
        out.extend(scalar_keys.iter().filter_map(|k| {
            match fields.get(*k) {
                Some(Value::Array(_) | Value::Object(_)) => Some(
                    Problem::error(
                        ProblemCode::BadField,
                        format!("`{k}` must be text, not a list or map"),
                    )
                    .at_line(line(k)),
                ),
                _ => None,
            }
        }));

        if present("invoice_num")
            && h.number.is_none()
            && !matches!(
                fields.get("invoice_num"),
                Some(Value::Array(_) | Value::Object(_))
            )
        {
            out.push(
                Problem::error(
                    ProblemCode::BadNumber,
                    "`invoice_num` can't be blank or hold a comma: it is the journal's `invoice:` tag",
                )
                .at_line(line("invoice_num")),
            );
        }
        if let Some(raw) = fields.get("type").and_then(Value::as_str)
            && h.doc_type.is_none()
        {
            out.push(
                Problem::error(
                    ProblemCode::BadField,
                    format!("`type: {raw}` isn't invoice, estimate or credit-note"),
                )
                .at_line(line("type")),
            );
        }
        for key in ["date", "due_date", "sent_date", "void_date"] {
            if let Some(text) = text_of(fields, &self.keys, key).filter(|t| !is_iso_date(t.trim()))
            {
                out.push(
                    Problem::error(
                        ProblemCode::BadDate,
                        format!("`{key}: {text}` isn't a date"),
                    )
                    .with_fix("write it as YYYY-MM-DD", None)
                    .at_line(line(key)),
                );
            }
        }
        if let Some(raw) = text_of(fields, &self.keys, "status") {
            let allowed = h.doc_type.unwrap_or(DocType::Invoice).statuses();
            if !h.status.is_some_and(|s| allowed.contains(&s)) {
                let names: Vec<&str> = allowed.iter().map(|s| s.as_str()).collect();
                out.push(
                    Problem::error(
                        ProblemCode::BadStatus,
                        format!("`status: {raw}` isn't one of {}", names.join(", ")),
                    )
                    .at_line(line("status")),
                );
            }
        }
        if present("terms") && !h.terms.is_some_and(|t| t >= 0) {
            out.push(
                Problem::error(
                    ProblemCode::BadTerms,
                    "`terms` must be a whole number of days",
                )
                .at_line(line("terms")),
            );
        }
        let valid_date = |d: &Option<String>| d.clone().filter(|d| is_iso_date(d));
        match (valid_date(&h.date), valid_date(&h.due_date), h.terms) {
            (Some(date), Some(due), _) if due < date => out.push(
                Problem::error(
                    ProblemCode::DueBeforeDate,
                    format!("`due_date` {due} is before `date` {date}"),
                )
                .at_line(line("due_date")),
            ),
            (Some(date), None, Some(terms)) if terms >= 0 && !present("due_date") => {
                let due = ledgeline_core::reports::add_days(&date, terms);
                out.push(
                    Problem::warning(ProblemCode::MissingField, "`due_date` is missing")
                        .with_fix(
                            format!("set it to {due} (date + terms)"),
                            Some(FixEdit::SetField {
                                key: "due_date".into(),
                                value: Value::String(due),
                            }),
                        )
                        .at_line(line("terms")),
                );
            }
            _ => {}
        }
        for key in ["invoicee_address", "invoicer_address"] {
            if self
                .keys
                .iter()
                .any(|k| k.key == key && k.raw_value.starts_with('>'))
            {
                out.push(
                    Problem::warning(
                        ProblemCode::FoldedText,
                        format!("`{key}` is written with `>`, which joins its lines"),
                    )
                    .with_fix("write it with `|`, which keeps each line", None)
                    .at_line(line(key)),
                );
            }
        }
        for (old, new) in [("po_num", "reference"), ("term", "terms")] {
            if present(old) {
                let edit = (!present(new)).then(|| FixEdit::RenameField {
                    from: old.into(),
                    to: new.into(),
                });
                out.push(
                    Problem::warning(ProblemCode::LegacyField, format!("`{old}` is now `{new}`"))
                        .with_fix(format!("rename it to `{new}`"), edit)
                        .at_line(line(old)),
                );
            }
        }
        if present("paid") && h.paid.is_none() {
            out.push(
                Problem::warning(
                    ProblemCode::BadPaid,
                    "`paid` isn't `no`, a date, or `partial <date> <amount>, …`; the reconciler rewrites it",
                )
                .at_line(line("paid")),
            );
        }
        if let Some(link) = &h.invoicee.link
            && crate::notes::wikilink_target(link).is_none()
        {
            out.push(
                Problem::warning(
                    ProblemCode::BadField,
                    format!("`customer: {link}` isn't a [[wikilink]]"),
                )
                .with_fix(
                    format!("write it as \"[[{link}]]\""),
                    Some(FixEdit::SetField {
                        key: "customer".into(),
                        value: Value::String(format!("[[{}]]", link.trim())),
                    }),
                )
                .at_line(line("customer")),
            );
        }
        if let Some(template) = &h.template
            && (!template.starts_with('/') || !template.to_ascii_lowercase().ends_with(".html"))
        {
            out.push(
                Problem::warning(
                    ProblemCode::BadField,
                    format!("`template: {template}` should be a `/`-rooted .html path"),
                )
                .at_line(line("template")),
            );
        }
        out
    }

    fn table_problems(&self, words: &Words, money: &Money) -> Vec<Problem> {
        let table_line = |e: &TableError| match e {
            TableError::Header { line, .. } => Some(*line),
            TableError::Row { at, .. } => Some(at.line),
            _ => None,
        };
        let table_error = |e: &TableError| {
            Problem::error(ProblemCode::Table, e.to_string()).at_line(table_line(e))
        };
        let table = match &self.bill {
            Err(e) => return vec![table_error(e)],
            Ok(None) => {
                return vec![Problem::error(
                    ProblemCode::NoBillTable,
                    format!(
                        "there is no bill table: no table has an `{}` column",
                        words.amount.first()
                    ),
                )];
            }
            Ok(Some(table)) => table,
        };
        let style = money.for_table(table);
        let recalc = match table.recompute(words, &style) {
            Ok(recalc) => recalc,
            Err(e) => return vec![table_error(&e)],
        };
        let mut out: Vec<Problem> = recalc
            .changes
            .iter()
            .map(|change| {
                let message = if change.before.trim().is_empty() {
                    format!(
                        "{}: the amount is blank; it should be {}",
                        change.at, change.after
                    )
                } else {
                    format!(
                        "{}: the amount says {}, but the arithmetic gives {}",
                        change.at, change.before, change.after
                    )
                };
                Problem::error(ProblemCode::Arithmetic, message)
                    .with_fix(
                        "recalc rewrites the computed amounts",
                        Some(FixEdit::Recalc),
                    )
                    .at_line(Some(change.at.line))
            })
            .collect();
        if let Some(last) = recalc.evaluation.final_total
            && let Some(row) = recalc.evaluation.rows[last + 1..]
                .iter()
                .find(|row| row.kind != RowKind::Spacer)
        {
            out.push(
                Problem::warning(
                    ProblemCode::RowsAfterTotal,
                    format!("{} is below the final total; it can't be booked", row.at),
                )
                .at_line(Some(row.at.line)),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests;
