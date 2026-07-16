// Wire → domain normalizer (WP-02). THE ONLY FILE that knows hledger JSON
// field names. Tolerates hledger 1.52 and 2.0-preview shapes:
//   - cost annotation: `acost` (UnitCost/TotalCost) or older `aprice` (UnitPrice/TotalPrice)
//   - decimal mark: `asdecimalmark` or older `asdecimalpoint`
//   - precision: number, "NaturalPrecision", or tagged object → falls back to the qty's own places
// Emits frozen domain objects; Dec is built from decimalMantissa/decimalPlaces
// with a Number.isSafeInteger guard (never a silent float fallback).

import type {AccountDecl} from "$lib/domain/accountTypes";
import {parseAccountTypeTag} from "$lib/domain/accountTypes";
import type {Dec} from "$lib/domain/money";
import {formatAmount} from "$lib/domain/money";
import type {Amount, AmountStyle, Posting, PriceDirective, Transaction, TxnStatus} from "$lib/domain/types";
import {ApiShapeError} from "./client";
import type {RawAccount, RawAmount, RawAmountStyle, RawMarketPrice, RawPosting, RawPriceDirective, RawQuantity, RawTransaction} from "./types.raw";

/** Shallow-freeze an array without losing its mutable-typed contract. */
function frozen<T>(items: T[]): T[] {
    return Object.freeze(items) as T[];
}

function toDec(quantity: RawQuantity | undefined, context: string): Dec {
    if (quantity === undefined || typeof quantity.decimalMantissa !== "number" || typeof quantity.decimalPlaces !== "number") {
        throw new ApiShapeError(`${context}: missing decimalMantissa/decimalPlaces`);
    }
    if (!Number.isSafeInteger(quantity.decimalMantissa)) {
        throw new ApiShapeError(`${context}: decimalMantissa ${quantity.decimalMantissa} is outside the safe integer range`);
    }
    if (!Number.isSafeInteger(quantity.decimalPlaces) || quantity.decimalPlaces < 0) {
        throw new ApiShapeError(`${context}: invalid decimalPlaces ${quantity.decimalPlaces}`);
    }
    return Object.freeze({m: BigInt(quantity.decimalMantissa), p: quantity.decimalPlaces});
}

function toStyle(style: RawAmountStyle | undefined, qty: Dec): AmountStyle {
    let precision = qty.p; // NaturalPrecision (string/tagged/absent) → the quantity's own places
    const rawPrecision = style?.asprecision;
    if (typeof rawPrecision === "number" && Number.isInteger(rawPrecision) && rawPrecision >= 0) {
        precision = rawPrecision;
    } else if (typeof rawPrecision === "object" && rawPrecision !== null && typeof rawPrecision.contents === "number") {
        precision = rawPrecision.contents;
    }
    let digitGroups: [string, number[]] | null = null;
    const rawGroups = style?.asdigitgroups;
    if (Array.isArray(rawGroups) && typeof rawGroups[0] === "string" && Array.isArray(rawGroups[1])) {
        const sizes = rawGroups[1].filter((size): size is number => typeof size === "number" && Number.isInteger(size) && size > 0);
        if (sizes.length > 0) {
            const pair: [string, number[]] = [rawGroups[0], frozen(sizes)];
            digitGroups = Object.freeze(pair) as [string, number[]];
        }
    }
    return Object.freeze({
        side: style?.ascommodityside === "R" ? ("R" as const) : ("L" as const),
        spaced: style?.ascommodityspaced === true,
        precision,
        decimalPoint: style?.asdecimalmark ?? style?.asdecimalpoint ?? ".",
        digitGroups,
    });
}

function toAmount(raw: RawAmount, context: string): Amount {
    const qty = toDec(raw.aquantity, context);
    const rawCost = raw.acost ?? raw.aprice; // 1.5x/2.0 vs older releases
    const amount: Amount = {commodity: raw.acommodity ?? "", qty, style: toStyle(raw.astyle, qty)};
    if (rawCost !== null && rawCost !== undefined && rawCost.contents !== undefined) {
        // hledger 1.52's JSON emits `@@`/inferred total costs SIGNED (negative on
        // sells), unlike journal syntax. Cost magnitudes are inherently positive,
        // so canonicalize to the absolute value — the domain contract is
        // "cost.qty is always unsigned" (see Amount.cost in domain/types.ts).
        const costQty = toDec(rawCost.contents.aquantity, `${context} cost`);
        amount.cost = Object.freeze({
            commodity: rawCost.contents.acommodity ?? "",
            qty: costQty.m < 0n ? Object.freeze({m: -costQty.m, p: costQty.p}) : costQty,
            per: rawCost.tag === "UnitCost" || rawCost.tag === "UnitPrice",
        });
    }
    return Object.freeze(amount);
}

function toStatus(raw: string | undefined): TxnStatus {
    if (raw === "Cleared") return "cleared";
    if (raw === "Pending") return "pending";
    return "unmarked";
}

