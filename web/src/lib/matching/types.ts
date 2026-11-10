// Domain types for transfer matching (plan 35). Pure TS: no Svelte/DOM
// imports. Decoded from the wire by `api/matchDecode.ts`; the model behind
// them lives in `crates/ledgeline-core/src/matching.rs` and the routes in
// `crates/ledgeline-server/src/match_api.rs`.

import type {TransferEnd} from "#lib/journalView/types.js";
import type {ReviewTxn} from "#lib/review/types.js";

export type {TransferEnd, TransferRow} from "#lib/journalView/types.js";

/**
 * One half of a proposed pair: the transaction as a review route serves it
 * (row, markers, fingerprint) plus `posting`, its clearing posting's position
 * among the postings the FILE writes. `{index: txn.index, posting,
 * fingerprint}` is exactly what a write names it by.
 */
export interface MatchHalf extends ReviewTxn {
    posting: number;
    /** The name of the file the transaction is written in, without its directory (the card's last line); "" when unknown. */
    file: string;
}

/** Another legal pair sharing a half with a proposal: the card's radio choices (D9). */
export interface MatchAlternative {
    a: MatchHalf;
    b: MatchHalf;
    gapDays: number;
    /** The D7 score: a tie-breaker. */
    score: number;
    /** The half it does not share with the proposal is in another proposal. */
    taken: boolean;
}

/** One proposed pair (D4, D6, D7). */
export interface MatchProposal {
    /** The earlier half (by effective date, then file order). */
    a: MatchHalf;
    b: MatchHalf;
    gapDays: number;
    score: number;
    /** Unambiguous with a non-negative score: the "clear" label, and in "Link all clear". */
    clear: boolean;
    /** A half had another partner with the same gap and score; `alternatives` lists them. */
    ambiguous: boolean;
    /** The header's "Checking → Amex": the source end and the destination. */
    from: TransferEnd;
    to: TransferEnd;
    /** Nearest first. */
    alternatives: MatchAlternative[];
    /**
     * Empty for a pair. For a suggested combination (Amendment A4): the many
     * side, 2–4 halves in different transactions summing to exactly `a`'s
     * negation, nearest first. Then `a` is the single posting (not
     * necessarily the earlier), `b` is `group[0]`, `gapDays` is the widest
     * gap, and the proposal is never `clear` — so never in "Link all". Link
     * it with `{members: [a, ...group]}`.
     */
    group: MatchHalf[];
}

/** `GET /api/matches`. */
export interface MatchesPage {
    generation: string;
    /** Whether any account declares `clearing:` (D2). False → Find Matches is disabled. */
    clearing: boolean;
    /** Over the proposals returned (so scoped by `since`). */
    counts: {clear: number; check: number};
    /** How many more proposals there would be with no lookback (A1): "N more older than the lookback". 0 when the lookback is 0. */
    older: number;
    /** Earliest first. */
    proposals: MatchProposal[];
}

/** `POST /api/matches/{link,reject,unlink,forget,ignore}` → 200. */
export interface MatchWriteResult {
    generation: string;
    /** The id written (`match`/`nomatch` value, or "ignore"), or the one removed. */
    id: string;
    /** Every transaction the write touched, in file order, with its new fingerprint. Their `flags` are empty. */
    edited: ReviewTxn[];
}

/** `POST /api/matches/link-all` → 200. */
export interface LinkAllResult {
    generation: string;
    linked: number;
    /** Each pair's new `match` id, in request order. */
    matchIds: string[];
}

/** Another posting offered for one of a transaction's (A2): a half plus its gap in days. */
export interface MatchPartner extends MatchHalf {
    gapDays: number;
}

/** What the matcher offers one unlinked clearing posting of a transaction (A2). */
export interface PostingMatches {
    /** The posting's position among the postings the FILE writes. */
    posting: number;
    /** Same account, exact opposite amount, ANY gap (no window, no lookback), not rejected with it: nearest first, at most 20. */
    exact: MatchPartner[];
    /** The suggested combinations it is in (as the single posting or a member), shaped as proposals with a non-empty `group`. */
    combinations: MatchProposal[];
    /** Same account and commodity, opposite sign, within ± the window: nearest first, at most 50 — for picking a group by hand. */
    nearby: MatchPartner[];
}

/** A `match` id on one of the transaction's postings: what Unlink names. */
export interface MatchLink {
    matchId: string;
    /** Not a valid link (the Problems "broken match"). */
    broken: boolean;
    /** Every posting carrying the id, in file order, the transaction's own included. */
    members: MatchHalf[];
}

/** `GET /api/matches/for/{index}` (A2). */
export interface TransactionMatches {
    generation: string;
    /** Whether any account declares `clearing:` (D2). */
    clearing: boolean;
    /** The transaction asked about, as a review route serves it. */
    transaction: ReviewTxn;
    /** One entry per unlinked, not ignored clearing posting, in posting order; empty when it has none. */
    postings: PostingMatches[];
    /** The links its postings carry, in id order. */
    links: MatchLink[];
}
