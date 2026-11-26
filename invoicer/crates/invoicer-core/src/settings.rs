//! `ledgeline.toml [invoicer]` (ARCHITECTURE §3.8): where new files go, the
//! default accounts, the filename pattern, the first number, and the
//! recognized words. There are no new config files; a books folder without
//! the table gets the defaults.

use crate::table::words::Words;
use ledgeline_core::vault::{Vault, VaultPath};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The settings file, beside the main journal.
pub const LEDGELINE_TOML: &str = "ledgeline.toml";

/// A number given as TOML text or an integer (`number-start = 1001` or
/// `"2026-0001"`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum NumberText {
    Int(i64),
    Text(String),
}

/// `[invoicer]`, with the defaults filled in. It serializes back to the
/// same kebab-case keys (`config` prints it).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Settings {
    /// Where new invoices go.
    pub invoices_dir: String,
    /// Where new customers go.
    pub customers_dir: String,
    /// Where starter templates are copied.
    pub templates_dir: String,
    /// The catalog note new catalog items go in.
    pub catalog: String,
    /// Your company: an organization note's title or `[[wikilink]]`.
    pub company: Option<String>,
    /// The receivable account (inferred when unset).
    pub receivable_account: Option<String>,
    /// The default revenue account (inferred when unset).
    pub revenue_account: Option<String>,
    /// Where deposit invoices book (`liabilities:deferred-revenue` when
    /// unset; Phase 2 infers `kpi: deferred-revenue`).
    pub deposit_account: Option<String>,
    /// Where negative adjustments book instead of pro rata.
    pub discount_account: Option<String>,
    /// The new-invoice filename, with `{date}`, `{customer}` and
    /// `{invoice_num}`; `title` mirrors it without `.md`.
    pub filename: String,
    /// The number used when no invoice has one yet (`number-start`, an
    /// integer or text in the TOML).
    #[serde(deserialize_with = "number_text")]
    pub number_start: Option<String>,
    /// A new invoice's payment terms, in net days, when neither it nor its
    /// customer names any (`0` to [`MAX_TERMS_DAYS`]).
    pub default_terms: Option<u32>,
    /// A new invoice's template, as its `template:` holds it
    /// (`/invoices/tpl/classic.html`), when neither it nor its customer
    /// names one; else the first template for its type.
    pub default_template: Option<String>,
    /// Where final PDFs are kept (ignored by discovery).
    pub pdf_dir: String,
    /// Cash books: how a deposit invoice's payment is booked.
    pub cash_deposits: CashDeposits,
    /// Cash books: how a payment is split over an invoice's rows.
    pub payment_allocation: PaymentAllocation,
    /// What the app does when an invoice's file is saved outside it and its
    /// written amounts no longer match its rows.
    pub recalc_on_save: RecalcOnSave,
    /// The dashboard's aging buckets: the last day of each bucket after
    /// Current, in days past due (`[30, 60, 90]`: Current, 1–30, 31–60,
    /// 61–90, 91+, Ledgeline's). Strictly increasing, at least one, at most
    /// eight.
    pub aging_buckets: Vec<u32>,
    /// The dashboard's days sales outstanding window, in days (90; `receivables
    /// --dso-days` overrides it on the command line).
    pub dso_days: u32,
    /// The dashboard's "paid in the last N days" window, in days (30): the
    /// period selector's default (PLAN R13).
    pub paid_window_days: u32,
    /// `[invoicer.words]`.
    pub words: Words,
}

/// The default aging bucket edges: Ledgeline's Current, 1–30, 31–60, 61–90
/// and 91+ days past due.
pub const DEFAULT_AGING_BUCKETS: [u32; 3] = [30, 60, 90];

/// The default days sales outstanding window.
pub const DEFAULT_DSO_DAYS: u32 = 90;

/// The default "paid in the last N days" window.
pub const DEFAULT_PAID_WINDOW_DAYS: u32 = 30;

/// The longest window `paid-window-days` (and the dashboard's `paid_days`)
/// may ask for: ten years.
pub const MAX_PAID_WINDOW_DAYS: u32 = 3660;

