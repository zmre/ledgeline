//! The invoicer's performance corpus (ARCHITECTURE §5.6): a books folder
//! with 5,000 invoices, 500 customers and 10 years of journal.
//!
//! Not a bench target: `benches/documents.rs` includes it, together with
//! ledgeline-core's `benches/corpus.rs`, whose deterministic generator writes
//! the journal's base (a [`SHAPE`] of 50,000 transactions over 10 years).
//! The invoices are then booked into it as Phase 2 will book them, so the
//! same folder serves the Phase 2 benches.
//!
//! # Determinism
//!
//! A pure function of the constants below: one SplitMix64 seeded from
//! [`SEED`], integer cents, dates from `ledgeline_core::reports::add_days`,
//! no clock, no hashing. Change the output and bump [`VERSION`]: it names
//! the folder and the stamp, so a stale corpus is never benchmarked.
//!
//! # Layout (under `target/perf/invoicer-v<VERSION>/`)
//!
//! | Path                       | What                                                            |
//! |----------------------------|-----------------------------------------------------------------|
//! | `books.journal`            | The base journal plus every non-draft invoice, payment and void |
//! | `ledgeline.toml`           | `[invoicer] company`                                            |
//! | `invoices/*.md`            | 5,000 documents in one folder (the default `invoices-dir`)      |
//! | `customers/*.md`           | 500 tagged organizations, a tenth with a portal                 |
//! | `contacts/*.md`            | 100 contacts that aren't customers                              |
//! | `notes/*.md`               | 500 unrelated notes: a books folder is often a vault            |
//! | `invoices/tpl/`, `catalog` | A template, two snippets and the catalog                        |

#![allow(dead_code)]

use crate::corpus::{self, Shape, V1};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Bump on any change to the output.
pub const VERSION: u32 = 1;
/// Invoices.
pub const INVOICES: usize = 5_000;
/// Customers.
pub const CUSTOMERS: usize = 500;
/// Contacts that aren't customers.
pub const CONTACTS: usize = 100;
/// Unrelated notes.
pub const NOTES: usize = 500;
/// The base journal: ledgeline-core's corpus over 10 years.
pub const SHAPE: Shape = Shape {
    label: "inv",
    txns: 50_000,
    years: 10,
    price_months: 120,
    ..V1
};
/// The first invoice number.
pub const FIRST_NUMBER: usize = 10_001;
/// The corpus's last day ("today").
pub const TODAY: &str = "2025-12-31";
const START: &str = "2016-01-01";
const SEED: u64 = 0x1E70_1CE5_C0DE_0001;
const AR: &str = "assets:accounts receivable";
/// Payments land in their own account: the base corpus asserts its bank
/// balances, and the books must stay valid hledger.
const BANK: &str = "assets:bank:operating";

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % (n as u64)).unwrap_or(0)
    }
}

/// `$1,234.56`, sign first.
fn usd(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let dollars = (abs / 100).to_string();
    let grouped: Vec<String> = dollars
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    format!("{sign}${}.{:02}", grouped.join(","), abs % 100)
}

/// `$1234.56` / `$-1234.56`, as the journal writes it.
fn journal_usd(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("${sign}{}.{:02}", abs / 100, abs % 100)
}

fn date(day: usize) -> String {
    ledgeline_core::reports::add_days(START, i64::try_from(day).unwrap_or(0))
}

fn plus(date: &str, days: i64) -> String {
    ledgeline_core::reports::add_days(date, days)
}

const COMPANIES: [&str; 10] = [
    "Acme", "Globex", "Initech", "Umbrella", "Hooli", "Vandelay", "Stark", "Wayne", "Tyrell",
    "Soylent",
];

fn customer_name(k: usize) -> String {
    format!("{} {k:03}", COMPANIES[k % COMPANIES.len()])
}

fn customer_terms(k: usize) -> i64 {
    [15, 30, 45, 60][k % 4]
}

fn has_portal(k: usize) -> bool {
    k.is_multiple_of(10)
}

/// (description, rate in dollars, books cell, catalog code)
const SERVICES: [(&str, i64, &str, &str); 8] = [
    ("Senior architect hours", 800, "revenue:consulting", "ARCH"),
    ("Junior engineer", 200, "revenue:consulting:delivery", "JR"),
    ("Discovery workshop", 2_000, "revenue:consulting", "WKSHP"),
    ("Implementation", 150, "revenue:consulting:delivery", "IMPL"),
    ("Support retainer", 1_200, "revenue:support", "SUP"),
    ("Training session", 600, "revenue:training", "TRAIN"),
    ("Site visit", 800, "revenue:consulting", "VISIT"),
    (
        "Platform subscription",
        12_000,
        "revenue:subscriptions",
        "SAAS",
    ),
];

