// Exact decimal money math (WP-02). Non-negotiable: money is NEVER accumulated
// as floats. `Dec` is a scaled bigint built from hledger's decimalMantissa /
// decimalPlaces; rounding happens only in formatDec at display time.
// Pure TS: no Svelte/DOM imports.

import type {Amount, AmountStyle} from "./types";

/** Exact decimal: value = m / 10^p (mantissa, decimal places). */
export interface Dec {
    m: bigint;
    p: number;
}

export function dec(m: bigint | number, p: number): Dec {
    if (!Number.isInteger(p) || p < 0) {
        throw new RangeError(`dec: decimal places must be a non-negative integer, got ${p}`);
    }
    if (typeof m === "number") {
        if (!Number.isSafeInteger(m)) {
            throw new RangeError(`dec: mantissa ${m} is not a safe integer; construct from bigint instead`);
        }
        return {m: BigInt(m), p};
    }
    return {m, p};
}

const POW10: bigint[] = [1n];
function pow10(n: number): bigint {
    while (POW10.length <= n) POW10.push(POW10[POW10.length - 1] * 10n);
    return POW10[n];
}

/** Exact rescale to a HIGHER-or-equal number of decimal places (never rounds). */
function rescale(a: Dec, p: number): Dec {
    return a.p === p ? a : {m: a.m * pow10(p - a.p), p};
}

/** Exact addition: rescales the lower-p operand up; never rounds. */
export function add(a: Dec, b: Dec): Dec {
    const p = Math.max(a.p, b.p);
    return {m: rescale(a, p).m + rescale(b, p).m, p};
}

export function sub(a: Dec, b: Dec): Dec {
    return add(a, neg(b));
}

export function neg(a: Dec): Dec {
    return {m: -a.m, p: a.p};
}

/** Exact multiplication; result precision is a.p + b.p (price conversion only). */
export function mul(a: Dec, b: Dec): Dec {
    return {m: a.m * b.m, p: a.p + b.p};
}

export function cmp(a: Dec, b: Dec): -1 | 0 | 1 {
    const p = Math.max(a.p, b.p);
    const am = rescale(a, p).m;
    const bm = rescale(b, p).m;
    return am < bm ? -1 : am > bm ? 1 : 0;
}

export function isZero(a: Dec): boolean {
    return a.m === 0n;
}

/** DISPLAY ONLY (charts/export boundaries). Loses exactness by design. */
export function toNumber(a: Dec): number {
    return Number(a.m) / 10 ** a.p;
}

/** Multi-commodity amount: commodity symbol → exact quantity. */
export type MixedAmount = Map<string, Dec>;

/** Commodity-wise sum; zero entries are dropped from the result. */
export function maAdd(a: MixedAmount, b: MixedAmount): MixedAmount {
    const out = new Map(a);
    for (const [commodity, qty] of b) {
        const prev = out.get(commodity);
        out.set(commodity, prev === undefined ? qty : add(prev, qty));
    }
    for (const [commodity, qty] of out) {
        if (isZero(qty)) out.delete(commodity);
    }
    return out;
}

export function maNeg(a: MixedAmount): MixedAmount {
    const out: MixedAmount = new Map();
    for (const [commodity, qty] of a) out.set(commodity, neg(qty));
    return out;
}

export function maIsZero(a: MixedAmount): boolean {
    for (const qty of a.values()) {
        if (!isZero(qty)) return false;
    }
    return true;
}

/**
 * Round half-away-from-zero to `p` decimal places (rescales up exactly when
 * p >= d.p). DISPLAY ONLY — exported so export boundaries (xlsx) can round the
 * exact Dec themselves instead of letting a consumer re-round a float.
 */
export function roundTo(d: Dec, p: number): Dec {
    if (p >= d.p) return rescale(d, p);
    const divisor = pow10(d.p - p);
    const quotient = d.m / divisor;
    const remainder = d.m % divisor;
    const absRemainderTwice = (remainder < 0n ? -remainder : remainder) * 2n;
    if (absRemainderTwice >= divisor) {
        return {m: quotient + (d.m < 0n ? -1n : 1n), p};
    }
    return {m: quotient, p};
}

