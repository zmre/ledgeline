// Every decision the Find Matches overlay and the journal's transfer rows make
// (plan 35, Phase 3), as pure functions: no Svelte, no DOM, no fetch, so the
// node vitest project covers them.
//
// A proposal half names its clearing posting by its position among the
// postings the FILE writes (`writtenPostings`), which is how every write
// names it back.

import type {WireMatchHalf} from "#lib/api/native.js";
import {add, cmp, dec, formatAmount, neg} from "#lib/domain/money.js";
import type {Amount, Posting, Transaction} from "#lib/domain/types.js";
import {writtenPostings} from "#lib/journal/rowModel.js";
import {daysBetween} from "#lib/reports/periods.js";
import type {MatchHalf, MatchPartner, MatchProposal} from "./types";

/** The posting tag a link writes (D1), and a rejection (D12). Fixed names in v1. */
export const MATCH_TAG = "match";
export const NOMATCH_TAG = "nomatch";
/** `match:ignore`: never propose this posting (D1). */
export const IGNORE_VALUE = "ignore";
/** The account tag that makes a clearing account (D2); a posting's tags carry it inherited. */
export const CLEARING_TAG = "clearing";
/** The most postings one link may name (`POST /api/matches/link` takes 2–16). */
export const MAX_LINK_MEMBERS = 16;

/** `liabilities:amex` → `amex`. */
export function lastSegment(account: string): string {
    return account.slice(account.lastIndexOf(":") + 1);
}

/** One half as the card shows it: its clearing leg, and the real account on the other side. */
export interface HalfView {
    /** The clearing posting, or null when `posting` names none (never, from a fresh proposal). */
    clearing: Posting | null;
    /** The first other written posting: the real account the money moved from or to. */
    real: Posting | null;
    /** The real posting's amount as written (signed), or the clearing amount negated when the real leg's is elided. */
    realAmount: Amount | null;
}

export function halfView(half: MatchHalf): HalfView {
    const written = writtenPostings(half.txn);
    const clearing = written[half.posting] ?? null;
    const real = written.find((_, at) => at !== half.posting) ?? null;
    const own = real?.amounts.length === 1 ? (real.amounts[0] ?? null) : null;
    const fromClearing =
        clearing?.amounts.length === 1 && clearing.amounts[0] !== undefined ? {...clearing.amounts[0], qty: neg(clearing.amounts[0].qty)} : null;
    return {clearing, real, realAmount: own ?? fromClearing};
}

/** A choice on the card (D9): the proposal itself first, then each alternative, nearest first. */
export interface MatchChoice {
    a: MatchHalf;
    b: MatchHalf;
    gapDays: number;
    score: number;
    /** The half it does not share with the proposal is in another proposal. Always false for the proposal itself. */
    taken: boolean;
}

export function choicesOf(proposal: MatchProposal): MatchChoice[] {
    const own: MatchChoice = {a: proposal.a, b: proposal.b, gapDays: proposal.gapDays, score: proposal.score, taken: false};
    return [own, ...proposal.alternatives.map((alt) => ({a: alt.a, b: alt.b, gapDays: alt.gapDays, score: alt.score, taken: alt.taken}))];
}

/** A half's identity across a re-read: its text (fingerprint) and its posting. Indices move on every write; untouched text does not. */
export function halfKey(half: MatchHalf): string {
    return `${half.fingerprint ?? `#${half.txn.index}`}/${half.posting}`;
}

/** A pair's identity across a re-read, either way round (Skip remembers it for the run). */
export function pairKey(a: MatchHalf, b: MatchHalf): string {
    return [halfKey(a), halfKey(b)].sort().join("|");
}

/** The half of `choice` that is not shared with the proposal: what a radio option names. */
export function otherHalf(choice: MatchChoice, proposal: MatchProposal): MatchHalf {
    const shared = (half: MatchHalf): boolean => halfKey(half) === halfKey(proposal.a) || halfKey(half) === halfKey(proposal.b);
    return shared(choice.a) && !shared(choice.b) ? choice.b : choice.a;
}

/** The card's header (D9): "Checking → Amex" and the amount moved. */
export interface TransferHeader {
    from: string;
    to: string;
    /** The full account names, for a title. */
    fromAccount: string | null;
    toAccount: string | null;
    amount: string | null;
}

/** Short labels for two accounts: last segments, or the full names when those collide (D11). */
export function endLabels(from: string | null, to: string | null): [string, string] {
    const short = (account: string | null): string => (account === null ? "?" : lastSegment(account));
    if (from !== null && to !== null && short(from) === short(to) && from !== to) return [from, to];
    return [short(from), short(to)];
}

