// Insights chart data (WP-05). Pure TS: no Svelte/DOM imports.
//
// All accumulation is exact (`Dec`); `toNumber()` only at the chart display
// boundary (PieDatum.value / LineSeries point values).
//
// Bucket math comes from `lib/reports/periods.ts`, the canonical date engine.
// This module used to carry its own `daysFromCivil`/`civilFromDays`/`bucketKey`
// against a note promising to "reconcile when WP-06 lands"; it landed, and the
// two had drifted — the local weekly key was the ISO date of the week's Monday
// ("2026-07-06") where the canonical one is the ISO-8601 week ("2026-W28").
// The canonical form won (DRY-2): it is unambiguous on a chart axis, where a
// Monday date is indistinguishable from a daily bucket, and it keeps this file
// agreeing with the Rust engine's `reports::periods`.

import {accountMatches, categorize, clampAccount, type RootCategory} from "$lib/domain/accounts";
import {resolveAccountType, type AccountType} from "$lib/domain/accountTypes";
import {add, cmp, dec, formatAmount, neg, sub, toNumber, type Dec} from "$lib/domain/money";
import type {Amount, AmountStyle, Transaction} from "$lib/domain/types";
import {absDec, DEFAULT_AMOUNT_STYLE, ZERO} from "$lib/format/amounts";
import {OTHER_LABEL} from "$lib/format/palette";
import {bucketKey, nextBucket, type Interval as PeriodsInterval} from "$lib/reports/periods";

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

/**
 * The intervals the journal chart offers: a deliberate NARROWING of the
 * canonical `periods.Interval` (no quarterly/yearly), so picking one that
 * `ChartWidget`'s selector does not offer is a type error. Derived via `Extract`
 * rather than restated, so renaming an interval upstream breaks this loudly
 * instead of silently re-forking the pair.
 */
export type Interval = Extract<PeriodsInterval, "daily" | "weekly" | "monthly">;

/**
 * Label used for the folded tail of small accounts (parens: not a legal hledger
 * segment clash risk). One literal, shared with the holdings pie — see
 * `$lib/format/palette`, which also owns the muted colour it is drawn in.
 */
export const OTHER = OTHER_LABEL;

// NOTE the behaviour change: this module's own default style used
// `digitGroups: null`, so a commodity the journal feed gave no style for
// charted as `1234.56` while every other surface rendered `1,234.56`. The
// shared default groups.
const DEFAULT_STYLE = DEFAULT_AMOUNT_STYLE;

/**
 * Optional account selection (the filter bar's subtree roots). Insights receive
 * transactions filtered at the TXN level (a txn matches when ANY posting
 * matches), but charts/summaries must not count the txn's other legs — e.g.
 * filtering to `expenses` must not chart the checking-account side. Empty or
 * undefined = all postings.
 */
export type AccountSelection = ReadonlySet<string> | undefined;

/** Declared account types; absent/empty means "classify by name" (the old behaviour). */
export type DeclaredTypes = ReadonlyMap<string, AccountType> | undefined;

/** `AccountType` → the coarser `RootCategory` these charts group by. */
const TYPE_TO_CATEGORY: Record<AccountType, RootCategory> = {
    asset: "asset",
    cash: "asset", // a subtype of asset, not a category of its own
    liability: "liability",
    equity: "equity",
    conversion: "equity",
    revenue: "revenue",
    expense: "expense",
};

/**
 * An account's category, preferring its DECLARED type over its name.
 *
 * Names are only a fallback: a chart of accounts that books costs under `cogs:`
 * — or in a language other than English — is `other` to the name heuristic, so
 * the income/expense tiles and every category-scoped chart would read zero.
 */
export function categoryOf(account: string, declared: DeclaredTypes): RootCategory {
    if (declared !== undefined && declared.size > 0) {
        const type = resolveAccountType(account, declared);
        if (type !== null) return TYPE_TO_CATEGORY[type];
    }
    return categorize(account);
}

function postingIncluded(account: string, accounts: AccountSelection, category?: RootCategory, declared?: DeclaredTypes): boolean {
    if (category !== undefined && categoryOf(account, declared) !== category) return false;
    if (accounts === undefined || accounts.size === 0) return true;
    for (const sel of accounts) {
        if (accountMatches(sel, account)) return true;
    }
    return false;
}

export type SignFactor = 1 | -1;
export interface SignConventions {
    revenue: SignFactor;
    expense: SignFactor;
}

/**
 * Detect the journal's sign conventions for revenue and expense postings, per
 * commodity. Display rule: each category shows with the sign that makes its
 * DOMINANT money flow positive — income displays positive when money came in,
 * expenses display positive when money was spent. hledger's standard records
 * revenue negative / expenses positive, but real-world journals (typically CSV
 * imports keeping the bank statement's sign) invert one or both. Dominance is
 * magnitude-weighted (total |negative| vs total |positive|, ties → positive
 * dominant, i.e. don't flip), so a few large outliers or refunds can't fool a
 * count-based majority. Pass the WHOLE journal, not the filtered period, so
 * the detected convention is stable across filter changes.
 */