/** Group integer digits right-to-left; the last group size repeats (hledger semantics). */
function groupDigits(intDigits: string, [separator, sizes]: [string, number[]]): string {
    if (sizes.length === 0) return intDigits;
    const groups: string[] = [];
    let rest = intDigits;
    let i = 0;
    while (rest.length > 0) {
        const size = sizes[Math.min(i, sizes.length - 1)];
        if (size <= 0 || rest.length <= size) {
            groups.push(rest);
            break;
        }
        groups.push(rest.slice(-size));
        rest = rest.slice(0, -size);
        i += 1;
    }
    return groups.reverse().join(separator);
}

/**
 * Display cap for MONEY: never render more than two decimal places, whatever
 * the wire style or Dec precision says. Exact Decs keep full precision
 * internally; only formatting rounds.
 *
 * This is a rule about MONEY specifically, whose unit of account is the cent —
 * a third decimal on a balance is noise. It is NOT a rule about numbers in
 * general: a unit count (0.00123456 BTC) and a per-unit rate are not amounts of
 * money, and rounding them to cents is what makes them read as `0`. Callers
 * that format a non-money quantity pass MAX_QUANTITY_DECIMALS instead.
 */
export const MAX_DISPLAY_DECIMALS = 2;

/**
 * Display cap for NON-money quantities: unit/share counts and per-unit rates,
 * whose unit of account is whatever the journal wrote, not the cent.
 *
 * 8 places is the finest subdivision in wide use (a satoshi is 1e-8 BTC) and
 * still bounds a table column's width; below it the journal's own precision is
 * the real limit, since `displayPlaces` never asks for more places than the
 * value has.
 */
export const MAX_QUANTITY_DECIMALS = 8;

/**
 * How many decimal places to render `d` at: `min(precision, maxDecimals)`,
 * with one exception.
 *
 * Rounding may COMPRESS a number; it must never DELETE one. When the cap would
 * round a non-zero value to a string of zeros — a $0.00012345 price rendered
 * "0.00", a 0.00123456 BTC balance rendered "0" — the places are relaxed to the
 * value's own precision, bounded by MAX_QUANTITY_DECIMALS. The relaxation only
 * applies if it actually reveals a digit, so a value below 1e-8 keeps the short
 * styled zero rather than printing a row of zeros.
 *
 * Blast radius is exactly that exception: for every value that already renders
 * as something other than zero, this returns `min(precision, maxDecimals)` —
 * the pre-existing behaviour, unchanged.
 */
export function displayPlaces(d: Dec, precision: number, maxDecimals: number = MAX_DISPLAY_DECIMALS): number {
    const places = Math.max(0, Math.min(precision, maxDecimals));
    if (d.m === 0n || roundTo(d, places).m !== 0n) return places;
    const relaxed = Math.min(d.p, MAX_QUANTITY_DECIMALS);
    return relaxed > places && roundTo(d, relaxed).m !== 0n ? relaxed : places;
}

/** Format a Dec per style. Rounding (see `displayPlaces`) happens HERE only. */
export function formatDec(d: Dec, style: AmountStyle, maxDecimals: number = MAX_DISPLAY_DECIMALS): string {
    const rounded = roundTo(d, displayPlaces(d, style.precision, maxDecimals));
    const negative = rounded.m < 0n;
    const digits = (negative ? -rounded.m : rounded.m).toString().padStart(rounded.p + 1, "0");
    const intDigits = digits.slice(0, digits.length - rounded.p);
    const fracDigits = rounded.p > 0 ? digits.slice(digits.length - rounded.p) : "";
    const intPart = style.digitGroups === null ? intDigits : groupDigits(intDigits, style.digitGroups);
    const fracPart = fracDigits === "" ? "" : style.decimalPoint + fracDigits;
    return (negative ? "-" : "") + intPart + fracPart;
}

/**
 * Format qty + commodity honoring side/spacing/precision/groups, e.g.
 * "$-1,234.56" or "45,00 EUR". Money-capped by default; pass
 * MAX_QUANTITY_DECIMALS for an amount that is a unit count, not money.
 */
export function formatAmount(a: Amount, maxDecimals: number = MAX_DISPLAY_DECIMALS): string {
    const num = formatDec(a.qty, a.style, maxDecimals);
    if (a.commodity === "") return num;
    const space = a.style.spaced ? " " : "";
    return a.style.side === "L" ? a.commodity + space + num : num + space + a.commodity;
}
