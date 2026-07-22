// Period/bucket math (WP-06). Pure string + integer date arithmetic — NEVER
// `new Date("YYYY-MM-DD")` (parses as UTC and shifts a day in negative-offset
// zones). The ONLY permitted Date usage in the codebase's date math is
// `today()`, which reads local Date parts.
//
// Bucket key formats:
//   daily      "2026-07-08"
//   weekly     "2026-W28"   (ISO-8601 week; weeks run Mon–Sun, W01 contains Jan 4)
//   monthly    "2026-07"
//   quarterly  "2026-Q3"
//   yearly     "2026"

import type {ISODate} from "../domain/types";

export type Interval = "daily" | "weekly" | "monthly" | "quarterly" | "yearly";

const pad2 = (n: number): string => String(n).padStart(2, "0");
const pad4 = (n: number): string => String(n).padStart(4, "0");

function parts(date: ISODate): [number, number, number] {
    return [Number(date.slice(0, 4)), Number(date.slice(5, 7)), Number(date.slice(8, 10))];
}

function isLeap(y: number): boolean {
    return y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0);
}

const MONTH_DAYS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

function daysInMonth(y: number, m: number): number {
    return m === 2 && isLeap(y) ? 29 : MONTH_DAYS[m - 1];
}

/** Days since 1970-01-01 in the proleptic Gregorian calendar (Howard Hinnant's `days_from_civil`). */
function daysFromCivil(y: number, m: number, d: number): number {
    y -= m <= 2 ? 1 : 0;
    const era = Math.floor(y / 400);
    const yoe = y - era * 400;
    const doy = Math.floor((153 * (m + (m > 2 ? -3 : 9)) + 2) / 5) + d - 1;
    const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
    return era * 146097 + doe - 719468;
}

/** Inverse of `daysFromCivil` (Howard Hinnant's `civil_from_days`). */
function civilFromDays(z: number): [number, number, number] {
    z += 719468;
    const era = Math.floor(z / 146097);
    const doe = z - era * 146097;
    const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365);
    const y = yoe + era * 400;
    const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100));
    const mp = Math.floor((5 * doy + 2) / 153);
    const d = doy - Math.floor((153 * mp + 2) / 5) + 1;
    const m = mp + (mp < 10 ? 3 : -9);
    return [y + (m <= 2 ? 1 : 0), m, d];
}

const toISO = (y: number, m: number, d: number): ISODate => `${pad4(y)}-${pad2(m)}-${pad2(d)}`;

/** ISO weekday for a `daysFromCivil` day number: 1 = Monday … 7 = Sunday. (1970-01-01 was a Thursday.) */
function isoWeekday(days: number): number {
    return ((((days + 3) % 7) + 7) % 7) + 1;
}

/** ["2026-W28"] → Monday day-number of that ISO week. */
function isoWeekMonday(year: number, week: number): number {
    const jan4 = daysFromCivil(year, 1, 4); // Jan 4 is always in ISO week 1
    return jan4 - (isoWeekday(jan4) - 1) + (week - 1) * 7;
}

const WEEK_RE = /^(\d{4})-W(\d{2})$/;
const MONTH_RE = /^(\d{4})-(\d{2})$/;
const QUARTER_RE = /^(\d{4})-Q([1-4])$/;
const YEAR_RE = /^\d{4}$/;
const DAY_RE = /^\d{4}-\d{2}-\d{2}$/;

/** Bucket key containing `date`, e.g. "2026-07", "2026-Q3", "2026-W28". */
export function bucketKey(date: ISODate, interval: Interval): string {
    switch (interval) {
        case "daily":
            return date;
        case "weekly": {
            const [y, m, d] = parts(date);
            const days = daysFromCivil(y, m, d);
            const thursday = days + (4 - isoWeekday(days)); // the week's Thursday fixes the ISO week-year
            const [wy] = civilFromDays(thursday);
            const week = Math.floor((thursday - daysFromCivil(wy, 1, 1)) / 7) + 1;
            return `${pad4(wy)}-W${pad2(week)}`;
        }
        case "monthly":
            return date.slice(0, 7);
        case "quarterly": {
            const month = Number(date.slice(5, 7));
            return `${date.slice(0, 4)}-Q${Math.floor((month - 1) / 3) + 1}`;
        }
        case "yearly":
            return date.slice(0, 4);
    }
}

