//! The words the invoicer recognizes (ARCHITECTURE §3.8): column headers,
//! total-row labels and the rest, read from `ledgeline.toml
//! [invoicer.words]`. Setting a key replaces that key's whole default list;
//! the first entry of a list is the word the invoicer writes.

use serde::Deserialize;
use thiserror::Error;

/// A non-empty list of words, matched case-insensitively with runs of
/// whitespace treated as one space.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Vec<String>")]
pub struct WordList(Vec<String>);

impl TryFrom<Vec<String>> for WordList {
    type Error = String;

    fn try_from(words: Vec<String>) -> Result<Self, Self::Error> {
        if words.is_empty() {
            return Err("a word list may not be empty: its first entry is the word written".into());
        }
        if words.iter().any(|word| word.trim().is_empty()) {
            return Err("a word list may not contain a blank word".into());
        }
        Ok(Self(words))
    }
}

impl WordList {
    fn of(words: &[&str]) -> Self {
        Self(words.iter().map(|w| (*w).to_string()).collect())
    }

    /// The word the invoicer writes.
    #[must_use]
    pub fn first(&self) -> &str {
        // Construction guarantees at least one word.
        self.0.first().map_or("", String::as_str)
    }

    /// All the words, in preference order.
    #[must_use]
    pub fn words(&self) -> &[String] {
        &self.0
    }

    /// Whether `text` is one of the words.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        self.0.iter().any(|word| same_word(word, text))
    }
}

/// Case-insensitive, whitespace-collapsing comparison, without allocating:
/// it runs on every cell of every row.
#[must_use]
pub fn same_word(a: &str, b: &str) -> bool {
    fn folded(text: &str) -> impl Iterator<Item = char> + '_ {
        text.split_whitespace()
            .enumerate()
            .flat_map(|(i, word)| (i > 0).then_some(' ').into_iter().chain(word.chars()))
            .flat_map(char::to_lowercase)
    }
    folded(a).eq(folded(b))
}

/// The recognized-word settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Words {
    /// Description column headers.
    pub description: WordList,
    /// Quantity column headers.
    pub quantity: WordList,
    /// Rate column headers.
    pub rate: WordList,
    /// Amount column headers.
    pub amount: WordList,
    /// Date column headers.
    pub date: WordList,
    /// Unit column headers.
    pub unit: WordList,
    /// Item-code (catalog key) column headers.
    pub item: WordList,
    /// Total rows the editor writes mid-table.
    pub subtotal: WordList,
    /// Final total rows; the first is the one the editor writes last.
    pub total: WordList,
    /// The label of an "apply deposit" row.
    pub deposit: WordList,
    /// The default label of a tax row.
    pub tax: WordList,
    /// The tag that marks a contact as a customer.
    pub customer_tag: String,
    /// Email and address labels, in preference order.
    pub billing_labels: WordList,
}

impl Default for Words {
    fn default() -> Self {
        Self {
            description: WordList::of(&["Description", "Item", "Service", "Product"]),
            quantity: WordList::of(&["Qty", "Quantity", "Hours", "Units"]),
            rate: WordList::of(&["Rate", "Price", "Unit price", "Unit cost"]),
            amount: WordList::of(&["Amount", "Total", "Line total"]),
            date: WordList::of(&["Date", "Service date"]),
            unit: WordList::of(&["Unit"]),
            item: WordList::of(&["Code", "SKU"]),
            subtotal: WordList::of(&["Subtotal"]),
            total: WordList::of(&["Total due", "Total", "Balance due", "Amount due"]),
            deposit: WordList::of(&["Less deposit"]),
            tax: WordList::of(&["Sales tax"]),
            customer_tag: "customer".to_string(),
            billing_labels: WordList::of(&["billing", "work"]),
        }
    }
}

/// Why `[invoicer.words]` could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WordsError {
    /// The file is not TOML, or the table has a bad or unknown key.
    #[error("ledgeline.toml [invoicer.words]: {0}")]
    Invalid(String),
}

impl Words {
    /// The words from a whole `ledgeline.toml`'s text: the defaults, with
    /// each key `[invoicer.words]` sets replacing its list.
    ///
    /// # Errors
    /// [`WordsError::Invalid`] for TOML that doesn't parse, an unknown key,
    /// an empty list, or a blank `customer-tag`.
    pub fn from_ledgeline_toml(text: &str) -> Result<Self, WordsError> {
        let invalid = |e: &dyn std::fmt::Display| WordsError::Invalid(e.to_string());
        let doc: toml::Table = toml::from_str(text).map_err(|e| invalid(&e))?;
        let table = doc
            .get("invoicer")
            .and_then(|invoicer| invoicer.get("words"))
            .cloned();
        let words: Self = match table {
            None => Self::default(),
            Some(value) => value.try_into().map_err(|e| invalid(&e))?,
        };
        if words.customer_tag.trim().is_empty() {
            return Err(WordsError::Invalid("customer-tag may not be blank".into()));
        }
        Ok(words)
    }

    /// Whether `text` labels a total row: a subtotal word or a total word.
    #[must_use]
    pub fn is_total_label(&self, text: &str) -> bool {
        self.subtotal.matches(text) || self.total.matches(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_ignores_case_and_whitespace_runs() {
        assert!(same_word("Unit  price", "unit price"));
        assert!(same_word(" TOTAL DUE ", "Total due"));
        assert!(!same_word("Unit", "Unit price"));
        assert!(same_word("ÜBERSICHT", "übersicht"));
    }

    #[test]
    fn a_set_key_replaces_its_list_and_the_rest_default() {
        let words = Words::from_ledgeline_toml(
            "[ui]\ntheme = \"x\"\n[invoicer.words]\nquantity = [\"Menge\"]\n",
        )
        .expect("reads");
        assert_eq!(words.quantity.words(), ["Menge"]);
        assert!(!words.quantity.matches("Qty"));
        assert_eq!(words.rate, Words::default().rate);
    }

    #[test]
    fn no_table_is_the_defaults() {
        assert_eq!(
            Words::from_ledgeline_toml("").expect("reads"),
            Words::default()
        );
    }

    #[test]
    fn bad_settings_are_rejected() {
        for text in [
            "[invoicer.words]\nquantity = []\n",
            "[invoicer.words]\nquantitiy = [\"Menge\"]\n",
            "[invoicer.words]\ncustomer-tag = \" \"\n",
            "[invoicer.words]\nrate = [\"\"]\n",
            "not toml = = =",
        ] {
            assert!(Words::from_ledgeline_toml(text).is_err(), "{text}");
        }
    }
}