/// The most aging bucket edges `aging-buckets` may list.
pub const MAX_AGING_BUCKETS: usize = 8;

/// The longest `default-terms`: ten years of net days.
pub const MAX_TERMS_DAYS: u32 = 3650;

/// `cash-deposits`: how cash books book a deposit (accounting policy; there
/// is more than one valid way, so it is a setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CashDeposits {
    /// A deposit is income the day it arrives (the default): its payment
    /// goes to revenue, and the final invoice's deposit row books nothing.
    #[default]
    IncomeOnReceipt,
    /// A deposit is held in the deposit account until the final invoice
    /// applies it: its payment goes to the liability, and the final
    /// invoice's payment (or, fully covered, its send) debits it.
    LiabilityUntilFinal,
}

/// `payment-allocation`: how a cash-books payment is split over an
/// invoice's rows (accounting policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaymentAllocation {
    /// The default: after every payment and the deposit receipts it applies,
    /// each row account's total moves toward the invoice's row for it, and
    /// the payment that settles the invoice leaves them equal.
    #[default]
    MatchRows,
    /// Every payment pro rata over the rows (net of any deposit), whatever
    /// the deposits already booked.
    ProRata,
}

/// `recalc-on-save`: whether the app rewrites a file's subtotal and total
/// cells after an outside edit (an editor) leaves them disagreeing with the
/// rows. A sent invoice is never recalculated on its own: its amount is
/// booked, so the app shows the mismatch and recalculating it re-books it,
/// after a confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecalcOnSave {
    /// Drafts are recalculated once the edit settles (the default): a draft
    /// touches no books, so its amounts are only arithmetic.
    #[default]
    Drafts,
    /// Nothing is rewritten; the invoice page shows the mismatch and a
    /// Recalculate button.
    Never,
}

fn number_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match NumberText::deserialize(d)? {
        NumberText::Int(n) => Some(n.to_string()),
        NumberText::Text(text) => Some(text),
    })
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            invoices_dir: "invoices".to_string(),
            customers_dir: "customers".to_string(),
            templates_dir: "invoices/tpl".to_string(),
            catalog: "invoices/catalog.md".to_string(),
            company: None,
            receivable_account: None,
            revenue_account: None,
            deposit_account: None,
            discount_account: None,
            filename: "{date} {customer} Invoice {invoice_num}.md".to_string(),
            number_start: None,
            default_terms: None,
            default_template: None,
            pdf_dir: "invoices/pdf".to_string(),
            cash_deposits: CashDeposits::default(),
            payment_allocation: PaymentAllocation::default(),
            recalc_on_save: RecalcOnSave::default(),
            aging_buckets: DEFAULT_AGING_BUCKETS.to_vec(),
            dso_days: DEFAULT_DSO_DAYS,
            paid_window_days: DEFAULT_PAID_WINDOW_DAYS,
            words: Words::default(),
        }
    }
}

/// Why `[invoicer]` can't be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SettingsError {
    /// Not TOML, an unknown setting, a bad value. `key` names the setting
    /// as the file does (`filename`, `words.quantity`), `None` for the file
    /// as a whole; `message` is a plain sentence that names no key, for the
    /// Settings page.
    #[error("ledgeline.toml [invoicer]{}: {message}", key.as_deref().map(|k| format!(" {k}")).unwrap_or_default())]
    Invalid {
        /// The setting.
        key: Option<String>,
        /// Why, in plain words.
        message: String,
    },
    /// The file is there but can't be read.
    #[error("ledgeline.toml can't be read: {0}")]
    Unreadable(String),
}

impl SettingsError {
    /// A problem with the setting `key`.
    #[must_use]
    pub fn invalid(key: &str, message: impl Into<String>) -> Self {
        Self::Invalid {
            key: Some(key.to_string()),
            message: message.into(),
        }
    }

    /// A problem with the file as a whole.
    #[must_use]
    pub fn invalid_file(message: impl Into<String>) -> Self {
        Self::Invalid {
            key: None,
            message: message.into(),
        }
    }
}

