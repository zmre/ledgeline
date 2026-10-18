// Domain types for the engine's journal-view routes (plan 26 B3–B5). Pure TS:
// no Svelte/DOM imports. Decoded from the wire by `api/journalViewDecode.ts`;
// the computations behind them live in `crates/ledgeline-core/src/view/`.

import type {Problem} from "#lib/checks/engine.js";
import type {RootCategory} from "#lib/domain/accounts.js";
import type {Dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, ISODate, Transaction} from "#lib/domain/types.js";

/** One row of the Balances view: one account, one commodity. */
export interface BalanceRow {
    /** Full colon path, exactly as posted to — never rolled up into parents. */
    account: string;
    qty: Dec;
    /** Effective date of the account's most recent counted posting. */
    asOf: ISODate;
    /** A posting on that date carried a balance assertion. */
    confirmed: boolean;
}

/** The Balances view for one commodity: two lists and the three headline figures. */
export interface BalanceSummary {
    commodity: string;
    /** The style every figure in this commodity displays with. */
    style: AmountStyle;
    /** Cash accounts, largest magnitude first (an overdrawn account sorts by magnitude too). */
    cash: BalanceRow[];
    /** Liability accounts, largest debt first. */
    liabilities: BalanceRow[];
    totalCash: Dec;
    /** Negative when money is owed, matching how the rows read. */
    totalLiabilities: Dec;
    /** totalCash + totalLiabilities. */
    net: Dec;
}

/** `GET /api/balances/cash?asOf=` — every commodity's summary, so the currency selector costs no request. */
export interface CashBalancesView {
    asOf: ISODate;
    /** Most-used first: the selector's order, and the first is the default. */
    commodities: string[];
    /** One per entry of `commodities`, same order. */
    summaries: BalanceSummary[];
    /** Accounts left out because a word in their NAME looked long-term (never a tag), sorted. */
    guessedLongTerm: string[];
}

/** One row of the windowed journal: the domain transaction plus its check findings. */
export interface JournalRow {
    txn: Transaction;
    /** The findings anchored to this transaction (as of the client's today), for the row's flag dot. */
    flags: Problem[];
}

/** `GET /api/journal/rows` — one window of the filtered journal. */
export interface RowsPage {
    /** The snapshot the rows came from (`/api/journal`'s `generation`). */
    generation: string;
    /** Every transaction the filter keeps, not just this window. */
    total: number;
    offset: number;
    rows: JournalRow[];
}

/** Income / expenses / net and the chart inputs for one commodity of the filtered view. */
export interface CommoditySummary {
    commodity: string;
    style: AmountStyle;
    /** Sign-adjusted: positive when money came in. */
    income: Dec;
    /** Sign-adjusted: positive when money was spent. */
    expenses: Dec;
    /** income − expenses. */
    net: Dec;
    /** Root categories with a posting in this commodity, expenses first. */
    categories: RootCategory[];
}

/** `GET /api/journal/summary` — the footer, the big numbers and the chart selectors for one filter. */
export interface JournalSummary {
    generation: string;
    total: number;
    /** The primary commodity's net (0 or 1 entries): the footer's "Visible Journal Total". */
    visibleNet: Amount[];
    /** Most-used first, under the account selection. */
    commodities: string[];
    /** One per entry of `commodities`, same order. */
    perCommodity: CommoditySummary[];
    /** Deepest account under the account selection; at least 1. */
    maxDepth: number;
    primary: string | null;
}

/** One pie slice: a display-boundary number and the exact amount, formatted. */
export interface PieDatum {
    account: string;
    value: number;
    formatted: string;
}

/** One group's activity per bucket, every bucket of the range present. */
export interface LineSeries {
    account: string;
    points: {bucket: string; value: number}[];
}

/** `GET /api/journal/chart`. Exactly one of `pie` and `line` is present: the mode asked for. */
export interface ChartData {
    generation: string;
    commodity: string;
    /** Groups in rank order (folded to the request's max) — the colour slots. */
    groups: string[];
    pie: PieDatum[] | null;
    line: LineSeries[] | null;
}

/** `GET /api/journal/position` — where a transaction sits in the filtered view. */
export interface RowPosition {
    generation: string;
    /** 0-based row, or null when the filter hides it (or it does not exist). */
    position: number | null;
    /** The transaction's date (to widen a date filter), or null when it does not exist. */
    date: ISODate | null;
    total: number;
}
