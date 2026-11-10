// Wire → domain decoders for the engine's journal-view routes (plan 26 B3–B5).
//
// Same discipline as nativeDecode.ts: every field is checked, a missing or
// mistyped one throws `ApiShapeError` rather than rendering as a quiet zero,
// and decoded objects are frozen. Exact decimals travel as `{m: "<mantissa>",
// p: places}` — the mantissa a STRING, so no digit is lost past 2^53.

import type {Problem, Severity} from "#lib/checks/engine.js";
import type {RootCategory} from "#lib/domain/accounts.js";
import type {Dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, Posting, PostingType, Transaction, TxnStatus} from "#lib/domain/types.js";
import type {
    BalanceRow,
    BalanceSummary,
    CashBalancesView,
    ChartData,
    CommoditySummary,
    JournalRow,
    JournalSummary,
    LineSeries,
    PieDatum,
    RowPosition,
    RowsPage,
    AssertionOp,
    BalanceSeries,
    BalanceSeriesFailure,
    BalanceSeriesPoint,
    RunningAssertion,
    RunningAssertionPoint,
    RunningCell,
    RunningCommodity,
    RunningInfo,
    SeriesInterval,
    TransferEnd,
    TransferRow,
} from "#lib/journalView/types.js";
import type {ReviewCounts} from "#lib/review/types.js";
import {ApiShapeError} from "./client";

type Raw = Record<string, unknown>;

export function obj(raw: unknown, context: string): Raw {
    if (typeof raw !== "object" || raw === null || Array.isArray(raw)) throw new ApiShapeError(`${context}: expected an object`);
    return raw as Raw;
}

export function arr(raw: unknown, context: string): unknown[] {
    if (!Array.isArray(raw)) throw new ApiShapeError(`${context}: expected an array`);
    return raw;
}

export function str(raw: unknown, context: string): string {
    if (typeof raw !== "string") throw new ApiShapeError(`${context}: expected a string`);
    return raw;
}

export function num(raw: unknown, context: string): number {
    if (typeof raw !== "number" || !Number.isFinite(raw)) throw new ApiShapeError(`${context}: expected a number`);
    return raw;
}

export function bool(raw: unknown, context: string): boolean {
    if (typeof raw !== "boolean") throw new ApiShapeError(`${context}: expected a boolean`);
    return raw;
}

export function strings(raw: unknown, context: string): string[] {
    return Object.freeze(arr(raw, context).map((item, i) => str(item, `${context}[${i}]`))) as string[];
}

/** `{m, p}` → a frozen Dec. */
export function decodeViewDec(raw: unknown, context: string): Dec {
    const value = obj(raw, context);
    const m = str(value.m, `${context}.m`);
    if (!/^-?\d+$/.test(m)) throw new ApiShapeError(`${context}: mantissa ${JSON.stringify(m)} is not an integer`);
    const p = num(value.p, `${context}.p`);
    if (!Number.isSafeInteger(p) || p < 0) throw new ApiShapeError(`${context}: invalid places ${p}`);
    return Object.freeze({m: BigInt(m), p});
}

/** The SPA-shaped style the engine serves: `{side, spaced, precision, decimalPoint, digitGroups}`. */
export function decodeViewStyle(raw: unknown, context: string): AmountStyle {
    const style = obj(raw, context);
    const side = str(style.side, `${context}.side`);
    if (side !== "L" && side !== "R") throw new ApiShapeError(`${context}: side ${JSON.stringify(side)}`);
    let digitGroups: [string, number[]] | null = null;
    if (style.digitGroups !== null) {
        const pair = arr(style.digitGroups, `${context}.digitGroups`);
        const sizes = arr(pair[1], `${context}.digitGroups[1]`).map((size, i) => num(size, `${context}.digitGroups[1][${i}]`));
        digitGroups = Object.freeze([str(pair[0], `${context}.digitGroups[0]`), Object.freeze(sizes) as number[]]) as [string, number[]];
    }
    return Object.freeze({
        side,
        spaced: bool(style.spaced, `${context}.spaced`),
        precision: num(style.precision, `${context}.precision`),
        decimalPoint: str(style.decimalPoint, `${context}.decimalPoint`),
        digitGroups,
    });
}