/// The settings `[invoicer]` may set, as the file names them, in the
/// Settings page's order. `words` is the `[invoicer.words]` table
/// ([`WORD_KEYS`]).
pub const KEYS: [&str; 20] = [
    "company",
    "filename",
    "number-start",
    "default-terms",
    "default-template",
    "receivable-account",
    "revenue-account",
    "deposit-account",
    "discount-account",
    "cash-deposits",
    "payment-allocation",
    "recalc-on-save",
    "aging-buckets",
    "dso-days",
    "paid-window-days",
    "invoices-dir",
    "customers-dir",
    "templates-dir",
    "catalog",
    "pdf-dir",
];

/// The settings `[invoicer.words]` may set; `words.<key>` elsewhere.
pub const WORD_KEYS: [&str; 13] = [
    "description",
    "quantity",
    "rate",
    "amount",
    "date",
    "unit",
    "item",
    "subtotal",
    "total",
    "deposit",
    "tax",
    "customer-tag",
    "billing-labels",
];

/// A setting's name in plain words, for messages ("the file name", never
/// `filename`): `key` as [`KEYS`] names it, or `words.<key>`.
#[must_use]
pub fn label(key: &str) -> &'static str {
    match key {
        "company" => "your company",
        "invoices-dir" => "the invoices folder",
        "customers-dir" => "the customers folder",
        "templates-dir" => "the templates folder",
        "catalog" => "the catalog note",
        "pdf-dir" => "the PDF folder",
        "filename" => "the new invoice file name",
        "number-start" => "the first invoice number",
        "default-terms" => "the default payment terms",
        "default-template" => "the default template",
        "receivable-account" => "the receivable account",
        "revenue-account" => "the revenue account",
        "deposit-account" => "the deposit account",
        "discount-account" => "the discount account",
        "cash-deposits" => "how deposit payments book",
        "payment-allocation" => "how a payment is split over rows",
        "recalc-on-save" => "recalculating drafts after outside edits",
        "aging-buckets" => "the aging ranges",
        "dso-days" => "the days-sales-outstanding window",
        "paid-window-days" => "the paid window",
        "words.description" => "the description column's words",
        "words.quantity" => "the quantity column's words",
        "words.rate" => "the rate column's words",
        "words.amount" => "the amount column's words",
        "words.date" => "the date column's words",
        "words.unit" => "the unit column's words",
        "words.item" => "the item code column's words",
        "words.subtotal" => "the subtotal row's words",
        "words.total" => "the total row's words",
        "words.deposit" => "the deposit row's words",
        "words.tax" => "the tax row's words",
        "words.customer-tag" => "the customer tag",
        "words.billing-labels" => "the billing labels",
        _ => "this setting",
    }
}

/// Whether `key` names a setting: one of [`KEYS`], or `words.` and one of
/// [`WORD_KEYS`].
#[must_use]
pub fn is_key(key: &str) -> bool {
    KEYS.contains(&key)
        || key
            .strip_prefix("words.")
            .is_some_and(|word| WORD_KEYS.contains(&word))
}

/// What a value of `key` must be, in plain words (after "must be").
fn expected(key: &str) -> String {
    match key {
        "invoices-dir" | "customers-dir" | "templates-dir" | "pdf-dir" => {
            "a folder inside the books folder".to_string()
        }
        "catalog" => "a note inside the books folder".to_string(),
        "default-template" => "an .html template inside the books folder".to_string(),
        "filename" | "company" | "receivable-account" | "revenue-account" | "deposit-account"
        | "discount-account" | "words.customer-tag" => "text".to_string(),
        "number-start" => "a number, or text such as 2026-0001".to_string(),
        "default-terms" => format!("a whole number of days from 0 to {MAX_TERMS_DAYS}"),
        "cash-deposits" | "payment-allocation" | "recalc-on-save" => {
            "one of the choices offered".to_string()
        }
        "aging-buckets" => format!(
            "1 to {MAX_AGING_BUCKETS} increasing numbers of days above zero, such as 30, 60, 90"
        ),
        "dso-days" => format!("a whole number of days from 1 to {MAX_PAID_WINDOW_DAYS}"),
        "paid-window-days" => format!("a whole number of days from 1 to {MAX_PAID_WINDOW_DAYS}"),
        _ => "a list of words, none of them blank".to_string(),
    }
}

