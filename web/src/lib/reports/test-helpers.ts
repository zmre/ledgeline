// Builders for inline report-engine test fixtures. Imported ONLY by
// colocated *.test.ts files — never by runtime code.

import {dec} from "../domain/money";
import type {Amount, AmountStyle, Posting, Transaction} from "../domain/types";
import {bucketEnd, bucketLabel, bucketStart} from "./periods";
import type {PeriodInfo} from "./types";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};

/** USD amount from integer cents (exact). */
export function usd(cents: number | bigint): Amount {
    return {commodity: "$", qty: dec(cents, 2), style: STYLE};
}

/** Arbitrary-commodity amount from mantissa + decimal places (exact). */
export function amt(commodity: string, mantissa: number | bigint, places: number): Amount {
    return {commodity, qty: dec(mantissa, places), style: {...STYLE, precision: places}};
}

let nextIndex = 1;

/** Cleared transaction from [account, ...amounts] posting tuples. */
export function txn(date: string, postings: [string, ...Amount[]][], description = ""): Transaction {
    const built: Posting[] = postings.map(([account, ...amounts]) => ({account, amounts, status: "unmarked", comment: "", tags: []}));
    nextIndex += 1;
    return {index: nextIndex, date, status: "cleared", description, code: "", comment: "", tags: [], postings: built};
}

/**
 * The `periods` the engine serves beside a period report's `buckets` (plan 26
 * B1), for hand-built report literals: each key's label and natural bounds, as
 * `period_info` computes them. Unparseable keys label and bound themselves.
 */
export function periodsOf(buckets: readonly string[]): PeriodInfo[] {
    const safe = (f: (key: string) => string, key: string): string => {
        try {
            return f(key);
        } catch {
            return key;
        }
    };
    return buckets.map((key) => {
        const range = key.split("..");
        return range.length === 2
            ? {key, label: bucketLabel(key), start: range[0], end: range[1]}
            : {key, label: bucketLabel(key), start: safe(bucketStart, key), end: safe(bucketEnd, key)};
    });
}