/// The target directory corpora are written under.
fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../target")
                .components()
                .collect()
        },
        PathBuf::from,
    )
}

/// The corpus folder.
#[must_use]
pub fn dir() -> PathBuf {
    target_dir()
        .join("perf")
        .join(format!("invoicer-v{VERSION}"))
}

fn stamp() -> String {
    format!(
        "invoicer corpus v{VERSION}: {INVOICES} invoices, {CUSTOMERS} customers, {CONTACTS} contacts, {NOTES} notes, {} base transactions over {} years\n",
        SHAPE.txns, SHAPE.years
    )
}

/// The corpus folder, generated first unless its stamp says it is current.
///
/// # Panics
/// If it can't be written.
#[must_use]
pub fn ensure() -> PathBuf {
    let dir = dir();
    let stamp_path = dir.join(".corpus-stamp");
    if std::fs::read_to_string(&stamp_path).ok().as_deref() == Some(stamp().as_str()) {
        return dir;
    }
    let _ = std::fs::remove_dir_all(&dir);
    for sub in [
        "invoices/tpl/snippets",
        "customers",
        "contacts",
        "notes",
        "company",
    ] {
        std::fs::create_dir_all(dir.join(sub)).unwrap_or_else(|e| panic!("create {sub}: {e}"));
    }
    for (path, text) in generate() {
        std::fs::write(dir.join(&path), text).unwrap_or_else(|e| panic!("write {path}: {e}"));
    }
    std::fs::write(&stamp_path, stamp()).unwrap_or_else(|e| panic!("stamp: {e}"));
    dir
}

/// Every file: `(relative path, text)`.
#[must_use]
pub fn generate() -> Vec<(String, String)> {
    let mut rng = Rng(SEED);
    let mut files: Vec<(String, String)> = Vec::with_capacity(INVOICES + CUSTOMERS + NOTES + 16);
    files.extend(library());
    files.extend((0..CUSTOMERS).map(customer));
    files.extend((0..CONTACTS).map(|i| {
        (
            format!("contacts/Contact {i:03}.md"),
            format!("---\ntitle: Contact {i:03}\ntype: person\nemails:\n  - home: contact{i}@example.com\n---\n\nMet at a conference.\n"),
        )
    }));
    files.extend((0..NOTES).map(|i| {
        (
            format!("notes/Meeting {i:04}.md"),
            format!(
                "---\ntitle: Meeting {i:04}\ntype: note\ntags: [meeting, q{}]\n---\n\n# Meeting {i:04}\n\nDiscussed the roadmap with {}.\n\n- action one\n- action two\n",
                i % 4 + 1,
                customer_name(i % CUSTOMERS)
            ),
        )
    }));
    let mut journal = String::with_capacity(SHAPE.txns * 160 + INVOICES * 700);
    journal.push_str(
        "; Invoicer performance corpus — GENERATED by invoicer-core/benches/invoicer_corpus.rs.\n\
         account assets:accounts receivable  ; type: A, kpi: receivable\n\
         account revenue:consulting  ; type: R\n\
         account revenue:consulting:delivery  ; type: R\n\
         account revenue:support  ; type: R\n\
         account revenue:training  ; type: R\n\
         account revenue:subscriptions  ; type: R\n\
         account liabilities:sales-tax  ; type: L\n\
         account assets:bank:operating  ; type: C\n\n",
    );
    journal.push_str(&corpus::generate_shape(&SHAPE));
    for i in 0..INVOICES {
        let (path, text, books) = invoice(i, &mut rng);
        files.push((path, text));
        journal.push_str(&books);
    }
    files.push(("books.journal".to_string(), journal));
    files
}

