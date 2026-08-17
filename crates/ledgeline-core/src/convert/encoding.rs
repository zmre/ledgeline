//! Turning a statement's bytes into text — the one order that does not misread
//! a real bank file, shared by every format that has to do it.
//!
//! Both the delimited and the OFX lanes need this, they need it to answer the
//! same way, and the two places it is easy to get wrong are the two places a
//! wrong answer is silent rather than loud. So it lives here once.
//!
//! # The order is mandatory, not an optimisation
//!
//! [`decode`] resolves in exactly this sequence:
//!
//! 1. **A byte-order mark**, believed outright.
//! 2. **The encoding the format declared**, when the format has somewhere to
//!    declare one (OFX headers do; a CSV does not).
//! 3. **UTF-8 validity**, which is a *fact* about the bytes rather than a guess
//!    about them, and therefore owes the user no note.
//! 4. **The residual**, per [`Guess`] — either `chardetng` or a fixed
//!    assumption.
//!
//! Sniffing the BOM first is the load-bearing step. [`chardetng`] cannot detect
//! UTF-16 at all, and handed BOM'd UTF-16LE it does not decline — it confidently
//! answers `windows-1252`, which decodes every ASCII character into a character
//! followed by a NUL. That input is not exotic: it is exactly what Excel's
//! "Unicode Text (\*.txt)" export writes, and it is one of the two ways a
//! non-technical user gets a statement out of a spreadsheet. Swapping any two of
//! these steps is a wrong answer rather than a slower one.
//!
//! # `1252` means Windows-1252, not ISO-8859-1
//!
//! The other trap, in [`for_label`]. The two encodings agree everywhere except
//! `0x80`–`0x9F`, which is precisely where smart quotes, the em dash and the euro
//! sign live — i.e. the bytes a real bank memo actually contains. Reading `0x92`
//! as ISO-8859-1 gives a C1 control character; reading it as Windows-1252 gives
//! the `’` the bank meant.
//!
//! # What lives here and what does not
//!
//! This module maps a charset *label* to a decoder and runs the pipeline. It
//! does **not** know how any format spells its declaration: OFX's `ENCODING:` /
//! `CHARSET:` header lines and its `<?xml encoding=…?>` attribute are OFX
//! grammar and are parsed in [`super::ofx`], which hands the resulting label
//! here. Nothing in this module touches the filesystem or sees a path.

use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};
use encoding_rs_io::DecodeReaderBytesBuilder;
use std::io::Read;

/// The byte-order marks that are believed outright. Deliberately no UTF-32:
/// `encoding_rs` implements the WHATWG encoding set, which does not include it,
/// and a UTF-32 statement has never existed.
///
/// This is the same set `encoding_rs` itself sniffs, so the decision made here
/// and the decode performed by [`transcode`] cannot disagree.
const BOMS: [&[u8]; 3] = [
    &[0xEF, 0xBB, 0xBF], // UTF-8
    &[0xFF, 0xFE],       // UTF-16LE — Excel's "Unicode Text" export
    &[0xFE, 0xFF],       // UTF-16BE
];

/// Labels that must resolve to Windows-1252 whatever they claim to be.
///
/// `iso-8859-1` and `us-ascii` are here on purpose. The WHATWG encoding standard
/// already maps the former to Windows-1252 and `encoding_rs` follows it, but
/// stating it here means the rule survives a future decoder swap. `us-ascii` is
/// here because a file that *declares* ASCII and then contains a byte over 0x7F
/// is a file written by something that lied, and Windows-1252 is what it almost
/// certainly meant. `none` is OFX's `CHARSET:NONE`.
const WINDOWS_1252_LABELS: [&str; 12] = [
    "1252",
    "cp1252",
    "windows-1252",
    "windows1252",
    "x-cp1252",
    "latin1",
    "latin-1",
    "iso-8859-1",
    "iso8859-1",
    "8859-1",
    "us-ascii",
    "none",
];

/// Decoded text, plus the label of the encoding **if it had to be guessed**.
pub(super) struct Decoded {
    pub(super) text: String,
    /// `Some` only when nothing in the bytes declared or proved the encoding, so
    /// a [`super::ConvertNote::EncodingGuessed`] is owed to the user.
    pub(super) guessed: Option<String>,
}

/// What a format does once a BOM, its own declaration and UTF-8 validity have
/// all come up empty.
///
/// The two lanes genuinely differ here, and the difference is a property of the
/// format rather than an inconsistency:
///
/// - Delimited text declares nothing at all, so every non-UTF-8 file lands in
///   the residual case and a detector is the only thing left to ask.
/// - OFX declares its encoding in the header, so reaching the residual case
///   already means the file omitted a header field the spec requires. What is
///   left is a two-way choice between UTF-8 and Windows-1252 that step 3 has
///   just settled, and a detector could only move the answer *away* from the
///   one the format's own semantics imply.
pub(super) enum Guess {
    /// Ask `chardetng`.
    Detect,
    /// Assume this encoding without asking.
    Assume(&'static Encoding),
}

impl Guess {
    /// The encoding to use, given the bytes that reached the residual case.
    fn resolve(&self, bytes: &[u8]) -> &'static Encoding {
        match self {
            Self::Detect => detect(bytes),
            Self::Assume(encoding) => encoding,
        }
    }
}

