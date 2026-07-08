// Insights chart data (WP-05). Pure TS: no Svelte/DOM imports.
//
// All accumulation is exact (`Dec`); `toNumber()` only at the chart display
// boundary (PieDatum.value / LineSeries point values). `bucketKey` is a local
// pure-string-math implementation (WP-06's lib/reports/periods.ts owns the
// canonical one; reconcile when it lands — same keys: daily "YYYY-MM-DD",
// weekly = ISO date of the week's Monday, monthly "YYYY-MM").

import {accountMatches, categorize, clampAccount, type RootCategory} from "$lib/domain/accounts";
import {add, cmp, dec, formatAmount, neg, sub, toNumber, type Dec} from "$lib/domain/money";
import type {AmountStyle, ISODate, Transaction} from "$lib/domain/types";

export interface PieDatum {
    account: string;
    /** Display-sign-adjusted period total (display-boundary number); see displayQty. */
    value: number;
    /** formatAmount string for tooltips. */
    formatted: string;
}

export interface LineSeries {
    account: string;
    points: {bucket: string; value: number}[];
}

export type Interval = "daily" | "weekly" | "monthly";

/** Label used for the folded tail of small accounts (parens: not a legal hledger segment clash risk). */
export const OTHER = "(other)";

const ZERO: Dec = dec(0n, 0);

const DEFAULT_STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};

// ---------- date bucketing (pure integer/string math; never `new Date(...)`) ----------

/** Days since civil 1970-01-01 (Howard Hinnant's days_from_civil, integer math). */
function daysFromCivil(y: number, m: number, d: number): number {
    y -= m <= 2 ? 1 : 0;
    const era = Math.floor(y / 400);
    const yoe = y - era * 400;
    const doy = Math.floor((153 * (m + (m > 2 ? -3 : 9)) + 2) / 5) + d - 1;
    const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
    return era * 146097 + doe - 719468;
}

/** Inverse of daysFromCivil. */
function civilFromDays(z: number): {y: number; m: number; d: number} {
    z += 719468;
    const era = Math.floor(z / 146097);
    const doe = z - era * 146097;
    const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365);
    const y = yoe + era * 400;
    const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100));
    const mp = Math.floor((5 * doy + 2) / 153);
    const d = doy - Math.floor((153 * mp + 2) / 5) + 1;
    const m = mp + (mp < 10 ? 3 : -9);
    return {y: y + (m <= 2 ? 1 : 0), m, d};
}

function pad(n: number, width: number): string {
    return String(n).padStart(width, "0");
}

function isoFromDays(days: number): ISODate {
    const {y, m, d} = civilFromDays(days);
    return `${pad(y, 4)}-${pad(m, 2)}-${pad(d, 2)}`;
}

function daysFromIso(date: ISODate): number {
    return daysFromCivil(Number(date.slice(0, 4)), Number(date.slice(5, 7)), Number(date.slice(8, 10)));
}

/**
 * Bucket an ISO date: daily → the date itself; weekly → the ISO date of that
 * week's Monday; monthly → "YYYY-MM". Keys compare/sort lexically within an interval.
 */
export function bucketKey(date: ISODate, interval: Interval): string {
    switch (interval) {
        case "daily":
            return date;
        case "weekly": {
            const days = daysFromIso(date);
            // 1970-01-01 was a Thursday; make Monday offset 0.
            const monday = days - ((((days + 3) % 7) + 7) % 7);
            return isoFromDays(monday);
        }
        case "monthly":
            return date.slice(0, 7);
    }
}

/** The bucket immediately after `bucket` (used to zero-fill gaps in line series). */
function nextBucket(bucket: string, interval: Interval): string {
    switch (interval) {
        case "daily":
            return isoFromDays(daysFromIso(bucket) + 1);
        case "weekly":
            return isoFromDays(daysFromIso(bucket) + 7);
        case "monthly": {
            const y = Number(bucket.slice(0, 4));
            const m = Number(bucket.slice(5, 7));
            return m === 12 ? `${pad(y + 1, 4)}-01` : `${pad(y, 4)}-${pad(m + 1, 2)}`;
        }
    }
}

// ---------- shared accumulation helpers ----------

function absDec(d: Dec): Dec {
    return d.m < 0n ? neg(d) : d;
}

/**
 * Optional account selection (the filter bar's subtree roots). Insights receive
 * transactions filtered at the TXN level (a txn matches when ANY posting
 * matches), but charts/summaries must not count the txn's other legs — e.g.
 * filtering to `expenses` must not chart the checking-account side. Empty or
 * undefined = all postings.
 */
export type AccountSelection = ReadonlySet<string> | undefined;

function postingIncluded(account: string, accounts: AccountSelection): boolean {
    if (accounts === undefined || accounts.size === 0) return true;
    for (const sel of accounts) {
        if (accountMatches(sel, account)) return true;
    }
    return false;
}

