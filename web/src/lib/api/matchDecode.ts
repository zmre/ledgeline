// Wire → domain decoders for the transfer-matching routes (plan 35;
// `crates/ledgeline-server/src/match_api.rs`).
//
// Same discipline as reviewDecode.ts, whose review-transaction decoder every
// half goes through: every field is checked, a missing or mistyped one throws
// `ApiShapeError`, and decoded objects are frozen.

import type {
    LinkAllResult,
    MatchAlternative,
    MatchesPage,
    MatchHalf,
    MatchLink,
    MatchPartner,
    MatchProposal,
    MatchWriteResult,
    PostingMatches,
    TransactionMatches,
} from "#lib/matching/types.js";
import type {ReviewTxn} from "#lib/review/types.js";
import {ApiShapeError} from "./client";
import {arr, bool, decodeTransferEnd, num, obj, str, strings} from "./journalViewDecode";
import {decodeReviewTxn} from "./reviewDecode";

function count(raw: unknown, context: string): number {
    const n = num(raw, context);
    if (!Number.isSafeInteger(n) || n < 0) throw new ApiShapeError(`${context}: invalid count ${n}`);
    return n;
}

function integer(raw: unknown, context: string): number {
    const n = num(raw, context);
    if (!Number.isSafeInteger(n)) throw new ApiShapeError(`${context}: invalid integer ${n}`);
    return n;
}

/** A pair's half: a review transaction plus its clearing posting's written position and its file's name. */
function decodeHalf(raw: unknown, context: string): MatchHalf {
    const value = obj(raw, context);
    const txn = decodeReviewTxn(value, context);
    const posting = count(value.posting, `${context}.posting`);
    if (posting >= txn.txn.postings.length) throw new ApiShapeError(`${context}: posting ${posting} is not one of the transaction's`);
    return Object.freeze({...txn, posting, file: str(value.file, `${context}.file`)});
}

function decodeAlternative(raw: unknown, context: string): MatchAlternative {
    const value = obj(raw, context);
    return Object.freeze({
        a: decodeHalf(value.a, `${context}.a`),
        b: decodeHalf(value.b, `${context}.b`),
        gapDays: count(value.gapDays, `${context}.gapDays`),
        score: integer(value.score, `${context}.score`),
        taken: bool(value.taken, `${context}.taken`),
    });
}

function decodeProposal(raw: unknown, context: string): MatchProposal {
    const value = obj(raw, context);
    const a = decodeHalf(value.a, `${context}.a`);
    const b = decodeHalf(value.b, `${context}.b`);
    if (a.txn.index === b.txn.index) throw new ApiShapeError(`${context}: both halves are transaction #${a.txn.index}`);
    const from = decodeTransferEnd(value.from, `${context}.from`);
    const to = decodeTransferEnd(value.to, `${context}.to`);
    const halves = new Set([a.txn.index, b.txn.index]);
    if (from.index === to.index || !halves.has(from.index) || !halves.has(to.index)) {
        throw new ApiShapeError(`${context}: from and to must name the two halves`);
    }
    const alternatives = arr(value.alternatives, `${context}.alternatives`).map((alt, i) => decodeAlternative(alt, `${context}.alternatives[${i}]`));
    const group = arr(value.group, `${context}.group`).map((half, i) => decodeHalf(half, `${context}.group[${i}]`));
    if (group.length > 0) {
        // A combination (A4): `b` is the group's first, and every posting is in its own transaction.
        const txns = new Set([a.txn.index, ...group.map((half) => half.txn.index)]);
        if (group.length < 2 || group[0]?.txn.index !== b.txn.index || group[0]?.posting !== b.posting || txns.size !== group.length + 1) {
            throw new ApiShapeError(`${context}: a combination's group must be 2+ halves in their own transactions, starting with b`);
        }
    }
    return Object.freeze({
        a,
        b,
        gapDays: count(value.gapDays, `${context}.gapDays`),
        score: integer(value.score, `${context}.score`),
        clear: bool(value.clear, `${context}.clear`),
        ambiguous: bool(value.ambiguous, `${context}.ambiguous`),
        from,
        to,
        alternatives: Object.freeze(alternatives) as MatchAlternative[],
        group: Object.freeze(group) as MatchHalf[],
    });
}

/**
 * `GET /api/matches` → the proposals. `counts` must agree with them: a body
 * that says otherwise is refused rather than badging a number the overlay
 * cannot show.
 */
