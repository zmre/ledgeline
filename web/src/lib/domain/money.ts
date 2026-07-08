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

/** Round half-away-from-zero to `p` decimal places (rescales up exactly when p >= d.p). */
function roundTo(d: Dec, p: number): Dec {
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
 * Display cap: never render more than two decimal places, whatever the wire
 * style or Dec precision says. Exact Decs keep full precision internally;
 * only formatting rounds.
 */
export const MAX_DISPLAY_DECIMALS = 2;

/** Format a Dec per style. Rounding (to min(style.precision, 2)) happens HERE only. */
export function formatDec(d: Dec, style: AmountStyle): string {
    const rounded = roundTo(d, Math.min(style.precision, MAX_DISPLAY_DECIMALS));
    const negative = rounded.m < 0n;
    const digits = (negative ? -rounded.m : rounded.m).toString().padStart(rounded.p + 1, "0");
    const intDigits = digits.slice(0, digits.length - rounded.p);
    const fracDigits = rounded.p > 0 ? digits.slice(digits.length - rounded.p) : "";
    const intPart = style.digitGroups === null ? intDigits : groupDigits(intDigits, style.digitGroups);
    const fracPart = fracDigits === "" ? "" : style.decimalPoint + fracDigits;
    return (negative ? "-" : "") + intPart + fracPart;
}

/** Format qty + commodity honoring side/spacing/precision/groups, e.g. "$-1,234.56" or "45,00 EUR". */
export function formatAmount(a: Amount): string {
    const num = formatDec(a.qty, a.style);
    if (a.commodity === "") return num;
    const space = a.style.spaced ? " " : "";
    return a.style.side === "L" ? a.commodity + space + num : num + space + a.commodity;
}
