// What the import screens' review buttons count and hand the overlay (plan 32
// D26, D30), and how a held override reads when the panel lists it.
//
// Pure: no Svelte, no fetch. `stagedReview.test.ts` pins it.

import type {WireEditOp} from "#lib/api/native.js";
import {refKey} from "./reviewModel";
import type {StagedOverride, StagedRef, StagedTxn} from "./reviewTypes";

/** Which staged run a button opens. */
export type StagedRunKind = "unknowns" | "all";

/** Ops that recategorise: an account moved, or the whole transaction re-rendered. */
function categorises(ops: readonly WireEditOp[]): boolean {
    return ops.some((op) => op.op === "setPostingAccount" || op.op === "replace");
}

/**
 * The staged transactions a run of `kind` covers. Review All is every one.
 * Review Unknowns is every unknown one, less those a held override already
 * recategorised.
 *
 * A CSV dry run applies its overrides, so those rows already read as
 * categorised. A QuickBooks preview takes no overrides and shows the
 * export's own text, so without this an unknown fixed in the last run would
 * come back as an unknown in the next.
 */
export function stagedRunItems(kind: StagedRunKind, txns: readonly StagedTxn[], overrides: readonly StagedOverride[]): StagedTxn[] {
    if (kind === "all") return [...txns];
    const fixed = new Set(overrides.filter((override) => categorises(override.ops)).map((override) => refKey(override.ref)));
    return txns.filter((txn) => txn.unknown && !fixed.has(refKey(txn.ref)));
}

/** The two buttons' badges. */
export function stagedRunCounts(txns: readonly StagedTxn[], overrides: readonly StagedOverride[]): {unknowns: number; all: number} {
    return {unknowns: stagedRunItems("unknowns", txns, overrides).length, all: txns.length};
}

/** "N reviewed edits · M existing transactions edited", or null when the import carries none. */
export function editCountLine(overrideCount: number, journalEditCount: number): string | null {
    const parts: string[] = [];
    if (overrideCount > 0) parts.push(`${overrideCount} reviewed edit${overrideCount === 1 ? "" : "s"}`);
    if (journalEditCount > 0) parts.push(`${journalEditCount} existing transaction${journalEditCount === 1 ? "" : "s"} edited`);
    return parts.length === 0 ? null : parts.join(" · ");
}

/** What one override does, in a few words per op: "→ expenses:food", "renamed “Blue Bottle”", "reviewed". */
export function describeOps(ops: readonly WireEditOp[]): string[] {
    return ops.map((op) => {
        switch (op.op) {
            case "setPostingAccount":
                return `posting ${op.posting + 1} → ${op.account}`;
            case "setDescription":
                return `renamed “${op.description}”`;
            case "setTxnComment":
                return op.comment === "" ? "comment cleared" : "comment edited";
            case "setTxnTag":
                if (op.name === "reviewed") return op.value === "ignore" ? "ignored" : "reviewed";
                return op.value === null ? `tag ${op.name} removed` : `tag ${op.name}:${op.value}`;
            case "setDate":
                return `dated ${op.date}`;
            case "setDate2":
                return op.date2 === null ? "secondary date removed" : `secondary date ${op.date2}`;
            case "setStatus":
                return `marked ${op.status}`;
            case "setCode":
                return op.code === null ? "code removed" : `code ${op.code}`;
            case "setPostingComment":
                return op.comment === null ? `posting ${op.posting + 1} comment cleared` : `posting ${op.posting + 1} comment edited`;
            case "removePostingAssertion":
                return `posting ${op.posting + 1} assertion removed`;
            case "replace":
                return "amounts or postings edited";
        }
    });
}

/** What an import panel hands its review strip and edits note: the counts, the held edits, and what the buttons do. */
export interface StagedReviewProps {
    readonly unknowns: number;
    readonly all: number;
    readonly overrideCount: number;
    readonly journalEditCount: number;
    readonly flagged: readonly StagedOverride[];
    readonly onReview: (kind: StagedRunKind) => void;
    readonly onDiscard: (refs: readonly StagedRef[]) => void;
}
