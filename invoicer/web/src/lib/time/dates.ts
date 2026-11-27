// ISO calendar dates (`YYYY-MM-DD`) for every time page: Track, Entries'
// periods and week grid, the dashboard's periods, and billing. The
// arithmetic runs on UTC midnights, so no time zone or daylight-saving
// change can move a date. Weeks start on Monday (ISO 8601, as the server's
// `2026-W48` buckets do); working days are Monday to Friday.

const ISO = /^(\d{4})-(\d{2})-(\d{2})$/;
const DAY_MS = 86_400_000;

/** Whether `text` is a real `YYYY-MM-DD` date. */
export function isIsoDate(text: string): boolean {
    const m = ISO.exec(text);
    if (m === null) return false;
    const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
    return d.getUTCFullYear() === Number(m[1]) && d.getUTCMonth() === Number(m[2]) - 1 && d.getUTCDate() === Number(m[3]);
}

const toUtc = (date: string): Date => {
    const [y, m, d] = date.split("-").map(Number);
    return new Date(Date.UTC(y, (m ?? 1) - 1, d ?? 1));
};

const fromUtc = (d: Date): string => d.toISOString().slice(0, 10);

/** `date` plus `days` (negative goes back). */
export function addDays(date: string, days: number): string {
    return fromUtc(new Date(toUtc(date).getTime() + days * DAY_MS));
}

/** Days from `from` to `to` (0 when equal; negative when `to` is earlier). */
export function daysBetween(from: string, to: string): number {
    return Math.round((toUtc(to).getTime() - toUtc(from).getTime()) / DAY_MS);
}

/** Monday 0 … Sunday 6. */
export function weekday(date: string): number {
    return (toUtc(date).getUTCDay() + 6) % 7;
}

export const isWorkingDay = (date: string): boolean => weekday(date) < 5;

/** The Monday of `date`'s week. */
export const mondayOf = (date: string): string => addDays(date, -weekday(date));

/** The seven dates of the week starting `monday`. */
export function weekDates(monday: string): string[] {
    return Array.from({length: 7}, (_, i) => addDays(monday, i));
}

/** The last day of `date`'s month. */
export function monthEnd(date: string): string {
    const d = toUtc(date);
    return fromUtc(new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 0)));
}

export const monthStart = (date: string): string => `${date.slice(0, 8)}01`;

/** The first and last day of `date`'s month. */
export const monthRange = (date: string): {from: string; to: string} => ({from: monthStart(date), to: monthEnd(date)});

export const isMonthStart = (date: string): boolean => date.endsWith("-01");

export const isMonthEnd = (date: string): boolean => monthEnd(date) === date;

/** The first day of `date`'s quarter. */
export function quarterStart(date: string): string {
    const month = Number(date.slice(5, 7));
    const first = Math.floor((month - 1) / 3) * 3 + 1;
    return `${date.slice(0, 4)}-${String(first).padStart(2, "0")}-01`;
}

/**
 * `date` moved by `months`, its day kept, or the month's last day when that
 * month is shorter (2026-03-31 back one month is 2026-02-28). A month's last
 * day stays a last day, so a whole month maps onto a whole month.
 */
export function addMonths(date: string, months: number): string {
    const d = toUtc(date);
    const target = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + months, 1));
    const last = new Date(Date.UTC(target.getUTCFullYear(), target.getUTCMonth() + 1, 0)).getUTCDate();
    const day = isMonthEnd(date) ? last : Math.min(d.getUTCDate(), last);
    return fromUtc(new Date(Date.UTC(target.getUTCFullYear(), target.getUTCMonth(), day)));
}

/** A month (`YYYY-MM`) moved by `months`: `2026-11` back one is `2026-10`. */
export const shiftMonth = (month: string, months: number): string => addMonths(`${month}-01`, months).slice(0, 7);

/** Monday-to-Friday days in `from`..=`to` (0 when `to` is before `from`). */
export function workingDays(from: string, to: string): number {
    const span = daysBetween(from, to) + 1;
    if (span <= 0) return 0;
    const whole = Math.floor(span / 7);
    let count = whole * 5;
    const start = weekday(from);
    for (let i = 0; i < span % 7; i++) if ((start + i) % 7 < 5) count += 1;
    return count;
}

/** Every date in `from`..=`to`. */
export function eachDay(from: string, to: string): string[] {
    const span = daysBetween(from, to) + 1;
    return Array.from({length: Math.max(0, span)}, (_, i) => addDays(from, i));
}

const DAY_MONTH = new Intl.DateTimeFormat("en-US", {month: "short", day: "numeric", timeZone: "UTC"});
const DAY_MONTH_YEAR = new Intl.DateTimeFormat("en-US", {month: "short", day: "numeric", year: "numeric", timeZone: "UTC"});
const MONTH = new Intl.DateTimeFormat("en-US", {month: "short", timeZone: "UTC"});
const MONTH_LONG = new Intl.DateTimeFormat("en-US", {month: "long", timeZone: "UTC"});
const MONTH_YEAR = new Intl.DateTimeFormat("en-US", {month: "short", year: "numeric", timeZone: "UTC"});
const MONTH_YEAR_LONG = new Intl.DateTimeFormat("en-US", {month: "long", year: "numeric", timeZone: "UTC"});
const WEEKDAY = new Intl.DateTimeFormat("en-US", {weekday: "short", timeZone: "UTC"});

/** "Nov 26". */
export const dayMonth = (date: string): string => DAY_MONTH.format(toUtc(date));
/** "Nov 26, 2026". */
export const dayMonthYear = (date: string): string => DAY_MONTH_YEAR.format(toUtc(date));
/** "Nov". */
export const monthName = (date: string): string => MONTH.format(toUtc(date));
/** "November". */
export const longMonthName = (date: string): string => MONTH_LONG.format(toUtc(date));
/** "Nov 2026". */
export const monthYear = (date: string): string => MONTH_YEAR.format(toUtc(date));
/** "November 2026" for a month (`2026-11`) or any date in it. */
export const longMonthYear = (month: string): string => MONTH_YEAR_LONG.format(toUtc(month.length === 7 ? `${month}-01` : month));
/** "Thu". */
export const weekdayName = (date: string): string => WEEKDAY.format(toUtc(date));
/** "Thu, Nov 26": a day heading. */
export const dayHeading = (date: string): string => `${weekdayName(date)}, ${dayMonth(date)}`;

/** "Nov 1 – 26, 2026", "Oct 27 – Nov 2, 2026", "Dec 1, 2025 – Nov 30, 2026". */
export function rangeText(from: string, to: string): string {
    if (from === to) return dayMonthYear(from);
    const sameYear = from.slice(0, 4) === to.slice(0, 4);
    if (!sameYear) return `${dayMonthYear(from)} – ${dayMonthYear(to)}`;
    if (from.slice(0, 7) === to.slice(0, 7)) return `${dayMonth(from)} – ${Number(to.slice(8))}, ${to.slice(0, 4)}`;
    return `${dayMonth(from)} – ${dayMonthYear(to)}`;
}