/// Turn `bytes` into text, in the one order that does not misread UTF-16.
///
/// `declared` is whatever the format's own header said, already resolved to a
/// decoder — `None` for a format with nowhere to say it. `guess` decides the
/// residual case. See the module docs: the sequence is BOM, declaration, UTF-8
/// validity, residual, and swapping any two of those is a wrong answer rather
/// than a slower one.
pub(super) fn decode(bytes: &[u8], declared: Option<&'static Encoding>, guess: &Guess) -> Decoded {
    if BOMS.iter().any(|bom| bytes.starts_with(bom)) {
        // Left to `encoding_rs_io`: it is the same BOM sniff, and it also owns
        // the transcode, so the decision and the decode cannot disagree. A
        // lone-surrogate or truncated UTF-16 unit becomes U+FFFD rather than an
        // error, which is right for a preview whose whole job is to show the
        // user what is in the file.
        return declared_text(transcode(bytes, None));
    }
    // A file that says UTF-8 and is not UTF-8 is common enough to plan for:
    // believing it would litter the payees with replacement characters.
    if let Some(encoding) = declared
        && (encoding != UTF_8 || std::str::from_utf8(bytes).is_ok())
    {
        return declared_text(transcode(bytes, Some(encoding)));
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        // Valid UTF-8 is a fact about the bytes, not a guess about them, so it
        // owes the user no note.
        return declared_text(text.to_string());
    }

    let encoding = guess.resolve(bytes);
    Decoded {
        text: transcode(bytes, Some(encoding)),
        guessed: Some(encoding.name().to_string()),
    }
}

/// Text whose encoding was established rather than guessed, so no note is owed.
fn declared_text(text: String) -> Decoded {
    Decoded {
        text,
        guessed: None,
    }
}

/// The encoding a declared charset label names, with the Windows-1252 family
/// forced onto Windows-1252.
///
/// `label` may still carry the keyword it arrived with (`CHARSET:1252`,
/// `ENCODING:USASCII`), because that is how OFX headers spell it; everything up
/// to and including the last `:` is dropped.
///
/// Returns `None` for a label no decoder recognises, so a caller can say "that
/// encoding is not one we read" — or substitute its own default — rather than
/// having one chosen for it silently.
pub(super) fn for_label(label: &str) -> Option<&'static Encoding> {
    let lowered = label.trim().to_ascii_lowercase();
    let value = lowered.rsplit(':').next().unwrap_or(&lowered).trim();
    if WINDOWS_1252_LABELS.contains(&value) {
        return Some(WINDOWS_1252);
    }
    Encoding::for_label(value.as_bytes())
}

/// `chardetng`'s answer, with the Windows-1252 family collapsed onto one
/// encoding.
///
/// The API here is 1.0's, which is **not** the one nearly every example online
/// shows: `new` takes an [`chardetng::Iso2022JpDetection`], `guess` takes an
/// [`chardetng::Utf8Detection`], and `guess_assess` is gone.
///
/// ISO-2022-JP is denied because it is a stateful escape-sequence encoding: a
/// false positive on a Latin-1 bank file does not mangle a few characters, it
/// swallows runs of them. `Utf8Detection::Allow` costs nothing — this is only
/// reached once [`decode`] has already proved the bytes are *not* valid UTF-8.
fn detect(bytes: &[u8]) -> &'static Encoding {
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let guess = detector.guess(None, chardetng::Utf8Detection::Allow);
    for_label(guess.name()).unwrap_or(guess)
}