/**
 * Detect the journal's expense sign convention: hledger's standard makes
 * expense postings positive (debits), but real-world journals (typically CSV
 * imports keeping the bank statement's sign) record spending as negative.
 * Majority sign of the included expense postings wins (ties → standard), so a
 * genuinely refund-dominated period under the standard convention still nets
 * negative rather than being silently flipped.
 */
export function expenseSignFactor(txns: Transaction[], commodity: string, accounts?: AccountSelection): 1 | -1 {
    let positive = 0;
    let negative = 0;
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (categorize(posting.account) !== "expense" || !postingIncluded(posting.account, accounts)) continue;
            for (const amount of posting.amounts) {
                if (amount.commodity !== commodity) continue;
                if (amount.qty.m > 0n) positive += 1;
                else if (amount.qty.m < 0n) negative += 1;
            }
        }
    }
    return negative > positive ? -1 : 1;
}

/**
 * Display sign for a posting amount: revenue flips (hledger revenue postings
 * are negative; money-in charts positive), expenses flip only when the journal
 * records spending as negative (see expenseSignFactor), everything else is raw.
 */
function displayQty(qty: Dec, category: RootCategory, expenseSign: 1 | -1): Dec {
    if (category === "revenue") return neg(qty);
    if (category === "expense" && expenseSign === -1) return neg(qty);
    return qty;
}

/**
 * Accounts (clamped to `depth`) that have postings in `commodity`, ranked by
 * total absolute posting volume (descending; ties alphabetical). Both pie and
 * line rank from this list so an account keeps the same color in either mode.
 */
export function rankedAccounts(txns: Transaction[], depth: number, commodity: string, accounts?: AccountSelection): string[] {
    const magnitude = new Map<string, Dec>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (!postingIncluded(posting.account, accounts)) continue;
            for (const amount of posting.amounts) {
                if (amount.commodity !== commodity) continue;
                const account = clampAccount(posting.account, depth);
                magnitude.set(account, add(magnitude.get(account) ?? ZERO, absDec(amount.qty)));
            }
        }
    }
    return [...magnitude.entries()].sort(([aName, aMag], [bName, bMag]) => cmp(bMag, aMag) || (aName < bName ? -1 : 1)).map(([name]) => name);
}

/** Fold ranked accounts into at most `max` groups: top max-1 keep their name, the rest map to OTHER. */
function foldTail(ranked: string[], max: number): Map<string, string> {
    const out = new Map<string, string>();
    const keep = ranked.length > max ? max - 1 : ranked.length;
    ranked.forEach((account, i) => out.set(account, i < keep ? account : OTHER));
    return out;
}

/** Display style for a commodity: the first style seen on a matching posting amount. */
export function styleFor(txns: Transaction[], commodity: string): AmountStyle {
    for (const txn of txns) {
        for (const posting of txn.postings) {
            for (const amount of posting.amounts) {
                if (amount.commodity === commodity) return amount.style;
            }
        }
    }
    return DEFAULT_STYLE;
}

/** Deepest account name (segment count) among postings matching `accounts`; ≥ 1 for non-empty input. */
export function maxAccountDepth(txns: Transaction[], accounts?: AccountSelection): number {
    let max = 1;
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (!postingIncluded(posting.account, accounts)) continue;
            const depth = posting.account.split(":").length;
            if (depth > max) max = depth;
        }
    }
    return max;
}

// ---------- contract functions ----------

/**
 * Period totals per account clamped to `depth`, one commodity, ranked by
 * magnitude with the tail folded into OTHER (`maxSlices` groups at most,
 * default 6). Only postings matching `accounts` contribute. Values are signed
 * after display adjustment (revenue money-in positive, expenses spending
 * positive per detected convention); zero-total accounts are dropped.
 */
export function pieData(txns: Transaction[], opts: {depth: number; commodity: string; maxSlices?: number; accounts?: AccountSelection}): PieDatum[] {
    const {depth, commodity, maxSlices = 6, accounts} = opts;
    const ranked = rankedAccounts(txns, depth, commodity, accounts);
    const groupOf = foldTail(ranked, Math.max(1, maxSlices));
    const expenseSign = expenseSignFactor(txns, commodity, accounts);
    const totals = new Map<string, Dec>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (!postingIncluded(posting.account, accounts)) continue;
            const category = categorize(posting.account);
            for (const amount of posting.amounts) {
                if (amount.commodity !== commodity) continue;
                const group = groupOf.get(clampAccount(posting.account, depth)) ?? OTHER;
                totals.set(group, add(totals.get(group) ?? ZERO, displayQty(amount.qty, category, expenseSign)));
            }
        }
    }
    const style = styleFor(txns, commodity);
    const order = [...new Set(ranked.map((account) => groupOf.get(account) ?? OTHER))];
    const out: PieDatum[] = [];
    for (const account of order) {
        const total = totals.get(account);
        if (total === undefined || total.m === 0n) continue;
        out.push({account, value: toNumber(total), formatted: formatAmount({commodity, qty: total, style})});
    }
    return out;
}

