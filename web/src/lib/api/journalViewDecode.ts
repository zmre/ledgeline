// Wire → domain decoders for the engine's journal-view routes (plan 26 B3–B5).
//
// Same discipline as nativeDecode.ts: every field is checked, a missing or
// mistyped one throws `ApiShapeError` rather than rendering as a quiet zero,
// and decoded objects are frozen. Exact decimals travel as `{m: "<mantissa>",
// p: places}` — the mantissa a STRING, so no digit is lost past 2^53.

import type {Problem, Severity} from "#lib/checks/engine.js";
import type {Dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import type {BalanceRow, BalanceSummary, CashBalancesView} from "#lib/journalView/types.js";
import {ApiShapeError} from "./client";

type Raw = Record<string, unknown>;

function obj(raw: unknown, context: string): Raw {
    if (typeof raw !== "object" || raw === null || Array.isArray(raw)) throw new ApiShapeError(`${context}: expected an object`);
    return raw as Raw;
}

function arr(raw: unknown, context: string): unknown[] {
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
