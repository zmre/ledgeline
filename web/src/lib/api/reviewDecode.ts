// Wire → domain decoders for the transaction-review routes (plan 32;
// `crates/ledgeline-server/src/review_api.rs`).
//
// Same discipline as journalViewDecode.ts, whose row decoder every review
// transaction goes through: every field is checked, a missing or mistyped one
// throws `ApiShapeError` rather than rendering as a quiet zero, and decoded
// objects are frozen.

import type {
    BatchResult,
    DescriptionSuggestions,
    PayeeList,
    PayeeUse,
    Precedent,
    QueueItem,
    QueueKind,
    QueuePage,
    RecurringFill,
    ReviewItem,
    ReviewTxn,
    SimilarSet,
    Suggestion,
    SuggestionSource,
    TagList,
    TagUse,
} from "#lib/review/types.js";
import {ApiShapeError} from "./client";
import type {RulesCandidate, RulesFor} from "#lib/imports/reviewTypes.js";
import {arr, bool, decodeRow, num, obj, str, strings} from "./journalViewDecode";
import {decodeDec} from "./nativeDecode";

const SOURCES: ReadonlySet<string> = new Set<SuggestionSource>(["exactKey", "ruleFile", "payeePrefix", "recurring"]);

function count(raw: unknown, context: string): number {
    const n = num(raw, context);
    if (!Number.isSafeInteger(n) || n < 0) throw new ApiShapeError(`${context}: invalid count ${n}`);
    return n;
}

function index(raw: unknown, context: string): number {
    const n = num(raw, context);
    if (!Number.isSafeInteger(n) || n < 1) throw new ApiShapeError(`${context}: invalid index ${n}`);
    return n;
}

/** The keyed route's `index`, which names no transaction and must be 0. */
function zero(raw: unknown, context: string): number {
    const n = num(raw, context);
    if (n !== 0) throw new ApiShapeError(`${context}: expected 0 for a keyed similar set, got ${n}`);
    return n;
}

function nullableStr(raw: unknown, context: string): string | null {
    return raw === null ? null : str(raw, context);
}

/** A review transaction: a journal row (markers required here) plus its fingerprint. */
export function decodeReviewTxn(raw: unknown, context = "review txn"): ReviewTxn {
    const value = obj(raw, context);
    // `decodeRow` tolerates absent markers (forecast rows have none); a review
    // route always sends them, so their absence here means a different engine.
    bool(value.unknown, `${context}.unknown`);
    bool(value.reviewed, `${context}.reviewed`);
    return Object.freeze({...decodeRow(value, context), fingerprint: nullableStr(value.fingerprint, `${context}.fingerprint`)});
}

/** `{amount: {commodity, quantity: {mantissa, places}}, account}` (D36). */
function decodeRecurringFill(raw: unknown, context: string): RecurringFill {
    const value = obj(raw, context);
    const amount = obj(value.amount, `${context}.amount`);
    return Object.freeze({
        amount: Object.freeze({
            commodity: str(amount.commodity, `${context}.amount.commodity`),
            qty: decodeDec(amount.quantity as Parameters<typeof decodeDec>[0], `${context}.amount.quantity`),
        }),
        account: str(value.account, `${context}.account`),
    });
}

function decodeSuggestion(raw: unknown, context: string): Suggestion {
    const value = obj(raw, context);
    const source = str(value.source, `${context}.source`);
    if (!SOURCES.has(source)) throw new ApiShapeError(`${context}: unknown source ${JSON.stringify(source)}`);
    const suggestion: Suggestion = {
        account: str(value.account, `${context}.account`),
        description: nullableStr(value.description, `${context}.description`),
        source: source as SuggestionSource,
        confidence: count(value.confidence, `${context}.confidence`),
        strong: bool(value.strong, `${context}.strong`),
        count: count(value.count, `${context}.count`),
    };
    // Additive (D36): only the description route sends it.
    if (value.recurring !== undefined) suggestion.recurring = decodeRecurringFill(value.recurring, `${context}.recurring`);
    return Object.freeze(suggestion);
}