fn library() -> Vec<(String, String)> {
    let catalog_rows: String = SERVICES
        .iter()
        .map(|(desc, rate, books, code)| {
            format!("| {code} | {desc} | ea | {} | {books} |\n", usd(rate * 100))
        })
        .collect();
    vec![
        (
            "ledgeline.toml".into(),
            "[invoicer]\ncompany = \"[[Corpus Co]]\"\n".into(),
        ),
        (
            "company/Corpus Co.md".into(),
            "---\ntitle: Corpus Co\ntype: organization\nemails:\n  - billing: billing@corpus.example\naddresses:\n  - work: |\n      1 Corpus Way\n      Springfield, IL 62701\n---\n".into(),
        ),
        (
            "invoices/tpl/default.html".into(),
            "{# invoicer: name=\"Corpus\" kinds=\"invoice\" #}\n<h1>Invoice {{ frontmatter.invoice_num }}</h1>\n{{ markdown | safe }}\n".into(),
        ),
        (
            "invoices/tpl/snippets/net-30.md".into(),
            "---\ntitle: Net 30 ACH\ntype: snippet\nsnippet_kind: payment\n---\n\n## Payment\n\nPay by ACH within 30 days.\n".into(),
        ),
        (
            "invoices/tpl/snippets/thanks.md".into(),
            "---\ntitle: Thank you\ntype: snippet\nsnippet_kind: note\n---\n\nThank you for your business.\n".into(),
        ),
        (
            "invoices/catalog.md".into(),
            format!("---\ntitle: Catalog\ntype: catalog\n---\n\n| Code | Description | Unit | Rate | *Account* |\n|:--|:--|:--|--:|:--|\n{catalog_rows}"),
        ),
    ]
}

fn customer(k: usize) -> (String, String) {
    let name = customer_name(k);
    let slug = name.to_lowercase().replace(' ', "-");
    let mut text = format!(
        "---\ntitle: {name}\ntype: organization\ntags: [customer]\nemails:\n  - billing: ap@{slug}.example\n  - work: hello@{slug}.example\n"
    );
    if k.is_multiple_of(2) {
        let _ = write!(
            text,
            "addresses:\n  - billing: |\n      {k} Main St.\n      Springfield, CO 80000\n"
        );
    } else {
        let _ = write!(
            text,
            "addresses:\n  - billing:\n      street: {k} Market St.\n      city: Springfield\n      region: IL\n      postcode: \"62701\"\n      country: USA\n"
        );
    }
    let _ = write!(
        text,
        "terms: {}\ninvoice_template: /invoices/tpl/default.html\nsnippets: [\"[[Net 30 ACH]]\"]\n",
        customer_terms(k)
    );
    if has_portal(k) {
        let _ = write!(
            text,
            "open_pos:\n  - reference: \"PO #{k}\"\n    amount: $40,000\nbilling_portal:\n  name: Coupa\n  url: https://{slug}.coupahost.example/\n  login: billing@corpus.example\n  requires_reference: true\n"
        );
    }
    text.push_str("---\n\nAccount notes.\n");
    (format!("customers/{name}.md"), text)
}

/// Add `cents` to `account`'s share, keeping first-use order.
fn add(account: &'static str, cents: i64, by: &mut Vec<(&'static str, i64)>) {
    match by.iter_mut().find(|(a, _)| *a == account) {
        Some((_, sum)) => *sum += cents,
        None => by.push((account, cents)),
    }
}

