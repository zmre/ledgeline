// Holdings report contracts (WP-10). Pure TS: no Svelte/DOM imports — ports
// to Rust later. Computed by ./engine.ts over the normalized journal.

import type {Dec, MixedAmount} from "../domain/money";
import type {ISODate} from "../domain/types";

/**
 * Gain/loss window selector. "all" = all-time gain (marketValue − basis, the
 * original behavior). "ytd"/"12mo" are WINDOWED: the report's gain/gainPct
 * become marketValue − value-at-window-start, keyed off a `gainSince` date
 * derived from the scope's asOf (see ui/gainPeriod.ts). basis stays all-time.
 */
export type GainPeriod = "all" | "ytd" | "12mo";

export interface HoldingsScope {
    /** Subtree roots, same invariant as JournalFilter. */
    accounts: ReadonlySet<string>;
    /** include + empty set = everything. */
    mode: "include" | "exclude";
    asOf: ISODate;
    /** Gain window: "all" sends no gainSince (all-time), others narrow the gain to a period. */
    gainPeriod: GainPeriod;
}

export interface Holding {
    symbol: string;
    /** `name:` tag, else symbol. */
    name: string;
    /** In-scope accounts currently holding shares; empty for a net-short row. */
    accounts: string[];
    /**
     * Non-zero by construction (a sold-out position is dropped), but NOT always
     * positive: a symbol sold without ever being bought is reported net-SHORT,
     * because the balance sheet carries and values exactly those shares. The
     * holdings page hides such rows and explains it — see
     * ui/view.ts `partitionShortPositions`.
     */
    shares: Dec;
    /** null = tainted (some lot lacks a cost, or the pool went short). */
    basis: Dec | null;
    /** Date the current position was opened (first lot since shares were last ≤ 0); null only if never bought in scope. */
    firstBasisDate: ISODate | null;
    price: {qty: Dec; date: ISODate; source: "directive" | "cost"} | null;
    /** shares × price, null when unpriced. */
    marketValue: Dec | null;
    /** marketValue − basis, null when either is missing. */
    gain: Dec | null;
    /** Display-boundary number; null when basis missing/zero. */
    gainPct: number | null;
}

export interface HoldingsWarning {
    symbol: string;
    kind: "missing-basis" | "negative-shares" | "unpriced";
    message: string;
}

export interface HoldingsReport {
    asOf: ISODate;
    base: string;
    /** shares !== 0, sorted market value desc (unpriced last, by symbol). */
    holdings: Holding[];
    /**
     * Every account that could EVER be a stock row — the scope chooser's option
     * list, sorted, computed over the whole journal with the scope and `asOf`
     * deliberately ignored so options never vanish while a scope is being
     * composed.
     *
     * From the ENGINE, and it has to be: this was once derived in the SPA as
     * "every account that ever held a non-currency commodity", which offered a
     * `holdings: other` house — booked as `1 HOME` so P directives can revalue it
     * — as a STOCKS filter, where selecting it can only ever produce an empty
     * report. Membership turns on the `holdings:` account tag, and the SPA's
     * account declarations carry a name and a type, never the tags.
     */
    accounts: string[];
    /** `marketValue` sums every priced row, net-short rows included (negatively), so it reconciles with net worth. */
    totals: {marketValue: Dec; basis: Dec | null; gain: Dec | null; gainPct: number | null};
    /** gainPct > 0 only, sorted desc, ≤ 5. */
    topGainers: Holding[];
    /** gainPct < 0 only, sorted asc, ≤ 5 (zero-gain holdings in neither list). */
    topLosers: Holding[];
    /** Scope-local, rendered inline on the page. */
    warnings: HoldingsWarning[];
}