function decodeBalanceRow(raw: unknown, context: string): BalanceRow {
    const row = obj(raw, context);
    return Object.freeze({
        account: str(row.account, `${context}.account`),
        qty: decodeViewDec(row.qty, `${context}.qty`),
        asOf: str(row.asOf, `${context}.asOf`),
        confirmed: bool(row.confirmed, `${context}.confirmed`),
    });
}

function decodeBalanceSummary(raw: unknown, context: string): BalanceSummary {
    const summary = obj(raw, context);
    const rows = (field: string): BalanceRow[] =>
        Object.freeze(arr(summary[field], `${context}.${field}`).map((row, i) => decodeBalanceRow(row, `${context}.${field}[${i}]`))) as BalanceRow[];
    return Object.freeze({
        commodity: str(summary.commodity, `${context}.commodity`),
        style: decodeViewStyle(summary.style, `${context}.style`),
        cash: rows("cash"),
        liabilities: rows("liabilities"),
        totalCash: decodeViewDec(summary.totalCash, `${context}.totalCash`),
        totalLiabilities: decodeViewDec(summary.totalLiabilities, `${context}.totalLiabilities`),
        net: decodeViewDec(summary.net, `${context}.net`),
    });
}

/** `GET /api/balances/cash` → the Balances view. */
export function decodeCashBalances(raw: unknown): CashBalancesView {
    const view = obj(raw, "cash balances");
    const commodities = strings(view.commodities, "cash balances commodities");
    const summaries = arr(view.summaries, "cash balances summaries").map((summary, i) => decodeBalanceSummary(summary, `cash balances summaries[${i}]`));
    if (summaries.length !== commodities.length) throw new ApiShapeError("cash balances: one summary per commodity expected");
    return Object.freeze({
        asOf: str(view.asOf, "cash balances asOf"),
        commodities,
        summaries: Object.freeze(summaries) as BalanceSummary[],
        guessedLongTerm: strings(view.guessedLongTerm, "cash balances guessedLongTerm"),
    });
}

// ---------------------------------------------------------------------------
// `/api/diagnostics` — every problem the badge counts and the drawer lists
// ---------------------------------------------------------------------------

/** What `/api/diagnostics` answers: the whole list's size and worst severity, plus (up to `limit`) the entries. */
export interface DiagnosticsPayload {
    /** Every finding, even when `problems` was truncated by a `limit`. */
    count: number;
    maxSeverity: Severity | null;
    problems: Problem[];
}

const SEVERITIES: ReadonlySet<string> = new Set<Severity>(["error", "warning", "info"]);

/**
 * One wire finding → a `Problem`, or null when the entry is unusable.
 *
 * Deliberately does NOT throw, unlike the decoders above: these are advisory
 * findings about a journal the engine opened, so a junk entry must cost that
 * ONE finding, never the badge. An entry needs a known severity, a non-blank
 * message, and exactly one usable anchor — a positive integer `tindex`, or a
 * non-blank `account`. A junk `tindex` is refused outright rather than quietly
 * demoted to an account anchor.
 *
 * There is no RULE allow-list any more: the engine is the only producer of
 * every rule (plan 26 B4), and the drawer labels an unknown one by its id.
 */
function decodeProblem(raw: unknown): Problem | null {
    if (typeof raw !== "object" || raw === null) return null;
    const entry = raw as Raw;
    if (typeof entry.rule !== "string" || entry.rule.trim() === "") return null;
    if (typeof entry.severity !== "string" || !SEVERITIES.has(entry.severity)) return null;
    if (typeof entry.message !== "string" || entry.message.trim() === "") return null;
    const severity = entry.severity as Severity;
    if (entry.tindex !== undefined) {
        if (!Number.isSafeInteger(entry.tindex) || (entry.tindex as number) < 1) return null;
        const problem: Problem = {txnIndex: entry.tindex as number, rule: entry.rule, severity, message: entry.message};
        if (typeof entry.date === "string") problem.date = entry.date;
        if (typeof entry.description === "string") problem.description = entry.description;
        return Object.freeze(problem);
    }
    if (typeof entry.account !== "string" || entry.account.trim() === "") return null;
    return Object.freeze({txnIndex: null, account: entry.account, rule: entry.rule, severity, message: entry.message});
}

/**
 * `GET /api/diagnostics` → the problem list. The envelope is checked strictly
 * (a missing `count` means this is not the engine answering); entries are
 * decoded leniently (see `decodeProblem`), and exact duplicates — same anchor,
 * rule and message — collapse, because the drawer keys its rows on them.
 */