/** `GET /api/review/suggest` → a description's suggestions (D36), in the queue item's shape. */
export function decodeDescriptionSuggestions(raw: unknown): DescriptionSuggestions {
    const body = obj(raw, "suggest");
    const suggestions = arr(body.suggestions, "suggest suggestions").map((s, i) => decodeSuggestion(s, `suggest suggestions[${i}]`));
    return Object.freeze({
        generation: str(body.generation, "suggest generation"),
        key: nullableStr(body.key, "suggest key"),
        suggestions: Object.freeze(suggestions) as Suggestion[],
    });
}

/** One queue item: a review transaction plus its similarity key and suggestions. Also the staged-card base (`importReviewDecode.ts`). */
export function decodeQueueItem(raw: unknown, context: string): QueueItem {
    const value = obj(raw, context);
    const suggestions = arr(value.suggestions, `${context}.suggestions`).map((s, i) => decodeSuggestion(s, `${context}.suggestions[${i}]`));
    return Object.freeze({
        ...decodeReviewTxn(value, context),
        key: nullableStr(value.key, `${context}.key`),
        suggestions: Object.freeze(suggestions) as Suggestion[],
    });
}

/** `GET /api/review/item/{index}` → one transaction as a queue item (D31). */
export function decodeReviewItem(raw: unknown): ReviewItem {
    const body = obj(raw, "review item");
    return Object.freeze({generation: str(body.generation, "review item generation"), item: decodeQueueItem(body.item, "review item")});
}

/** `GET /api/review/queue` → one page of a queue. */
export function decodeQueuePage(raw: unknown): QueuePage {
    const page = obj(raw, "review queue");
    const kind = str(page.kind, "review queue kind");
    if (kind !== "unknowns" && kind !== "recents") throw new ApiShapeError(`review queue: unknown kind ${JSON.stringify(kind)}`);
    const offset = count(page.offset, "review queue offset");
    const items = arr(page.items, "review queue items").map((item, i) => decodeQueueItem(item, `review queue items[${offset + i}]`));
    return Object.freeze({
        generation: str(page.generation, "review queue generation"),
        kind: kind as QueueKind,
        total: count(page.total, "review queue total"),
        offset,
        notEditable: count(page.notEditable, "review queue notEditable"),
        items: Object.freeze(items) as QueueItem[],
    });
}

/** `GET /api/review/similar/{index}` → the candidates, the queued ones among them, and the precedent. */
export function decodeSimilarSet(raw: unknown): SimilarSet {
    return similarSet(raw, false);
}

/**
 * `GET /api/review/similar-key` → the same body for a similarity key rather
 * than a journal transaction (a staged import card, plan 32 D24). Its `index`
 * is 0: there is no journal transaction it is similar TO.
 */
export function decodeKeySimilarSet(raw: unknown): SimilarSet {
    return similarSet(raw, true);
}

function similarSet(raw: unknown, keyed: boolean): SimilarSet {
    const body = obj(raw, "similar");
    const candidates = arr(body.candidates, "similar candidates").map((c, i) => decodeReviewTxn(c, `similar candidates[${i}]`));
    const inQueue = arr(body.inQueue, "similar inQueue").map((i, at) => index(i, `similar inQueue[${at}]`));
    let precedent: Precedent | null = null;
    if (body.precedent !== null) {
        const value = obj(body.precedent, "similar precedent");
        precedent = Object.freeze({
            account: str(value.account, "similar precedent account"),
            count: count(value.count, "similar precedent count"),
            of: count(value.of, "similar precedent of"),
        });
    }
    return Object.freeze({
        generation: str(body.generation, "similar generation"),
        index: keyed ? zero(body.index, "similar index") : index(body.index, "similar index"),
        key: nullableStr(body.key, "similar key"),
        total: count(body.total, "similar total"),
        candidates: Object.freeze(candidates) as ReviewTxn[],
        inQueue: Object.freeze(inQueue) as number[],
        precedent,
    });
}