/**
 * Activity per bucket per account (clamped to `depth`, one commodity), top
 * `maxSeries` (default 6) groups by magnitude with the tail folded into OTHER.
 * Only postings matching `accounts` contribute; values are display-sign
 * adjusted like pieData. Every series carries the full bucket range (gaps
 * zero-filled) so lines are continuous.
 */
export function lineData(
    txns: Transaction[],
    opts: {depth: number; commodity: string; interval: Interval; maxSeries?: number; accounts?: AccountSelection}
): LineSeries[] {
    const {depth, commodity, interval, maxSeries = 6, accounts} = opts;
    const ranked = rankedAccounts(txns, depth, commodity, accounts);
    const groupOf = foldTail(ranked, Math.max(1, maxSeries));
    const expenseSign = expenseSignFactor(txns, commodity, accounts);
    const sums = new Map<string, Map<string, Dec>>();
    let minBucket: string | null = null;
    let maxBucket: string | null = null;
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (!postingIncluded(posting.account, accounts)) continue;
            const category = categorize(posting.account);
            for (const amount of posting.amounts) {
                if (amount.commodity !== commodity) continue;
                const group = groupOf.get(clampAccount(posting.account, depth)) ?? OTHER;
                const bucket = bucketKey(posting.date ?? txn.date, interval);
                if (minBucket === null || bucket < minBucket) minBucket = bucket;
                if (maxBucket === null || bucket > maxBucket) maxBucket = bucket;
                let perBucket = sums.get(group);
                if (perBucket === undefined) {
                    perBucket = new Map();
                    sums.set(group, perBucket);
                }
                perBucket.set(bucket, add(perBucket.get(bucket) ?? ZERO, displayQty(amount.qty, category, expenseSign)));
            }
        }
    }
    if (minBucket === null || maxBucket === null) return [];
    const buckets: string[] = [];
    for (let b = minBucket; b <= maxBucket; b = nextBucket(b, interval)) buckets.push(b);
    const order = [...new Set(ranked.map((account) => groupOf.get(account) ?? OTHER))];
    return order
        .filter((account) => sums.has(account))
        .map((account) => {
            const perBucket = sums.get(account) as Map<string, Dec>;
            return {account, points: buckets.map((bucket) => ({bucket, value: toNumber(perBucket.get(bucket) ?? ZERO)}))};
        });
}

/**
 * Income / Expenses / Net for the given (already filtered) transactions, one
 * commodity, counting only postings that match `accounts`. Sign-adjusted for
 * display: income is positive when money came in (hledger revenue postings are
 * negative), and expenses are positive when money was spent — including in
 * journals that record spending as negative (see expenseSignFactor).
 * net = income - expenses.
 */
export function bigNumbers(txns: Transaction[], commodity: string, accounts?: AccountSelection): {income: Dec; expenses: Dec; net: Dec} {
    const expenseSign = expenseSignFactor(txns, commodity, accounts);
    let revenue = ZERO;
    let expenses = ZERO;
    for (const txn of txns) {
        for (const posting of txn.postings) {
            const category = categorize(posting.account);
            if (category !== "revenue" && category !== "expense") continue;
            if (!postingIncluded(posting.account, accounts)) continue;
            for (const amount of posting.amounts) {
                if (amount.commodity !== commodity) continue;
                if (category === "revenue") revenue = add(revenue, amount.qty);
                else expenses = add(expenses, displayQty(amount.qty, category, expenseSign));
            }
        }
    }
    const income = neg(revenue);
    return {income, expenses, net: sub(income, expenses)};
}

/**
 * Format a display-boundary number (a chart value that already went through
 * `toNumber`) back into the commodity's display style, e.g. for axis ticks and
 * line tooltips. Exact Dec strings (PieDatum.formatted, big numbers) are still
 * preferred wherever the Dec is available.
 */
export function formatChartValue(value: number, commodity: string, style: AmountStyle): string {
    const scaled = Math.round(value * 10 ** style.precision);
    if (!Number.isSafeInteger(scaled)) return `${value} ${commodity}`; // out of exact range; charts never get here in practice
    return formatAmount({commodity, qty: dec(scaled, style.precision), style});
}

/** Commodities appearing in posting amounts matching `accounts`, most-used first (ties alphabetical). */
export function commoditiesInUse(txns: Transaction[], accounts?: AccountSelection): string[] {
    const counts = new Map<string, number>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (!postingIncluded(posting.account, accounts)) continue;
            for (const amount of posting.amounts) {
                counts.set(amount.commodity, (counts.get(amount.commodity) ?? 0) + 1);
            }
        }
    }
    return [...counts.entries()].sort(([aName, aCount], [bName, bCount]) => bCount - aCount || (aName < bName ? -1 : 1)).map(([name]) => name);
}
