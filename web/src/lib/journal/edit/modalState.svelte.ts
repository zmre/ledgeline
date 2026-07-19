// Global open/mode/prefill state for the transaction popup. A module store (not
// prop-drilling) so the "Add transaction" button on the page and the per-row
// edit affordance deep inside the virtualized table can both drive the single
// <TransactionModal> mounted at the page level.

import type {Transaction} from "$lib/domain/types";

export type TxnModalMode = "add" | "edit";

let open = $state(false);
let mode = $state<TxnModalMode>("add");
let target = $state<Transaction | null>(null);

export const txnModal = {
    get open(): boolean {
        return open;
    },
    get mode(): TxnModalMode {
        return mode;
    },
    /** The transaction being edited (edit mode), else null. */
    get target(): Transaction | null {
        return target;
    },
    /** Open a blank popup to add a new transaction. */
    openAdd(): void {
        mode = "add";
        target = null;
        open = true;
    },
    /** Open the popup prefilled from `txn` to replace it (PUT). */
    openEdit(txn: Transaction): void {
        mode = "edit";
        target = txn;
        open = true;
    },
    close(): void {
        open = false;
        target = null;
    },
};
