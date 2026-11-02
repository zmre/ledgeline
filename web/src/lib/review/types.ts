// Domain types for transaction review (plan 32). Pure TS: no Svelte/DOM
// imports. Decoded from the wire by `api/reviewDecode.ts`; the model behind
// them lives in `crates/ledgeline-core/src/review.rs` and the routes in
// `crates/ledgeline-server/src/review_api.rs`.

import type {Dec} from "#lib/domain/money.js";
import type {ISODate, TxnStatus} from "#lib/domain/types.js";
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
    /**
     * A recurring charge's usual amount and funding account, for a
     * transaction that does not exist yet (D36: "Netflix · $15.49 from
     * Visa"). Only `GET /api/review/suggest` sends it; absent otherwise.
     */
    recurring?: RecurringFill;
}

/** What an add form fills in from a recurring charge (D36). */
export interface RecurringFill {
    /** The typical charge, positive, as the category leg carries it. */
    amount: {commodity: string; qty: Dec};
    /** The account the newest such charge was paid from. */
    account: string;
}

/** `GET /api/review/suggest` — a queue item's suggestions for a description alone (D36). */
export interface DescriptionSuggestions {
    generation: string;
    /** The description's similarity key, or null (then there are no suggestions). */
    key: string | null;
    suggestions: Suggestion[];
}

/** One queued card. */
export interface QueueItem extends ReviewTxn {
    /** The normalised payee similar transactions share (D10), or null when the description has none. */
    key: string | null;
    /** Best first, at most 3: the chip and its runners-up. */
    suggestions: Suggestion[];
}

/** `GET /api/review/item/{index}` — one transaction as a queue item, for the card's single mode (D31). */
export interface ReviewItem {
    generation: string;
    item: QueueItem;
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
    /** The secondary date (`DATE=DATE2`), `YYYY-MM-DD`; `""` when there is none. Saved with `setDate2`. */
    date2: string;
    /** The transaction's status. Saved with `setStatus`. */
    status: TxnStatus;
    /** The `(CODE)`; `""` when there is none. Saved with `setCode`. */
    code: string;
    description: string;
    /**
     * The whole transaction comment's FREE TEXT, one line per comment line
     * that has any (`commentFreeText`): the header's same-line comment and the
     * `;` lines above the first posting, tags left out. Saved with
     * `setTxnComment`, which keeps every tag on its own line.
     */
    comment: string;
    /** The header's tags as `[name, value]`, in order; hidden and machine tags left out. */
    tags: [string, string][];
    /**
     * The postings the file writes (generated ones are read-only and not
     * here), as `txnToForm` builds them. An elided leg's `amount` is `""`.
     * A row's `comment` is its whole comment (`setPostingComment`); setting a
     * row's `balanceAssertion` to null removes it (`removePostingAssertion`).
     * Any other change to a row — amount, commodity, status — or a row added
     * or removed is a re-render (`replace`).
     */
    postings: PostingForm[];
}
