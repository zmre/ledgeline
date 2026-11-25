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
    /// `[invoicer.words]`.
    pub words: Words,
}

/// The default aging bucket edges: Ledgeline's Current, 1–30, 31–60, 61–90
/// and 91+ days past due.
pub const DEFAULT_AGING_BUCKETS: [u32; 3] = [30, 60, 90];

/// The default days sales outstanding window.
pub const DEFAULT_DSO_DAYS: u32 = 90;

/// The most aging bucket edges `aging-buckets` may list.
pub const MAX_AGING_BUCKETS: usize = 8;

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
            pdf_dir: "invoices/pdf".to_string(),
            cash_deposits: CashDeposits::default(),
            payment_allocation: PaymentAllocation::default(),
            recalc_on_save: RecalcOnSave::default(),
            aging_buckets: DEFAULT_AGING_BUCKETS.to_vec(),
            dso_days: DEFAULT_DSO_DAYS,
            words: Words::default(),
        }
    }
}

/// Why `[invoicer]` can't be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SettingsError {
    /// Not TOML, an unknown key, a bad value.
    #[error("ledgeline.toml [invoicer]: {0}")]
    Invalid(String),
    /// The file is there but can't be read.
    #[error("ledgeline.toml can't be read: {0}")]
    Unreadable(String),
}

impl Settings {
    /// The settings from a whole `ledgeline.toml`'s text.
    ///
    /// # Errors
    /// [`SettingsError::Invalid`] for TOML that doesn't parse, an unknown
    /// key, an empty word list, a blank `customer-tag`, or a filename
    /// pattern without `{invoice_num}`, `aging-buckets` that aren't one to
    /// eight strictly increasing positive day counts, or a zero `dso-days`.
    pub fn from_ledgeline_toml(text: &str) -> Result<Self, SettingsError> {
        let invalid = |e: &dyn std::fmt::Display| SettingsError::Invalid(e.to_string());
        let doc: toml::Table = toml::from_str(text).map_err(|e| invalid(&e))?;
        let settings: Self = match doc.get("invoicer").cloned() {
            None => Self::default(),
            Some(value) => value.try_into().map_err(|e| invalid(&e))?,
        };
        if settings.words.customer_tag.trim().is_empty() {
            return Err(invalid(&"words.customer-tag may not be blank"));
        }
        if !settings.filename.contains("{invoice_num}") {
            return Err(invalid(
                &"filename must contain {invoice_num}, or two invoices could share a file",
            ));
        }
        let edges = &settings.aging_buckets;
        if edges.is_empty()
            || edges.len() > MAX_AGING_BUCKETS
            || edges[0] == 0
            || edges.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(invalid(&format!(
                "aging-buckets = {edges:?}: give 1 to {MAX_AGING_BUCKETS} increasing day counts above zero, like [30, 60, 90]"
            )));
        }
        if settings.dso_days == 0 {
            return Err(invalid(&"dso-days must be at least 1"));
        }
        for (key, dir) in [
            ("invoices-dir", &settings.invoices_dir),
            ("customers-dir", &settings.customers_dir),
            ("templates-dir", &settings.templates_dir),
            ("catalog", &settings.catalog),
            ("pdf-dir", &settings.pdf_dir),
        ] {
            VaultPath::new(dir.trim_start_matches('/'))
                .map_err(|e| invalid(&format!("{key} = {dir:?}: {e}")))?;
        }
        Ok(settings)
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
        VaultPath::new(path.trim_start_matches('/'))
            .map_err(|e| SettingsError::Invalid(format!("{path:?}: {e}")))
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