const MONTH_NAMES = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** Human label for a bucket key: "2026-07" → "Jul 2026", "2026-Q3" → "Q3 2026", "2026-W28" → "W28 2026". */
export function bucketLabel(key: string): string {
    const month = MONTH_RE.exec(key);
    if (month !== null) return `${MONTH_NAMES[Number(month[2]) - 1]} ${month[1]}`;
    const quarter = QUARTER_RE.exec(key);
    if (quarter !== null) return `Q${quarter[2]} ${quarter[1]}`;
    const week = WEEK_RE.exec(key);
    if (week !== null) return `W${week[2]} ${week[1]}`;
    return key; // yearly and daily keys label themselves
}

/** First date in a bucket (companion to `bucketEnd`; contract addition, see plans/06-reports-engine.md). */
export function bucketStart(key: string): ISODate {
    if (DAY_RE.test(key)) return key;
    if (YEAR_RE.test(key)) return `${key}-01-01`;
    const month = MONTH_RE.exec(key);
    if (month !== null) return `${key}-01`;
    const quarter = QUARTER_RE.exec(key);
    if (quarter !== null) return toISO(Number(quarter[1]), (Number(quarter[2]) - 1) * 3 + 1, 1);
    const week = WEEK_RE.exec(key);
    if (week !== null) return toISO(...civilFromDays(isoWeekMonday(Number(week[1]), Number(week[2]))));
    throw new RangeError(`bucketStart: unrecognized bucket key "${key}"`);
}

/** Last date in a bucket (leap-aware; weekly buckets end on Sunday). */
export function bucketEnd(key: string): ISODate {
    if (DAY_RE.test(key)) return key;
    if (YEAR_RE.test(key)) return `${key}-12-31`;
    const month = MONTH_RE.exec(key);
    if (month !== null) {
        const [y, m] = [Number(month[1]), Number(month[2])];
        return toISO(y, m, daysInMonth(y, m));
    }
    const quarter = QUARTER_RE.exec(key);
    if (quarter !== null) {
        const [y, m] = [Number(quarter[1]), Number(quarter[2]) * 3];
        return toISO(y, m, daysInMonth(y, m));
    }
    const week = WEEK_RE.exec(key);
    if (week !== null) return toISO(...civilFromDays(isoWeekMonday(Number(week[1]), Number(week[2])) + 6));
    throw new RangeError(`bucketEnd: unrecognized bucket key "${key}"`);
}

/** The `n` consecutive bucket keys ending with the bucket containing `end`, oldest → newest. */
export function lastNBuckets(end: ISODate, interval: Interval, n: number): string[] {
    const out: string[] = [];
    let key = bucketKey(end, interval);
    for (let i = 0; i < n; i += 1) {
        out.push(key);
        const [y, m, d] = parts(bucketStart(key));
        key = bucketKey(toISO(...civilFromDays(daysFromCivil(y, m, d) - 1)), interval); // day before the bucket → previous bucket
    }
    return out.reverse();
}

/** Number of monthly buckets spanning `from`…`to` inclusive (min 1). "2026-01"→"2026-07" = 7. */
export function monthsBetween(from: ISODate, to: ISODate): number {
    const [fy, fm] = [Number(from.slice(0, 4)), Number(from.slice(5, 7))];
    const [ty, tm] = [Number(to.slice(0, 4)), Number(to.slice(5, 7))];
    return Math.max(1, (ty - fy) * 12 + (tm - fm) + 1);
}

/** Today's LOCAL date — the only allowed `Date` usage in date math. */
export function today(): ISODate {
    const now = new Date();
    return toISO(now.getFullYear(), now.getMonth() + 1, now.getDate());
}

/** Lexical ISO-date comparison. */
export function compareISO(a: ISODate, b: ISODate): -1 | 0 | 1 {
    return a < b ? -1 : a > b ? 1 : 0;
}