/// `label` with its first letter in capitals, to start a sentence.
fn sentence(label: &str) -> String {
    let mut chars = label.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// "The X must be Y.", for `key`.
fn must_be(key: &str) -> SettingsError {
    SettingsError::invalid(
        key,
        format!("{} must be {}.", sentence(label(key)), expected(key)),
    )
}

/// The `[invoicer]` table of a whole `ledgeline.toml`'s text, if it has one.
fn invoicer_table(text: &str) -> Result<Option<toml::Table>, SettingsError> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| {
        let line = e
            .span()
            .map(|span| text[..span.start.min(text.len())].lines().count().max(1));
        SettingsError::invalid_file(match line {
            Some(line) => format!("ledgeline.toml isn't valid TOML (line {line})."),
            None => "ledgeline.toml isn't valid TOML.".to_string(),
        })
    })?;
    match doc.get("invoicer") {
        None => Ok(None),
        Some(toml::Value::Table(table)) => Ok(Some(table.clone())),
        Some(_) => Err(SettingsError::invalid_file(
            "The invoicer settings in ledgeline.toml must be a table ([invoicer]).",
        )),
    }
}

/// The one problem with `key = value`, checked alone (every check is
/// about one setting). `key` is a top-level key, or `words.<key>`.
fn check_one(key: &str, value: &toml::Value) -> Result<(), SettingsError> {
    if let Some(word) = key.strip_prefix("words.") {
        if !WORD_KEYS.contains(&word) {
            return Err(unknown(key));
        }
        let words: Words = toml::Table::from_iter([(word.to_string(), value.clone())])
            .try_into()
            .map_err(|_| must_be(key))?;
        if word == "customer-tag" && words.customer_tag.trim().is_empty() {
            return Err(SettingsError::invalid(
                key,
                "The customer tag may not be blank.",
            ));
        }
        return Ok(());
    }
    if !KEYS.contains(&key) {
        return Err(unknown(key));
    }
    let optional_text = [
        "company",
        "number-start",
        "default-template",
        "receivable-account",
        "revenue-account",
        "deposit-account",
        "discount-account",
    ];
    if optional_text.contains(&key) && value.as_str().is_some_and(|text| text.trim().is_empty()) {
        return Err(SettingsError::invalid(
            key,
            format!(
                "{} may not be blank: choose “Use the default” instead.",
                sentence(label(key))
            ),
        ));
    }
    let one: Settings = toml::Table::from_iter([(key.to_string(), value.clone())])
        .try_into()
        .map_err(|_| must_be(key))?;
    match key {
        "filename" if !one.filename.contains("{invoice_num}") => Err(SettingsError::invalid(
            key,
            "The new invoice file name must include the invoice number, or two invoices could \
             share a file.",
        )),
        "aging-buckets" => {
            let edges = &one.aging_buckets;
            if edges.is_empty()
                || edges.len() > MAX_AGING_BUCKETS
                || edges[0] == 0
                || edges.windows(2).any(|w| w[0] >= w[1])
            {
                Err(must_be(key))
            } else {
                Ok(())
            }
        }
        "dso-days" if one.dso_days == 0 => Err(must_be(key)),
        "paid-window-days" if !(1..=MAX_PAID_WINDOW_DAYS).contains(&one.paid_window_days) => {
            Err(must_be(key))
        }
        "default-terms" if one.default_terms.is_some_and(|t| t > MAX_TERMS_DAYS) => {
            Err(must_be(key))
        }
        "default-template" => match one.default_template.as_deref().map(str::trim) {
            Some(path)
                if VaultPath::new(path.trim_start_matches('/'))
                    .ok()
                    .and_then(|p| p.extension().map(|e| e.eq_ignore_ascii_case("html")))
                    != Some(true) =>
            {
                Err(must_be(key))
            }
            _ => Ok(()),
        },
        "invoices-dir" | "customers-dir" | "templates-dir" | "catalog" | "pdf-dir" => {
            let dir = match key {
                "invoices-dir" => &one.invoices_dir,
                "customers-dir" => &one.customers_dir,
                "templates-dir" => &one.templates_dir,
                "catalog" => &one.catalog,
                _ => &one.pdf_dir,
            };
            VaultPath::new(dir.trim_start_matches('/'))
                .map(|_| ())
                .map_err(|_| must_be(key))
        }
        _ => Ok(()),
    }
}

