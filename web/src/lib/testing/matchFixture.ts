// Transfer matching (plan 35) as the engine serves it: transfer halves through
// a clearing account, proposals for `GET /api/matches`, and collapsed transfer
// rows for `/api/journal/rows` — so a component test drives the real stores
// with literal wire JSON.

import type {Transaction} from "#lib/domain/types.js";
import {wireRow} from "./journalViewFixture";
import {reviewTxnWire} from "./reviewFixture";

const STYLE = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null} as const;
const usd = (cents: number) => ({commodity: "$", qty: {m: BigInt(cents), p: 2}, style: STYLE});

/** The clearing account every half here goes through. */
export const CLEARING = "transfers:cc";

/**
 * One half of a transfer: `real` moves `cents` (negative: money out), and the
 * clearing posting (written position 1) balances it, carrying `tags`.
 */
export function transferHalf(index: number, date: string, description: string, real: string, cents: number, tags: [string, string][] = []): Transaction {
    return {
        index,
        date,
        status: "cleared",
        description,
        code: "",
        comment: "",
        tags: [],
        postings: [
            {account: real, amounts: [usd(cents)], status: "unmarked", comment: "", tags: []},
            {account: CLEARING, amounts: [usd(-cents)], status: "unmarked", comment: tags.map(([name, value]) => `${name}:${value}`).join(", "), tags},
        ],
    };
}

/**
 * `transferHalf` as the engine serves it once the account declares
 * `clearing:`: the clearing posting's tags carry the inherited
 * `clearing` tag (not written in its comment) ahead of its own `tags`.
 */
export function clearingHalf(index: number, date: string, description: string, real: string, cents: number, tags: [string, string][] = []): Transaction {
    const txn = transferHalf(index, date, description, real, cents, tags);
    const [first, clearing] = txn.postings;
    if (first === undefined || clearing === undefined) throw new Error("a transfer half has two postings");
    return {...txn, postings: [first, {...clearing, tags: [...tags, ["clearing", ""]]}]};
}

const lastSegment = (account: string): string => account.slice(account.lastIndexOf(":") + 1);
const realOf = (txn: Transaction): string => txn.postings[0]?.account ?? "";

/**
 * A proposal half: the review transaction plus its clearing posting's written
 * position and its file, one per feed as an import writes them
 * (`checking.journal` for a half on `assets:checking`).
 */
export function matchHalfWire(txn: Transaction, fingerprint?: string): Record<string, unknown> {
    return {...reviewTxnWire(txn, {unknown: false, fingerprint}), posting: 1, file: `${lastSegment(realOf(txn))}.journal`};
}

/** A proposal for `GET /api/matches`: `out` is the source (its real posting negative). */
export function proposalWire(out: Transaction, into: Transaction, overrides: Record<string, unknown> = {}): Record<string, unknown> {
    const [a, b] = out.date <= into.date ? [out, into] : [into, out];
    return {
        a: matchHalfWire(a),
        b: matchHalfWire(b),
        gapDays: Math.abs(Date.parse(into.date) - Date.parse(out.date)) / 86_400_000,
        score: 2,
        clear: true,
        ambiguous: false,
        from: {index: out.index, account: realOf(out), label: lastSegment(realOf(out))},
        to: {index: into.index, account: realOf(into), label: lastSegment(realOf(into))},
        alternatives: [],
        group: [],
        ...overrides,
    };
}

/**
 * A suggested combination (A4) for `GET /api/matches`: `single` pays into the
 * clearing account what `members` (2–4, nearest first) take out. Never clear.
 */
export function combinationWire(single: Transaction, members: Transaction[], overrides: Record<string, unknown> = {}): Record<string, unknown> {
    const first = members[0];
    if (first === undefined) throw new Error("a combination needs members");
    const days = (txn: Transaction) => Math.abs(Date.parse(txn.date) - Date.parse(single.date)) / 86_400_000;
    const [out, into] = (single.postings[0]?.amounts[0]?.qty.m ?? 0n) < 0n ? [single, first] : [first, single];
    return {
        ...proposalWire(out, into),
        a: matchHalfWire(single),
        b: matchHalfWire(first),
        gapDays: Math.max(...members.map(days)),
        score: 0,
        clear: false,
        group: members.map((member) => matchHalfWire(member)),
        ...overrides,
    };
}

/** A `GET /api/matches` body over `proposals`, with counts that agree. */
export function matchesPageWire(proposals: Record<string, unknown>[], overrides: Record<string, unknown> = {}): Record<string, unknown> {
    const clear = proposals.filter((proposal) => proposal.clear === true).length;
    return {generation: "fake-1", clearing: true, counts: {clear, check: proposals.length - clear}, older: 0, proposals, ...overrides};
}

/** A collapsed transfer row anchored on `anchor` (one of the two halves), as `/api/journal/rows` serves it. */
export function transferRowWire(matchId: string, out: Transaction, into: Transaction, anchor: Transaction = out): Record<string, unknown> {
    const halves = [out, into]
        .sort((x, y) => (x.date < y.date ? -1 : x.date > y.date ? 1 : x.index - y.index))
        .map((half) => ({...wireRow(half), linked: true}));
    return {
        ...wireRow(anchor),
        linked: true,
        transfer: {
            matchId,
            halves,
            from: {index: out.index, account: realOf(out), label: lastSegment(realOf(out))},
            to: {index: into.index, account: realOf(into), label: lastSegment(realOf(into))},
        },
    };
}
