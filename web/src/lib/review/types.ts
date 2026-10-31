// Domain types for transaction review (plan 32). Pure TS: no Svelte/DOM
// imports. Decoded from the wire by `api/reviewDecode.ts`; the model behind
// them lives in `crates/ledgeline-core/src/review.rs` and the routes in
// `crates/ledgeline-server/src/review_api.rs`.

import type {ISODate} from "#lib/domain/types.js";
import type {JournalRow} from "#lib/journalView/types.js";
import type {PostingForm} from "#lib/api/editMapping.js";

export type {WireEditOp as EditOp, WireBatchEdit as BatchEdit} from "#lib/api/native.js";

/** Which queue (D4): every unknown in the window, or every untagged transaction in it. */
export type QueueKind = "unknowns" | "recents";

/**
 * A transaction as a review route serves it: the journal row (with its
 * `unknown`/`reviewed` markers) plus the fingerprint a batch must echo back to
 * edit it — null when the engine has no editor (read-only), so it cannot be
 * edited at all.
 */
export interface ReviewTxn extends JournalRow {
    fingerprint: string | null;
}

/** Where a suggestion came from (D13a), best first. */
export type SuggestionSource = "exactKey" | "ruleFile" | "payeePrefix" | "recurring";

/** One suggested category for a card. */
export interface Suggestion {
    account: string;
    /** A rename reviewed transactions with this payee received ("→ Blue Bottle Coffee"), or null. */
    description: string | null;
    source: SuggestionSource;
    /** Basis points out of 10000. */
    confidence: number;
    /** Strong enough (≥ 50%) for the one-keystroke chip. Only the FIRST suggestion may be the chip. */
    strong: boolean;
    /** How many transactions back it up. */
    count: number;
}

/** One queued card. */
export interface QueueItem extends ReviewTxn {
    /** The normalised payee similar transactions share (D10), or null when the description has none. */
    key: string | null;
    /** Best first, at most 3: the chip and its runners-up. */
    suggestions: Suggestion[];
}

/** `GET /api/review/queue` — one page of a queue. */
export interface QueuePage {
    generation: string;
    kind: QueueKind;
    /** The whole queue's editable count, not just this page. */
    total: number;
    offset: number;
    /** How many more qualify but live in a file that cannot be written ("N not editable"). */
    notEditable: number;
    items: QueueItem[];
}

/** The most common category among the payee's categorised transactions. */
export interface Precedent {
    account: string;
    count: number;
    /** How many categorised transactions there were. */
    of: number;
}

/** `GET /api/review/similar/{index}` — what a card's change may be applied to (D11). */
export interface SimilarSet {
    generation: string;
    index: number;
    key: string | null;
    /** How many there are; `candidates` holds at most 500 of them. */
    total: number;
    /** Unreviewed, editable, same key, newest first. */
    candidates: ReviewTxn[];
    /** The candidates' indices that are also in the run's queue. */
    inQueue: number[];
    precedent: Precedent | null;
}

/** `POST /api/transactions/batch` → 200: every edited transaction as it now reads, in request order. */
export interface BatchResult {
    generation: string;
    /** Their `flags` are empty: the new snapshot's problem list arrives with the next revalidation. */
    edited: ReviewTxn[];
}

/** One payee the journal uses (description completion, D15). */
export interface PayeeUse {
    payee: string;
    count: number;
    last: ISODate;
}

/** `GET /api/payees`. */
export interface PayeeList {
    generation: string;
    /** Most used first. */
    payees: PayeeUse[];
}

/** One transaction tag name and its commonest values (tag completion, D15). */
export interface TagUse {
    name: string;
    count: number;
    values: {value: string; count: number}[];
}

/** `GET /api/tags`. */
export interface TagList {
    generation: string;
    tags: TagUse[];
}

/** `review` in `GET /api/journal/summary`: the toolbar badges (D18). They ignore the filter. */
export interface ReviewCounts {
    /** How many Review Unknowns would queue. */
    unknown: number;
    /** How many Review Recents would queue (capped at the limit asked for). */
    unreviewedRecent: number;
    /** Unknowns left out because their file cannot be written. */
    notEditable: number;
    /** The same for Recents (the two overlap, so they are not summed). */
    notEditableRecent: number;
}

/**
 * The review card's editable fields (D8). Unlike the popup's `TxnForm`, the
 * header's tags are chips (`tags`) and `comment` is only its FREE TEXT, which
 * is what lets a save write them surgically (D9).
 */
export interface ReviewForm {
    date: ISODate;
    description: string;
    /** The same-line header comment's free text (not its tags, not its continuation lines). */
    comment: string;
    /** The header's tags as `[name, value]`, in order; hidden and machine tags left out. */
    tags: [string, string][];
    /** The postings the file writes (generated ones are read-only and not here), as `txnToForm` builds them. */
    postings: PostingForm[];
}