export function decodeDiagnostics(raw: unknown): DiagnosticsPayload {
    const body = obj(raw, "diagnostics");
    const count = num(body.count, "diagnostics count");
    const worst = body.maxSeverity;
    if (worst !== null && (typeof worst !== "string" || !SEVERITIES.has(worst)))
        throw new ApiShapeError("diagnostics maxSeverity: expected a severity or null");
    const problems: Problem[] = [];
    const seen = new Set<string>();
    for (const item of arr(body.diagnostics, "diagnostics entries")) {
        const problem = decodeProblem(item);
        if (problem === null) continue;
        const key = `${problem.txnIndex ?? problem.account}\u0000${problem.rule}\u0000${problem.message}`;
        if (seen.has(key)) continue;
        seen.add(key);
        problems.push(problem);
    }
    return Object.freeze({count, maxSeverity: worst as Severity | null, problems: Object.freeze(problems) as Problem[]});
}

// ---------------------------------------------------------------------------
// The windowed journal (`/api/journal/rows|summary|chart|position`,
// `/api/transactions/{index}`)
// ---------------------------------------------------------------------------

const STATUSES: ReadonlySet<string> = new Set<TxnStatus>(["unmarked", "pending", "cleared"]);
const POSTING_TYPES: ReadonlySet<string> = new Set<PostingType>(["virtual", "balancedVirtual"]);
const CATEGORIES: ReadonlySet<string> = new Set<RootCategory>(["asset", "liability", "equity", "revenue", "expense", "other"]);

function status(raw: unknown, context: string): TxnStatus {
    const value = str(raw, context);
    if (!STATUSES.has(value)) throw new ApiShapeError(`${context}: unknown status ${JSON.stringify(value)}`);
    return value as TxnStatus;
}

function tags(raw: unknown, context: string): [string, string][] {
    return Object.freeze(
        arr(raw, context).map((pair, i) => {
            const entry = arr(pair, `${context}[${i}]`);
            return Object.freeze([str(entry[0], `${context}[${i}][0]`), str(entry[1], `${context}[${i}][1]`)]) as [string, string];
        })
    ) as [string, string][];
}

function decodeAmount(raw: unknown, context: string): Amount {
    const value = obj(raw, context);
    const amount: Amount = {
        commodity: str(value.commodity, `${context}.commodity`),
        qty: decodeViewDec(value.qty, `${context}.qty`),
        style: decodeViewStyle(value.style, `${context}.style`),
    };
    if (value.cost !== undefined) {
        const cost = obj(value.cost, `${context}.cost`);
        amount.cost = Object.freeze({
            commodity: str(cost.commodity, `${context}.cost.commodity`),
            qty: decodeViewDec(cost.qty, `${context}.cost.qty`),
            per: bool(cost.per, `${context}.cost.per`),
        });
    }
    return Object.freeze(amount);
}

function decodePosting(raw: unknown, context: string): Posting {
    const value = obj(raw, context);
    const posting: Posting = {
        account: str(value.account, `${context}.account`),
        amounts: Object.freeze(arr(value.amounts, `${context}.amounts`).map((a, i) => decodeAmount(a, `${context}.amounts[${i}]`))) as Amount[],
        status: status(value.status, `${context}.status`),
        comment: str(value.comment, `${context}.comment`),
        tags: tags(value.tags, `${context}.tags`),
    };
    if (value.date !== undefined) posting.date = str(value.date, `${context}.date`);
    if (value.type !== undefined) {
        const type = str(value.type, `${context}.type`);
        if (!POSTING_TYPES.has(type)) throw new ApiShapeError(`${context}: unknown posting type ${JSON.stringify(type)}`);
        posting.type = type as PostingType;
    }
    if (value.generated !== undefined) {
        const generated = obj(value.generated, `${context}.generated`);
        const rule = generated.rule;
        if (typeof rule !== "number" || !Number.isInteger(rule) || rule < 0) {
            throw new ApiShapeError(`${context}.generated.rule: expected a rule index`);
        }
        posting.generated = Object.freeze({rule, query: str(generated.query, `${context}.generated.query`)});
    }
    if (value.elided !== undefined && bool(value.elided, `${context}.elided`)) posting.elided = true;
    if (value.balanceAssertion !== undefined) {
        const assertion = obj(value.balanceAssertion, `${context}.balanceAssertion`);
        posting.balanceAssertion = Object.freeze({
            amount: decodeAmount(assertion.amount, `${context}.balanceAssertion.amount`),
            inclusive: bool(assertion.inclusive, `${context}.balanceAssertion.inclusive`),
            total: bool(assertion.total, `${context}.balanceAssertion.total`),
        });
    }
    return Object.freeze(posting);
}

