//! "No ASCII control character": the one test every write path and handle
//! validator shares.
//!
//! A `\n` in a value that renders onto one line of a journal, a rules file or a
//! config file is how a second line gets smuggled in; `\r` is invisible damage;
//! the rest of the range (DEL included) has no meaning in any of them. A path
//! component holding one is never a name a person typed. So each of those
//! callers refuses the whole ASCII control range, and each keeps its own error
//! type and message — only the test lives here.
//!
//! **ASCII only, on purpose.** The Unicode-wide [`char::is_control`] checks
//! (C1 controls such as `U+0085`) elsewhere in the engine answer a different
//! question and are deliberately not this one.

/// The first ASCII control character in `text` — `U+0000`..=`U+001F` or DEL
/// (`U+007F`), exactly [`char::is_ascii_control`] — that is not in `allowed`.
///
/// `allowed` is for the few fields that may span lines (a comment may hold
/// `\t` and `\n`); everything else passes `&[]`, or calls
/// [`has_ascii_control`].
pub fn first_ascii_control(text: &str, allowed: &[char]) -> Option<char> {
    text.chars()
        .find(|c| c.is_ascii_control() && !allowed.contains(c))
}

/// Does `text` hold any ASCII control character at all?
pub fn has_ascii_control(text: &str) -> bool {
    first_ascii_control(text, &[]).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ascii_control_character_is_found_including_del() {
        for control in (0u8..0x20).chain([0x7f]).map(char::from) {
            let text = format!("a{control}b");
            assert_eq!(first_ascii_control(&text, &[]), Some(control));
            assert!(has_ascii_control(&text), "{control:?}");
        }
    }

    #[test]
    fn printable_and_non_ascii_text_has_none() {
        // Space and `~` bound the printable range; `U+0085` (a C1 control) and
        // `U+2028` are Unicode's business, not this test's.
        for text in ["", "plain", " ", "~", "café", "a\u{85}b", "a\u{2028}b"] {
            assert_eq!(first_ascii_control(text, &[]), None, "{text:?}");
            assert!(!has_ascii_control(text), "{text:?}");
        }
    }

    #[test]
    fn the_first_one_is_reported() {
        assert_eq!(first_ascii_control("a\rb\nc", &[]), Some('\r'));
    }

    #[test]
    fn allowed_characters_are_skipped_and_the_rest_still_caught() {
        let comment = &['\t', '\n'];
        assert_eq!(first_ascii_control("one\ttwo\nthree", comment), None);
        assert_eq!(first_ascii_control("one\ntwo\r", comment), Some('\r'));
        assert_eq!(first_ascii_control("\t\u{7f}", comment), Some('\u{7f}'));
    }
}