/** The amount a half's clearing posting carries, unsigned, formatted. */
function movedAmount(half: MatchHalf): string | null {
    const amount = halfView(half).clearing?.amounts[0];
    if (amount === undefined) return null;
    return formatAmount(amount.qty.m < 0n ? {...amount, qty: neg(amount.qty)} : amount);
}

/**
 * The header for `choice`. For the proposal itself the engine named the ends
 * (`from`/`to`); for an alternative they are worked out the same way: the
 * source is the half whose real posting is negative — whose clearing leg is
 * positive.
 */
export function headerOf(choice: MatchChoice, proposal: MatchProposal): TransferHeader {
    const amount = movedAmount(choice.a);
    if (choice.a === proposal.a && choice.b === proposal.b) {
        return {from: proposal.from.label ?? "?", to: proposal.to.label ?? "?", fromAccount: proposal.from.account, toAccount: proposal.to.account, amount};
    }
    const aClearing = halfView(choice.a).clearing?.amounts[0];
    const [source, dest] = aClearing !== undefined && aClearing.qty.m > 0n ? [choice.a, choice.b] : [choice.b, choice.a];
    const fromAccount = halfView(source).real?.account ?? null;
    const toAccount = halfView(dest).real?.account ?? null;
    const [from, to] = endLabels(fromAccount, toAccount);
    return {from, to, fromAccount, toAccount, amount};
}

/** "same day", "1 day apart", "12 days apart". */
export function gapLabel(days: number): string {
    if (days === 0) return "same day";
    return `${days} ${days === 1 ? "day" : "days"} apart`;
}

/** What a write names a half by, with its new description when it was edited (D9 "Link and Save"). */
export function wireHalf(half: MatchHalf, description?: string): WireMatchHalf {
    const out: WireMatchHalf = {index: half.txn.index, posting: half.posting, fingerprint: half.fingerprint ?? ""};
    const next = description?.trim();
    if (next !== undefined && next !== half.txn.description) out.description = next;
    return out;
}

/** The written positions of `txn`'s postings that carry `name:value`. */
export function postingsTagged(txn: Transaction, name: string, value: string): number[] {
    return writtenPostings(txn).flatMap((posting, at) => (posting.tags.some(([tag, tagged]) => tag === name && tagged === value) ? [at] : []));
}

/** Every `nomatch` value `txn`'s written postings carry (D12), once each, in order. */
export function rejectionsOf(txn: Transaction): string[] {
    const values = writtenPostings(txn).flatMap((posting) => posting.tags.filter(([tag]) => tag === NOMATCH_TAG).map(([, value]) => value));
    return [...new Set(values)];
}

/**
 * The `since` scope (`JOURNAL_ID:COUNT`) of an import's Find Matches (D9
 * "Import"): the transactions a CSV commit appended to the end of
 * `journalId`. Null — Find Matches over the whole journal — when nothing was
 * written to a journal, or once a re-sort has moved them from the file's end.
 */
export function importSince(journalId: string | null, imported: number, resorted: boolean): string | null {
    return journalId === null || imported <= 0 || resorted ? null : `${journalId}:${imported}`;
}

// -- Amendment A: a matcher for one transaction --------------------------------

/**
 * The written positions of `txn`'s postings the matcher could still pair (D3,
 * as far as the transaction alone can tell): on a clearing account (a
 * posting's tags carry the account's inherited `clearing:` tag), real, with
 * no `match` tag (`match:ignore` included), one amount, no cost and no
 * assertion. The lookback does not apply: Find match looks at any date (A2).
 * What the journal row and the editor read to offer ⇄ Find match without
 * asking the engine.
 */
export function findableClearing(txn: Transaction): number[] {
    return writtenPostings(txn).flatMap((posting, at) => {
        const clearing = posting.tags.some(([tag]) => tag === CLEARING_TAG);
        const tagged = posting.tags.some(([tag]) => tag === MATCH_TAG);
        const real = (posting.type ?? "regular") === "regular";
        const plain = posting.amounts.length === 1 && posting.amounts[0]?.cost === undefined && posting.balanceAssertion === undefined;
        return clearing && !tagged && real && plain ? [at] : [];
    });
}

/** Every link id (`match:ID`, not `ignore`) `txn`'s written postings carry, once each, in order. */
export function linkIdsOf(txn: Transaction): string[] {
    const values = writtenPostings(txn).flatMap((posting) =>
        posting.tags.filter(([tag, value]) => tag === MATCH_TAG && value !== IGNORE_VALUE).map(([, value]) => value)
    );
    return [...new Set(values)];
}

