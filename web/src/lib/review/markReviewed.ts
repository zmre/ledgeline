// Mark one transaction reviewed straight from the journal list (plan 42: the
// narrow card's swipe right and its long-press menu), without opening the
// review card.
//
// The same `reviewed:<date>` tag the card's "Reviewed" box writes (plan 32),
// through the same batch route. Journal rows carry no fingerprint, so the
// review item route is asked for it first, inside the one write, as the
// matcher's unlink does (stores/matches.svelte.ts `taggedHalf`).

import {REVIEWED_TAG} from "#lib/api/editMapping.js";
import type {LedgelineApi, WireEditOp} from "#lib/api/native.js";
import {decodeBatchResult, decodeReviewItem} from "#lib/api/reviewDecode.js";
import type {ISODate} from "#lib/domain/types.js";
import type {BatchResult} from "#lib/review/types.js";
import {editing, type WriteResult} from "#lib/stores/editing.svelte.js";

/** The one op that marks a transaction reviewed on `today`. */
export function reviewedOps(today: ISODate): WireEditOp[] {
    return [{op: "setTxnTag", name: REVIEWED_TAG, value: today}];
}

/** The write itself, given an API: fetch the fingerprint, then tag. */
export async function markReviewedWith(api: LedgelineApi, index: number, today: ISODate): Promise<BatchResult> {
    const {item} = decodeReviewItem(await api.reviewItem(index, today));
    if (item.fingerprint === null) throw new Error(`Transaction #${index} cannot be edited.`);
    return decodeBatchResult(await api.batchTransactions({edits: [{index, fingerprint: item.fingerprint, ops: reviewedOps(today)}]}));
}

/** Mark transaction `index` reviewed on `today`; the journal is re-read on success. */
export function markReviewed(index: number, today: ISODate): Promise<WriteResult<BatchResult>> {
    return editing.write((api) => markReviewedWith(api, index, today));
}
