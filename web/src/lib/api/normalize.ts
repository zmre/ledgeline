// Wire → domain normalizer (WP-02). THE ONLY FILE that knows hledger JSON
// field names. Tolerates hledger 1.52 and 2.0-preview shapes:
//   - cost annotation: `acost` (UnitCost/TotalCost) or older `aprice` (UnitPrice/TotalPrice)
//   - decimal mark: `asdecimalmark` or older `asdecimalpoint`
//   - precision: number, "NaturalPrecision", or tagged object → falls back to the qty's own places
// Emits frozen domain objects; Dec is built from decimalMantissa/decimalPlaces
// with a Number.isSafeInteger guard (never a silent float fallback).

import type {Problem, Severity} from "$lib/checks/engine";
import type {AccountDecl} from "$lib/domain/accountTypes";
import {parseAccountTypeTag} from "$lib/domain/accountTypes";
import type {Dec} from "$lib/domain/money";
import {formatAmount} from "$lib/domain/money";
import type {Amount, AmountStyle, BalanceAssertion, Posting, PostingType, PriceDirective, Transaction, TxnStatus} from "$lib/domain/types";
import {ApiShapeError} from "./client";
import type {
    RawAccount,
    RawAmount,
    RawAmountStyle,
    RawBalanceAssertion,
    RawDiagnostic,
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
function toBalanceAssertion(raw: RawBalanceAssertion | null | undefined, context: string): BalanceAssertion | undefined {
    if (raw === null || raw === undefined || raw.baamount === undefined) return undefined;
    return Object.freeze({
        amount: toAmount(raw.baamount, `${context} balance assertion`),
        inclusive: raw.bainclusive === true,
        total: raw.batotal === true,
    });
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
    // Both are set only when they carry information, matching `date` above — so
    // "absent" is the ordinary posting and every consumer defaults the same way.
    const type = toPostingType(raw.ptype);
    if (type !== "regular") posting.type = type;
    const assertion = toBalanceAssertion(raw.pbalanceassertion, `${context} posting "${account}"`);
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

export function normalizeTransactions(raw: unknown): Transaction[] {
    const list = journalTransactions(raw);
    if (list === null) throw new ApiShapeError("GET /transactions: expected a JSON array");
    return list.map((txn) => toTransaction(txn));
}

/**
 * The only `rule` values the engine emits. Adding one = a single entry here.
 *
 * Mirrors `DIAGNOSTIC_RULES` in `crates/ledgeline-core/src/wire.rs`; a Rust test
 * (`diagnostic_rules_match_the_spa_allow_list`) reads this very line and fails
 * if the two drift, because a rule the engine emits and this set omits is
 * silently dropped — a finding that vanishes with no error anywhere.
 *
 * The three `stock-*` rules arrived when the SPA stopped computing them from its
 * own copy of the holdings engine (DRY-1) and started reading the engine's.
 * `account-tag` arrived when the four closed-vocabulary `account` tags stopped
 * answering a typo with a 400 that took the whole tab down.
 *
 * MUST STAY ON ONE PHYSICAL LINE. The parity test reads this file as TEXT and
 * takes the first line whose text declares this set, then pulls the quoted
 * strings out of it — so a prettier reflow across lines, or any prose above that
 * repeats the declaration verbatim, silently reduces it to an empty list.
 * Hence the `prettier-ignore`, and hence this paragraph not spelling the name.
 */
// prettier-ignore
const DIAGNOSTIC_RULES: ReadonlySet<string> = new Set(["unbalanced", "assertion", "account-tag", "stock-missing-basis", "stock-negative", "stock-unpriced"]);
/** Valid `Severity` values (the domain enum); anything else is junk we refuse to hand the UI. */
const DIAGNOSTIC_SEVERITIES: ReadonlySet<string> = new Set<Severity>(["error", "warning", "info"]);

/**
 * One wire diagnostic → a `Problem`, or null when the entry is unusable.
 *
 * Deliberately does NOT throw, unlike every other decoder in this file: these
 * are advisory findings the engine attaches to a journal it opened
 * successfully, so a junk entry must cost us that ONE finding, never the whole
 * journal load.
 *
 * The index translation is the subtle part. The wire `txnIndex` is a 0-based
 * position in the served array, but `Problem.txnIndex` is matched against
 * `Transaction.index` (hledger's 1-based `tindex`) by the row flags, the
 * drawer's date/description lookup and `problems.requestFocus`. So the position
 * is resolved through `txns` to the transaction's own index. A position outside
 * the array cannot be anchored to a row at all, so it is dropped.
 *
 * An entry may instead be anchored to an ACCOUNT (the `account-tag` rule): a
 * finding about an `account` DIRECTIVE, which has no transaction. It carries
 * `account` and no `txnIndex`, and survives with `txnIndex: null`. What is
 * dropped is an entry with NEITHER anchor — the drawer could show it but nothing
 * could say what it was about, which is worse than losing the one finding.
 */
function toDiagnostic(raw: unknown, txns: readonly Transaction[]): Problem | null {
    if (typeof raw !== "object" || raw === null) return null;
    const entry = raw as RawDiagnostic;
    if (typeof entry.rule !== "string" || !DIAGNOSTIC_RULES.has(entry.rule)) return null;
    if (typeof entry.severity !== "string" || !DIAGNOSTIC_SEVERITIES.has(entry.severity)) return null;
    if (typeof entry.message !== "string" || entry.message.trim() === "") return null;

    const position = entry.txnIndex;
    // `undefined` is "not anchored to a transaction"; anything else present must
    // be a real in-range position, so a junk txnIndex is still refused rather
    // than quietly demoted to an account anchor.
    if (position !== undefined) {
        if (!Number.isInteger(position) || position < 0 || position >= txns.length) return null;
        return Object.freeze({
            txnIndex: txns[position].index,
            rule: entry.rule,
            severity: entry.severity as Severity,
            message: entry.message,
        });
    }

    const account = entry.account;
    if (typeof account !== "string" || account.trim() === "") return null;
    return Object.freeze({
        txnIndex: null,
        account,
        rule: entry.rule,
        severity: entry.severity as Severity,
        message: entry.message,
    });
}

/**
 * Engine-computed journal diagnostics off the journal payload → `Problem`s, for
 * merging into the checks pipeline (see CheckContext.diagnostics).
 *
 * Total and non-throwing by contract: a missing/null/non-array `diagnostics`
 * field, a payload that is a bare transactions array (older engine), or an
 * entry with a bad rule/severity/message/txnIndex all degrade to "no
 * diagnostics" or a skipped entry. Exact duplicates are collapsed — the drawer
 * keys its list by `txnIndex + message`, and Svelte throws on a duplicate key.
 */
export function normalizeDiagnostics(raw: unknown, txns: readonly Transaction[]): Problem[] {
    let list: unknown[];
    if (Array.isArray(raw)) list = raw;
    else if (typeof raw === "object" && raw !== null && Array.isArray((raw as RawJournalPayload).diagnostics)) {
        list = (raw as RawJournalPayload).diagnostics as unknown[];
    } else return frozen([]);

    const problems: Problem[] = [];
    const seen = new Set<string>();
    for (const item of list) {
        const problem = toDiagnostic(item, txns);
        if (problem === null) continue;
        const key = `${problem.txnIndex}\u0000${problem.rule}\u0000${problem.message}`;
        if (seen.has(key)) continue;
        seen.add(key);
        problems.push(problem);
    }
    return frozen(problems);
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

/** How many entries the last `normalizeAccounts` call dropped as unusable. Exported for tests. */
let skippedAccounts = 0;

/** Entries dropped by the most recent `normalizeAccounts` call (no usable `aname`). */
export function lastSkippedAccountCount(): number {
    return skippedAccounts;
}

/**
 * /accounts → the declared `type:` per account (the only field we read).
 * Accounts inherited into the tree but never declared carry `type: null`; the
 * `type:` tag lives in adeclarationinfo.aditags as ["type", "C"|"Cash"|…].
 *
 * A MALFORMED entry (no string `aname`) is still skipped rather than thrown on
 * — one junk record must not cost the whole journal load — but it is now
 * COUNTED and reported. Silence was the wrong default here specifically: every
 * report classifies accounts by their DECLARED type, so a dropped declaration
 * doesn't degrade gracefully, it re-buckets a whole subtree and makes its
 * totals read zero, which is indistinguishable from a correct answer.
 *
 * `aname: ""` is NOT malformed — it is hledger's tree root, present in every
 * healthy payload — so it is skipped in silence. Counting it would make the
 * warning fire on every successful load and mean nothing.
 */
export function normalizeAccounts(raw: unknown): AccountDecl[] {
    if (!Array.isArray(raw)) throw new ApiShapeError("GET /accounts: expected a JSON array");
    const decls: AccountDecl[] = [];
    let skipped = 0;
    for (const item of raw) {
        const account = item as RawAccount;
        if (typeof account.aname !== "string") {
            skipped += 1;
            continue;
        }
        if (account.aname === "") continue;
        const typeTag = toTags(account.adeclarationinfo?.aditags).find(([key]) => key === "type");
        decls.push(Object.freeze({name: account.aname, type: typeTag !== undefined ? parseAccountTypeTag(typeTag[1]) : null}));
    }
    skippedAccounts = skipped;
    if (skipped > 0) {
        console.warn(
            `GET /accounts: skipped ${skipped} malformed ${skipped === 1 ? "entry" : "entries"} (no usable "aname"). ` +
                `Any account type they declared is lost, so those subtrees will be classified by name instead and may total zero.`
        );
    }
    return frozen(decls);
}