/** The signed amount a half's clearing posting carries, or null when it has not exactly one. */
export function clearingAmount(half: MatchHalf): Amount | null {
    const amounts = halfView(half).clearing?.amounts;
    return amounts?.length === 1 ? (amounts[0] ?? null) : null;
}

/** What moved on the real side: the clearing amount negated, signed (a list line's amount). */
export function flowAmount(half: MatchHalf): Amount | null {
    const amount = clearingAmount(half);
    return amount === null ? null : {...amount, qty: neg(amount.qty)};
}

const magnitude = (amount: Amount): Amount => (amount.qty.m < 0n ? {...amount, qty: neg(amount.qty)} : amount);

/** The focused view's running total (A2): the picked postings against the one being matched. */
export interface SelectionTotal {
    /** "$2,247.50 of $2,247.50". */
    label: string;
    /** The picked postings cancel the focused one exactly: Link is allowed. */
    balanced: boolean;
    /** Why they cannot be linked as they are, beyond not adding up; null when they can or nothing is picked. */
    problem: string | null;
}

/**
 * Sum `picked`'s clearing amounts against `focused`'s, exactly (decimals,
 * never floats). Balanced when something is picked, every amount is in the
 * focused commodity, each posting is in its own transaction, there are at
 * most `MAX_LINK_MEMBERS`, and the whole set sums to zero.
 */
export function selectionTotal(focused: MatchHalf, picked: readonly MatchHalf[]): SelectionTotal {
    const own = clearingAmount(focused);
    if (own === null) return {label: "", balanced: false, problem: "This posting has no single amount to match."};
    const target = {...own, qty: neg(own.qty)};
    let sum = dec(0, 0);
    let problem: string | null = null;
    for (const half of picked) {
        const amount = clearingAmount(half);
        if (amount === null || amount.commodity !== own.commodity) problem = "Every posting in a link must be in the same commodity.";
        else sum = add(sum, amount.qty);
    }
    const txns = new Set([focused.txn.index, ...picked.map((half) => half.txn.index)]);
    if (problem === null && txns.size !== picked.length + 1) problem = "Two postings of one transaction can't be in the same link.";
    if (problem === null && picked.length + 1 > MAX_LINK_MEMBERS) problem = `A link can hold at most ${MAX_LINK_MEMBERS} postings.`;
    const label = `${formatAmount(magnitude({...target, qty: sum}))} of ${formatAmount(magnitude(target))}`;
    return {label, balanced: picked.length > 0 && problem === null && cmp(sum, target.qty) === 0, problem};
}

/** A focused half: the transaction asked about, as a write names its posting. It has no file name (the view does not show it). */
export function focusedHalf(transaction: Omit<MatchHalf, "posting" | "file">, posting: number): MatchHalf {
    return {...transaction, posting, file: ""};
}

/**
 * The link a focused selection writes: a pair (`{a, b}`) for one partner, a
 * group (`{members}`, A3) for several. The focused posting comes first.
 */
export function focusLinkBody(focused: MatchHalf, picked: readonly MatchHalf[]): {a: WireMatchHalf; b: WireMatchHalf} | {members: WireMatchHalf[]} {
    const [only] = picked;
    if (picked.length === 1 && only !== undefined) return {a: wireHalf(focused), b: wireHalf(only)};
    return {members: [wireHalf(focused), ...picked.map((half) => wireHalf(half))]};
}

/** Every posting a combination links, as halves, the single first. */
export function combinationMembers(proposal: MatchProposal): MatchHalf[] {
    return [proposal.a, ...proposal.group];
}

/** Where an exact partner falls outside the settings (A2): "outside the 10-day window", "before the lookback", or null. */
export function partnerFlags(partner: MatchPartner, today: string, window: number, lookback: number): string[] {
    const flags: string[] = [];
    if (partner.gapDays > window) flags.push(`outside the ${window}-day window`);
    if (lookback > 0 && daysBetween(partner.txn.date, today) > lookback) flags.push("before the lookback");
    return flags;
}

/** The lookback presets of the ▾ (A1), in days; 0 is All time. */
export const LOOKBACK_PRESETS: readonly {days: number; label: string}[] = [
    {days: 90, label: "90 days"},
    {days: 365, label: "1 year"},
    {days: 730, label: "2 years"},
    {days: 0, label: "All time"},
];

/** "12 more older than the lookback" (A1), or null when there are none. */
export function olderLabel(older: number): string | null {
    return older > 0 ? `${older} more older than the lookback` : null;
}
