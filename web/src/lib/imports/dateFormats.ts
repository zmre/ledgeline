// `date-format` made friendly: a catalogue of the strptime patterns real bank
// CSVs use, each with a worked example, plus a renderer for a pattern the
// catalogue does not have.
//
// The single most confusing setting in an hledger rules file is `date-format`,
// because `%m/%d/%Y` and `%d/%m/%Y` are indistinguishable until an import puts a
// March transaction in December. Showing what the pattern DOES to one known date
// is the whole fix.
//
// NO `Date` IS EVER CONSTRUCTED HERE. The reference is a fixed tuple of already-
// decided fields — including the weekday, which is asserted rather than computed
// — so `strftimeExample` is a total pure function of its two arguments with no
// clock, no zone and no locale in it. That also sidesteps the ESLint ban on
// `new Date("YYYY-MM-DD")` (UTC midnight read back through local getters lands
// on the previous day in every negative-offset zone) by having nothing to ban.

/** Every field a supported specifier can ask for, decided up front. */
export interface ReferenceInstant {
    /** Four-digit year. */
    readonly year: string;
    /** Two-digit year. */
    readonly year2: string;
    /** Zero-padded month, 01-12. */
    readonly month: string;
    /** Month with no padding. */
    readonly monthPlain: string;
    readonly monthAbbrev: string;
    readonly monthFull: string;
    /** Zero-padded day of month. */
    readonly day: string;
    /** Day of month with no padding. */
    readonly dayPlain: string;
    readonly weekdayAbbrev: string;
    readonly weekdayFull: string;
    /** Zero-padded 24-hour hour. */
    readonly hour24: string;
    /** Zero-padded 12-hour hour. */
    readonly hour12: string;
    readonly minute: string;
    readonly second: string;
    readonly meridiem: string;
    /** Zero-padded day of year. */
    readonly dayOfYear: string;
}

/**
 * Thursday, 15 January 2026, 13:45:07.
 *
 * Chosen so every ambiguity in a rendered example resolves itself: the day (15)
 * is greater than 12, so `%m/%d` and `%d/%m` produce visibly different strings —
 * which is the one mistake this control exists to prevent. The weekday is
 * committed to as data because computing it would need a `Date`.
 */
export const REFERENCE: ReferenceInstant = Object.freeze({
    year: "2026",
    year2: "26",
    month: "01",
    monthPlain: "1",
    monthAbbrev: "Jan",
    monthFull: "January",
    day: "15",
    dayPlain: "15",
    weekdayAbbrev: "Thu",
    weekdayFull: "Thursday",
    hour24: "13",
    hour12: "01",
    minute: "45",
    second: "07",
    meridiem: "PM",
    dayOfYear: "015",
});

/** One entry of the picker: the pattern, what to call it, and what it produces. */
export interface DateFormatOption {
    readonly pattern: string;
    readonly label: string;
    /**
     * The example, written out rather than rendered.
     *
     * A STATIC string on purpose: this is the list a user scans to recognize
     * their own bank's format, so it must be right even if `strftimeExample`
     * ever is not. `dateFormats.test.ts` asserts the renderer reproduces every
     * one of them, which turns the duplication into a cross-check.
     */
    readonly example: string;
}

/** The value the `<select>` carries when the pattern is not one of the catalogue's. */
export const CUSTOM_OPTION = "__custom__";

/**
 * The formats real bank exports use, commonest first.
 *
 * Deliberately short. A picker with forty entries is a text field with extra
 * steps; anything not here goes through the custom escape hatch, which renders
 * its own example from the same reference instant.
 */
export const DATE_FORMATS: readonly DateFormatOption[] = Object.freeze([
    {pattern: "%Y-%m-%d", label: "ISO — year first", example: "2026-01-15"},
    {pattern: "%m/%d/%Y", label: "US — month first", example: "01/15/2026"},
    {pattern: "%d/%m/%Y", label: "European — day first", example: "15/01/2026"},
    {pattern: "%m/%d/%y", label: "US, two-digit year", example: "01/15/26"},
    {pattern: "%d/%m/%y", label: "European, two-digit year", example: "15/01/26"},
    {pattern: "%d.%m.%Y", label: "Dotted, day first", example: "15.01.2026"},
    {pattern: "%Y/%m/%d", label: "Slashed, year first", example: "2026/01/15"},
    {pattern: "%Y%m%d", label: "Compact digits", example: "20260115"},
    {pattern: "%d-%b-%Y", label: "Day, short month name", example: "15-Jan-2026"},
    {pattern: "%b %d, %Y", label: "Short month name first", example: "Jan 15, 2026"},
    {pattern: "%d %B %Y", label: "Full month name", example: "15 January 2026"},
]);

