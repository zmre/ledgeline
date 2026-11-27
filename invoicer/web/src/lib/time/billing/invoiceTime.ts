// An invoice's time (plan 46 T10): what its delete or void gives back to
// Unbilled. Books with no time files, or an invoice no time is on, have
// none: the invoice dialogs then say nothing about time.
import {EditorApiError} from "#lib/editorApi.js";
import {fetchEntries} from "../api/entries.js";
import type {Qty} from "../types.js";

/** The time tagged `invoice: number`, or null when there is none (or time tracking isn't set up). */
export async function fetchInvoiceTime(number: string): Promise<Qty | null> {
    try {
        const list = await fetchEntries(new URLSearchParams({invoice: number}).toString());
        return list.entries.length === 0 ? null : list.total;
    } catch (error: unknown) {
        if (!(error instanceof EditorApiError && error.code === "not-set-up")) console.warn("invoice time:", error);
        return null;
    }
}
