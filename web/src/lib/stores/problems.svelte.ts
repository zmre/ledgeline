// Problems store (WP-08): the engine's problem list, plus the UI plumbing the
// badge/drawer/table share (drawer visibility and "scroll to this txn" focus
// requests).
//
// The checks run in the ENGINE (plan 26 B4): `/api/diagnostics?today=` answers
// the engine diagnostics and the five local rules together, as of the
// browser's local date. This used to be a `$derived` over the whole downloaded
// journal (30–46 ms and 21k findings at 150k transactions).
//
// Two sizes of answer, because two consumers want very different amounts:
//
//   - the BADGE (always mounted, via the layout) needs a count and the worst
//     severity: `limit=0`, a few bytes, once per (generation, today);
//   - the DRAWER needs every finding, but only while it is open: fetched then,
//     and re-fetched if the journal moves while it stays open.
//
// A row's own flags arrive embedded in its row (`/api/journal/rows`), so
// nothing here is indexed per transaction any more.

import {LedgelineApi} from "#lib/api/native.js";
import {decodeDiagnostics, type DiagnosticsPayload} from "#lib/api/journalViewDecode.js";
import type {Problem, Severity} from "#lib/checks/engine.js";

let all = $state.raw<Problem[]>([]);
let count = $state(0);
let worst = $state<Severity | null>(null);

interface Target {
    serverUrl: string;
    generation: string;
    today: string;
}

/** The newest (server, generation, today) the layout asked about. */
let target: Target | null = null;
const keyOf = (t: Target): string => `${t.serverUrl}\u0000${t.generation}\u0000${t.today}`;
/** What the held count and list answer, and what is in flight for each. */
let countKey: string | null = null;
let countRequested: string | null = null;
let listKey = $state<string | null>(null);
let listRequested: string | null = null;
let countToken = 0;
let listToken = 0;

let drawerOpen = $state(false);

export interface FocusRequest {
    txnIndex: number;
    /** Monotonic, so re-requesting the same txn re-triggers the table's effect. */
    nonce: number;
}

let focusRequest = $state<FocusRequest | null>(null);
let nonce = 0;

async function fetchPayload(t: Target, limit?: number): Promise<DiagnosticsPayload> {
    return decodeDiagnostics(await new LedgelineApi(t.serverUrl).diagnostics(t.today, limit));
}

/** Fetch the whole list for the current target, unless it is held or in flight. */
async function loadList(): Promise<void> {
    const t = target;
    if (t === null) return;
    const key = keyOf(t);
    if (key === listKey || key === listRequested) return;
    listRequested = key;
    const token = ++listToken;
    try {
        const payload = await fetchPayload(t);
        if (token !== listToken) return;
        all = payload.problems;
        count = payload.count;
        worst = payload.maxSeverity;
        listKey = key;
        countKey = key;
    } catch (error) {
        if (token === listToken && import.meta.env.DEV) console.debug("[problems] list load failed", error);
    } finally {
        if (token === listToken) listRequested = null;
    }
}

export const problems = {
    /** Every finding, engine diagnostics first — loaded while the drawer is open (stale until then). */
    get all(): Problem[] {
        return all;
    },
    /** True while the drawer's list does not yet describe the current journal. */
    get listLoading(): boolean {
        return target !== null && listKey !== keyOf(target);
    },
    get count(): number {
        return count;
    },
    /** Worst severity present (badge color), or null when clean. */
    get maxSeverity(): Severity | null {
        return worst;
    },
    /**
     * Point the store at `generation` as of `today`: the badge's count and
     * worst severity (and the full list too, while the drawer is open). Dedupes
     * on (server, generation, today); a failure keeps the previous figures
     * (stale beats blank for an advisory badge) and is retried by the next call.
     */
    async load(serverUrl: string, generation: string, today: string): Promise<void> {
        target = {serverUrl, generation, today};
        if (drawerOpen) return loadList();
        const t = target;
        const key = keyOf(t);
        if (key === countKey || key === countRequested) return;
        countRequested = key;
        const token = ++countToken;
        try {
            const payload = await fetchPayload(t, 0);
            if (token !== countToken) return;
            count = payload.count;
            worst = payload.maxSeverity;
            countKey = key;
        } catch (error) {
            if (token === countToken && import.meta.env.DEV) console.debug("[problems] load failed", error);
        } finally {
            if (token === countToken) countRequested = null;
        }
    },
    get drawerOpen(): boolean {
        return drawerOpen;
    },
    set drawerOpen(open: boolean) {
        drawerOpen = open;
        if (open) void loadList();
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