/**
 * One wire row → `{txn, flags}`. The transaction is the domain shape every row
 * component and the edit form already read; its flags are its own findings, so
 * a row needs no second lookup to draw its dot.
 */
export function decodeRow(raw: unknown, context = "row"): JournalRow {
    const value = obj(raw, context);
    const index = num(value.index, `${context}.index`);
    if (!Number.isSafeInteger(index) || index < 1) throw new ApiShapeError(`${context}: invalid index ${index}`);
    const txn: Transaction = {
        index,
        date: str(value.date, `${context}.date`),
        status: status(value.status, `${context}.status`),
        description: str(value.description, `${context}.description`),
        code: str(value.code, `${context}.code`),
        comment: str(value.comment, `${context}.comment`),
        tags: tags(value.tags, `${context}.tags`),
        postings: Object.freeze(arr(value.postings, `${context}.postings`).map((p, i) => decodePosting(p, `${context}.postings[${i}]`))) as Posting[],
    };
    if (value.date2 !== undefined) txn.date2 = str(value.date2, `${context}.date2`);
    const flags = arr(value.flags, `${context}.flags`).flatMap((flag) => {
        const problem = decodeProblem({...obj(flag, `${context}.flags[]`), tindex: index});
        return problem === null ? [] : [problem];
    });
    const row: JournalRow = {
        txn: Object.freeze(txn),
        flags: Object.freeze(flags) as Problem[],
        // Absent on a row that is not a journal transaction (a projection's
        // forecast rows): nothing to review there. Present, they must be booleans.
        unknown: value.unknown === undefined ? false : bool(value.unknown, `${context}.unknown`),
        reviewed: value.reviewed === undefined ? false : bool(value.reviewed, `${context}.reviewed`),
        // Present only when the request pinned an account (`balance=`).
        running: value.running === undefined ? null : decodeRunningCell(value.running, `${context}.running`),
    };
    // Plan 35 D11, additive: only `/api/journal/rows` sends them, and only on a linked row.
    if (value.linked !== undefined && bool(value.linked, `${context}.linked`)) row.linked = true;
    if (value.transfer !== undefined) row.transfer = decodeTransfer(value.transfer, `${context}.transfer`, index);
    return Object.freeze(row);
}

/** `{index, account, label}`: one end of a transfer (plan 35). Shared with `matchDecode.ts`. */
export function decodeTransferEnd(raw: unknown, context: string): TransferEnd {
    const value = obj(raw, context);
    const index = num(value.index, `${context}.index`);
    if (!Number.isSafeInteger(index) || index < 1) throw new ApiShapeError(`${context}: invalid index ${index}`);
    const nullable = (field: unknown, at: string): string | null => (field === null ? null : str(field, at));
    return Object.freeze({index, account: nullable(value.account, `${context}.account`), label: nullable(value.label, `${context}.label`)});
}

/**
 * A collapsed transfer row's pair: both halves (earliest first), one of
 * which must be the row itself, and two ends naming those two halves.
 */
function decodeTransfer(raw: unknown, context: string, anchor: number): TransferRow {
    const value = obj(raw, context);
    const halves = arr(value.halves, `${context}.halves`).map((half, i) => decodeRow(half, `${context}.halves[${i}]`));
    const [first, second] = halves;
    if (halves.length !== 2 || first === undefined || second === undefined) throw new ApiShapeError(`${context}: expected two halves, got ${halves.length}`);
    const indices = new Set([first.txn.index, second.txn.index]);
    if (indices.size !== 2 || !indices.has(anchor)) throw new ApiShapeError(`${context}: the halves must be two transactions, one of them the row`);
    const from = decodeTransferEnd(value.from, `${context}.from`);
    const to = decodeTransferEnd(value.to, `${context}.to`);
    if (from.index === to.index || !indices.has(from.index) || !indices.has(to.index)) {
        throw new ApiShapeError(`${context}: from and to must name the two halves`);
    }
    return Object.freeze({matchId: str(value.matchId, `${context}.matchId`), halves: Object.freeze([first, second] as const), from, to});
}