/** The catalogue entry for `pattern`, or null when it needs the custom field. */
export function findDateFormat(pattern: string): DateFormatOption | null {
    return DATE_FORMATS.find((option) => option.pattern === pattern) ?? null;
}

/** One `%X` specifier → its text, or null when this module does not know it. */
function expand(specifier: string, at: ReferenceInstant): string | null {
    switch (specifier) {
        case "Y":
            return at.year;
        case "y":
            return at.year2;
        case "m":
            return at.month;
        case "d":
            return at.day;
        case "e":
            // `%e` is space-padded day of month; the reference day is two digits.
            return at.dayPlain;
        case "b":
        case "h":
            return at.monthAbbrev;
        case "B":
            return at.monthFull;
        case "a":
            return at.weekdayAbbrev;
        case "A":
            return at.weekdayFull;
        case "H":
            return at.hour24;
        case "I":
            return at.hour12;
        case "M":
            return at.minute;
        case "S":
            return at.second;
        case "p":
            return at.meridiem;
        case "j":
            return at.dayOfYear;
        case "F":
            return `${at.year}-${at.month}-${at.day}`;
        case "D":
            return `${at.month}/${at.day}/${at.year2}`;
        case "T":
            return `${at.hour24}:${at.minute}:${at.second}`;
        case "n":
            return " ";
        case "t":
            return " ";
        default:
            return null;
    }
}

/**
 * Render `pattern` against a fixed instant, so a user can see what their
 * `date-format` will do before an import does it to a year of transactions.
 *
 * Supports `%-` and `%0` padding flags (`%-m` → `1`, not `01`) because hledger
 * passes the pattern to Haskell's `parseTimeM`, which accepts them.
 *
 * A specifier this module does not know is emitted VERBATIM — `%q` renders as
 * `%q`. That is the honest answer and it is visible: an example with a raw `%q`
 * in it tells the user Ledgeline cannot preview that piece, where substituting a
 * plausible-looking value would tell them a confident lie about what their bank
 * file has to contain. `%%` is a literal percent, and a trailing lone `%` is
 * itself.
 */
export function strftimeExample(pattern: string, at: ReferenceInstant = REFERENCE): string {
    let out = "";
    let i = 0;
    while (i < pattern.length) {
        const char = pattern[i] ?? "";
        if (char !== "%") {
            out += char;
            i += 1;
            continue;
        }
        // A `%` with nothing after it is not a specifier; hledger would reject
        // the pattern, and echoing it is how the user sees that it is truncated.
        if (i + 1 >= pattern.length) {
            out += "%";
            break;
        }
        let cursor = i + 1;
        let flag = "";
        while (cursor < pattern.length && (pattern[cursor] === "-" || pattern[cursor] === "0" || pattern[cursor] === "_")) {
            flag = pattern[cursor] ?? "";
            cursor += 1;
        }
        const specifier = pattern[cursor];
        if (specifier === "%") {
            out += "%";
            i = cursor + 1;
            continue;
        }
        const expanded = specifier === undefined ? null : expand(specifier, at);
        if (expanded === null) {
            // Unknown (or an escape flag with nothing to flag): copy the whole
            // run through so what is unsupported stays legible.
            out += pattern.slice(i, cursor + 1);
            i = cursor + 1;
            continue;
        }
        out += flag === "-" || flag === "_" ? stripPad(expanded, flag) : expanded;
        i = cursor + 1;
    }
    return out;
}

/** `%-` drops leading zeroes; `%_` replaces them with spaces. Only ever applied to a numeric expansion. */
function stripPad(text: string, flag: string): string {
    if (!/^0\d/.test(text)) return text;
    const trimmed = text.replace(/^0+(?=\d)/, "");
    return flag === "_" ? trimmed.padStart(text.length, " ") : trimmed;
}