export function signConventions(txns: Transaction[], commodity: string, declared?: DeclaredTypes): SignConventions {
    const flows = {
        revenue: {pos: ZERO, neg: ZERO},
        expense: {pos: ZERO, neg: ZERO},
    };
    for (const txn of txns) {
        for (const posting of txn.postings) {
            const category = categoryOf(posting.account, declared);
            if (category !== "revenue" && category !== "expense") continue;
            for (const amount of posting.amounts) {
                if (amount.commodity !== commodity) continue;
                const flow = flows[category];
                if (amount.qty.m > 0n) flow.pos = add(flow.pos, amount.qty);
                else if (amount.qty.m < 0n) flow.neg = add(flow.neg, absDec(amount.qty));
            }
        }
    }
    const factor = (flow: {pos: Dec; neg: Dec}): SignFactor => (cmp(flow.neg, flow.pos) > 0 ? -1 : 1);
    return {revenue: factor(flows.revenue), expense: factor(flows.expense)};
}

/** Display sign for a posting amount per the detected conventions; non-revenue/expense categories are raw. */
function displayQty(qty: Dec, category: RootCategory, signs: SignConventions): Dec {
    if (category === "revenue") return signs.revenue === -1 ? neg(qty) : qty;
    if (category === "expense") return signs.expense === -1 ? neg(qty) : qty;
    return qty;
}

/**
 * Accounts (clamped to `depth`) that have postings in `commodity`, ranked by
 * total absolute posting volume (descending; ties alphabetical). Both pie and
 * line rank from this list so an account keeps the same color in either mode.
 */
export function rankedAccounts(
    txns: Transaction[],
    depth: number,
    commodity: string,
    accounts?: AccountSelection,
    category?: RootCategory,
    declared?: DeclaredTypes
): string[] {
    const magnitude = new Map<string, Dec>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (!postingIncluded(posting.account, accounts, category, declared)) continue;
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
 * positive per detected convention — pass the unfiltered journal as
 * `conventionTxns` for a stable convention); zero-total accounts are dropped.
 */
