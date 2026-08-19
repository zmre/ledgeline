// "Open an inline editor on this row" — a request the table makes and one row
// answers.
//
// `e` and `c` need state that lives INSIDE a row (`editingDesc` in
// TransactionRow, `editingAccount` in AccountsCell), and the table cannot reach
// it: the list is virtualized, so the target row may not even be mounted when
// the key is pressed.
//
// Same shape as `problems.focusRequest` (stores/problems.svelte.ts) and for the
// same reason — the monotonic nonce means re-requesting the SAME row re-fires
// the consumer's effect, which a bare `{txnIndex, action}` would not.

export type RowAction = "description" | "category";

export interface RowActionRequest {
    txnIndex: number;
    action: RowAction;
    /** Monotonic, so asking twice for the same row and action still fires twice. */
    nonce: number;
}

let request = $state<RowActionRequest | null>(null);
let nonce = 0;

export const rowActions = {
    /** The pending request, or null. Consumers filter it by their own `txn.index`. */
    get request(): RowActionRequest | null {
        return request;
    },
    /**
     * Ask row `txnIndex` to open `action`.
     *
     * Callers must REVEAL the row first: the reveal changes `scrollTop`, which
     * recomputes the render window in the same flush, so the row mounts and its
     * effect sees this request. Requesting first would fire into nothing.
     */
    open(txnIndex: number, action: RowAction): void {
        nonce += 1;
        request = {txnIndex, action, nonce};
    },
    /**
     * Clear a request the caller has handled.
     *
     * Takes the nonce so a late consumer cannot wipe a NEWER request: two rows
     * can be mounted with effects pending when the user presses `c` twice.
     */
    consume(handled: number): void {
        if (request !== null && request.nonce === handled) request = null;
    },
    /** Test-only: module-level runes state is shared across a test file. */
    reset(): void {
        request = null;
    },
};
