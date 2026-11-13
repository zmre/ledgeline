//! What a QIF export dropped or reshaped, counted so the user can see it
//! before trusting the result (plan 39: "showing users what will be lost is
//! part of the pitch").

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One QIF file in the export, as the report lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSummary {
    /// The file name the export suggests (`books-USD.qif`).
    pub file_name: String,
    /// The commodity every amount in it is denominated in.
    pub commodity: String,
    /// Records (`^`-terminated transactions) written to it.
    pub records: usize,
}

/// Every count a QIF export reports. A field that is zero is something the
/// journal did not use; [`LossReport::render`] leaves it out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LossReport {
    /// Transactions read from the journal.
    pub transactions: usize,
    /// QIF records written across all files.
    pub records: usize,
    /// The files written, primary commodity first.
    pub files: Vec<FileSummary>,

    // --- Dropped: QIF has nowhere to put these. ---
    /// `= AMOUNT` balance assertions on postings.
    pub balance_assertions: usize,
    /// `P` market-price directives with no QIF form: the price of something
    /// that is not a security (a house), or of a security in a currency it
    /// has no records in.
    pub price_directives: usize,
    /// `P` directives between two currencies (exchange rates).
    pub exchange_rates: usize,
    /// `~` periodic transactions (budgets and forecasts).
    pub periodic_transactions: usize,
    /// `account` declarations carrying tags other than `type:`/`qif:`
    /// (`kpi:`, `cfgroup:`, `holdings:` …).
    pub account_tags: usize,
    /// Transactions with tags other than `class:` (which becomes the QIF class).
    pub transaction_tags: usize,
    /// Postings with tags other than `class:`.
    pub posting_tags: usize,
    /// Transactions with a secondary date (`date2`); QIF has one date.
    pub secondary_dates: usize,
    /// Postings with their own `date:`; the transaction's date is used.
    pub posting_dates: usize,
    /// Unbalanced virtual `(account)` postings, which do not balance and so
    /// cannot be a QIF split.
    pub virtual_postings: usize,

    // --- Kept, but changed to fit QIF. ---
    /// `=` auto-posting rules; their postings are written as ordinary splits.
    pub auto_posting_rules: usize,
    /// Postings those rules generated and the export baked in.
    pub auto_postings_applied: usize,
    /// Transactions written wholly or partly through the holding account: no
    /// bank-like account, an equity leg, or a conversion.
    pub holding_account_transactions: usize,
    /// Opening-balance transactions written as QIF `Opening Balance` records.
    pub opening_balances: usize,
    /// Postings with a cost (`@`/`@@`), written as the cash they cost; the
    /// units (shares, foreign currency) are not carried.
    pub cost_postings: usize,
    /// Transactions converting between commodities — a currency exchange on a
    /// bank account, or two commodities with no written cost; each
    /// commodity's side was balanced through the holding account.
    pub conversions: usize,
    /// Posting comments folded into the record memo because the record has
    /// one split and so no split memo line.
    pub posting_comments_merged: usize,
    /// Transactions moving a security written as investment (`!Type:Invst`)
    /// records: Buy, Sell, Div, ReinvDiv, ShrsIn …
    pub investment_transactions: usize,
    /// Security prices written to a `!Type:Prices` list.
    pub prices_written: usize,
    /// Sells whose realized gain or loss posting was left out; the target
    /// app works the gain out from its own lots.
    pub realized_gains: usize,
    /// `Div`/`ReinvDiv` records, which have no field for the income account.
    pub dividend_categories: usize,
    /// Transactions moving a security that no investment action fits,
    /// written as cash at cost instead, counted by reason.
    pub investment_fallbacks: BTreeMap<String, usize>,
    /// Accounts renamed because QIF reserves a character in their name.
    pub renamed_accounts: Vec<(String, String)>,
    /// Accounts with no resolvable type, exported as expense categories.
    pub untyped_accounts: Vec<String>,
    /// Liabilities exported as `CCard` (the rest are `Oth L`).
    pub credit_cards: Vec<String>,
}

