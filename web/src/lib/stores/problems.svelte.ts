// Problems store (WP-08): $derived checks over the journal, plus the UI
// plumbing the badge/drawer/table share (drawer visibility and "scroll to this
// txn" focus requests). Re-running checks is O(txns) and the journal store
// only swaps state on real changes (fingerprint skip), so a plain $derived is
// cheap here — no requestIdleCallback scheduling needed at MVP journal sizes.

import {groupByTxn, maxSeverity, runChecks, type Problem, type Severity} from "$lib/checks/engine";
import {journal} from "$lib/stores/journal.svelte";

const all = $derived.by(() => runChecks(journal.txns, {prices: journal.prices}));
const byTxn = $derived.by(() => groupByTxn(all));
const worst = $derived.by(() => maxSeverity(all));

let drawerOpen = $state(false);

export interface FocusRequest {
    txnIndex: number;
    /** Monotonic, so re-requesting the same txn re-triggers the table's effect. */
    nonce: number;
}

let focusRequest = $state<FocusRequest | null>(null);
let nonce = 0;

export const problems = {
    /** All problems from ALL_RULES over the current journal, in rule order. */
    get all(): Problem[] {
        return all;
    },
    /** Problems grouped by transaction index for row flag lookup. */
    get byTxn(): Map<number, Problem[]> {
        return byTxn;
    },
    get count(): number {
        return all.length;
    },
    /** Worst severity present (badge color), or null when clean. */
    get maxSeverity(): Severity | null {
        return worst;
    },
    get drawerOpen(): boolean {
        return drawerOpen;
    },
    set drawerOpen(open: boolean) {
        drawerOpen = open;
    },
    /** Pending "scroll the journal table to this txn" request; the table consumes and clears it. */
    get focusRequest(): FocusRequest | null {
        return focusRequest;
    },
    requestFocus(txnIndex: number): void {
        nonce += 1;
        focusRequest = {txnIndex, nonce};
    },
    clearFocus(): void {
        focusRequest = null;
    },
};