/** `POST /api/transactions/batch` → 200: the new generation and every edited transaction. */
export function decodeBatchResult(raw: unknown): BatchResult {
    const body = obj(raw, "batch");
    const edited = arr(body.edited, "batch edited").map((txn, i) => decodeReviewTxn(txn, `batch edited[${i}]`));
    return Object.freeze({generation: str(body.generation, "batch generation"), edited: Object.freeze(edited) as ReviewTxn[]});
}

/** `GET /api/payees` → every payee, most used first. */
export function decodePayees(raw: unknown): PayeeList {
    const body = obj(raw, "payees");
    const payees = arr(body.payees, "payees list").map((entry, i): PayeeUse => {
        const value = obj(entry, `payees[${i}]`);
        return Object.freeze({
            payee: str(value.payee, `payees[${i}].payee`),
            count: count(value.count, `payees[${i}].count`),
            last: str(value.last, `payees[${i}].last`),
        });
    });
    return Object.freeze({generation: str(body.generation, "payees generation"), payees: Object.freeze(payees) as PayeeUse[]});
}

/** `GET /api/tags` → every tag name with its commonest values. */
export function decodeTags(raw: unknown): TagList {
    const body = obj(raw, "tags");
    const tags = arr(body.tags, "tags list").map((entry, i): TagUse => {
        const value = obj(entry, `tags[${i}]`);
        const values = arr(value.values, `tags[${i}].values`).map((v, j) => {
            const pair = obj(v, `tags[${i}].values[${j}]`);
            return Object.freeze({value: str(pair.value, `tags[${i}].values[${j}].value`), count: count(pair.count, `tags[${i}].values[${j}].count`)});
        });
        return Object.freeze({
            name: str(value.name, `tags[${i}].name`),
            count: count(value.count, `tags[${i}].count`),
            values: Object.freeze(values) as TagUse["values"],
        });
    });
    return Object.freeze({generation: str(body.generation, "tags generation"), tags: Object.freeze(tags) as TagUse[]});
}

function decodeRulesCandidate(raw: unknown, context: string): RulesCandidate {
    const value = obj(raw, context);
    const year = value.year === null || value.year === undefined ? null : count(value.year, `${context}.year`);
    return Object.freeze({
        rulesId: str(value.rulesId, `${context}.rulesId`),
        label: str(value.label, `${context}.label`),
        family: str(value.family, `${context}.family`),
        familySize: index(value.familySize, `${context}.familySize`),
        year,
        account1: str(value.account1, `${context}.account1`),
        reasons: Object.freeze(strings(value.reasons, `${context}.reasons`)),
        recommended: bool(value.recommended, `${context}.recommended`),
        matchField: nullableStr(value.matchField, `${context}.matchField`),
        fields: value.fields === null || value.fields === undefined ? null : Object.freeze(strings(value.fields, `${context}.fields`)),
    });
}

/**
 * `GET /api/review/rules-for/{index}` (D27) → the rules files that could have
 * produced the transaction, best first. At most one may be `recommended`, and
 * `needsChoice` must agree with there being none: a body that says otherwise
 * is refused rather than preselecting the wrong file.
 */
export function decodeRulesFor(raw: unknown): RulesFor {
    const body = obj(raw, "rules for");
    const candidates = arr(body.candidates, "rules for candidates").map((candidate, i) => decodeRulesCandidate(candidate, `rules for candidates[${i}]`));
    const needsChoice = bool(body.needsChoice, "rules for needsChoice");
    const recommended = candidates.filter((candidate) => candidate.recommended).length;
    if (recommended > 1) throw new ApiShapeError("rules for: more than one candidate is recommended");
    if (needsChoice !== (candidates.length > 0 && recommended === 0)) throw new ApiShapeError("rules for: needsChoice disagrees with the recommendation");
    return Object.freeze({index: index(body.index, "rules for index"), needsChoice, candidates: Object.freeze(candidates)});
}
