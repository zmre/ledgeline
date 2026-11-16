//! Merchant category codes (ISO 18245) as words a rules file can match on
//! (plan 43): a feed's `mcc` `5411` becomes `Grocery Stores and Supermarkets`
//! in the sync CSV's `mcc_name` column, and the transaction's `category:` tag.
//!
//! The names in `mcc.tsv` are written for that use, from the public-domain
//! list at <https://github.com/greggles/mcc-codes> (the Unlicense): short,
//! and never with a comma, because a comma ends an hledger tag's value.
//! Codes 3000–3999 name single airlines, car-rental firms and hotel chains;
//! they read as the kind of business instead, so one rule covers them all.

/// The code list: `code<TAB>name` per line, sorted by code.
const TABLE: &str = include_str!("mcc.tsv");

/// `raw` as a four-digit code: digits only (a provider may send `742` as a
/// number for `0742`), at most four of them. `None` for anything else.
#[must_use]
pub fn normalize_code(raw: &str) -> Option<String> {
    let raw = raw.trim();
    (!raw.is_empty() && raw.len() <= 4 && raw.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("{raw:0>4}"))
}

/// The name for a four-digit `code`, or `None` for one the list doesn't have.
#[must_use]
pub fn name(code: &str) -> Option<&'static str> {
    let number: u16 = code.parse().ok()?;
    match number {
        3000..=3350 => return Some("Airlines"),
        3351..=3500 => return Some("Car Rental"),
        3501..=3999 => return Some("Hotels and Lodging"),
        _ => {}
    }
    TABLE.lines().find_map(|line| {
        let (key, value) = line.split_once('\t')?;
        (key == code).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_four_digits() {
        assert_eq!(normalize_code("5411").as_deref(), Some("5411"));
        assert_eq!(normalize_code(" 742 ").as_deref(), Some("0742"));
        for bad in ["", "54111", "54a1", "-541", "5.41"] {
            assert_eq!(normalize_code(bad), None, "{bad}");
        }
    }

    #[test]
    fn names_read_as_words() {
        assert_eq!(name("5411"), Some("Grocery Stores and Supermarkets"));
        assert_eq!(name("0742"), Some("Veterinary Services"));
        assert_eq!(name("3000"), Some("Airlines"));
        assert_eq!(name("3357"), Some("Car Rental"));
        assert_eq!(name("3509"), Some("Hotels and Lodging"));
        assert_eq!(name("0000"), None);
        assert_eq!(name("9999"), None);
    }

    #[test]
    fn the_table_is_sorted_unique_and_tag_safe() {
        let codes: Vec<&str> = TABLE
            .lines()
            .map(|line| {
                let (code, value) = line.split_once('\t').expect("code<TAB>name");
                assert_eq!(normalize_code(code).as_deref(), Some(code), "{line}");
                assert!(!value.trim().is_empty(), "{line}");
                assert_eq!(value, value.trim(), "{line}");
                assert!(!value.contains([',', ';', '\t']), "{line}");
                code
            })
            .collect();
        assert!(
            codes.windows(2).all(|pair| pair[0] < pair[1]),
            "sorted, no repeats"
        );
    }
}
