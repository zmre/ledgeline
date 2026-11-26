//! Text output: aligned columns and `key  value` blocks. `--json` prints the
//! same values with serde instead; nothing here decides anything.

use invoicer_core::books::{
    Basis, PaymentApplied, PaymentOptions, Preview, Rebooked, Recognized, Sent, Voided,
};
use invoicer_core::customer_import::ImportReport;
use invoicer_core::doc::{Header, Money, Paid};
use invoicer_core::edit::{Created, RecalcReport, SetOutcome};
use invoicer_core::problem::Problem;
use invoicer_core::receivables::ReceivablesView;
use invoicer_core::reconcile::{MirrorChange, Reconciled};
use invoicer_core::views::{
    CheckReport, ConfigView, Counts, CustomerDetail, CustomerEntry, DocDetail, DocEntry,
};
use ledgeline_core::display::format_amount;
use ledgeline_core::edit::render_amount;
use ledgeline_core::parse::parse_journal;
use ledgeline_core::vault::VaultPath;
use std::fmt::Write as _;
use unicode_width::UnicodeWidthStr;

/// A column: its header and whether it is right-aligned (numbers, money).
struct Col {
    header: &'static str,
    right: bool,
}

const fn left(header: &'static str) -> Col {
    Col {
        header,
        right: false,
    }
}

const fn right(header: &'static str) -> Col {
    Col {
        header,
        right: true,
    }
}

/// Rows under a header, padded by display width; trailing blanks trimmed.
fn columns(cols: &[Col], rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            rows.iter()
                .map(|r| r.get(i).map_or(0, |s| s.width()))
                .chain([c.header.width()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(cols)
            .zip(&widths)
            .map(|((text, col), width)| {
                let pad = " ".repeat(width.saturating_sub(text.width()));
                if col.right {
                    format!("{pad}{text}")
                } else {
                    format!("{text}{pad}")
                }
            })
            .collect();
        format!("{}\n", padded.join("  ").trim_end())
    };
    let mut out = line(cols.iter().map(|c| c.header).collect());
    for row in rows {
        out.push_str(&line(row.iter().map(String::as_str).collect()));
    }
    out
}

/// `key  value` lines, keys padded; multi-line values indented under the
/// first line.
fn fields(pairs: &[(&str, String)]) -> String {
    let width = pairs.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
    pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            let indent = " ".repeat(width + 4);
            let value = v.trim_end().replace('\n', &format!("\n{indent}"));
            format!("  {k:<width$}  {value}\n")
        })
        .collect()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn counts_note(counts: Counts) -> Vec<String> {
    let mut out = Vec::new();
    if counts.errors > 0 {
        out.push(plural(counts.errors, "error", "errors"));
    }
    if counts.warnings > 0 {
        out.push(plural(counts.warnings, "warning", "warnings"));
    }
    out
}

fn customer_name(header: &Header) -> String {
    header
        .invoicee
        .name
        .clone()
        .or_else(|| header.invoicee.link.clone())
        .unwrap_or_default()
}

fn paid_text(paid: Option<&Paid>) -> String {
    paid.map(ToString::to_string).unwrap_or_default()
}

fn opt(text: Option<&str>) -> String {
    text.unwrap_or("").to_string()
}