// ---------------------------------------------------------------------------
// Running balances (plan 36): `running` on a row, and `/api/journal/running`
// ---------------------------------------------------------------------------

const ASSERTION_OPS: ReadonlySet<string> = new Set<AssertionOp>(["=", "==", "=*", "==*"]);
const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

function isoDate(raw: unknown, context: string): string {
    const value = str(raw, context);
    if (!ISO_DATE.test(value)) throw new ApiShapeError(`${context}: ${JSON.stringify(value)} is not a YYYY-MM-DD date`);
    return value;
}

function decodeRunningAssertion(raw: unknown, context: string): RunningAssertion {
    const value = obj(raw, context);
    const op = str(value.op, `${context}.op`);
    if (!ASSERTION_OPS.has(op)) throw new ApiShapeError(`${context}: unknown assertion operator ${JSON.stringify(op)}`);
    const ok = bool(value.ok, `${context}.ok`);
    // A failure without the figure it found would render as a ✗ with no
    // explanation, which is the one thing the mark exists to give.
    if (!ok && value.calculated === undefined) throw new ApiShapeError(`${context}: a failing assertion needs its calculated amount`);
    return Object.freeze({
        account: str(value.account, `${context}.account`),
        op: op as AssertionOp,
        asserted: decodeAmount(value.asserted, `${context}.asserted`),
        calculated: value.calculated === undefined ? null : decodeAmount(value.calculated, `${context}.calculated`),
        ok,
    });
}

/** A row's `running` object → the Change/Balance cells and their assertion marks. */
export function decodeRunningCell(raw: unknown, context = "running"): RunningCell {
    const value = obj(raw, context);
    if (value.change === undefined) throw new ApiShapeError(`${context}.change: expected an amount or null`);
    return Object.freeze({
        change: value.change === null ? null : decodeAmount(value.change, `${context}.change`),
        balance: decodeAmount(value.balance, `${context}.balance`),
        touches: bool(value.touches, `${context}.touches`),
        postedOn: value.postedOn === undefined ? null : isoDate(value.postedOn, `${context}.postedOn`),
        assertions: Object.freeze(
            arr(value.assertions, `${context}.assertions`).map((a, i) => decodeRunningAssertion(a, `${context}.assertions[${i}]`))
        ) as RunningAssertion[],
    });
}

/** `GET /api/journal/running` → the pinned subtree's commodities and its assertions in evaluation order. */
export function decodeRunningInfo(raw: unknown): RunningInfo {
    const body = obj(raw, "journal running");
    const commodities = arr(body.commodities, "journal running commodities").map((entry, i): RunningCommodity => {
        const context = `journal running commodities[${i}]`;
        const value = obj(entry, context);
        const postings = num(value.postings, `${context}.postings`);
        if (!Number.isSafeInteger(postings) || postings < 0) throw new ApiShapeError(`${context}: invalid postings count ${postings}`);
        return Object.freeze({commodity: str(value.commodity, `${context}.commodity`), postings});
    });
    const assertions: Record<string, readonly RunningAssertionPoint[]> = {};
    for (const [commodity, list] of Object.entries(obj(body.assertions, "journal running assertions"))) {
        const context = `journal running assertions[${JSON.stringify(commodity)}]`;
        assertions[commodity] = Object.freeze(
            arr(list, context).map((entry, i): RunningAssertionPoint => {
                const value = obj(entry, `${context}[${i}]`);
                const tindex = num(value.tindex, `${context}[${i}].tindex`);
                if (!Number.isSafeInteger(tindex) || tindex < 1) throw new ApiShapeError(`${context}[${i}]: invalid tindex ${tindex}`);
                return Object.freeze({tindex, date: isoDate(value.date, `${context}[${i}].date`), ok: bool(value.ok, `${context}[${i}].ok`)});
            })
        );
    }
    return Object.freeze({
        generation: str(body.generation, "journal running generation"),
        account: str(body.account, "journal running account"),
        commodities: Object.freeze(commodities) as RunningCommodity[],
        assertions: Object.freeze(assertions),
    });
}