function toTags(raw: unknown[] | undefined): [string, string][] {
    const tags: [string, string][] = [];
    for (const entry of raw ?? []) {
        if (Array.isArray(entry) && typeof entry[0] === "string") {
            const pair: [string, string] = [entry[0], typeof entry[1] === "string" ? entry[1] : ""];
            tags.push(Object.freeze(pair) as [string, string]);
        }
    }
    return frozen(tags);
}

function toPosting(raw: RawPosting, context: string): Posting {
    const account = raw.paccount ?? "";
    const posting: Posting = {
        account,
        amounts: frozen((raw.pamount ?? []).map((amount) => toAmount(amount, `${context} posting "${account}"`))),
        status: toStatus(raw.pstatus),
        comment: (raw.pcomment ?? "").trimEnd(),
        tags: toTags(raw.ptags),
    };
    if (typeof raw.pdate === "string") posting.date = raw.pdate;
    return Object.freeze(posting);
}

/** Lowercase search text: description + comments + accounts + amounts + commodities. */
function buildHaystack(txn: Omit<Transaction, "haystack">): string {
    const parts: string[] = [txn.description, txn.code, txn.comment];
    for (const posting of txn.postings) {
        parts.push(posting.account, posting.comment);
        for (const amount of posting.amounts) {
            parts.push(formatAmount(amount), amount.commodity);
        }
    }
    return parts
        .filter((part) => part !== "")
        .join("\n")
        .toLowerCase();
}

function toTransaction(raw: RawTransaction): Transaction {
    if (typeof raw.tindex !== "number" || typeof raw.tdate !== "string") {
        throw new ApiShapeError(`transaction ${JSON.stringify(raw.tindex ?? null)}: missing tindex/tdate`);
    }
    const context = `transaction #${raw.tindex} "${raw.tdescription ?? ""}" (${raw.tdate})`;
    const base: Omit<Transaction, "haystack"> = {
        index: raw.tindex,
        date: raw.tdate,
        status: toStatus(raw.tstatus),
        description: raw.tdescription ?? "",
        code: raw.tcode ?? "",
        comment: (raw.tcomment ?? "").trimEnd(),
        tags: toTags(raw.ttags),
        postings: frozen((raw.tpostings ?? []).map((posting) => toPosting(posting, context))),
    };
    const txn: Transaction = {...base, haystack: buildHaystack(base)};
    if (typeof raw.tdate2 === "string") txn.date2 = raw.tdate2;
    return Object.freeze(txn);
}

export function normalizeTransactions(raw: unknown): Transaction[] {
    if (!Array.isArray(raw)) throw new ApiShapeError("GET /transactions: expected a JSON array");
    return raw.map((txn) => toTransaction(txn as RawTransaction));
}

const marketPriceStyle = (qty: Dec): AmountStyle => Object.freeze({side: "L" as const, spaced: false, precision: qty.p, decimalPoint: ".", digitGroups: null});

function toPriceDirective(raw: unknown): PriceDirective {
    const directive = raw as RawPriceDirective;
    if (typeof directive.pddate === "string" && typeof directive.pdcommodity === "string" && directive.pdamount !== undefined) {
        // Older shape: full price directive with a styled amount.
        return Object.freeze({
            date: directive.pddate,
            commodity: directive.pdcommodity,
            price: toAmount(directive.pdamount, `price directive ${directive.pdcommodity} (${directive.pddate})`),
        });
    }
    const market = raw as RawMarketPrice;
    if (typeof market.mpdate !== "string" || typeof market.mpfrom !== "string" || typeof market.mpto !== "string") {
        throw new ApiShapeError("GET /prices: unrecognized price record shape");
    }
    const qty = toDec(market.mprate, `market price ${market.mpfrom} (${market.mpdate})`);
    return Object.freeze({
        date: market.mpdate,
        commodity: market.mpfrom,
        price: Object.freeze({commodity: market.mpto, qty, style: marketPriceStyle(qty)}),
    });
}

export function normalizePrices(raw: unknown): PriceDirective[] {
    if (!Array.isArray(raw)) throw new ApiShapeError("GET /prices: expected a JSON array");
    return raw.map(toPriceDirective);
}

/**
 * /accounts → the declared `type:` per account (the only field we read).
 * Accounts inherited into the tree but never declared carry `type: null`; the
 * `type:` tag lives in adeclarationinfo.aditags as ["type", "C"|"Cash"|…].
 */
export function normalizeAccounts(raw: unknown): AccountDecl[] {
    if (!Array.isArray(raw)) throw new ApiShapeError("GET /accounts: expected a JSON array");
    const decls: AccountDecl[] = [];
    for (const item of raw) {
        const account = item as RawAccount;
        if (typeof account.aname !== "string" || account.aname === "") continue;
        const typeTag = toTags(account.adeclarationinfo?.aditags).find(([key]) => key === "type");
        decls.push(Object.freeze({name: account.aname, type: typeTag !== undefined ? parseAccountTypeTag(typeTag[1]) : null}));
    }
    return frozen(decls);
}
