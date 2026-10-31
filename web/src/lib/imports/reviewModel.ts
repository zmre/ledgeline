// The decisions behind reviewing an import before it is written (plan 32 D23,
// D25, D30): how a card's successive saves combine into one override, and
// what becomes of the held overrides when the import is regenerated — after a
// rule was saved, a rules file edited in another tab, an alias added.
//
// Pure: no Svelte, no fetch. `stagedEdits.svelte.ts` holds the state and calls
// these; `reviewModel.test.ts` pins them.

import {commentFreeText} from "#lib/api/editMapping.js";
import type {WireEditOp} from "#lib/api/native.js";
import type {Posting, Transaction} from "#lib/domain/types.js";
import type {ReplayReport, StagedOverride, StagedRef, StagedTxn} from "./reviewTypes";

/** The key an override is held by: the ordinal of a CSV row, the Trans # of a QuickBooks one. */
export function refKey(ref: StagedRef): string {
    return ref.kind === "csv" ? `csv:${ref.ordinal}` : `qb:${ref.id}`;
}

/** What one surgical op writes, so a later op on the same thing replaces it. `null` for `replace`. */
function slot(op: WireEditOp): string | null {
    switch (op.op) {
        case "setDescription":
            return "description";
        case "setTxnComment":
            return "comment";
        case "setTxnTag":
            return `tag:${op.name}`;
        case "setPostingAccount":
            return `posting:${op.posting}`;
        case "replace":
            return null;
    }
}

/**
 * The ops of a card saved again: `next` was computed against the card as it
 * read after `existing`, so applied in order the two give what the user sees.
 *
 * - `next` starting with `replace` re-renders the whole transaction from the
 *   form, which already holds every earlier edit: it alone is the answer.
 * - Otherwise both, compacted: one op per thing written (the description, the
 *   comment, each tag, each posting's account), the last one winning, in the
 *   order their winners were written. A leading `replace` stays first — the
 *   engine refuses one anywhere else.
 */
export function mergeOps(existing: readonly WireEditOp[], next: readonly WireEditOp[]): WireEditOp[] {
    if (next[0]?.op === "replace") return [...next];
    const combined = [...existing, ...next];
    const last = new Map<string, number>();
    combined.forEach((op, at) => {
        const key = slot(op);
        if (key !== null) last.set(key, at);
    });
    return combined.filter((op, at) => {
        const key = slot(op);
        return key === null ? at === 0 : last.get(key) === at;
    });
}

/** The postings the file writes — what `setPostingAccount.posting` counts. */
function written(txn: Transaction): Posting[] {
    return txn.postings.filter((posting) => posting.generated === undefined);
}

function firstTag(txn: Transaction, name: string): string | undefined {
    return txn.tags.find(([tag]) => tag === name)?.[1];
}

/** Whether `op` would leave `txn` as it is. A `replace` is never judged a no-op: its content is the whole transaction. */
function isNoOp(op: WireEditOp, txn: Transaction): boolean {
    switch (op.op) {
        case "setDescription":
            return txn.description === op.description;
        case "setTxnComment":
            return commentFreeText(txn.comment) === op.comment;
        case "setTxnTag":
            return op.value === null ? firstTag(txn, op.name) === undefined : firstTag(txn, op.name) === op.value;
        case "setPostingAccount":
            return written(txn)[op.posting]?.account === op.account;
        case "replace":
            return false;
    }
}

/**
 * Everything that differs between two readings of one transaction, in the
 * same vocabulary as {@link slot}, plus `other` for whatever no surgical op
 * writes (the date, the status, an amount, a posting added or removed).
 */
function differences(before: Transaction, after: Transaction): Set<string> {
    const found = new Set<string>();
    if (before.description !== after.description) found.add("description");
    if (commentFreeText(before.comment) !== commentFreeText(after.comment)) found.add("comment");
    for (const name of new Set([...before.tags, ...after.tags].map(([tag]) => tag))) {
        if (firstTag(before, name) !== firstTag(after, name)) found.add(`tag:${name}`);
    }
    const was = written(before);
    const now = written(after);
    was.forEach((posting, at) => {
        if (now[at] !== undefined && now[at].account !== posting.account) found.add(`posting:${at}`);
    });
    const rest = (txn: Transaction, postings: Posting[]): string =>
        JSON.stringify(
            {
                date: txn.date,
                date2: txn.date2 ?? null,
                status: txn.status,
                code: txn.code,
                postings: postings.map((posting) => ({...posting, account: null})),
            },
            // Amounts are `Dec`s with bigint mantissas.
            (_, value: unknown) => (typeof value === "bigint" ? value.toString() : value)
        );
    if (was.length !== now.length || rest(before, was) !== rest(after, now)) found.add("other");
    return found;
}

