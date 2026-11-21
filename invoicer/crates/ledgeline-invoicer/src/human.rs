//! Text output: aligned columns and `key  value` blocks. `--json` prints the
//! same values with serde instead; nothing here decides anything.

use invoicer_core::doc::{Header, Paid};
use invoicer_core::edit::{Created, RecalcReport, SetOutcome};
use invoicer_core::problem::Problem;
use invoicer_core::views::{
    CheckReport, ConfigView, Counts, CustomerDetail, CustomerEntry, DocDetail, DocEntry,
};
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
const STANDARD_KEYS: [&str; 24] = [
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
    "invoicee_address",
    "invoicer",
    "invoicer_name",
    "invoicer_email",
    "invoicer_address",
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
            let written = match &c.entry {
                invoicer_core::customers::ContractRef::Note { link } => link.clone(),
                invoicer_core::customers::ContractRef::File { path } => path.clone(),
            };
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
pub fn config(view: &ConfigView<'_>) -> Result<String, toml::ser::Error> {
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