/// One invoice: its file, and what the books hold for it.
fn invoice(i: usize, rng: &mut Rng) -> (String, String, String) {
    let k = (i * 7) % CUSTOMERS;
    let name = customer_name(k);
    let slug = name.to_lowercase().replace(' ', "-");
    let number = FIRST_NUMBER + i;
    let day = i * 3_650 / INVOICES;
    let date = date(day);
    let terms = customer_terms(k);
    let due = plus(&date, terms);
    let kind = i % 20;
    let filename = format!("{date} {name} Invoice {number}");
    let reference = (has_portal(k) || i.is_multiple_of(3)).then(|| format!("PO #{k}-{i}"));

    // Rows, in cents, with each account's share.
    let mut table = String::from(
        "| Code | Description | Qty | Rate | Amount | *Account* |\n|:--|:--|--:|--:|--:|:--|\n",
    );
    let mut running: i64 = 0;
    let mut by_account: Vec<(&'static str, i64)> = Vec::new();
    let items = 2 + i % 5;
    for j in 0..items {
        let (desc, rate, books, code) = SERVICES[(i + j * 3 + rng.below(3)) % SERVICES.len()];
        let qty = 1 + i64::try_from((i + j) % 12).unwrap_or(0);
        let amount = qty * rate * 100;
        running += amount;
        add(books, amount, &mut by_account);
        let _ = writeln!(
            table,
            "| {code} | {desc} | {qty} | {} | {} | {books} |",
            usd(rate * 100),
            usd(amount)
        );
        if j == 1 && i.is_multiple_of(3) {
            let discount = -amount / 10;
            running += discount;
            add(books, discount, &mut by_account);
            let _ = writeln!(
                table,
                "| | Loyalty discount | | -10% | {} | |",
                usd(discount)
            );
        }
    }
    if i.is_multiple_of(4) {
        let _ = writeln!(table, "| | | | **Subtotal** | **{}** | |", usd(running));
        let tax = (running * 825 + 5_000) / 10_000;
        running += tax;
        add("liabilities:sales-tax", tax, &mut by_account);
        let _ = writeln!(
            table,
            "| | Sales tax | | 8.25% | {} | liabilities:sales-tax |",
            usd(tax)
        );
    }
    let total = running;
    let _ = writeln!(table, "| | | | **Total due** | **{}** | |", usd(total));

    let status = match kind {
        15 => "void",
        19 => "draft",
        _ => "sent",
    };
    let first = plus(&date, 20);
    let second = plus(&date, 40);
    let paid = match kind {
        0..=11 => Some(plus(&date, 25)),
        12 | 13 => Some(second.clone()),
        14 => Some(format!("partial {first} {}", usd(total / 3))),
        19 => None,
        _ => Some("no".to_string()),
    };
    let mut text = format!(
        "---\ntitle: {filename}\ntype: invoice\ntemplate: /invoices/tpl/default.html\nstatus: {status}\ninvoice_num: {number}\ndate: {date}\nterms: {terms}\ndue_date: {due}\n"
    );
    if let Some(reference) = &reference {
        let _ = writeln!(text, "reference: \"{reference}\"");
    }
    let _ = write!(
        text,
        "customer: \"[[{name}]]\"\ninvoicee_name: {name}\ninvoicee_email: ap@{slug}.example\ninvoicee_address: |\n  {k} Main St.\n  Springfield, CO 80000\ninvoicer: \"[[Corpus Co]]\"\ninvoicer_name: Corpus Co\ninvoicer_email: billing@corpus.example\ninvoicer_address: |\n  1 Corpus Way\n  Springfield, IL 62701\n"
    );
    if status != "draft" {
        let _ = writeln!(text, "sent_date: {date}\nsent_via: email");
    }
    if status == "void" {
        let _ = writeln!(text, "void_date: {}", plus(&date, 5));
    }
    if let Some(paid) = &paid {
        let _ = writeln!(text, "paid: {paid}");
    }
    let _ = write!(
        text,
        "---\n\nHi, here is the work for {date}.\n\n{table}\n## Payment\n\nPay by ACH within 30 days.\n"
    );

    // The books.
    let mut books = String::new();
    if status != "draft" {
        let link = format!("i{i:05}");
        let tag = match kind {
            0..=14 => format!(", match:{link}"),
            15 => format!(", match:v{i:05}"),
            _ => String::new(),
        };
        let reference_tag = reference
            .as_deref()
            .map(|r| format!(", reference: {r}"))
            .unwrap_or_default();
        let _ = writeln!(
            books,
            "\n{date} {name} | Invoice {number}  ; invoice: {number}, document: invoices/{filename}.md, due: {due}{reference_tag}"
        );
        let _ = writeln!(
            books,
            "    {AR}  {}  ; customer: {name}{tag}",
            journal_usd(total)
        );
        for (account, cents) in &by_account {
            let _ = writeln!(
                books,
                "    {account}  {}  ; customer: {name}",
                journal_usd(-cents)
            );
        }
        let pay = |books: &mut String, on: &str, part: i64| {
            let _ = write!(
                books,
                "\n{on} {name} payment  ; invoice: {number}\n    {BANK}  {}\n    {AR}  {}  ; customer: {name}, match:{link}\n",
                journal_usd(part),
                journal_usd(-part)
            );
        };
        match kind {
            0..=11 => pay(&mut books, &plus(&date, 25), total),
            12 | 13 => {
                pay(&mut books, &first, total / 2);
                pay(&mut books, &second, total - total / 2);
            }
            14 => pay(&mut books, &first, total / 3),
            15 => {
                let void = plus(&date, 5);
                let _ = writeln!(
                    books,
                    "\n{void} {name} | Invoice {number} (void)  ; invoice: {number}, void: {void}\n    {AR}  {}  ; customer: {name}, match:v{i:05}",
                    journal_usd(-total)
                );
                for (account, cents) in &by_account {
                    let _ = writeln!(books, "    {account}  {}", journal_usd(*cents));
                }
            }
            _ => {}
        }
    }
    (format!("invoices/{filename}.md"), text, books)
}
