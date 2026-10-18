// Wire → domain normalizer (WP-02). THE ONLY FILE that knows hledger JSON
// field names. Tolerates hledger 1.52 and 2.0-preview shapes:
//   - cost annotation: `acost` (UnitCost/TotalCost) or older `aprice` (UnitPrice/TotalPrice)
//   - decimal mark: `asdecimalmark` or older `asdecimalpoint`
//   - precision: number, "NaturalPrecision", or tagged object → falls back to the qty's own places
// Emits frozen domain objects; Dec is built from decimalMantissa/decimalPlaces
// with a Number.isSafeInteger guard (never a silent float fallback).

import type {Dec} from "#lib/domain/money.js";
import {formatAmount} from "#lib/domain/money.js";
import type {Amount, AmountStyle, BalanceAssertion, Posting, PostingType, PriceDirective, Transaction, TxnStatus} from "#lib/domain/types.js";
import {ApiShapeError} from "./client";
import type {
    RawAmount,
    RawAmountStyle,
    RawBalanceAssertion,
    RawJournalPayload,
    RawMarketPrice,
    RawPosting,
    RawPriceDirective,
    RawQuantity,
    RawTransaction,
} from "./types.raw";

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

/** Declared display style per commodity symbol — see `normalizeCommodityStyles`. */
export type CommodityStyles = ReadonlyMap<string, AmountStyle>;

const NO_DECLARED_STYLES: CommodityStyles = new Map();

/**
 * The style an amount displays in. A declared `commodity` style is canonical
 * (hledger renders `$0.6` as `$0.60` under `commodity $1,000.00`), except that
 * its precision is a FLOOR, never a cap: a 10.25-share lot under `commodity 1.0
 * VTI` must not read as 10.3. Money's own two-place cap still applies at format
 * time (`MAX_DISPLAY_DECIMALS`).
 */
function amountStyle(declared: AmountStyle | undefined, raw: RawAmountStyle | undefined, qty: Dec): AmountStyle {
    if (declared === undefined) return toStyle(raw, qty);
    return qty.p <= declared.precision ? declared : Object.freeze({...declared, precision: qty.p});
}

function toAmount(raw: RawAmount, context: string, declared: CommodityStyles = NO_DECLARED_STYLES): Amount {
    const qty = toDec(raw.aquantity, context);
    const rawCost = raw.acost ?? raw.aprice; // 1.5x/2.0 vs older releases
    const commodity = raw.acommodity ?? "";
    const amount: Amount = {commodity, qty, style: amountStyle(declared.get(commodity), raw.astyle, qty)};
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

/**
 * hledger's `ptype` → the domain enum. Anything unrecognized (including the
 * field being absent, as in older wire dumps) reads as `"regular"`, which is
 * what hledger itself means by an unbracketed account.
 */
function toPostingType(raw: string | undefined): PostingType {
    if (raw === "VirtualPosting") return "virtual";
    if (raw === "BalancedVirtualPosting") return "balancedVirtual";
    return "regular";
}

/**
 * `pbalanceassertion` → the domain assertion, or undefined when the posting
 * asserts nothing. A record without a usable `baamount` is dropped rather than
 * thrown on: a junk assertion must not cost the whole journal load, and the
 * edit form treats "no assertion" as its safe default anyway.
 */
function toBalanceAssertion(raw: RawBalanceAssertion | null | undefined, context: string, declared: CommodityStyles): BalanceAssertion | undefined {
    if (raw === null || raw === undefined || raw.baamount === undefined) return undefined;
    return Object.freeze({
        amount: toAmount(raw.baamount, `${context} balance assertion`, declared),
        inclusive: raw.bainclusive === true,
        total: raw.batotal === true,
    });
}

function toPosting(raw: RawPosting, context: string, declared: CommodityStyles): Posting {
    const account = raw.paccount ?? "";
    const posting: Posting = {
        account,
        amounts: frozen((raw.pamount ?? []).map((amount) => toAmount(amount, `${context} posting "${account}"`, declared))),
        status: toStatus(raw.pstatus),
        comment: (raw.pcomment ?? "").trimEnd(),
        tags: toTags(raw.ptags),
    };
    if (typeof raw.pdate === "string") posting.date = raw.pdate;
    // Both are set only when they carry information, matching `date` above — so
    // "absent" is the ordinary posting and every consumer defaults the same way.
    const type = toPostingType(raw.ptype);
    if (type !== "regular") posting.type = type;
    const assertion = toBalanceAssertion(raw.pbalanceassertion, `${context} posting "${account}"`, declared);
    if (assertion !== undefined) posting.balanceAssertion = assertion;
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

function toTransaction(raw: RawTransaction, declared: CommodityStyles): Transaction {
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
        postings: frozen((raw.tpostings ?? []).map((posting) => toPosting(posting, context, declared))),
    };
    const txn: Transaction = {...base, haystack: buildHaystack(base)};
    if (typeof raw.tdate2 === "string") txn.date2 = raw.tdate2;
    return Object.freeze(txn);
}

/**
 * The transactions array out of either journal payload shape: a bare array
 * (plain hledger-web / pre-diagnostics engine) or a `{transactions, diagnostics}`
 * envelope. Returns null when it is neither.
 */
function journalTransactions(raw: unknown): RawTransaction[] | null {
    if (Array.isArray(raw)) return raw as RawTransaction[];
    if (typeof raw === "object" && raw !== null) {
        const envelope = (raw as RawJournalPayload).transactions;
        if (Array.isArray(envelope)) return envelope;
    }
    return null;
}

/**
 * `declared` is the journal's `commodity`/`D` directive styles: hledger keeps
 * each amount's as-written precision on the wire and applies the declared style
 * only when rendering, so this is where the SPA applies it.
 */
export function normalizeTransactions(raw: unknown, declared: CommodityStyles = NO_DECLARED_STYLES): Transaction[] {
    const list = journalTransactions(raw);
    if (list === null) throw new ApiShapeError("GET /transactions: expected a JSON array");
    return list.map((txn) => toTransaction(txn, declared));
}

/**
 * `/api/journal`'s `commodityStyles` (symbol → hledger `astyle`) → a map.
 * Lenient, never throws: absent (an older engine, a plain hledger-web) or
 * malformed reads as "nothing declared", and a junk entry costs only itself —
 * a style hint must never fail a journal load.
 */
export function normalizeCommodityStyles(raw: unknown): CommodityStyles {
    if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return NO_DECLARED_STYLES;
    const styles = new Map<string, AmountStyle>();
    for (const [commodity, style] of Object.entries(raw)) {
        if (typeof style !== "object" || style === null || Array.isArray(style)) continue;
        if (typeof (style as RawAmountStyle).asprecision !== "number") continue;
        styles.set(commodity, toStyle(style as RawAmountStyle, ZERO_QTY));
    }
    return styles;
}

/** `toStyle`'s precision fallback, never reached here: declared styles always carry a numeric `asprecision`. */
const ZERO_QTY: Dec = Object.freeze({m: 0n, p: 0});

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
