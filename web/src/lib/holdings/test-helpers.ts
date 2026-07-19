// Builders for inline holdings-engine test fixtures (mirrors
// lib/reports/test-helpers.ts, plus tags/costs/explicit indices which the
// holdings tests need). Imported ONLY by colocated *.test.ts files.

import {dec} from "../domain/money";
import type {Amount, AmountStyle, ISODate, Posting, PriceDirective, Transaction} from "../domain/types";
import type {HoldingsScope} from "./types";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};

/** USD amount from integer cents (exact). */
export function usd(cents: number | bigint): Amount {
    return {commodity: "$", qty: dec(cents, 2), style: STYLE};
}

/** Arbitrary-commodity amount from mantissa + decimal places (exact). */
export function amt(commodity: string, mantissa: number | bigint, places: number): Amount {
    return {commodity, qty: dec(mantissa, places), style: {...STYLE, precision: places}};
}

/** Attach a cost annotation in cents (`per` ⇒ `@` per-unit, else `@@` total). */
export function withCost(amount: Amount, costCents: number | bigint, per: boolean, commodity = "$"): Amount {
    return {...amount, cost: {commodity, qty: dec(costCents, 2), per}};
}

export interface PostingSpec {
    account: string;
    amounts: Amount[];
    tags?: [string, string][];
}

/** Cleared transaction with an explicit index (txnIndex anchoring matters here). */
export function txn(index: number, date: ISODate, postings: PostingSpec[], tags: [string, string][] = []): Transaction {
    const built: Posting[] = postings.map((p) => ({account: p.account, amounts: p.amounts, status: "unmarked", comment: "", tags: p.tags ?? []}));
    return {index, date, status: "cleared", description: `txn ${index}`, code: "", comment: "", tags, postings: built, haystack: ""};
}

/** P directive: `commodity` priced at `priceCents` of `target` on `date`. */
export function pd(date: ISODate, commodity: string, priceCents: number | bigint, target = "$"): PriceDirective {
    return {date, commodity, price: {commodity: target, qty: dec(priceCents, 2), style: STYLE}};
}

/** Scope shorthand; defaults to include-everything, all-time gain. */
export function scope(asOf: ISODate, mode: "include" | "exclude" = "include", accounts: string[] = []): HoldingsScope {
    return {accounts: new Set(accounts), mode, asOf, gainPeriod: "all"};
}