/// An unknown setting. Its name is quoted: it is the one way to find it in
/// the file.
fn unknown(key: &str) -> SettingsError {
    SettingsError::invalid(
        key,
        format!(
            "The invoicer doesn't know a setting called “{key}”: correct its spelling in \
             ledgeline.toml, or remove it."
        ),
    )
}

/// Every problem with `[invoicer]`, each setting checked on its own.
fn problems_in(table: &toml::Table) -> Vec<SettingsError> {
    let mut problems = Vec::new();
    for (key, value) in table {
        if key == "words" {
            match value {
                toml::Value::Table(words) => {
                    problems.extend(words.iter().filter_map(|(word, value)| {
                        check_one(&format!("words.{word}"), value).err()
                    }))
                }
                _ => problems.push(SettingsError::invalid_file(
                    "The invoicer's words in ledgeline.toml must be a table ([invoicer.words]).",
                )),
            }
        } else if let Err(problem) = check_one(key, value) {
            problems.push(problem);
        }
    }
    problems
}

impl Settings {
    /// The settings from a whole `ledgeline.toml`'s text.
    ///
    /// # Errors
    /// [`SettingsError::Invalid`], keyed, for the first problem
    /// [`Settings::problems`] finds: TOML that doesn't parse, an unknown
    /// key, an empty word list, a blank `customer-tag`, a filename pattern
    /// without `{invoice_num}`, `aging-buckets` that aren't one to eight
    /// strictly increasing positive day counts, a zero `dso-days`, a
    /// `paid-window-days` outside 1 to [`MAX_PAID_WINDOW_DAYS`], a
    /// `default-terms` over [`MAX_TERMS_DAYS`], a `default-template` that
    /// isn't an `.html` path, or a folder outside the books folder.
    pub fn from_ledgeline_toml(text: &str) -> Result<Self, SettingsError> {
        let Some(table) = invoicer_table(text)? else {
            return Ok(Self::default());
        };
        if let Some(problem) = problems_in(&table).into_iter().next() {
            return Err(problem);
        }
        toml::Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| SettingsError::invalid_file(e.to_string()))
    }

    /// Every problem with `[invoicer]` in a whole `ledgeline.toml`'s text,
    /// one per setting (or one for the file when it doesn't parse); empty
    /// when it reads.
    #[must_use]
    pub fn problems(text: &str) -> Vec<SettingsError> {
        match invoicer_table(text) {
            Err(problem) => vec![problem],
            Ok(None) => Vec::new(),
            Ok(Some(table)) => problems_in(&table),
        }
    }

    /// The settings of the books in `vault` (its root's `ledgeline.toml`),
    /// or the defaults when there is none.
    ///
    /// # Errors
    /// As [`Settings::from_ledgeline_toml`], or
    /// [`SettingsError::Unreadable`].
    pub fn read(vault: &dyn Vault) -> Result<Self, SettingsError> {
        let path =
            VaultPath::new(LEDGELINE_TOML).map_err(|e| SettingsError::Unreadable(e.to_string()))?;
        match vault.read_to_string(&path) {
            Ok(text) => Self::from_ledgeline_toml(&text),
            Err(e) if e.is_not_found() => Ok(Self::default()),
            Err(e) => Err(SettingsError::Unreadable(e.to_string())),
        }
    }

    /// The first number: `number-start`, else `1`.
    #[must_use]
    pub fn first_number(&self) -> String {
        self.number_start.clone().unwrap_or_else(|| "1".to_string())
    }

    /// A settings path as a vault path (`/`-rooted or not).
    ///
    /// # Errors
    /// When the path can't name anything in the vault (checked at read).
    pub fn vault_path(path: &str) -> Result<VaultPath, SettingsError> {
        VaultPath::new(path.trim_start_matches('/')).map_err(|_| {
            SettingsError::invalid_file(format!("“{path}” isn't inside the books folder."))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::vault::MemVault;

    #[test]
    fn defaults_without_a_table() {
        let settings = Settings::from_ledgeline_toml("[books]\nbasis = \"cash\"\n").expect("reads");
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.first_number(), "1");
        assert_eq!(settings.words, Words::default());
    }

    #[test]
    fn reads_the_table_and_its_words() {
        let settings = Settings::from_ledgeline_toml(
            "[invoicer]\ninvoices-dir = \"billing\"\ncompany = \"[[My Company]]\"\nnumber-start = 1001\n\
             [invoicer.words]\nquantity = [\"Menge\"]\n",
        )
        .expect("reads");
        assert_eq!(settings.invoices_dir, "billing");
        assert_eq!(settings.company.as_deref(), Some("[[My Company]]"));
        assert_eq!(settings.first_number(), "1001");
        assert_eq!(settings.words.quantity.words(), ["Menge"]);
        assert_eq!(settings.customers_dir, "customers");
        assert_eq!(settings.payment_allocation, PaymentAllocation::MatchRows);
        let policy = Settings::from_ledgeline_toml(
            "[invoicer]\ncash-deposits = \"liability-until-final\"\npayment-allocation = \"pro-rata\"\n",
        )
        .expect("reads");
        assert_eq!(policy.cash_deposits, CashDeposits::LiabilityUntilFinal);
        assert_eq!(policy.payment_allocation, PaymentAllocation::ProRata);
        assert_eq!(settings.recalc_on_save, RecalcOnSave::Drafts);
        let never = Settings::from_ledgeline_toml("[invoicer]\nrecalc-on-save = \"never\"\n")
            .expect("reads");
        assert_eq!(never.recalc_on_save, RecalcOnSave::Never);
        assert!(
            Settings::from_ledgeline_toml("[invoicer]\nrecalc-on-save = \"always\"\n").is_err()
        );
        assert!(
            Settings::from_ledgeline_toml("[invoicer]\npayment-allocation = \"fifo\"\n").is_err()
        );
        let text = Settings::from_ledgeline_toml("[invoicer]\nnumber-start = \"2026-0001\"\n")
            .expect("reads");
        assert_eq!(text.first_number(), "2026-0001");
    }

    #[test]
    fn rejects_typos_and_unsafe_values() {
        for text in [
            "[invoicer]\ninvoice-dir = \"x\"\n",
            "[invoicer]\nfilename = \"{date}.md\"\n",
            "[invoicer]\ninvoices-dir = \"../outside\"\n",
            "[invoicer.words]\ncustomer-tag = \"\"\n",
            "not = = toml",
            "[invoicer]\naging-buckets = []\n",
            "[invoicer]\naging-buckets = [0, 30]\n",
            "[invoicer]\naging-buckets = [30, 30]\n",
            "[invoicer]\naging-buckets = [60, 30]\n",
            "[invoicer]\naging-buckets = [1, 2, 3, 4, 5, 6, 7, 8, 9]\n",
            "[invoicer]\ndso-days = 0\n",
            "[invoicer]\npaid-window-days = 0\n",
            "[invoicer]\npaid-window-days = 5000\n",
        ] {
            assert!(Settings::from_ledgeline_toml(text).is_err(), "{text}");
        }
    }

    #[test]
    fn aging_buckets_and_the_dso_window_are_settings() {
        let settings = Settings::default();
        assert_eq!(settings.aging_buckets, vec![30, 60, 90]);
        assert_eq!(settings.dso_days, 90);
        let settings =
            Settings::from_ledgeline_toml("[invoicer]\naging-buckets = [15, 45]\ndso-days = 365\n")
                .expect("reads");
        assert_eq!(settings.aging_buckets, vec![15, 45]);
        assert_eq!(settings.dso_days, 365);
        assert_eq!(settings.paid_window_days, 30, "the default window");
        let settings =
            Settings::from_ledgeline_toml("[invoicer]\npaid-window-days = 90\n").expect("reads");
        assert_eq!(settings.paid_window_days, 90);
    }

    #[test]
    fn default_terms_are_net_days_up_to_ten_years() {
        let settings =
            Settings::from_ledgeline_toml("[invoicer]\ndefault-terms = 30\n").expect("reads");
        assert_eq!(settings.default_terms, Some(30));
        assert_eq!(Settings::default().default_terms, None);
        assert!(Settings::from_ledgeline_toml("[invoicer]\ndefault-terms = 0\n").is_ok());
        assert!(Settings::from_ledgeline_toml("[invoicer]\ndefault-terms = 3650\n").is_ok());
        for bad in ["3651", "-1", "\"30\"", "1.5"] {
            let error =
                Settings::from_ledgeline_toml(&format!("[invoicer]\ndefault-terms = {bad}\n"))
                    .expect_err(bad);
            assert_eq!(
                error,
                SettingsError::invalid(
                    "default-terms",
                    "The default payment terms must be a whole number of days from 0 to 3650."
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_default_template_is_an_html_path_in_the_books() {
        let settings = Settings::from_ledgeline_toml(
            "[invoicer]\ndefault-template = \"/invoices/tpl/modern.html\"\n",
        )
        .expect("reads");
        assert_eq!(
            settings.default_template.as_deref(),
            Some("/invoices/tpl/modern.html")
        );
        for bad in ["\"../up.html\"", "\"invoices/tpl/x.md\"", "\"\"", "3"] {
            let error =
                Settings::from_ledgeline_toml(&format!("[invoicer]\ndefault-template = {bad}\n"))
                    .expect_err(bad);
            assert!(
                matches!(&error, SettingsError::Invalid { key: Some(k), .. } if k == "default-template"),
                "{bad}: {error}"
            );
        }
    }

    #[test]
    fn errors_are_keyed_and_plain() {
        let problems = Settings::problems(
            "[invoicer]\nfilename = \"{date}.md\"\ninvoice-dir = \"x\"\ndso-days = 0\n\
             [invoicer.words]\nquantity = []\ncustomer-tag = \" \"\nnope = [\"a\"]\n",
        );
        let mut keys: Vec<Option<&str>> = problems
            .iter()
            .map(|p| match p {
                SettingsError::Invalid { key, .. } => key.as_deref(),
                SettingsError::Unreadable(_) => None,
            })
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                Some("dso-days"),
                Some("filename"),
                Some("invoice-dir"),
                Some("words.customer-tag"),
                Some("words.nope"),
                Some("words.quantity"),
            ]
        );
        for problem in &problems {
            let SettingsError::Invalid { key, message } = problem else {
                panic!("{problem}");
            };
            let key = key.as_deref().expect("keyed");
            // A known key never appears in its own message; an unknown one
            // is quoted, the one way to find it.
            if is_key(key) {
                assert!(!message.contains(key), "{message}");
                assert!(
                    !message.contains('{') && !message.contains("[["),
                    "{message}"
                );
            }
            assert!(message.ends_with('.'), "{message}");
        }
        // The CLI's text still names the key, for whoever edits the file.
        assert!(problems.iter().any(|p| {
            p.to_string()
                .starts_with("ledgeline.toml [invoicer] filename: ")
        }));
        let not_toml = Settings::problems("a = = 1");
        assert!(
            matches!(&not_toml[..], [SettingsError::Invalid { key: None, message }] if message.contains("line 1")),
            "{not_toml:?}"
        );
        assert!(Settings::problems("[books]\nbasis = \"cash\"\n").is_empty());
        for key in KEYS {
            assert_ne!(label(key), "this setting", "{key}");
        }
        for word in WORD_KEYS {
            assert_ne!(label(&format!("words.{word}")), "this setting", "{word}");
        }
    }

    #[test]
    fn read_from_a_vault() {
        let empty = MemVault::new();
        assert_eq!(
            Settings::read(&empty).expect("defaults"),
            Settings::default()
        );
        let vault =
            MemVault::new().with_file("ledgeline.toml", "[invoicer]\npdf-dir = \"out/pdf\"\n");
        assert_eq!(Settings::read(&vault).expect("reads").pdf_dir, "out/pdf");
    }
}