/** The fresh transaction an override was keyed by, if it is still there. */
function atRef(fresh: readonly StagedTxn[], ref: StagedRef): StagedTxn | undefined {
    return fresh.find((txn) => refKey(txn.ref) === refKey(ref));
}

/**
 * Replay the held overrides over a freshly generated import (D25). `fresh` is
 * a dry run's (or a QuickBooks preview's) transactions WITHOUT overrides, so
 * every row is the generated text.
 *
 * 1. **Unaffected → kept.** A fresh transaction with the override's
 *    fingerprint is the same text: at the same place first, then anywhere
 *    (re-keyed to where it is now — a row added above it shifts ordinals).
 * 2. **Gone → dropped.** Nothing is at its place any more.
 * 3. **Covered by a rule → dropped.** The text at its place changed and every
 *    op is now a no-op there: a rule writes what the override wrote.
 * 4. **Changed underneath → kept and flagged**, re-targeted at the new text
 *    with the no-op ops pruned. Flagged when the text changed in some way the
 *    pruned ops do not account for — the user never saw this version. A text
 *    change the pruned ops explain (the user's own new rule) is not flagged:
 *    the rest of their edit, a reviewed stamp say, simply carries on.
 *
 * Each fresh transaction is claimed by at most one override.
 */
export function replayOverrides(held: readonly StagedOverride[], fresh: readonly StagedTxn[]): ReplayReport {
    const claimed = new Set<string>();
    const kept: StagedOverride[] = [];
    const dropped: ReplayReport["dropped"][number][] = [];
    const flagged: StagedOverride[] = [];
    const claim = (txn: StagedTxn): void => void claimed.add(refKey(txn.ref));
    const free = (txn: StagedTxn | undefined): txn is StagedTxn => txn !== undefined && !claimed.has(refKey(txn.ref));

    // Unchanged texts first, so a moved one is not mistaken for a changed one
    // by the override that used to sit where it now is.
    const pending: StagedOverride[] = [];
    for (const override of held) {
        const here = atRef(fresh, override.ref);
        const same =
            free(here) && here.fingerprint === override.fingerprint ? here : fresh.find((txn) => free(txn) && txn.fingerprint === override.fingerprint);
        if (same === undefined) {
            pending.push(override);
            continue;
        }
        claim(same);
        kept.push(Object.freeze({...override, ref: same.ref, original: same}));
    }

    for (const override of pending) {
        const now = atRef(fresh, override.ref);
        if (!free(now)) {
            dropped.push(Object.freeze({override, reason: "gone" as const}));
            continue;
        }
        claim(now);
        const leadingReplace = override.ops[0]?.op === "replace";
        const ops = leadingReplace ? [...override.ops] : override.ops.filter((op) => !isNoOp(op, now.txn));
        if (ops.length === 0) {
            dropped.push(Object.freeze({override, reason: "covered" as const}));
            continue;
        }
        const explained = new Set(
            override.ops
                .filter((op) => !ops.includes(op))
                .map(slot)
                .filter((key): key is string => key !== null)
        );
        const unexplained = leadingReplace || [...differences(override.original.txn, now.txn)].some((key) => !explained.has(key));
        const retargeted: StagedOverride = Object.freeze({
            ref: now.ref,
            fingerprint: now.fingerprint ?? override.fingerprint,
            ops: Object.freeze(ops),
            original: now,
            flagged: override.flagged || unexplained,
        });
        kept.push(retargeted);
        if (retargeted.flagged) flagged.push(retargeted);
    }
    for (const override of kept) if (override.flagged && !flagged.includes(override)) flagged.push(override);
    return Object.freeze({kept: Object.freeze(kept), dropped: Object.freeze(dropped), flagged: Object.freeze(flagged)});
}
