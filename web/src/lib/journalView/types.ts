// Domain types for the engine's journal-view routes (plan 26 B3–B5). Pure TS:
// no Svelte/DOM imports. Decoded from the wire by `api/journalViewDecode.ts`;
// the computations behind them live in `crates/ledgeline-core/src/view/`.

import type {Problem} from "#lib/checks/engine.js";
import type {RootCategory} from "#lib/domain/accounts.js";
import type {Dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, ISODate, Transaction} from "#lib/domain/types.js";
import type {ReviewCounts} from "#lib/review/types.js";

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
    /**
     * Review marker (plan 32 D20): a posting — one an `=` rule generated
     * included — is to an uncategorised account, and the transaction is not
     * `reviewed:ignore`. Exactly what puts it in Review Unknowns (bar the window).
     */
    unknown: boolean;
    /** Review marker: it carries a `reviewed` tag of any value (`ignore` included), so Recents skips it. */
    reviewed: boolean;
    /**
     * The pinned account's running balance at this row (plan 36): null when
     * nothing is pinned, absent on rows from routes that never carry one (the
     * review queue).
     */
    running?: RunningCell | null;
    /**
     * Plan 35 D11: the transaction has a half in a valid transfer link (the ⇄
     * icon) — including one in two links, which does not collapse. Absent
     * (read as false) on every other row, and on rows outside `/api/journal/rows`.
     */
    linked?: boolean;
    /**
     * Plan 35 D11: this row IS a collapsed linked pair. The row's own `txn` is
     * the anchoring half (the earliest half the filter keeps); `transfer`
     * carries both halves for the expanded view. Absent on an ordinary row.
     */
    transfer?: TransferRow;
}

/** One end of a transfer: its transaction and its real (non-clearing) account. */
export interface TransferEnd {
    /** The half's transaction index. */
    index: number;
    /** The real account on this side, or null when the half has none. */
    account: string | null;
    /** Its short label — the last segment, or the full name when both ends' last segments collide. */
    label: string | null;
}

/** A linked pair shown as one journal row (plan 35 D11): "Checking → Amex". */
export interface TransferRow {
    /** The `match` id both clearing postings carry. */
    matchId: string;
    /** Both halves as ordinary rows, earliest (date, index) first. */
    halves: readonly [JournalRow, JournalRow];
    /** The source: the half whose real posting is negative. */
    from: TransferEnd;
    to: TransferEnd;
}

/** A balance assertion's operator, as written: `=` single-commodity, `==` total, `*` inclusive of subaccounts. */
export type AssertionOp = "=" | "==" | "=*" | "==*";

/**
 * One balance assertion on a posting in the pinned subtree, as the ENGINE's
 * assertion checker judged it — never by comparing it to the Balance column,
 * which sums the whole subtree while a plain `=` on a child asserts only that
 * child.
 */
export interface RunningAssertion {
    /** The posting's account: the pinned root, or a child of it. */
    account: string;
    op: AssertionOp;
    asserted: Amount;
    /** What the checker found instead; present exactly when `ok` is false. */
    calculated: Amount | null;
    ok: boolean;
}

/**
 * The pinned subtree at one row, in one commodity (`GET /api/journal/rows?balance=`).
 *
 * The balance is computed over the WHOLE journal in hledger's order (posting
 * date, then file order), so no filter changes it — filters only choose which
 * rows are shown, exactly like `hledger reg -H`.
 */
export interface RunningCell {
    /** The subtree's net change in the chosen commodity on this row; null when the row has no posting there. */
    change: Amount | null;
    /** The subtree's balance after this row (carried from earlier when it does not touch the subtree). */
    balance: Amount;
    /** The row has a posting in the pinned subtree (in any commodity). */
    touches: boolean;
    /** The day the subtree's leg is counted on, when a `date:` tag moves it off the row's own date. */
    postedOn: ISODate | null;
    assertions: RunningAssertion[];
}

/** One commodity the pinned subtree holds, and how many postings it has there (the picker's ranking). */
export interface RunningCommodity {
    commodity: string;
    postings: number;
}

/** One balance assertion in the subtree, in evaluation (chronological) order. */
export interface RunningAssertionPoint {
    tindex: number;
    date: ISODate;
    ok: boolean;
}

/** `GET /api/journal/running?account=` — what the pin's chip, picker, "first failure" and "narrow to gap" need. */
export interface RunningInfo {
    generation: string;
    account: string;
    /** Most postings first; the first is the default. */
    commodities: RunningCommodity[];
    /** Per commodity, every assertion in the subtree in hledger's evaluation order. */
    assertions: Readonly<Record<string, readonly RunningAssertionPoint[]>>;
}

/** The grain a balance-over-time series is drawn at. */
export type SeriesInterval = "daily" | "weekly" | "monthly" | "quarterly" | "yearly";

/** A failing balance assertion inside one period of the balance line. */
export interface BalanceSeriesFailure {
    tindex: number;
    /** The day the assertion is checked on (its posting's effective date). */
    date: ISODate;
    /** The asserted account: the pinned root, or a child of it. */
    account: string;
    op: AssertionOp;
    asserted: Dec;
    calculated: Dec;
    /**
     * Where the line would be at the period's end had the books matched the
     * assertion: the point's balance shifted by `asserted − calculated`. For a
     * plain `=` on a child (which asserts the child alone) this still draws the
     * GAP from the subtree's line, so the vertical distance always reads "off by".
     */
    expected: Dec;
}

/** One period of the balance line: the subtree's balance at its (window-clamped) end. */
export interface BalanceSeriesPoint {
    /** The engine's bucket key (`2026-03`, `2026-W10`, `2026-03-02`). */
    bucket: string;
    /** The engine's label for the key ("Mar 2026", "W10 2026"). */
    label: string;
    /** The day the balance is taken at. */
    end: ISODate;
    balance: Dec;
    /** The newest transaction counted in the period (what a click scrolls to); null when nothing posted. */
    tindex: number | null;
    failures: BalanceSeriesFailure[];
}

/**
 * `GET /api/journal/running/series` — the pinned subtree's balance at the end
 * of every period of the journal filter's date range (plan 36 amendment A1).
 * Every figure is in `commodity` and displays in `style`.
 */
export interface BalanceSeries {
    generation: string;
    account: string;
    /** The commodity drawn: the requested one, or the engine's default for the subtree. */
    commodity: string;
    style: AmountStyle;
    interval: SeriesInterval;
    /** The window drawn, both ends resolved (an open start is the first posting, an open end today). */
    from: ISODate;
    to: ISODate;
    points: BalanceSeriesPoint[];
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
    /** Rows, as the journal shows them: a collapsed transfer pair counts once (plan 35 D11). */
    total: number;
    /** How many of `total` are collapsed transfer pairs; nonzero means "rows", not "transactions". 0 from an older engine. */
    transfers: number;
    /** The primary commodity's net (0 or 1 entries): the footer's "Visible Journal Total". */
    visibleNet: Amount[];
    /** Most-used first, under the account selection. */
    commodities: string[];
    /** One per entry of `commodities`, same order. */
    perCommodity: CommoditySummary[];
    /** Deepest account under the account selection; at least 1. */
    maxDepth: number;
    primary: string | null;
    /** The review queues' sizes for the toolbar badges (plan 32 D18), or null from an engine that predates them. */
    review: ReviewCounts | null;
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