/// Decode `bytes` to UTF-8, either by believing their BOM (`encoding` is `None`)
/// or by applying `encoding` unconditionally.
///
/// The read cannot fail: the source is a slice, so there is no I/O to go wrong,
/// and undecodable input becomes U+FFFD rather than an error. A failure would
/// still be handled — as empty text, which surfaces as
/// [`super::ConvertError::Empty`] — rather than by unwrapping.
fn transcode(bytes: &[u8], encoding: Option<&'static Encoding>) -> String {
    let mut decoder = DecodeReaderBytesBuilder::new()
        .encoding(encoding)
        // Only meaningful in the BOM branch, and only for a UTF-8 BOM: the
        // UTF-16 decoders consume theirs. The `csv` crate would strip a leading
        // UTF-8 BOM itself, but `delimited::to_csv` output and OFX are not read
        // by it, so the mark comes off here where every caller benefits.
        .strip_bom(true)
        .build(bytes);
    let mut text = String::new();
    match decoder.read_to_string(&mut text) {
        Ok(_) => text,
        // Unreachable for a slice source, and handled rather than unwrapped
        // because a panic here would take down a request that was only ever
        // asked to look at a file. Surfaces as `ConvertError::Empty`.
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` as UTF-16LE with the byte-order mark Excel writes.
    fn utf16le_bom(text: &str) -> Vec<u8> {
        [0xFF, 0xFE]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect()
    }

    #[test]
    fn windows_1252_wins_over_iso_8859_1() {
        // 0x92 is a smart quote in Windows-1252 and a C1 control in ISO-8859-1.
        for label in ["1252", "CHARSET:1252", "iso-8859-1", "latin1", "US-ASCII"] {
            assert_eq!(
                for_label(label),
                Some(WINDOWS_1252),
                "{label} must resolve to Windows-1252"
            );
        }
    }

    #[test]
    fn unknown_labels_are_refused_rather_than_substituted() {
        assert_eq!(for_label("not-an-encoding"), None);
        assert_eq!(for_label("utf-8"), Some(encoding_rs::UTF_8));
    }

    #[test]
    fn a_byte_order_mark_is_believed_before_any_guess() {
        let text = "Date,Description\n2026-01-05,CAF\u{c9} R\u{c9}PUBLIQUE\n";
        let bytes = utf16le_bom(text);

        // The trap, asserted rather than described: `chardetng` cannot detect
        // UTF-16 at all and does not decline. Whatever single-byte encoding it
        // lands on, its reading of these bytes is riddled with NULs — so if the
        // detector ever got to vote first, this would be visibly wrong.
        assert!(
            detect(&bytes)
                .decode_without_bom_handling(&bytes)
                .0
                .contains('\u{0}'),
            "the wrong answer must be visibly wrong, or this test proves nothing"
        );

        let decoded = decode(&bytes, None, &Guess::Detect);
        assert_eq!(decoded.text, text);
        // A BOM is a declaration, not a guess, so no note is owed.
        assert_eq!(decoded.guessed, None);
    }

    #[test]
    fn a_byte_order_mark_outranks_even_a_declared_encoding() {
        // A header that says one thing over bytes that plainly say another: the
        // mark is in the bytes themselves and wins.
        let text = "Betrag\n-12,50\n";
        let decoded = decode(&utf16le_bom(text), Some(WINDOWS_1252), &Guess::Detect);
        assert_eq!(decoded.text, text);
        assert_eq!(decoded.guessed, None);
    }

    #[test]
    fn every_believed_mark_round_trips_and_is_stripped() {
        let text = "a,\u{20ac}\n";
        let utf8 = [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat();
        let utf16be: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_be_bytes))
            .collect();

        for bytes in [utf8, utf16le_bom(text), utf16be] {
            let decoded = decode(&bytes, None, &Guess::Detect);
            assert_eq!(decoded.text, text, "the mark itself must not survive");
            assert_eq!(decoded.guessed, None);
        }
    }

    #[test]
    fn valid_utf8_is_a_fact_and_earns_no_note() {
        let decoded = decode("caf\u{e9}\n".as_bytes(), None, &Guess::Detect);
        assert_eq!(decoded.text, "caf\u{e9}\n");
        assert_eq!(decoded.guessed, None);
    }

    #[test]
    fn a_declared_encoding_is_believed_but_a_false_utf8_claim_is_not() {
        // 0x92 is a right single quotation mark in Windows-1252 and a lone
        // continuation byte — so, invalid — in UTF-8.
        let bytes = b"MOE\x92S TAVERN";

        let declared = decode(bytes, for_label("CHARSET:1252"), &Guess::Detect);
        assert_eq!(declared.text, "MOE\u{2019}S TAVERN");
        assert_eq!(declared.guessed, None, "declared, so not a guess");

        // A file that claims UTF-8 and is not UTF-8 gets its claim ignored
        // rather than its payees littered with replacement characters.
        let lying = decode(bytes, Some(UTF_8), &Guess::Assume(WINDOWS_1252));
        assert_eq!(lying.text, "MOE\u{2019}S TAVERN");
        assert_eq!(lying.guessed, Some("windows-1252".to_string()));
    }

    #[test]
    fn the_residual_case_admits_that_it_guessed() {
        let bytes = b"MCDONALD\x92S RESTAURANT, CAF\xc9 R\xc9PUBLIQUE, \x93QUOTED\x94\n";

        let assumed = decode(bytes, None, &Guess::Assume(WINDOWS_1252));
        assert_eq!(assumed.guessed, Some("windows-1252".to_string()));

        let detected = decode(bytes, None, &Guess::Detect);
        assert_eq!(detected.guessed, Some("windows-1252".to_string()));
        // The C1 range is where Windows-1252 and ISO-8859-1 disagree, and it is
        // exactly what a bank memo is full of.
        assert!(detected.text.contains('\u{2019}'));
        assert!(detected.text.contains('\u{201c}'));
    }
}