const SERIES_INTERVALS: ReadonlySet<string> = new Set<SeriesInterval>(["daily", "weekly", "monthly", "quarterly", "yearly"]);

function tindexOf(raw: unknown, context: string): number {
    const tindex = num(raw, context);
    if (!Number.isSafeInteger(tindex) || tindex < 1) throw new ApiShapeError(`${context}: invalid tindex ${tindex}`);
    return tindex;
}

function decodeSeriesFailure(raw: unknown, context: string): BalanceSeriesFailure {
    const value = obj(raw, context);
    const op = str(value.op, `${context}.op`);
    if (!ASSERTION_OPS.has(op)) throw new ApiShapeError(`${context}: unknown assertion operator ${JSON.stringify(op)}`);
    return Object.freeze({
        tindex: tindexOf(value.tindex, `${context}.tindex`),
        date: isoDate(value.date, `${context}.date`),
        account: str(value.account, `${context}.account`),
        op: op as AssertionOp,
        asserted: decodeViewDec(value.asserted, `${context}.asserted`),
        calculated: decodeViewDec(value.calculated, `${context}.calculated`),
        expected: decodeViewDec(value.expected, `${context}.expected`),
    });
}

function decodeSeriesPoint(raw: unknown, context: string): BalanceSeriesPoint {
    const value = obj(raw, context);
    // Both are omitted rather than empty: a period with no posting has no row
    // to scroll to, and a healthy period has no failures.
    const failures =
        value.failures === undefined ? [] : arr(value.failures, `${context}.failures`).map((f, i) => decodeSeriesFailure(f, `${context}.failures[${i}]`));
    return Object.freeze({
        bucket: str(value.bucket, `${context}.bucket`),
        label: str(value.label, `${context}.label`),
        end: isoDate(value.end, `${context}.end`),
        balance: decodeViewDec(value.balance, `${context}.balance`),
        tindex: value.tindex === undefined ? null : tindexOf(value.tindex, `${context}.tindex`),
        failures: Object.freeze(failures) as BalanceSeriesFailure[],
    });
}

/** `GET /api/journal/running/series` → the pinned subtree's balance line. */
export function decodeBalanceSeries(raw: unknown): BalanceSeries {
    const body = obj(raw, "balance series");
    const interval = str(body.interval, "balance series interval");
    if (!SERIES_INTERVALS.has(interval)) throw new ApiShapeError(`balance series: unknown interval ${JSON.stringify(interval)}`);
    const from = isoDate(body.from, "balance series from");
    const to = isoDate(body.to, "balance series to");
    if (from > to) throw new ApiShapeError(`balance series: window ${from} – ${to} is backwards`);
    return Object.freeze({
        generation: str(body.generation, "balance series generation"),
        account: str(body.account, "balance series account"),
        commodity: str(body.commodity, "balance series commodity"),
        style: decodeViewStyle(body.style, "balance series style"),
        interval: interval as SeriesInterval,
        from,
        to,
        points: Object.freeze(
            arr(body.points, "balance series points").map((p, i) => decodeSeriesPoint(p, `balance series points[${i}]`))
        ) as BalanceSeriesPoint[],
    });
}

/** `GET /api/journal/rows` → one window. */
export function decodeRowsPage(raw: unknown): RowsPage {
    const page = obj(raw, "journal rows");
    const offset = num(page.offset, "journal rows offset");
    return Object.freeze({
        generation: str(page.generation, "journal rows generation"),
        total: num(page.total, "journal rows total"),
        offset,
        rows: Object.freeze(arr(page.rows, "journal rows rows").map((row, i) => decodeRow(row, `journal rows[${offset + i}]`))) as JournalRow[],
    });
}

function category(raw: unknown, context: string): RootCategory {
    const value = str(raw, context);
    if (!CATEGORIES.has(value)) throw new ApiShapeError(`${context}: unknown category ${JSON.stringify(value)}`);
    return value as RootCategory;
}