/// `list`.
pub fn list(entries: &[DocEntry<'_>]) -> String {
    if entries.is_empty() {
        return "No documents match.\n".to_string();
    }
    let rows: Vec<Vec<String>> = entries
        .iter()
        .map(|e| {
            let h = e.header;
            let mut notes = Vec::new();
            if e.overdue {
                notes.push("overdue".to_string());
            }
            notes.extend(counts_note(e.problems));
            vec![
                h.number
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                opt(h.date.as_deref()),
                h.status.map(|s| s.as_str().to_string()).unwrap_or_default(),
                customer_name(h),
                opt(e.total),
                opt(h.due_date.as_deref()),
                paid_text(h.paid.as_ref()),
                notes.join(", "),
            ]
        })
        .collect();
    columns(
        &[
            left("NUMBER"),
            left("DATE"),
            left("STATUS"),
            left("CUSTOMER"),
            right("TOTAL"),
            left("DUE"),
            left("PAID"),
            left("NOTES"),
        ],
        &rows,
    )
}

fn problems_block(problems: &[Problem]) -> String {
    if problems.is_empty() {
        return String::new();
    }
    let mut out = String::from("\nProblems:\n");
    for p in problems {
        let _ = writeln!(out, "  {p}");
    }
    out
}

/// `show`.
pub fn show(detail: &DocDetail<'_>) -> String {
    let h = detail.header;
    let kind = h.doc_type.map_or("document", |t| t.as_str());
    let number = h
        .number
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let status = h.status.map_or("no status", |s| s.as_str());
    let mut out = format!(
        "{} {number} ({status})  {}\n",
        capitalize(kind),
        detail.path
    );
    let due = match (&h.due_date, h.terms) {
        (Some(d), Some(t)) => format!("{d} (net {t})"),
        (Some(d), None) => d.clone(),
        (None, Some(t)) => format!("net {t}"),
        (None, None) => String::new(),
    };
    let mut state = Vec::new();
    if detail.overdue {
        state.push("overdue");
    } else if detail.open {
        state.push("open");
    }
    let sent = match (&h.sent_date, &h.sent_via) {
        (Some(d), Some(v)) => format!("{d} via {v}"),
        (Some(d), None) => d.clone(),
        (None, Some(v)) => format!("via {v}"),
        (None, None) => String::new(),
    };
    out.push_str(&fields(&[
        ("Date", opt(h.date.as_deref())),
        ("Due", due),
        ("Customer", party(&h.invoicee)),
        ("From", party(&h.invoicer)),
        ("Reference", opt(h.reference.as_deref())),
        ("Template", opt(h.template.as_deref())),
        ("Paid", paid_text(h.paid.as_ref())),
        ("State", state.join(", ")),
        ("Sent", sent),
        ("Void", opt(h.void_date.as_deref())),
        ("Deposit for", opt(h.deposit_for.as_deref())),
    ]));
    match &detail.table {
        None => out.push_str("\n(no readable bill table)\n"),
        Some(table) => {
            out.push('\n');
            let rows: Vec<Vec<String>> = table
                .rows
                .iter()
                .map(|r| {
                    let wrong = table.discrepancies.iter().any(|d| d.row == r.row);
                    vec![
                        r.row.to_string(),
                        r.kind.to_string(),
                        r.description.clone(),
                        opt(r.quantity.as_deref()),
                        opt(r.rate.as_deref()),
                        format!(
                            "{}{}",
                            opt(r.amount.as_deref()),
                            if wrong { "*" } else { "" }
                        ),
                        opt(r.books.as_deref()),
                    ]
                })
                .collect();
            out.push_str(&indent(&columns(
                &[
                    right("#"),
                    left("KIND"),
                    left("DESCRIPTION"),
                    right("QTY"),
                    right("RATE"),
                    right("AMOUNT"),
                    left("ACCOUNT"),
                ],
                &rows,
            )));
            let _ = writeln!(out, "\n  Total due  {}", table.total);
            if !table.discrepancies.is_empty() {
                out.push_str("  * the written amount differs (recalc rewrites it):\n");
                for d in &table.discrepancies {
                    let written = if d.written.is_empty() {
                        "blank"
                    } else {
                        d.written.as_str()
                    };
                    let _ = writeln!(
                        out,
                        "    row {}: {written}, should be {}",
                        d.row, d.expected
                    );
                }
            }
        }
    }
    let custom: Vec<(&str, String)> = detail
        .frontmatter
        .iter()
        .filter(|(k, _)| !STANDARD_KEYS.contains(&k.as_str()))
        .map(|(k, v)| {
            (
                k.as_str(),
                v.as_str().map_or_else(|| v.to_string(), str::to_string),
            )
        })
        .collect();
    if !custom.is_empty() {
        out.push_str("\nCustom fields:\n");
        out.push_str(&fields(&custom));
    }
    out.push_str(&problems_block(&detail.problems));
    out
}

/// The frontmatter keys `show` already prints (or that are noise there).
const STANDARD_KEYS: [&str; 27] = [
    "title",
    "type",
    "template",
    "status",
    "invoice_num",
    "date",
    "terms",
    "due_date",
    "reference",
    "customer",
    "invoicee_name",
    "invoicee_email",
    "invoicee_phone",
    "invoicee_address",
    "invoicer",
    "invoicer_name",
    "invoicer_email",
    "invoicer_phone",
    "invoicer_address",
    "invoicer_logo",
    "paid",
    "sent_date",
    "sent_via",
    "void_date",
    "deposit_for",
    "tags",
    "aliases",
];

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().chain(chars).collect::<String>()
    })
}

