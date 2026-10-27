/**
 * The short-date pattern a locale writes, as a hint: `m/d/y` in en-US,
 * `d/m/y` in en-GB, `d.m.y` in German, `y-m-d` in Swedish.
 *
 * Read from `Intl.DateTimeFormat.formatToParts` rather than a table, so the
 * field ORDER and the SEPARATORS are the locale's own. `undefined` is the
 * runtime's default locale — the one a native date input is also drawn in.
 */
const FIELD_LETTER: Partial<Record<Intl.DateTimeFormatPartTypes, string>> = {year: "y", month: "m", day: "d"};

// Bidi marks some locales wrap their separators in (he, ar); a hint has no use for them.
const BIDI_MARKS = /[\u200E\u200F\u061C]/g;

export function datePattern(locale?: string): string {
    const parts = new Intl.DateTimeFormat(locale, {year: "numeric", month: "2-digit", day: "2-digit"}).formatToParts(new Date(2000, 0, 2));
    return parts
        .map((part) => FIELD_LETTER[part.type] ?? (part.type === "literal" ? part.value.replace(BIDI_MARKS, "") : ""))
        .join("")
        .trim();
}
