// Problems store (WP-08): the engine's problem list, plus the UI plumbing the
// badge/drawer/table share (drawer visibility and "scroll to this txn" focus
// requests).
//
// The checks run in the ENGINE (plan 26 B4): `/api/diagnostics?today=` answers
// the engine diagnostics and the five local rules together, as of the
// browser's local date. This used to be a `$derived` over the whole downloaded
// journal (30–46 ms and 21k findings at 150k transactions); now it is one
// request per journal generation, made by the layout, which is always mounted.
//
// Keyed on (generation, today): the answer is a pure function of the two, and
// a stale answer is never dropped until its replacement arrives, so the badge
// does not flicker through every refresh.

import {LedgelineApi} from "#lib/api/native.js";
import {decodeDiagnostics} from "#lib/api/journalViewDecode.js";
import {groupByTxn, type Problem, type Severity} from "#lib/checks/engine.js";

let all = $state.raw<Problem[]>([]);
let count = $state(0);
let worst = $state<Severity | null>(null);
const byTxn = $derived.by(() => groupByTxn(all));

/** The (server, generation, today) the held list answers, and the newest one asked for. */
let loadedKey: string | null = null;
let requestedKey: string | null = null;
let seq = 0;

let drawerOpen = $state(false);

export interface FocusRequest {
    txnIndex: number;
    /** Monotonic, so re-requesting the same txn re-triggers the table's effect. */
    nonce: number;
}

let focusRequest = $state<FocusRequest | null>(null);
let nonce = 0;

export const problems = {
    /** Engine diagnostics first, then the local rules, in the engine's order. */
    get all(): Problem[] {
        return all;
    },
    /** Problems grouped by transaction index for row flag lookup. */
    get byTxn(): Map<number, Problem[]> {
        return byTxn;
    },
    get count(): number {
        return count;
    },
    /** Worst severity present (badge color), or null when clean. */
    get maxSeverity(): Severity | null {
        return worst;
    },
    /**
     * Fetch the list for `generation` as of `today`, unless that is already
     * held or in flight. A failure keeps the previous list (stale beats
     * blank for an advisory badge) and is retried by the next call.
     */
    async load(serverUrl: string, generation: string, today: string): Promise<void> {
        const key = `${serverUrl}\u0000${generation}\u0000${today}`;
        if (key === loadedKey || key === requestedKey) return;
        requestedKey = key;
        const token = ++seq;
        try {
            const payload = decodeDiagnostics(await new LedgelineApi(serverUrl).diagnostics(today));
            if (token !== seq) return;
            all = payload.problems;
            count = payload.count;
            worst = payload.maxSeverity;
            loadedKey = key;
        } catch (error) {
            if (token === seq && import.meta.env.DEV) console.debug("[problems] load failed", error);
        } finally {
            if (token === seq) requestedKey = null;
        }
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