impl LossReport {
    /// Files beyond the first, i.e. how many extra commodities needed a file
    /// of their own because QIF has no currency field.
    #[must_use]
    pub fn extra_commodity_files(&self) -> usize {
        self.files.len().saturating_sub(1)
    }

    /// The report as plain text for a terminal or a dialog.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "QIF export: {} transaction{} written as {} record{} in {} file{}.",
            self.transactions,
            plural(self.transactions),
            self.records,
            plural(self.records),
            self.files.len(),
            plural(self.files.len()),
        );
        for file in &self.files {
            let _ = writeln!(
                out,
                "  {} ({}): {} record{}",
                file.file_name,
                display_commodity(&file.commodity),
                file.records,
                plural(file.records)
            );
        }

        let dropped: Vec<String> = [
            (
                self.balance_assertions,
                "balance assertion",
                "balance assertions",
            ),
            (
                self.price_directives,
                "market price (P directive) of something other than a security",
                "market prices (P directives) of things other than securities",
            ),
            (
                self.exchange_rates,
                "exchange rate (P directive between currencies)",
                "exchange rates (P directives between currencies)",
            ),
            (
                self.realized_gains,
                "realized gain posting on a sale (the app works gains out from its own lots)",
                "realized gain postings on sales (the app works gains out from its own lots)",
            ),
            (
                self.periodic_transactions,
                "periodic transaction (budget or forecast)",
                "periodic transactions (budgets and forecasts)",
            ),
            (
                self.account_tags,
                "account declaration's tags",
                "account declarations' tags",
            ),
            (
                self.transaction_tags,
                "transaction's tags other than class:",
                "transactions' tags other than class:",
            ),
            (
                self.posting_tags,
                "posting's tags other than class:",
                "postings' tags other than class:",
            ),
            (
                self.secondary_dates,
                "secondary date (date2)",
                "secondary dates (date2)",
            ),
            (
                self.posting_dates,
                "posting date (date: tag)",
                "posting dates (date: tags)",
            ),
            (
                self.virtual_postings,
                "unbalanced virtual (account) posting",
                "unbalanced virtual (account) postings",
            ),
        ]
        .into_iter()
        .filter(|(count, _, _)| *count > 0)
        .map(|(count, one, many)| format!("{count} {}", if count == 1 { one } else { many }))
        .collect();
        if !dropped.is_empty() {
            out.push_str("\nNot exported (QIF has no place for these):\n");
            for line in dropped {
                let _ = writeln!(out, "  {line}");
            }
        }

        let mut changed: Vec<String> = Vec::new();
        if self.extra_commodity_files() > 0 {
            changed.push(format!(
                "{} extra commodit{} written to {} file{} of {} own (QIF has no currency field)",
                self.extra_commodity_files(),
                if self.extra_commodity_files() == 1 {
                    "y"
                } else {
                    "ies"
                },
                if self.extra_commodity_files() == 1 {
                    "a"
                } else {
                    "separate"
                },
                plural(self.extra_commodity_files()),
                if self.extra_commodity_files() == 1 {
                    "its"
                } else {
                    "their"
                },
            ));
        }
        if self.auto_posting_rules > 0 {
            changed.push(format!(
                "{} auto-posting rule{} applied: {} generated posting{} written as ordinary splits",
                self.auto_posting_rules,
                plural(self.auto_posting_rules),
                self.auto_postings_applied,
                plural(self.auto_postings_applied),
            ));
        }
        if self.holding_account_transactions > 0 {
            changed.push(format!(
                "{} transaction{} with no bank-like account, an equity leg or a conversion \
                 written through the \"{}\" account",
                self.holding_account_transactions,
                plural(self.holding_account_transactions),
                super::qif::HOLDING_ACCOUNT,
            ));
        }
        if self.opening_balances > 0 {
            changed.push(format!(
                "{} opening balance{} written as QIF Opening Balance record{}",
                self.opening_balances,
                plural(self.opening_balances),
                plural(self.opening_balances),
            ));
        }
        if self.investment_transactions > 0 {
            changed.push(format!(
                "{} security transaction{} written as investment records (Buy, Sell, Div …); \
                 lots and cost basis are not carried",
                self.investment_transactions,
                plural(self.investment_transactions),
            ));
        }
        if self.prices_written > 0 {
            changed.push(format!(
                "{} security price{} written to the QIF price list",
                self.prices_written,
                plural(self.prices_written),
            ));
        }
        if self.dividend_categories > 0 {
            changed.push(format!(
                "{} dividend{} written as Div or ReinvDiv, which name no income account",
                self.dividend_categories,
                plural(self.dividend_categories),
            ));
        }
        for (reason, count) in &self.investment_fallbacks {
            changed.push(format!(
                "{count} security transaction{} written as cash at cost ({reason})",
                plural(*count),
            ));
        }
        if self.cost_postings > 0 {
            changed.push(format!(
                "{} posting{} with a cost (investments, foreign-currency spending) written as the cash amount; \
                 units, lots and prices are not carried",
                self.cost_postings,
                plural(self.cost_postings),
            ));
        }
        if self.conversions > 0 {
            changed.push(format!(
                "{} currency conversion{} split into one record per currency, each balanced \
                 through the \"{}\" account",
                self.conversions,
                plural(self.conversions),
                super::qif::HOLDING_ACCOUNT,
            ));
        }
        if self.posting_comments_merged > 0 {
            changed.push(format!(
                "{} posting comment{} merged into the transaction memo",
                self.posting_comments_merged,
                plural(self.posting_comments_merged),
            ));
        }
        for (from, to) in &self.renamed_accounts {
            changed.push(format!("account {from} renamed to {to}"));
        }
        if !self.untyped_accounts.is_empty() {
            changed.push(format!(
                "{} account{} with no type exported as expense categor{}: {}",
                self.untyped_accounts.len(),
                plural(self.untyped_accounts.len()),
                if self.untyped_accounts.len() == 1 {
                    "y"
                } else {
                    "ies"
                },
                self.untyped_accounts.join(", "),
            ));
        }
        if !self.credit_cards.is_empty() {
            changed.push(format!(
                "liabilit{} exported as credit card{} (CCard): {}",
                if self.credit_cards.len() == 1 {
                    "y"
                } else {
                    "ies"
                },
                plural(self.credit_cards.len()),
                self.credit_cards.join(", "),
            ));
        }
        if !changed.is_empty() {
            out.push_str("\nChanged to fit QIF:\n");
            for line in changed {
                let _ = writeln!(out, "  {line}");
            }
        }
        out
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn display_commodity(commodity: &str) -> &str {
    if commodity.is_empty() {
        "no commodity"
    } else {
        commodity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_report_lists_only_the_summary() {
        let report = LossReport::default();
        assert_eq!(
            report.render(),
            "QIF export: 0 transactions written as 0 records in 0 files.\n"
        );
    }

    #[test]
    fn counts_render_only_when_present() {
        let report = LossReport {
            transactions: 3,
            records: 4,
            files: vec![
                FileSummary {
                    file_name: "b-USD.qif".into(),
                    commodity: "$".into(),
                    records: 3,
                },
                FileSummary {
                    file_name: "b-EUR.qif".into(),
                    commodity: "EUR".into(),
                    records: 1,
                },
            ],
            balance_assertions: 2,
            ..LossReport::default()
        };
        let text = report.render();
        assert!(text.contains("  b-EUR.qif (EUR): 1 record\n"), "{text}");
        assert!(text.contains("  2 balance assertions\n"), "{text}");
        assert!(
            text.contains("1 extra commodity written to a file of its own"),
            "{text}"
        );
        assert!(!text.contains("price"), "{text}");
    }
}