fn indent(text: &str) -> String {
    text.lines().map(|l| format!("  {l}\n")).collect()
}

fn party(p: &invoicer_core::doc::Party) -> String {
    let mut lines = Vec::new();
    match (&p.name, &p.link) {
        (Some(name), Some(link)) => lines.push(format!("{name}  {link}")),
        (Some(name), None) => lines.push(name.clone()),
        (None, Some(link)) => lines.push(link.clone()),
        (None, None) => {}
    }
    lines.extend(p.email.clone());
    lines.extend(p.phone.clone());
    lines.extend(p.address.as_deref().map(|a| a.trim_end().to_string()));
    lines.join("\n")
}

/// `check`.
pub fn check(report: &CheckReport) -> String {
    let mut out: String = report.problems.iter().map(|p| format!("{p}\n")).collect();
    let scope = format!(
        "{} and {}",
        plural(report.documents, "document", "documents"),
        plural(report.customers, "customer", "customers")
    );
    let notes = counts_note(report.counts);
    if notes.is_empty() {
        let _ = writeln!(out, "No problems in {scope}.");
    } else {
        let _ = writeln!(out, "{} in {scope}.", notes.join(" and "));
    }
    out
}

/// `customers list`.
pub fn customers(all: &[CustomerEntry]) -> String {
    if all.is_empty() {
        return "No customers. A customer is a person or organization note tagged `customer`.\n"
            .to_string();
    }
    let rows: Vec<Vec<String>> = all
        .iter()
        .map(|c| {
            vec![
                c.title.clone(),
                kind_name(c.kind),
                c.terms.map(|t| format!("net {t}")).unwrap_or_default(),
                opt(c.billing.email.as_deref()),
                opt(c.portal.as_deref()),
                c.invoices.to_string(),
                c.open.to_string(),
            ]
        })
        .collect();
    columns(
        &[
            left("CUSTOMER"),
            left("KIND"),
            left("TERMS"),
            left("BILLING EMAIL"),
            left("PORTAL"),
            right("DOCS"),
            right("OPEN"),
        ],
        &rows,
    )
}

/// `customers new`: the note on a dry run, else what was created.
pub fn customer_created(made: &crate::customers::NewOut) -> String {
    let saved = &made.saved;
    match &made.text {
        Some(text) => format!("Would create {}:\n\n{text}", saved.path),
        None => format!(
            "Created {} ({})\n  invoices link it as {}; the books know it as {}\n",
            saved.path,
            kind_name(saved.kind),
            saved.link,
            saved.ledger_customer
        ),
    }
}

/// `customers import-csv`: the customers created (or that would be) and
/// the rows skipped, with their lines.
pub fn customers_imported(report: &ImportReport) -> String {
    let verb = if report.dry_run {
        "Would create"
    } else {
        "Created"
    };
    let mut out = format!(
        "{verb} {}",
        plural(report.created.len(), "customer", "customers")
    );
    if !report.skipped.is_empty() {
        let _ = write!(
            out,
            "; skipped {} already there",
            plural(report.skipped.len(), "row", "rows")
        );
    }
    out.push_str(if report.created.is_empty() && report.skipped.is_empty() {
        ".\n"
    } else {
        ":\n"
    });
    let rows: Vec<Vec<String>> = report
        .created
        .iter()
        .map(|r| {
            vec![
                r.line.to_string(),
                r.saved.title.clone(),
                kind_name(r.saved.kind),
                r.saved.path.to_string(),
            ]
        })
        .chain(report.skipped.iter().map(|s| {
            vec![
                s.line.to_string(),
                s.title.clone(),
                "skipped".to_string(),
                s.path.to_string(),
            ]
        }))
        .collect();
    if !rows.is_empty() {
        out.push_str(&columns(
            &[right("LINE"), left("CUSTOMER"), left("KIND"), left("FILE")],
            &rows,
        ));
    }
    out
}

