// Wire → domain decoders for reviewing an import before it is written (plan 32
// Phase 4): the staged transactions and raw records a dry run carries, the
// QuickBooks preview's staged transactions, and the rule builder's live check.
// The engine side is `crates/ledgeline-server/src/import_api.rs` and
// `qb_journal_api.rs`.
//
// A staged transaction IS a review queue item plus where it sits in the
// import, so it goes through `reviewDecode.ts`'s own item decoder: one reading
// of the row shape for the journal overlay and the import one.

import type {ImportRulePreview, QbClassification, RulePreview, RulePreviewOk, StagedRecords, StagedRef, StagedTxn} from "#lib/imports/reviewTypes.js";
import {ApiShapeError} from "./client";
import {arr, bool, num, obj, str} from "./journalViewDecode";
import {decodeQueueItem} from "./reviewDecode";

const CLASSIFICATIONS: ReadonlySet<string> = new Set<QbClassification>(["new", "unchanged", "changed"]);

function count(raw: unknown, context: string): number {
    const n = num(raw, context);
    if (!Number.isSafeInteger(n) || n < 0) throw new ApiShapeError(`${context}: invalid count ${n}`);
    return n;
}

function strings(raw: unknown, context: string): readonly string[] {
    return Object.freeze(arr(raw, context).map((value, i) => str(value, `${context}[${i}]`)));
}

function nullableStrings(raw: unknown, context: string): readonly string[] | null {
    return raw === null || raw === undefined ? null : strings(raw, context);
}

function decodeStaged(raw: unknown, context: string, ref: StagedRef, classification: QbClassification | null): StagedTxn {
    const value = obj(raw, context);
    if (typeof value.fingerprint !== "string") throw new ApiShapeError(`${context}: a staged transaction needs a fingerprint`);
    return Object.freeze({
        ...decodeQueueItem(value, context),
        ref: Object.freeze(ref),
        overridden: value.overridden === undefined ? false : bool(value.overridden, `${context}.overridden`),
        classification,
    });
}

/** One CSV dry-run transaction: a queue item plus its 0-based `ordinal` in `entries`. */
export function decodeCsvStagedTxn(raw: unknown, context: string): StagedTxn {
    const ordinal = count(obj(raw, context).ordinal, `${context}.ordinal`);
    return decodeStaged(raw, context, {kind: "csv", ordinal}, null);
}

/** One QuickBooks preview transaction: a queue item plus its Trans # and classification. */
export function decodeQbStagedTxn(raw: unknown, context: string): StagedTxn {
    const value = obj(raw, context);
    const id = str(value.id, `${context}.id`);
    const classification = str(value.classification, `${context}.classification`);
    if (!CLASSIFICATIONS.has(classification)) throw new ApiShapeError(`${context}: unknown classification ${JSON.stringify(classification)}`);
    return decodeStaged(raw, context, {kind: "qb", id}, classification as QbClassification);
}

/** A list of staged transactions; absent (an engine that predates review) is the empty list. */
export function decodeStagedTxns(raw: unknown, context: string, one: (raw: unknown, context: string) => StagedTxn): readonly StagedTxn[] {
    if (raw === undefined || raw === null) return Object.freeze([]);
    return Object.freeze(arr(raw, context).map((txn, i) => one(txn, `${context}[${i}]`)));
}

/** What an engine that predates review answers: nothing aligned, so the rule builder matches whole records. */
const NO_RECORDS: StagedRecords = Object.freeze({aligned: false, header: null, fields: null, rows: Object.freeze([])});

/**
 * The raw records behind a dry run's transactions. `rows` must line up with
 * `txns` one to one when `aligned` — a record list of a different length would
 * hand the rule builder another row's text, so it is refused here.
 */
export function decodeStagedRecords(raw: unknown, context: string, txnCount: number): StagedRecords {
    if (raw === undefined || raw === null) return NO_RECORDS;
    const value = obj(raw, context);
    const aligned = bool(value.aligned, `${context}.aligned`);
    const rows = Object.freeze(arr(value.rows, `${context}.rows`).map((row, i) => strings(row, `${context}.rows[${i}]`)));
    if (aligned && rows.length !== txnCount)
        throw new ApiShapeError(`${context}: aligned records must be one per transaction (${rows.length} for ${txnCount})`);
    if (!aligned && rows.length !== 0) throw new ApiShapeError(`${context}: unaligned records carry no rows`);
    return Object.freeze({
        aligned,
        header: nullableStrings(value.header, `${context}.header`),
        fields: nullableStrings(value.fields, `${context}.fields`),
        rows,
    });
}

/** `POST /api/import/rule-preview` → how the candidate rule would change this import, or hledger's refusal. */
export function decodeRulePreview(raw: unknown): ImportRulePreview {
    const body = obj(raw, "rule preview");
    const ok = bool(body.ok, "rule preview ok");
    if (!ok) return Object.freeze({ok: false as const, stderr: str(body.stderr, "rule preview stderr")});
    return decodePreviewedRows(body);
}

/**
 * `POST /api/review/rule-preview` (D27): the import preview's numbers over the
 * rules file's newest statement (`basis: "csv"`, with its file name), or the
 * journal fallback's count (`basis: "journal"`), or hledger's refusal.
 * `noCsvMatch` is derived: a CSV-basis check whose pattern met no row.
 */
export function decodeJournalRulePreview(raw: unknown): RulePreview {
    const body = obj(raw, "rule preview");
    const ok = bool(body.ok, "rule preview ok");
    if (!ok) return Object.freeze({ok: false as const, stderr: str(body.stderr, "rule preview stderr")});
    const basis = str(body.basis, "rule preview basis");
    if (basis === "journal") {
        return Object.freeze({
            ok: true as const,
            basis: "journal" as const,
            matched: count(body.matched, "rule preview matched"),
            total: count(body.total, "rule preview total"),
        });
    }
    if (basis !== "csv") throw new ApiShapeError(`rule preview: unknown basis ${basis}`);
    const rows = decodePreviewedRows(body);
    return Object.freeze({...rows, basis: "csv" as const, file: str(body.file, "rule preview file"), noCsvMatch: rows.matched === 0});
}

/** The rows half of a preview: shared by the import's and the Journal view's CSV basis. */
function decodePreviewedRows(body: Record<string, unknown>): RulePreviewOk {
    const changed = arr(body.changed, "rule preview changed").map((entry, i) => {
        const value = obj(entry, `rule preview changed[${i}]`);
        return Object.freeze({
            ordinal: count(value.ordinal, `rule preview changed[${i}].ordinal`),
            before: str(value.before, `rule preview changed[${i}].before`),
            after: str(value.after, `rule preview changed[${i}].after`),
        });
    });
    return Object.freeze({
        ok: true as const,
        matched: count(body.matched, "rule preview matched"),
        total: count(body.total, "rule preview total"),
        alreadyCategorised: count(body.alreadyCategorised, "rule preview alreadyCategorised"),
        changed: Object.freeze(changed),
        changedTotal: count(body.changedTotal, "rule preview changedTotal"),
    });
}
