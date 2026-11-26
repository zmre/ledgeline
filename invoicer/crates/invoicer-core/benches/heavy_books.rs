//! A heavy user's books folder, for the benches the §5.6 corpus doesn't
//! reach: the accrual fixture (synthetic) copied into a temp dir, plus
//! [`TEMPLATES`] templates and partials, and [`DOCUMENTS`] generated drafts,
//! two thirds of which apply a deposit (two customers billed with deposits
//! every month for years).
//!
//! Not a bench target: `benches/documents.rs` includes it. Never built from
//! real data. Each run builds a fresh folder and [`HeavyBooks`] removes it
//! on drop.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Generated documents.
pub const DOCUMENTS: usize = 5_000;
/// Generated templates (each with a partial beside it).
pub const TEMPLATES: usize = 30;
/// A document `open_deposits` is asked about: Wingtip's sent invoice.
pub const DEPOSIT_TARGET: &str = "106";

/// The folder; removed on drop.
pub struct HeavyBooks {
    /// Its root.
    pub dir: PathBuf,
}

impl Drop for HeavyBooks {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl HeavyBooks {
    /// The main journal.
    #[must_use]
    pub fn journal(&self) -> PathBuf {
        self.dir.join("books.journal")
    }
}

fn copy_tree(src: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).expect("mkdir");
    for entry in std::fs::read_dir(src).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = dest.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// One generated draft: customer `who`, numbered `number`, applying a
/// deposit when `deposit`.
fn draft(who: &str, number: usize, deposit: bool) -> String {
    let day = 1 + number % 28;
    let month = 1 + (number / 28) % 12;
    let date = format!("2025-{month:02}-{day:02}");
    let mut text = format!(
        "---\ntitle: {date} {who} Invoice {number}\ntype: invoice\n\
         template: /invoices/tpl/myco-default.html\nstatus: draft\ninvoice_num: {number}\n\
         date: {date}\nterms: 30\ncustomer: \"[[{who}]]\"\ninvoicee_name: {who}\n\
         invoicer: \"[[My Company]]\"\ninvoicer_name: My Company\n---\n\n\
         | Description             | Qty | Rate   | Amount    | *Account*                     |\n\
         |:------------------------|----:|-------:|----------:|:------------------------------|\n\
         | Monthly retainer        | 1   | $100   | $100.00   | revenue:consulting            |\n"
    );
    if deposit {
        let _ = writeln!(
            text,
            "| Less deposit (Invoice 104) |  |      | -$1.00    | liabilities:customer-deposits |"
        );
    }
    let total = if deposit { "$99.00" } else { "$100.00" };
    let _ = writeln!(
        text,
        "|                         |     | **Total due** | **{total}** |                  |"
    );
    text
}

/// Build the folder.
#[must_use]
pub fn build() -> HeavyBooks {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/accrual");
    let dir = std::env::temp_dir().join(format!("invoicer-bench-heavy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_tree(&fixture, &dir);
    let books = HeavyBooks { dir };
    let tpl = books.dir.join("invoices/tpl");
    for t in 0..TEMPLATES {
        std::fs::write(
            tpl.join(format!("_part{t}.html")),
            "<div>{{ frontmatter.invoice_num }}</div>\n".repeat(40),
        )
        .expect("writes a partial");
        std::fs::write(
            tpl.join(format!("generated-{t}.html")),
            format!(
                "{{# invoicer: name=\"Generated {t}\" kinds=\"invoice\" #}}\n\
                 {{% extends \"_invoice_base.v1.html\" %}}\n\
                 {{% block theme %}}.sheet {{ color: #{t:06}; }}{{% endblock theme %}}\n{}",
                "<!-- padding: a real template is a few kilobytes -->\n".repeat(60)
            ),
        )
        .expect("writes a template");
    }
    let gen_dir = books.dir.join("invoices/generated");
    std::fs::create_dir_all(&gen_dir).expect("mkdir");
    for k in 0..DOCUMENTS {
        let number = 50_000 + k;
        let (who, deposit) = match k % 3 {
            0 => ("Wingtip", true),
            1 => ("Customer X", true),
            _ => ("Contoso", false),
        };
        std::fs::write(
            gen_dir.join(format!("{who} Invoice {number}.md")),
            draft(who, number, deposit),
        )
        .expect("writes a draft");
    }
    books
}