export function pieData(
    txns: Transaction[],
    opts: {
        depth: number;
        commodity: string;
        maxSlices?: number;
        accounts?: AccountSelection;
        conventionTxns?: Transaction[];
        category?: RootCategory;
        declared?: DeclaredTypes;
    }
): PieDatum[] {
    const {depth, commodity, maxSlices = 6, accounts, conventionTxns, category: categoryScope, declared} = opts;
    const ranked = rankedAccounts(txns, depth, commodity, accounts, categoryScope, declared);
    const groupOf = foldTail(ranked, Math.max(1, maxSlices));
    const signs = signConventions(conventionTxns ?? txns, commodity, declared);
    const totals = new Map<string, Dec>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (!postingIncluded(posting.account, accounts, categoryScope, declared)) continue;
            const category = categoryOf(posting.account, declared);
            for (const amount of posting.amounts) {
                if (amount.commodity !== commodity) continue;
                const group = groupOf.get(clampAccount(posting.account, depth)) ?? OTHER;
                totals.set(group, add(totals.get(group) ?? ZERO, displayQty(amount.qty, category, signs)));
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
    opts: {
        depth: number;
        commodity: string;
        interval: Interval;
        maxSeries?: number;
        accounts?: AccountSelection;
        conventionTxns?: Transaction[];
        category?: RootCategory;
        declared?: DeclaredTypes;
    }
): LineSeries[] {
    const {depth, commodity, interval, maxSeries = 6, accounts, conventionTxns, category: categoryScope, declared} = opts;
    const ranked = rankedAccounts(txns, depth, commodity, accounts, categoryScope, declared);
    const groupOf = foldTail(ranked, Math.max(1, maxSeries));
    const signs = signConventions(conventionTxns ?? txns, commodity, declared);
    const sums = new Map<string, Map<string, Dec>>();
    let minBucket: string | null = null;
    let maxBucket: string | null = null;
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (!postingIncluded(posting.account, accounts, categoryScope, declared)) continue;
            const category = categoryOf(posting.account, declared);
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
                perBucket.set(bucket, add(perBucket.get(bucket) ?? ZERO, displayQty(amount.qty, category, signs)));
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
 * commodity, counting only postings that match `accounts`. Sign-adjusted per
 * the detected conventions (see signConventions — pass the unfiltered journal
 * as `conventionTxns` for stability): income displays positive when money came
 * in, expenses display positive when money was spent, whichever raw sign the
 * journal records them with. net = income - expenses.
 */
export function bigNumbers(
    txns: Transaction[],
    commodity: string,
    accounts?: AccountSelection,
    conventionTxns?: Transaction[],
    declared?: DeclaredTypes
): {income: Dec; expenses: Dec; net: Dec} {
    const signs = signConventions(conventionTxns ?? txns, commodity, declared);
    let income = ZERO;
    let expenses = ZERO;
    for (const txn of txns) {
        for (const posting of txn.postings) {
            const category = categoryOf(posting.account, declared);
            if (category !== "revenue" && category !== "expense") continue;
            if (!postingIncluded(posting.account, accounts)) continue;
            for (const amount of posting.amounts) {
                if (amount.commodity !== commodity) continue;
                if (category === "revenue") income = add(income, displayQty(amount.qty, category, signs));
                else expenses = add(expenses, displayQty(amount.qty, category, signs));
            }
        }
    }
    return {income, expenses, net: sub(income, expenses)};
}

/**
 * The journal footer's "Visible Journal Total": the net (income − expenses) of
 * the filtered transactions for the PRIMARY commodity — the most-used one in the
 * filtered view, the same commodity the insights "Net" shows. Reuses bigNumbers,
 * so expenses pull the total negative, revenue pulls it positive, and equal
 * refunds/reimbursements offset to nothing (see signConventions). Only postings
 * matching `accounts` count. Returned as an array (0 or 1 element) so the footer
 * renders it uniformly: empty when the view has no postings or the net is exactly
 * zero. Scoping to one commodity keeps the cost a fixed number of journal scans
 * instead of one per commodity. Pass the whole journal as `conventionTxns` for a
 * sign convention that stays stable across filter changes.
 */
export function visibleNet(txns: Transaction[], accounts?: AccountSelection, conventionTxns?: Transaction[], declared?: DeclaredTypes): Amount[] {
    const commodity = commoditiesInUse(txns, accounts)[0];
    if (commodity === undefined) return [];
    const qty = bigNumbers(txns, commodity, accounts, conventionTxns, declared).net;
    return qty.m === 0n ? [] : [{commodity, qty, style: styleFor(txns, commodity)}];
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

/** Magnitude → abbreviation, largest unit first (K = 10^3 … T = 10^12). */
const COMPACT_UNITS: [divisor: number, suffix: string][] = [
    [1e3, "K"],
    [1e6, "M"],
    [1e9, "B"],
    [1e12, "T"],
];

/** Abbreviate a non-negative magnitude: >=1000 → mantissa (~1 decimal) + K/M/B/T, else a rounded integer with no suffix. */
function compactParts(abs: number): {mantissa: string; suffix: string} {
    if (abs < 1e3) return {mantissa: String(Math.round(abs)), suffix: ""};
    let i = 0;
    while (i + 1 < COMPACT_UNITS.length && abs >= COMPACT_UNITS[i + 1][0]) i += 1;
    let rounded = Number((abs / COMPACT_UNITS[i][0]).toFixed(1));
    // Boundary carry: 999_999 rounds to "1000.0K" → promote to the next unit ("1.0M").
    if (rounded >= 1000 && i + 1 < COMPACT_UNITS.length) {
        i += 1;
        rounded = Number((abs / COMPACT_UNITS[i][0]).toFixed(1));
    }
    return {mantissa: rounded.toFixed(1), suffix: COMPACT_UNITS[i][1]};
}

/**
 * Compact axis-tick variant of formatChartValue: abbreviates magnitude with a
 * K/M/B/T suffix at ~1 decimal so left-axis ticks stay short and never clip
 * (1234 → "$1.2K", 5_269_875 → "$5.3M", 1e9 → "$1.0B"). Zero → "$0";
 * sub-thousand values render as a plain rounded amount ("$500"). Sign and
 * commodity placement follow the style exactly like formatAmount ("$-1.2K").
 * DISPLAY-ONLY and lossy by design — tooltips/hover keep full precision via
 * formatChartValue; only axis ticks use this.
 */
export function formatCompactChartValue(value: number, commodity: string, style: AmountStyle): string {
    const negative = value < 0;
    const {mantissa, suffix} = compactParts(Math.abs(value));
    const point = style.decimalPoint === "." ? mantissa : mantissa.replace(".", style.decimalPoint);
    const num = (negative ? "-" : "") + point + suffix;
    if (commodity === "") return num;
    const space = style.spaced ? " " : "";
    return style.side === "L" ? commodity + space + num : num + space + commodity;
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

/** Preferred display order for the chart's category scope selector; expenses lead (the most useful journal view). */
const CATEGORY_ORDER: RootCategory[] = ["expense", "revenue", "asset", "liability", "equity", "other"];

/**
 * Root categories with at least one `commodity` posting matching `accounts`, in
 * a stable display order (expenses first). Drives the ChartWidget's category
 * scope selector; the default scope prefers "expense" when present.
 */
export function categoriesInUse(txns: Transaction[], commodity: string, accounts?: AccountSelection, declared?: DeclaredTypes): RootCategory[] {
    const present = new Set<RootCategory>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            if (!postingIncluded(posting.account, accounts)) continue;
            for (const amount of posting.amounts) {
                if (amount.commodity !== commodity) continue;
                present.add(categoryOf(posting.account, declared));
                break;
            }
        }
    }
    return CATEGORY_ORDER.filter((c) => present.has(c));
}