/** `GET /api/journal/summary` → the filtered view's figures. */
export function decodeJournalSummary(raw: unknown): JournalSummary {
    const body = obj(raw, "journal summary");
    const commodities = strings(body.commodities, "journal summary commodities");
    const perCommodity = arr(body.perCommodity, "journal summary perCommodity").map((entry, i): CommoditySummary => {
        const context = `journal summary perCommodity[${i}]`;
        const value = obj(entry, context);
        return Object.freeze({
            commodity: str(value.commodity, `${context}.commodity`),
            style: decodeViewStyle(value.style, `${context}.style`),
            income: decodeViewDec(value.income, `${context}.income`),
            expenses: decodeViewDec(value.expenses, `${context}.expenses`),
            net: decodeViewDec(value.net, `${context}.net`),
            categories: Object.freeze(
                arr(value.categories, `${context}.categories`).map((c, j) => category(c, `${context}.categories[${j}]`))
            ) as RootCategory[],
        });
    });
    if (perCommodity.length !== commodities.length) throw new ApiShapeError("journal summary: one perCommodity entry per commodity expected");
    const visibleNet = arr(body.visibleNet, "journal summary visibleNet").map((entry, i) => decodeAmount(entry, `journal summary visibleNet[${i}]`));
    const primary = body.primary === null ? null : str(body.primary, "journal summary primary");
    const review = body.review === undefined ? null : decodeReviewCounts(body.review, "journal summary review");
    const transfers = body.transfers === undefined ? 0 : num(body.transfers, "journal summary transfers");
    if (!Number.isSafeInteger(transfers) || transfers < 0) throw new ApiShapeError(`journal summary: invalid transfers ${transfers}`);
    return Object.freeze({
        generation: str(body.generation, "journal summary generation"),
        total: num(body.total, "journal summary total"),
        transfers,
        visibleNet: Object.freeze(visibleNet) as Amount[],
        commodities,
        perCommodity: Object.freeze(perCommodity) as CommoditySummary[],
        maxDepth: num(body.maxDepth, "journal summary maxDepth"),
        primary,
        review,
    });
}

/** `review` in the summary: four counts, every one required. */
export function decodeReviewCounts(raw: unknown, context: string): ReviewCounts {
    const value = obj(raw, context);
    const count = (field: keyof ReviewCounts): number => {
        const n = num(value[field], `${context}.${field}`);
        if (!Number.isSafeInteger(n) || n < 0) throw new ApiShapeError(`${context}.${field}: invalid count ${n}`);
        return n;
    };
    return Object.freeze({
        unknown: count("unknown"),
        unreviewedRecent: count("unreviewedRecent"),
        notEditable: count("notEditable"),
        notEditableRecent: count("notEditableRecent"),
    });
}

/** `GET /api/journal/chart` → the group order and the requested mode's data. */
export function decodeChart(raw: unknown): ChartData {
    const body = obj(raw, "journal chart");
    const pie =
        body.pie === undefined
            ? null
            : (Object.freeze(
                  arr(body.pie, "journal chart pie").map((entry, i): PieDatum => {
                      const value = obj(entry, `journal chart pie[${i}]`);
                      return Object.freeze({
                          account: str(value.account, `journal chart pie[${i}].account`),
                          value: num(value.value, `journal chart pie[${i}].value`),
                          formatted: str(value.formatted, `journal chart pie[${i}].formatted`),
                      });
                  })
              ) as PieDatum[]);
    const line =
        body.line === undefined
            ? null
            : (Object.freeze(
                  arr(body.line, "journal chart line").map((entry, i): LineSeries => {
                      const value = obj(entry, `journal chart line[${i}]`);
                      const points = arr(value.points, `journal chart line[${i}].points`).map((point, j) => {
                          const p = obj(point, `journal chart line[${i}].points[${j}]`);
                          return Object.freeze({bucket: str(p.bucket, "point bucket"), value: num(p.value, "point value")});
                      });
                      return Object.freeze({
                          account: str(value.account, `journal chart line[${i}].account`),
                          points: Object.freeze(points) as LineSeries["points"],
                      });
                  })
              ) as LineSeries[]);
    return Object.freeze({
        generation: str(body.generation, "journal chart generation"),
        commodity: str(body.commodity, "journal chart commodity"),
        groups: strings(body.groups, "journal chart groups"),
        pie,
        line,
    });
}

/** `GET /api/journal/position` → where a transaction sits in the filtered view. */
export function decodeRowPosition(raw: unknown): RowPosition {
    const body = obj(raw, "journal position");
    return Object.freeze({
        generation: str(body.generation, "journal position generation"),
        position: body.position === null ? null : num(body.position, "journal position position"),
        date: body.date === null ? null : str(body.date, "journal position date"),
        total: num(body.total, "journal position total"),
    });
}
