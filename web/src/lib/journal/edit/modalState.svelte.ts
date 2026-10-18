// Global open/mode/target state for the transaction popup. A module store (not
// prop-drilling) so the "Add transaction" button on the page and the per-row
// edit affordance deep inside the virtualized table can both drive the single
// <TransactionModal> mounted at the page level.
//
// EDIT opens on a transaction INDEX and fetches that one transaction from the
// engine (`GET /api/transactions/{index}`) rather than being handed a copy: the
// client holds only the rows near the viewport (plan 26 B5), and the popup must
// edit what the engine has now, not what a block cached a while ago.

import {LedgelineApi} from "#lib/api/native.js";
import {decodeRow} from "#lib/api/journalViewDecode.js";
import type {Transaction} from "#lib/domain/types.js";
import {localToday} from "#lib/stores/filters.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";

export type TxnModalMode = "add" | "edit";

let open = $state(false);
let mode = $state<TxnModalMode>("add");
let targetIndex = $state<number | null>(null);
let target = $state.raw<Transaction | null>(null);
let loadError = $state<string | null>(null);
/** Bumped per open, so a slow fetch for an earlier open cannot fill a later one. */
let ticket = 0;

async function load(index: number, mine: number): Promise<void> {
    const serverUrl = settings.serverUrl;
    if (serverUrl === null) return;
    try {
        const row = decodeRow(await new LedgelineApi(serverUrl).transaction(index, localToday()));
        if (mine === ticket) target = row.txn;
    } catch (error) {
        if (mine === ticket) loadError = error instanceof Error ? error.message : String(error);
    }
}

export const txnModal = {
    get open(): boolean {
        return open;
    },
    get mode(): TxnModalMode {
        return mode;
    },
    /** The tindex being edited (edit mode), else null. */
    get targetIndex(): number | null {
        return targetIndex;
    },
    /** The transaction being edited, once fetched (edit mode), else null. */
    get target(): Transaction | null {
        return target;
    },
    /** Why the edit target could not be fetched, or null. */
    get loadError(): string | null {
        return loadError;
    },
    /** Open a blank popup to add a new transaction. */
    openAdd(): void {
        ticket += 1;
        mode = "add";
        targetIndex = null;
        target = null;
        loadError = null;
        open = true;
    },
    /** Open the popup on the transaction with tindex `index`, to replace it (PUT). */
    openEdit(index: number): void {
        const mine = ++ticket;
        mode = "edit";
        targetIndex = index;
        target = null;
        loadError = null;
        open = true;
        void load(index, mine);
    },
    close(): void {
        ticket += 1;
        open = false;
        targetIndex = null;
        target = null;
        loadError = null;
    },
};
