// Which transaction the editor is open on. A module store (not prop-drilling)
// so the "Add transaction" button on the page and the per-row edit affordance
// deep inside the virtualized table can both drive the one editor the Journal
// page mounts.
//
// The editor is the review card's single-transaction mode (plan 32 D31, D39):
// the page mounts `ReviewOverlay` with `kind` = this store's `mode` while
// `open`. EDIT opens on a transaction INDEX; the review store fetches that one
// transaction from the engine (`GET /api/review/item/{index}`) rather than
// being handed a copy, because the client holds only the rows near the
// viewport (plan 26 B5) and the editor must edit what the engine has now.

export type TxnModalMode = "add" | "edit";

let open = $state(false);
let mode = $state<TxnModalMode>("add");
let targetIndex = $state<number | null>(null);
/** Bumped per open, so the page mounts a fresh editor even when one open follows another. */
let nonce = $state(0);

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
    /** Which open this is: the page keys the editor on it. */
    get nonce(): number {
        return nonce;
    },
    /** Open the editor on a blank card, to add a new transaction. */
    openAdd(): void {
        mode = "add";
        targetIndex = null;
        nonce += 1;
        open = true;
    },
    /** Open the editor on the transaction with tindex `index`. */
    openEdit(index: number): void {
        mode = "edit";
        targetIndex = index;
        nonce += 1;
        open = true;
    },
    close(): void {
        open = false;
        targetIndex = null;
    },
};