export function decodeMatches(raw: unknown): MatchesPage {
    const body = obj(raw, "matches");
    const proposals = arr(body.proposals, "matches proposals").map((p, i) => decodeProposal(p, `matches proposals[${i}]`));
    const counts = obj(body.counts, "matches counts");
    const clear = count(counts.clear, "matches counts.clear");
    const check = count(counts.check, "matches counts.check");
    const clearHere = proposals.filter((proposal) => proposal.clear).length;
    if (clear !== clearHere || check !== proposals.length - clearHere) throw new ApiShapeError("matches: counts disagree with the proposals");
    return Object.freeze({
        generation: str(body.generation, "matches generation"),
        clearing: bool(body.clearing, "matches clearing"),
        counts: Object.freeze({clear, check}),
        older: count(body.older, "matches older"),
        proposals: Object.freeze(proposals) as MatchProposal[],
    });
}

function decodePartner(raw: unknown, context: string): MatchPartner {
    const value = obj(raw, context);
    return Object.freeze({...decodeHalf(value, context), gapDays: count(value.gapDays, `${context}.gapDays`)});
}

function decodePartners(raw: unknown, context: string): MatchPartner[] {
    return Object.freeze(arr(raw, context).map((partner, i) => decodePartner(partner, `${context}[${i}]`))) as MatchPartner[];
}

function decodePostingMatches(raw: unknown, context: string, postings: number): PostingMatches {
    const value = obj(raw, context);
    const posting = count(value.posting, `${context}.posting`);
    if (posting >= postings) throw new ApiShapeError(`${context}: posting ${posting} is not one of the transaction's`);
    const combinations = arr(value.combinations, `${context}.combinations`).map((p, i) => decodeProposal(p, `${context}.combinations[${i}]`));
    if (combinations.some((proposal) => proposal.group.length === 0)) throw new ApiShapeError(`${context}: a combination without a group`);
    return Object.freeze({
        posting,
        exact: decodePartners(value.exact, `${context}.exact`),
        combinations: Object.freeze(combinations) as MatchProposal[],
        nearby: decodePartners(value.nearby, `${context}.nearby`),
    });
}

function decodeLink(raw: unknown, context: string): MatchLink {
    const value = obj(raw, context);
    const members = arr(value.members, `${context}.members`).map((half, i) => decodeHalf(half, `${context}.members[${i}]`));
    return Object.freeze({
        matchId: str(value.matchId, `${context}.matchId`),
        broken: bool(value.broken, `${context}.broken`),
        members: Object.freeze(members) as MatchHalf[],
    });
}

/** `GET /api/matches/for/{index}` → the matcher for one transaction (A2). */
export function decodeTransactionMatches(raw: unknown): TransactionMatches {
    const body = obj(raw, "transaction matches");
    const transaction = decodeReviewTxn(body.transaction, "transaction matches transaction");
    const postings = arr(body.postings, "transaction matches postings").map((p, i) =>
        decodePostingMatches(p, `transaction matches postings[${i}]`, transaction.txn.postings.length)
    );
    const links = arr(body.links, "transaction matches links").map((link, i) => decodeLink(link, `transaction matches links[${i}]`));
    return Object.freeze({
        generation: str(body.generation, "transaction matches generation"),
        clearing: bool(body.clearing, "transaction matches clearing"),
        transaction,
        postings: Object.freeze(postings) as PostingMatches[],
        links: Object.freeze(links) as MatchLink[],
    });
}

/** `POST /api/matches/{link,reject,unlink,forget,ignore}` → the id and every touched transaction. */
export function decodeMatchWrite(raw: unknown): MatchWriteResult {
    const body = obj(raw, "match write");
    const edited = arr(body.edited, "match write edited").map((txn, i) => decodeReviewTxn(txn, `match write edited[${i}]`));
    return Object.freeze({
        generation: str(body.generation, "match write generation"),
        id: str(body.id, "match write id"),
        edited: Object.freeze(edited) as ReviewTxn[],
    });
}

/** `POST /api/matches/link-all` → how many pairs were linked and their ids. */
export function decodeLinkAll(raw: unknown): LinkAllResult {
    const body = obj(raw, "link all");
    const linked = count(body.linked, "link all linked");
    const matchIds = strings(body.matchIds, "link all matchIds");
    if (matchIds.length !== linked) throw new ApiShapeError("link all: one id per linked pair expected");
    return Object.freeze({generation: str(body.generation, "link all generation"), linked, matchIds: Object.freeze(matchIds) as string[]});
}