// Other holdings (plans/14): the tracked assets that are neither a security nor
// cash — a house, a van, a partnership interest. Served by GET
// /api/holdings/other; the trend under it reuses HoldingsSeries below byte for
// byte, which is what lets HoldingsTrend.svelte render both tabs.
//
// The key is what makes this a second engine rather than a filter on the first:
// `Holding` is SYMBOL-keyed, and the stock engine drops every currency amount
// before it starts, so a house booked as `$150,000.00` produces no pool and no
// row. An other holding is ACCOUNT-keyed — the thing you own is the account, and
// its value is that account's balance.

export interface OtherHolding {
    /** Full account path. The row identity: rows are flat, one per posting-bearing account, no subtree roll-up. */
    account: string;
    /** Nearest declared `name:` tag, else the account's last segment. */
    name: string;
    /**
     * The balance AS WRITTEN — every commodity, unvalued. This is what lets the
     * table print "1 HOUSE" beside a dollar value: booking an asset as its own
     * commodity is the only way a dollar journal makes it revalue, and the unit
     * is the evidence that it can.
     */
    commodities: MixedAmount;
    /** Market value in `base` at `asOf`. null when any held commodity is unpriceable. */
    value: Dec | null;
    /** Balance at cost (`-B`), valued into `base`. */
    cost: Dec | null;
    /**
     * `value − reference`, where `reference` is `cost` for an all-time window and
     * the value recomputed at `gainSince` otherwise — the stock engine's rule
     * verbatim, so the scope bar's window control means the same thing on both
     * tabs. For a dollar-booked van, all-time change is honestly zero.
     */
    change: Dec | null;
    /** Display-boundary number; null when the reference is missing or zero. */
    changePct: number | null;
}

/**
 * The engine's totals, summed over the rows that carry the needed input and
 * NEVER recomputed in the UI. An unpriced row contributes to nothing and raises
 * a warning instead, exactly as on the Stocks tab — which is why every field but
 * `value` is nullable.
 */
export interface OtherHoldingsTotals {
    value: Dec;
    cost: Dec | null;
    change: Dec | null;
    changePct: number | null;
}

export interface OtherHoldingsWarning {
    account: string;
    /**
     * `unpriced`: the row's value has no price route, so it is excluded and
     * sorted last. `unpriced-cost`: the value is fine but the at-cost basis is
     * not, so cost/change are blank. The engine never emits both for one row.
     */
    kind: "unpriced" | "unpriced-cost";
    message: string;
}

export interface OtherHoldingsReport {
    asOf: ISODate;
    base: string;
    /** value desc, unpriced last, then by account. */
    holdings: OtherHolding[];
    /**
     * Every account that could EVER be an Other row — the scope chooser's option
     * list, sorted, computed over the whole journal with the scope and `asOf`
     * deliberately ignored so options never vanish while a scope is being
     * composed. Exactly `HoldingsReport.accounts`' contract, and disjoint from
     * it: nothing may be offered on both tabs.
     */
    accounts: string[];
    totals: OtherHoldingsTotals;
    /** Scope-local, rendered inline on the page. */
    warnings: OtherHoldingsWarning[];
}

// Holdings-over-time series (served by GET /api/holdings/series, and by
// /api/holdings/other/series in the identical shape). Kept here — the former
// client-side series.ts engine was dropped when /holdings went native.
export interface HoldingsPoint {
    /** Snapshot date: the bucket's last day, clamped so the final point never overshoots scope.asOf. */
    date: ISODate;
    /** Bucket key (e.g. "2026-07"), for axis labels. */
    bucket: string;
    /** Human bucket label (e.g. "Jul 2026"). */
    label: string;
    /** Total priced market value at `date`, in the base commodity (unpriced holdings excluded, per the honest-totals rule). */
    marketValue: Dec;
    /** Total cost basis at `date`, null when any held lot is tainted or unpriced (same refusal as HoldingsReport.totals.basis). */
    basis: Dec | null;
}

export interface HoldingsSeries {
    base: string;
    /** Oldest → newest, length = requested count. */
    points: HoldingsPoint[];
    /** True when at least one point has a non-null basis (so the UI knows whether to draw the basis line). */
    hasBasis: boolean;
}
