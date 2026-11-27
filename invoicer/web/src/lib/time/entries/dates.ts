// ISO calendar dates (`YYYY-MM-DD`) for Entries' periods and the week grid.
// The arithmetic runs on UTC midnights, so no time zone or daylight-saving
// change can move a date; weeks start on Monday (ISO 8601, as the server's
// `2026-W45` buckets do).

const ISO = /^(\d{4})-(\d{2})-(\d{2})$/;

/** Whether `text` is a real `YYYY-MM-DD` date. */
export function isIsoDate(text: string): boolean {
    const m = ISO.exec(text);
    if (m === null) return false;
    const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
    return d.getUTCFullYear() === Number(m[1]) && d.getUTCMonth() === Number(m[2]) - 1 && d.getUTCDate() === Number(m[3]);
}

const toUtc = (date: string): Date => {
    const [y, m, d] = date.split("-").map(Number);
    return new Date(Date.UTC(y, m - 1, d));
};

const fromUtc = (d: Date): string => d.toISOString().slice(0, 10);

/** `date` plus `days` (negative goes back). */
export function addDays(date: string, days: number): string {
    const d = toUtc(date);
    d.setUTCDate(d.getUTCDate() + days);
    return fromUtc(d);
}

/** The Monday of `date`'s week. */
export function mondayOf(date: string): string {
    const weekday = (toUtc(date).getUTCDay() + 6) % 7; // Monday 0 … Sunday 6
    return addDays(date, -weekday);
}

/** The seven dates of the week starting `monday`. */
export function weekDates(monday: string): string[] {
    return Array.from({length: 7}, (_, i) => addDays(monday, i));
}

/** The first and last day of `date`'s month. */
export function monthRange(date: string): {from: string; to: string} {
    const d = toUtc(date);
    const first = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), 1));
    const last = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 0));
    return {from: fromUtc(first), to: fromUtc(last)};
}

/** `YYYY-MM` moved by `months`. */
export function addMonths(month: string, months: number): string {
    const [y, m] = month.split("-").map(Number);
    const d = new Date(Date.UTC(y, m - 1 + months, 1));
    return fromUtc(d).slice(0, 7);
}

const WEEKDAY = new Intl.DateTimeFormat("en-US", {weekday: "short", timeZone: "UTC"});
const DAY_MONTH = new Intl.DateTimeFormat("en-US", {month: "short", day: "numeric", timeZone: "UTC"});
const MONTH_YEAR = new Intl.DateTimeFormat("en-US", {month: "long", year: "numeric", timeZone: "UTC"});

/** "Thu" for 2026-11-26. */
export const weekdayName = (date: string): string => WEEKDAY.format(toUtc(date));

/** "Nov 26" for 2026-11-26. */
export const shortDate = (date: string): string => DAY_MONTH.format(toUtc(date));

/** "Thu, Nov 26" for 2026-11-26: a day heading. */
export const dayHeading = (date: string): string => `${weekdayName(date)}, ${shortDate(date)}`;

/** "November 2026" for `2026-11`. */
export const monthName = (month: string): string => MONTH_YEAR.format(toUtc(`${month}-01`));
