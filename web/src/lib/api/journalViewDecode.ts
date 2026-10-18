// Wire → domain decoders for the engine's journal-view routes (plan 26 B3–B5).
//
// Same discipline as nativeDecode.ts: every field is checked, a missing or
// mistyped one throws `ApiShapeError` rather than rendering as a quiet zero,
// and decoded objects are frozen. Exact decimals travel as `{m: "<mantissa>",
// p: places}` — the mantissa a STRING, so no digit is lost past 2^53.

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