fn kind_name(kind: invoicer_core::customers::ContactKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// `customers show`.
pub fn customer(detail: &CustomerDetail<'_>) -> String {
    let e = &detail.entry;
    let mut out = format!("{} ({})  {}\n", e.title, kind_name(e.kind), e.path);
    let billing = [
        Some(e.billing.name.clone()),
        e.billing.email.clone(),
        e.billing.phone.clone(),
        e.billing
            .address
            .as_deref()
            .map(|a| a.trim_end().to_string()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");
    let contracts = detail
        .contracts
        .iter()
        .map(|c| {
            let written = attachment_text(&c.entry);
            if c.found.is_some() {
                written
            } else {
                format!("{written} (missing)")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let portal = detail
        .billing_portal
        .map(|p| {
            [
                p.name.clone(),
                p.url.clone(),
                p.login.as_ref().map(|l| format!("login {l}")),
                p.supplier_id.as_ref().map(|s| format!("supplier id {s}")),
                p.requires_reference
                    .then(|| "requires a reference".to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
        })
        .unwrap_or_default();
    let pos = detail
        .open_pos
        .iter()
        .map(|po| {
            [
                Some(po.reference.clone()),
                po.amount.clone(),
                po.expires.as_ref().map(|d| format!("expires {d}")),
                po.attachment.as_ref().map(attachment_text),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    out.push_str(&fields(&[
        ("Bill to", billing),
        ("In the books", e.ledger_customer.clone()),
        (
            "Terms",
            e.terms.map(|t| format!("net {t}")).unwrap_or_default(),
        ),
        ("Revenue account", opt(detail.revenue_account)),
        ("Template", opt(detail.invoice_template)),
        ("Portal", portal),
        ("Open POs", pos),
        ("Contracts", contracts),
        ("Snippets", detail.snippets.join("\n")),
    ]));
    if detail.documents.is_empty() {
        out.push_str("\nNo documents.\n");
    } else {
        out.push('\n');
        out.push_str(&indent(&list(&detail.documents)));
    }
    out.push_str(&problems_block(&detail.problems));
    out
}

/// `config`: the effective settings as the TOML they are written in.
///
/// # Errors
/// When the settings don't serialize (they always should).
/// The payment matcher's effective settings, as `config` shows them: the
/// UI preference keys they are kept under (shared with Ledgeline). Not
/// `settlementToleranceBps`: Ledgeline's, and no use to the invoicer's
/// matcher, which offers a short deposit as a part payment.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Matching {
    pub settlement_lookback_days: u32,
    pub settlement_lead_days: u32,
}

impl Matching {
    /// From the matcher's settings.
    #[must_use]
    pub fn of(settle: &ledgeline_core::accrual::settle::SettleSettings) -> Self {
        Self {
            settlement_lookback_days: settle.lookback_days,
            settlement_lead_days: settle.lead_days,
        }
    }
}

pub fn config(view: &ConfigView<'_>, matching: &Matching) -> Result<String, toml::ser::Error> {
    let mut out = format!(
        "# journal: {}\n# books folder: {}\n",
        view.journal, view.books_folder
    );
    for warning in view.warnings {
        let _ = writeln!(out, "# warning: {warning}");
    }
    #[derive(serde::Serialize)]
    struct File<'a> {
        books: &'a invoicer_core::views::BooksView,
        invoicer: &'a invoicer_core::settings::Settings,
    }
    out.push('\n');
    out.push_str(&toml::to_string(&File {
        books: &view.books,
        invoicer: view.invoicer,
    })?);
    // Not this file's unless pinned: shared with Ledgeline in ui-prefs.toml,
    // kept for these books under [ui].
    out.push_str(
        "\n# Payment matching, shared with Ledgeline (ui-prefs.toml; [ui] in this file\n\
         # keeps a value for these books only):\n",
    );
    for (key, value) in [
        ("settlementLookbackDays", matching.settlement_lookback_days),
        ("settlementLeadDays", matching.settlement_lead_days),
    ] {
        let _ = writeln!(out, "# {key} = {value}");
    }
    Ok(out)
}

/// `new`.
pub fn created(created: &Created, problems: &[Problem]) -> String {
    if let Some(text) = &created.text {
        return text.clone();
    }
    let mut out = format!("Created {}\n", created.path);
    let _ = writeln!(
        out,
        "  number {}, total {}",
        created.number,
        created.total.as_deref().unwrap_or("unknown")
    );
    out.push_str(&problems_block(problems));
    out
}

/// `set`.
pub fn set(outcome: &SetOutcome) -> String {
    let mut out = if outcome.written {
        format!("Updated {}: {}\n", outcome.path, outcome.changed.join(", "))
    } else {
        format!("No change: {}\n", outcome.path)
    };
    out.push_str(&problems_block(&outcome.problems));
    out
}

/// `recalc`.
pub fn recalc(report: &RecalcReport, dry_run: bool) -> String {
    let mut out = String::new();
    let verb = if dry_run { "Would rewrite" } else { "Rewrote" };
    for doc in &report.changed {
        let _ = writeln!(
            out,
            "{verb} {} in {}",
            plural(doc.changes.len(), "amount", "amounts"),
            doc.path
        );
        for c in &doc.changes {
            let before = if c.before.is_empty() {
                "blank"
            } else {
                c.before.as_str()
            };
            let row = if c.description.is_empty() {
                format!("row {}", c.row)
            } else {
                format!("row {} {}", c.row, c.description)
            };
            let _ = writeln!(out, "  {row}: {before} → {}", c.after);
        }
    }
    for skipped in &report.skipped {
        let _ = writeln!(out, "Skipped {}: {}", skipped.path, skipped.reason);
    }
    for failed in &report.failed {
        let _ = writeln!(out, "Failed {}: {}", failed.path, failed.reason);
    }
    if out.is_empty() {
        out.push_str("Every amount already agrees with its arithmetic.\n");
    }
    out
}

/// `value` in `commodity`, as the journal writes money.
fn cash(money: &Money, commodity: Option<&str>, value: ledgeline_core::Dec) -> String {
    money.style(commodity).format(value)
}

fn indent_block(text: &str) -> String {
    text.lines().map(|l| format!("    {l}\n")).collect()
}

fn basis_name(basis: Basis) -> &'static str {
    match basis {
        Basis::Accrual => "accrual",
        Basis::Cash => "cash",
    }
}

/// `transaction` (one, as the books write it) with each posting's amount
/// shown in its commodity's journal style, `$13,600.00` rather than the
/// booking's plain `$13600.00`: how the books show money (P3). Unchanged
/// when it doesn't read as one transaction.
fn journal_styled(transaction: &str, money: &Money) -> String {
    let Ok(journal) = parse_journal(transaction, "") else {
        return transaction.to_string();
    };
    let [txn] = journal.transactions.as_slice() else {
        return transaction.to_string();
    };
    // Posting lines, in order: indented and not a comment line.
    let mut postings = txn.postings.iter();
    transaction
        .lines()
        .enumerate()
        .map(|(at, line)| {
            let posting_line =
                at > 0 && line.starts_with([' ', '\t']) && !line.trim_start().starts_with(';');
            let restyled = posting_line
                .then(|| postings.next())
                .flatten()
                .and_then(|p| match p.amounts.as_slice() {
                    [amount] if amount.cost.is_none() => Some(amount),
                    _ => None,
                })
                .and_then(|amount| {
                    let style = money.style(Some(&amount.commodity.0));
                    let shown = format_amount(
                        &amount.commodity.0,
                        amount.quantity,
                        style.style(),
                        style.precision().max(amount.quantity.places),
                    );
                    let written = format!("  {}", render_amount(amount));
                    line.contains(&written)
                        .then(|| line.replacen(&written, &format!("  {shown}"), 1))
                });
            restyled.unwrap_or_else(|| line.to_string()) + "\n"
        })
        .collect()
}

/// `send --dry-run` and `rebook --dry-run`.
pub fn preview(p: &Preview, money: &Money) -> String {
    let mut out = format!(
        "Invoice {} ({}, {} books): {}\n",
        p.number,
        p.path,
        basis_name(p.basis),
        cash(money, Some(&p.booking.commodity), p.booking.total)
    );
    match (&p.would_book, &p.booked) {
        (None, _) => {
            out.push_str("  Cash books book nothing at send; Apply payment books its payments.\n")
        }
        (Some(would), None) => {
            let _ = write!(
                out,
                "  Would book:\n{}",
                indent_block(&journal_styled(would, money))
            );
        }
        (Some(would), Some(booked)) => {
            if p.in_step == Some(true) {
                let _ = write!(
                    out,
                    "  Booked, as its file says:\n{}",
                    indent_block(&booked.text)
                );
            } else {
                let _ = write!(
                    out,
                    "  Booked:\n{}  Its file now says to book:\n{}",
                    indent_block(&booked.text),
                    indent_block(&journal_styled(would, money))
                );
            }
        }
    }
    if p.linked {
        out.push_str("  Payments are linked to it.\n");
    }
    out
}

/// `send`.
pub fn sent(s: &Sent) -> String {
    let mut out = format!("Sent invoice {} ({})\n", s.number, s.path);
    match (&s.booked, s.journal_written) {
        (Some(txn), true) => {
            let _ = write!(out, "  Booked:\n{}", indent_block(&txn.text));
        }
        (Some(_), false) => out.push_str("  Already booked; only its file was finished.\n"),
        (None, _) => out.push_str("  Cash books: nothing booked until it is paid.\n"),
    }
    out
}

/// `pdf`.
pub fn exported(e: &crate::pdf::Exported) -> String {
    use crate::pdf::Destination;
    let path = e.path.as_deref().unwrap_or(e.file.as_str());
    let mut out = format!(
        "Wrote invoice {}'s PDF to {path} ({} bytes)\n",
        e.number, e.bytes
    );
    match e.destination {
        Destination::Kept => {
            if let Some(earlier) = &e.earlier {
                let _ = writeln!(out, "  The earlier export is kept as {earlier}");
            }
        }
        Destination::Temp => {
            let _ = writeln!(
                out,
                "  Not kept: it isn't sent. Send it (ledgeline-invoicer send {}), then export \
                 again to keep its PDF.",
                e.number
            );
        }
        Destination::Copy => {}
    }
    for warning in &e.warnings {
        let _ = writeln!(out, "  warning: {warning}");
    }
    out
}

/// `void`.
pub fn voided(v: &Voided, mirror: Option<&MirrorChange>) -> String {
    let mut out = format!("Voided invoice {} ({})\n", v.number, v.path);
    match (&v.reversal, v.journal_written) {
        (Some(txn), true) => {
            let _ = write!(out, "  Reversed:\n{}", indent_block(&txn.text));
        }
        (Some(_), false) => out.push_str("  Already reversed; only its file was finished.\n"),
        (None, _) => out.push_str("  Nothing was booked; only its file changed.\n"),
    }
    out.push_str(&mirror_line(mirror));
    out
}

fn mirror_line(mirror: Option<&MirrorChange>) -> String {
    mirror.map_or_else(String::new, |m| {
        format!(
            "  paid: {} → {}{}\n",
            m.before.as_deref().unwrap_or("(none)"),
            m.after,
            if m.written { "" } else { " (not written)" }
        )
    })
}

/// `rebook`.
pub fn rebooked(r: &Rebooked, mirror: Option<&MirrorChange>) -> String {
    let mut out = if r.changed {
        let mut out = format!("Re-booked invoice {} ({})\n", r.number, r.path);
        if let Some(txn) = &r.booked {
            let _ = write!(out, "  Now:\n{}", indent_block(&txn.text));
        }
        out
    } else if r.booked.is_some() {
        format!(
            "Invoice {}'s books already agree with its file.\n",
            r.number
        )
    } else {
        format!(
            "Invoice {}: {} books hold nothing to re-book.\n",
            r.number,
            basis_name(r.basis)
        )
    };
    for w in &r.warnings {
        let _ = writeln!(out, "  warning: {w}");
    }
    out.push_str(&mirror_line(mirror));
    out
}

fn short(reference: &str) -> String {
    reference.chars().take(12).collect()
}

/// `apply-payment NUM` (no `--txn`).
pub fn payment_options(o: &PaymentOptions, money: &Money) -> String {
    let c = Some(o.commodity.as_str());
    let mut out = format!(
        "Invoice {}: total {}, paid {}, due {}\n",
        o.number,
        cash(money, c, o.total),
        cash(money, c, o.paid),
        cash(money, c, o.remaining)
    );
    if o.candidates.is_empty() {
        out.push_str(if o.remaining.mantissa > 0 {
            "No deposit in the books fits it.\n"
        } else {
            "Nothing is due.\n"
        });
        return out;
    }
    let c_commodity = c;
    let rows: Vec<Vec<String>> = o
        .candidates
        .iter()
        .map(|c| {
            let mut why = Vec::new();
            if c.by_number {
                why.push("number");
            }
            if c.by_customer {
                why.push("customer");
            }
            why.push(if c.exact { "exact" } else { "partial" });
            vec![
                short(&c.reference),
                c.date.clone(),
                c.description.clone(),
                cash(money, c_commodity, c.received),
                why.join(", "),
            ]
        })
        .collect();
    out.push('\n');
    out.push_str(&columns(
        &[
            left("Ref"),
            left("Date"),
            left("Description"),
            right("Received"),
            left("Matches"),
        ],
        &rows,
    ));
    let _ = write!(
        out,
        "\nApply one: ledgeline-invoicer apply-payment {} --txn REF\n",
        o.number
    );
    out
}

/// `apply-payment NUM --txn REF`.
pub fn payment_applied(
    p: &PaymentApplied,
    money: &Money,
    undo_file: Option<&VaultPath>,
    mirror: Option<&MirrorChange>,
    warnings: &[String],
) -> String {
    let mut out = format!(
        "Applied {} to invoice {} ({} still due)\n",
        cash(money, None, p.received),
        p.number,
        cash(money, None, p.remaining)
    );
    let _ = write!(out, "{}", indent_block(&p.transaction.text));
    if let Some(file) = undo_file {
        let _ = writeln!(
            out,
            "  Undo: ledgeline-invoicer apply-payment {} --undo --txn {} (record in {file})",
            p.number,
            short(&p.undo.reference)
        );
    }
    out.push_str(&mirror_line(mirror));
    for w in warnings {
        let _ = writeln!(out, "  warning: {w}");
    }
    out
}

/// `apply-payment NUM --undo`.
pub fn payment_undone(
    number: &str,
    date: &str,
    description: &str,
    mirror: Option<&MirrorChange>,
    warnings: &[String],
) -> String {
    let mut out = format!(
        "Undid the payment applied to invoice {number}: {date} {description} is as it was\n"
    );
    out.push_str(&mirror_line(mirror));
    for w in warnings {
        let _ = writeln!(out, "  warning: {w}");
    }
    out
}

/// `reconcile`.
pub fn reconciled(r: &Reconciled, dry_run: bool) -> String {
    let mut out = String::new();
    let verb = if dry_run { "Would mirror" } else { "Mirrored" };
    for c in &r.changed {
        let _ = writeln!(
            out,
            "{verb} {}: paid: {} → {}",
            c.path,
            c.before.as_deref().unwrap_or("(none)"),
            c.after
        );
    }
    for f in &r.failed {
        let _ = writeln!(out, "Failed {}: {}", f.path, f.error);
    }
    for p in &r.problems {
        let _ = writeln!(out, "{p}");
    }
    let mut summary = format!(
        "{} checked ({} books)",
        plural(r.checked, "sent invoice", "sent invoices"),
        basis_name(r.basis)
    );
    if r.changed.is_empty() && r.failed.is_empty() {
        summary.push_str("; every paid: mirror agrees with the books");
    }
    let notes = counts_note(r.counts);
    if !notes.is_empty() {
        let _ = write!(summary, "; {}", notes.join(" and "));
    }
    let _ = writeln!(out, "{summary}.");
    out
}

/// `receivables [--aging]`.
pub fn receivables(v: &ReceivablesView, money: &Money, aging: bool) -> String {
    let mut out = String::new();
    if v.open.is_empty() {
        let _ = writeln!(
            out,
            "Nothing is owed ({} books, {}).",
            basis_name(v.basis),
            v.today
        );
    } else {
        let rows: Vec<Vec<String>> = v
            .open
            .iter()
            .map(|o| {
                vec![
                    o.number.clone().unwrap_or_default(),
                    o.customer.clone().unwrap_or_default(),
                    o.date.clone(),
                    o.due.clone(),
                    if o.days_past_due > 0 {
                        o.days_past_due.to_string()
                    } else {
                        String::new()
                    },
                    cash(money, Some(&o.commodity), o.amount),
                    cash(money, Some(&o.commodity), o.outstanding),
                ]
            })
            .collect();
        let _ = writeln!(
            out,
            "Open invoices ({} books, {}{}):\n",
            basis_name(v.basis),
            v.today,
            if v.basis == Basis::Cash {
                "; owed by their files, less payments applied"
            } else {
                ""
            }
        );
        out.push_str(&columns(
            &[
                left("Number"),
                left("Customer"),
                left("Date"),
                left("Due"),
                right("Late"),
                right("Amount"),
                right("Owed"),
            ],
            &rows,
        ));
    }
    if aging && !v.aging.is_empty() {
        out.push_str("\nAging:\n\n");
        let mut cols = vec![left("Account")];
        let labels: Vec<&'static str> = v
            .aging
            .first()
            .map(|a| a.buckets.iter().map(|b| b.label).collect())
            .unwrap_or_default();
        cols.extend(labels.iter().map(|l| right(l)));
        cols.push(right("Total"));
        let rows: Vec<Vec<String>> = v
            .aging
            .iter()
            .map(|a| {
                let c = Some(a.commodity.as_str());
                let mut row = vec![a.account.clone().unwrap_or_else(|| "(cash books)".into())];
                row.extend(a.buckets.iter().map(|b| cash(money, c, b.amount)));
                row.push(cash(money, c, a.total));
                row
            })
            .collect();
        out.push_str(&columns(&cols, &rows));
    }
    let mut facts = Vec::new();
    for d in &v.dso {
        if let Some(value) = &d.value {
            facts.push(format!(
                "DSO {value} days ({}, over {} days)",
                d.commodity, d.days
            ));
        }
    }
    if let Some(average) = &v.average_days_to_pay.average {
        facts.push(format!(
            "average days to pay {average} (over {})",
            plural(v.average_days_to_pay.invoices, "invoice", "invoices")
        ));
    }
    if !v.unapplied.is_empty() {
        facts.push(plural(
            v.unapplied.len(),
            "unapplied payment",
            "unapplied payments",
        ));
    }
    if !v.drafts.is_empty() {
        facts.push(plural(v.drafts.len(), "draft waiting", "drafts waiting"));
    }
    if !facts.is_empty() {
        let _ = writeln!(out, "\n{}.", capitalize(&facts.join("; ")));
    }
    out
}

/// `recognize`.
pub fn recognized(r: &Recognized) -> String {
    let mut out = String::new();
    let verb = if r.written { "Posted" } else { "Due" };
    if r.entries.is_empty() && r.adjustments.is_empty() {
        let _ = writeln!(out, "Nothing is due through {}.", r.through);
    } else {
        let _ = writeln!(
            out,
            "{verb} through {}: {}",
            r.through,
            plural(r.entries.len(), "entry", "entries")
        );
        for e in &r.entries {
            let _ = writeln!(out, "  {}  {}", e.date, e.id);
        }
        for id in &r.adjustments {
            let _ = writeln!(out, "  closed-period adjustment  {id}");
        }
    }
    if !r.closed.is_empty() {
        let _ = writeln!(
            out,
            "{} in closed periods left unposted (--include-closed folds them into adjustments).",
            plural(r.closed.len(), "entry", "entries")
        );
    }
    out
}

/// An attachment entry for the terminal: as written, or where a file
/// outside the books is.
fn attachment_text(entry: &invoicer_core::attachment::Attachment) -> String {
    match entry {
        invoicer_core::attachment::Attachment::Outside(outside) => {
            format!("{} (outside the books)", outside.location())
        }
        other => other.entry(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn would_book_text_takes_the_journal_style() {
        let journal =
            ledgeline_core::parse::parse_journal("commodity $1,000.00\n", "").expect("parses");
        let money = Money::of_journal(&journal);
        let txn = "2026-11-17 Customer X | Invoice 108  ; invoice: 108\n    \
                   assets:receivable    $13600.00  ; customer: X\n    \
                   revenue:consulting  $-13600.00\n";
        assert_eq!(
            journal_styled(txn, &money),
            "2026-11-17 Customer X | Invoice 108  ; invoice: 108\n    \
             assets:receivable    $13,600.00  ; customer: X\n    \
             revenue:consulting  $-13,600.00\n"
        );
    }
}
