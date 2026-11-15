// Small display helpers the bank sync screens share: money that arrives as
// decimal TEXT, epoch-second times, and ISO dates.
//
// Pure — the clock and the locale come in as arguments — so every wording is
// unit-tested without fake timers. Relative imports only.

/** Symbols for the currencies people most often have; anything else is written after the number. */
const SYMBOLS: Record<string, string> = {USD: "$", CAD: "$", AUD: "$", EUR: "€", GBP: "£", JPY: "¥"};

/**
 * `"-1032.18"` + `"USD"` → `"−$1,032.18"`.
 *
 * The text is never turned into a float: it is split, grouped and put back,
 * so `"1234567.891"` shows every digit it came with. A value that is not a
 * plain decimal (a server bug, a custom currency's oddity) is shown as sent.
 * The minus is U+2212, as the plan's wireframes write it.
 */
export function formatMoney(text: string, currency = "USD"): string {
    const m = /^([+-]?)(\d+)(?:\.(\d+))?$/.exec(text.trim());
    if (m === null) return currency === "" ? text : `${text} ${currency}`;
    const [, sign, whole, frac = ""] = m;
    const grouped = whole.replace(/^0+(?=\d)/, "").replace(/\B(?=(\d{3})+(?!\d))/g, ",");
    const fraction = SYMBOLS[currency] !== undefined && currency !== "JPY" ? frac.padEnd(2, "0") : frac;
    const number = fraction === "" ? grouped : `${grouped}.${fraction}`;
    const zero = /^0*$/.test(whole) && /^0*$/.test(frac);
    const minus = sign === "-" && !zero ? "−" : "";
    const symbol = SYMBOLS[currency];
    return symbol === undefined ? `${minus}${number}${currency === "" ? "" : ` ${currency}`}` : `${minus}${symbol}${number}`;
}

/** The text with its sign flipped (`"5"` → `"-5"`, `"-5"` → `"5"`), still text. */
export function negate(text: string): string {
    const t = text.trim();
    if (t.startsWith("-")) return t.slice(1);
    if (t.startsWith("+")) return `-${t.slice(1)}`;
    return /^0*(\.0*)?$/.test(t) ? t : `-${t}`;
}

/** `"7:12 AM"`. ICU puts a narrow no-break space before AM/PM; plain text reads better in a sentence. */
export function clock(epochSeconds: number, locale?: string): string {
    return new Intl.DateTimeFormat(locale, {hour: "numeric", minute: "2-digit"}).format(new Date(epochSeconds * 1000)).replace(/[\u202f\u00a0]/g, " ");
}

/** The local calendar day of an epoch, as a comparable number of days. */
function dayNumber(date: Date): number {
    return Math.round(new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime() / 86_400_000);
}

/** An ISO `YYYY-MM-DD` as a LOCAL date (never `new Date("YYYY-MM-DD")`, which is UTC midnight). */
export function isoToLocal(iso: string): Date | null {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
    return m === null ? null : new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
}

/** The local date of `epochSeconds` as ISO `YYYY-MM-DD`. */
export function localIso(epochSeconds: number): string {
    const d = new Date(epochSeconds * 1000);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** Whole days from `fromIso` to `toIso` (negative when `to` is earlier); null for a malformed date. */
export function daysBetween(fromIso: string, toIso: string): number | null {
    const from = isoToLocal(fromIso);
    const to = isoToLocal(toIso);
    return from === null || to === null ? null : dayNumber(to) - dayNumber(from);
}

/** `"Nov 11"`, or `"Nov 11, 2025"` when it is not this year (relative to `todayIso`). */
export function shortDate(iso: string, todayIso: string, locale?: string): string {
    const date = isoToLocal(iso);
    if (date === null) return iso;
    const sameYear = iso.slice(0, 4) === todayIso.slice(0, 4);
    return new Intl.DateTimeFormat(locale, sameYear ? {month: "short", day: "numeric"} : {month: "short", day: "numeric", year: "numeric"}).format(date);
}

/**
 * A moment as a person would place it relative to `now`: `"7:12 AM"` today,
 * `"yesterday 7:12 AM"`, `"tomorrow 7:00 AM"`, else the date (`"Nov 2"`).
 */
export function when(epochSeconds: number, now: number, locale?: string): string {
    const delta = dayNumber(new Date(epochSeconds * 1000)) - dayNumber(new Date(now * 1000));
    const time = clock(epochSeconds, locale);
    if (delta === 0) return time;
    if (delta === -1) return `yesterday ${time}`;
    if (delta === 1) return `tomorrow ${time}`;
    return shortDate(localIso(epochSeconds), localIso(now), locale);
}

/** `"1 account"`, `"3 accounts"`. */
export function plural(n: number, one: string, many = `${one}s`): string {
    return `${n} ${n === 1 ? one : many}`;
}
