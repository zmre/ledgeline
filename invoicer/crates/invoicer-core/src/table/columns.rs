//! Column roles, recognized from header text (ARCHITECTURE §3.3).

use super::gfm::Alignment;
use super::money::unbold;
use super::words::Words;

/// What a column holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// The row's description. Required.
    Description,
    /// A decimal quantity.
    Quantity,
    /// Money, or a signed percentage on an adjustment row.
    Rate,
    /// Money, written by the invoicer. Required.
    Amount,
    /// A service date.
    Date,
    /// A unit (`h`, `ea`, `mo`).
    Unit,
    /// A catalog key.
    Item,
    /// The accounting: the last column, with an italic header.
    Books,
    /// A column the invoicer doesn't interpret; kept as written.
    Other,
}

impl Role {
    /// The alignment a delimiter row gets for this role when the table had
    /// none: numbers right, everything else left.
    #[must_use]
    pub fn default_alignment(self) -> Alignment {
        match self {
            Self::Quantity | Self::Rate | Self::Amount => Alignment::Right,
            _ => Alignment::Left,
        }
    }
}

/// Where each role sits in a bill table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columns {
    roles: Vec<Role>,
}

/// Why a header can't be a bill table's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderProblem {
    /// Two columns claim the same role.
    Duplicate {
        /// The role.
        role: Role,
        /// The first column's header text.
        first: String,
        /// The second column's header text.
        second: String,
    },
    /// There is an amount column but no description column.
    NoDescription,
}

impl std::fmt::Display for HeaderProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate {
                role,
                first,
                second,
            } => write!(
                f,
                "`{first}` and `{second}` are both {} columns; rename one",
                format!("{role:?}").to_lowercase()
            ),
            Self::NoDescription => f.write_str("it has an amount column but no description column"),
        }
    }
}

/// `*Account*` or `_Account_`: italic, and not bold.
#[must_use]
pub fn is_italic(text: &str) -> bool {
    let text = text.trim();
    ['*', '_'].iter().any(|mark| {
        text.len() >= 3
            && text.starts_with(*mark)
            && text.ends_with(*mark)
            && !text[1..].starts_with(*mark)
            && !text[..text.len() - 1].ends_with(*mark)
    })
}

/// The role a header names, if any.
fn role_of(header: &str, words: &Words) -> Option<Role> {
    let text = unbold(header.trim()).unwrap_or(header).trim();
    [
        (&words.description, Role::Description),
        (&words.quantity, Role::Quantity),
        (&words.rate, Role::Rate),
        (&words.amount, Role::Amount),
        (&words.date, Role::Date),
        (&words.unit, Role::Unit),
        (&words.item, Role::Item),
    ]
    .iter()
    .find(|(list, _)| list.matches(text))
    .map(|(_, role)| *role)
}

impl Columns {
    /// Recognize a header. `Ok(None)` when no column is an amount column,
    /// which means this isn't a bill table.
    ///
    /// Two columns naming the same role (`| Item | Description |`, or
    /// `| Amount | Total |`) are an error naming both: the invoicer can't
    /// know which one the author meant.
    ///
    /// # Errors
    /// A [`HeaderProblem`] for two columns with one role, or a missing
    /// description.
    pub fn recognize(header: &[String], words: &Words) -> Result<Option<Self>, HeaderProblem> {
        let last = header.len().saturating_sub(1);
        let roles: Vec<Role> = header
            .iter()
            .enumerate()
            .map(|(i, text)| {
                if i == last && i > 0 && is_italic(text) {
                    Role::Books
                } else {
                    role_of(text, words).unwrap_or(Role::Other)
                }
            })
            .collect();
        if !roles.contains(&Role::Amount) {
            return Ok(None);
        }
        if let Some((second, first, role)) = roles.iter().enumerate().find_map(|(i, role)| {
            (*role != Role::Other)
                .then(|| roles[..i].iter().position(|r| r == role))
                .flatten()
                .map(|first| (i, first, *role))
        }) {
            return Err(HeaderProblem::Duplicate {
                role,
                first: header[first].clone(),
                second: header[second].clone(),
            });
        }
        if !roles.contains(&Role::Description) {
            return Err(HeaderProblem::NoDescription);
        }
        Ok(Some(Self { roles }))
    }

    /// The role of column `col`.
    #[must_use]
    pub fn role(&self, col: usize) -> Role {
        self.roles.get(col).copied().unwrap_or(Role::Other)
    }

    /// Every column's role, in order.
    #[must_use]
    pub fn roles(&self) -> &[Role] {
        &self.roles
    }

    /// The column holding `role`, if any.
    #[must_use]
    pub fn find(&self, role: Role) -> Option<usize> {
        self.roles.iter().position(|r| *r == role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| (*c).to_string()).collect()
    }

    #[test]
    fn roles_by_alias_in_any_order() {
        let cols = Columns::recognize(
            &header(&[
                "Line total",
                "unit PRICE",
                "Hours",
                "Service",
                "Notes",
                "*Books*",
            ]),
            &Words::default(),
        )
        .expect("ok")
        .expect("bill table");
        assert_eq!(
            cols.roles(),
            [
                Role::Amount,
                Role::Rate,
                Role::Quantity,
                Role::Description,
                Role::Other,
                Role::Books
            ]
        );
    }

    #[test]
    fn italic_only_counts_last() {
        let cols = Columns::recognize(&header(&["*x*", "Item", "Amount"]), &Words::default())
            .expect("ok")
            .expect("bill table");
        assert_eq!(cols.find(Role::Books), None);
        assert!(!is_italic("**Account**"));
        assert!(is_italic("_Account_"));
    }

    #[test]
    fn not_a_bill_table_and_header_problems() {
        let words = Words::default();
        assert_eq!(
            Columns::recognize(&header(&["Date", "Hours"]), &words),
            Ok(None)
        );
        assert_eq!(
            Columns::recognize(&header(&["Rate", "Amount"]), &words),
            Err(HeaderProblem::NoDescription)
        );
        assert!(matches!(
            Columns::recognize(&header(&["Item", "Amount", "amount"]), &words),
            Err(HeaderProblem::Duplicate {
                role: Role::Amount,
                ..
            })
        ));
        assert_eq!(
            Columns::recognize(&header(&["Date", "date"]), &words),
            Ok(None)
        );
    }

    #[test]
    fn two_columns_with_one_role_are_an_error_naming_both() {
        let words = Words::default();
        for (cells, message) in [
            (
                ["Item", "Description", "Amount"],
                "`Item` and `Description` are both description columns; rename one",
            ),
            (
                ["Description", "Amount", "Total"],
                "`Amount` and `Total` are both amount columns; rename one",
            ),
        ] {
            let problem = Columns::recognize(&header(&cells), &words).expect_err("ambiguous");
            assert_eq!(problem.to_string(), message);
        }
        // §3.6's catalog layout, its code column headed `Code`.
        let cols = Columns::recognize(
            &header(&["Code", "Description", "Unit", "Rate", "Amount"]),
            &words,
        )
        .expect("ok")
        .expect("bill table");
        assert_eq!(
            cols.roles(),
            [
                Role::Item,
                Role::Description,
                Role::Unit,
                Role::Rate,
                Role::Amount
            ]
        );
    }
}
