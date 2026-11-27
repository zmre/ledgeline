// Time tracking's Entries calls (plan 46 Phase 2, `app/time.rs`): the list
// with its filters, Add, Edit, Delete and the bulk actions. Refusals come
// back as an EditorApiError carrying the server's `code` (`locked`,
// `conflict`, `external-change`, `note-tag` with `offending` and
// `suggestion`, `invalid`, `bad-request`), so a form can say what to fix.
//
// Ids (`file@line`, `file@line b`, `file@sN`) name a line as the file is
// now: every write can move the lines below it, so after any write the
// caller fetches the list again rather than patching what it holds.
import {EditorApiError, send} from "#lib/editorApi.js";
import {errorText} from "#lib/labels.js";

import type {Qty} from "../types.js";
export type {Qty};

/** Which billing state a line is in (B-states). */
export type Billed = "unbilled" | "invoiced" | "nobill" | "not-billable";

/** One row of Entries. */
export interface EntryView {
    id: string;
    date: string;
    person: string | null;
    account: string;
    quantity: Qty;
    note: string;
    /** The line's own tags and its day's, as `[name, value]` pairs. */
    tags: [string, string][];
    invoice: string | null;
    bill: string | null;
    nobill: string | null;
    billed: Billed;
    billable: boolean;
    customer: string | null;
    file: string;
    line: number;
    format: "timedot" | "timeclock";
    /** Why it can't be changed (a sent invoice, a booked bill), when it can't. */
    locked: string | null;
}

/** Something the file parsers noticed (a timeclock overlap, a line hledger reads differently). */
export interface Finding {
    file: string;
    line: number;
    severity: "info" | "warning" | "error";
    message: string;
}

/** `GET /api/time/entries`. */
export interface EntryList {
    entries: EntryView[];
    total: Qty;
    findings: Finding[];
}

/** What a write answers: its result, the files written. */
export interface Written<T> {
    done: T;
    files: string[];
    retried: boolean;
    included: unknown;
}

/** `POST /api/time/entries`. */
export interface NewEntry {
    date: string;
    account: string;
    /** `45m`, `1h30`, `1.5`, `0:45`. */
    duration: string;
    note?: string;
    /** Whose file it goes in; the tracker's person when absent. */
    person?: string;
}

/** `PUT /api/time/entries`: what changes. A timeclock session takes only `start`/`end`. */
export interface EntryEdit {
    id: string;
    date?: string;
    account?: string;
    duration?: string;
    note?: string;
    /** `YYYY-MM-DDTHH:MM[:SS]`. */
    start?: string;
    end?: string;
}

/** `POST /api/time/entries/bulk`'s operation. */
export type BulkOp = {op: "billable"} | {op: "nobill"; reason: string} | {op: "move"; account: string} | {op: "person"; person: string};

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);

/** A Qty's shape. */
export const isQty = (body: unknown): body is Qty => isObject(body) && typeof body.seconds === "number" && typeof body.text === "string";

/** One entry's shape: the fields the page reads. */
export function isEntryView(body: unknown): body is EntryView {
    return (
        isObject(body) &&
        typeof body.id === "string" &&
        typeof body.date === "string" &&
        typeof body.account === "string" &&
        isQty(body.quantity) &&
        typeof body.note === "string" &&
        typeof body.billed === "string" &&
        typeof body.file === "string"
    );
}

/** The list's shape (every entry checked: a wrong one would be read as garbage, not skipped). */
export function isEntryList(body: unknown): body is EntryList {
    return isObject(body) && Array.isArray(body.entries) && body.entries.every(isEntryView) && isQty(body.total) && Array.isArray(body.findings);
}

const isWritten = (body: unknown): boolean => isObject(body) && "done" in body && Array.isArray(body.files);

/** `GET /api/time/entries?…`: `query` is the API's query string (`entriesApiQuery`), without the `?`. */
export function fetchEntries(query: string): Promise<EntryList> {
    const rest = query.replace(/^[?&]+/, "");
    return send<EntryList>("GET", `/api/time/entries${rest === "" ? "" : `?${rest}`}`, isEntryList);
}

/** `POST /api/time/entries`: add a line to the person's month file; answers the file. */
export function addEntry(entry: NewEntry): Promise<Written<string>> {
    return send<Written<string>>("POST", "/api/time/entries", isWritten, entry);
}

/** `PUT /api/time/entries`: change a line. */
export function editEntry(edit: EntryEdit): Promise<Written<unknown>> {
    return send<Written<unknown>>("PUT", "/api/time/entries", isWritten, edit);
}

/** `DELETE /api/time/entries?id=`. */
export function deleteEntry(id: string): Promise<Written<unknown>> {
    return send<Written<unknown>>("DELETE", `/api/time/entries?id=${encodeURIComponent(id)}`, isWritten);
}

/** `POST /api/time/entries/bulk`: answers how many lines changed. */
export function bulkEntries(ids: readonly string[], op: BulkOp): Promise<Written<number>> {
    return send<Written<number>>("POST", "/api/time/entries/bulk", isWritten, {ids, ...op});
}

/** A write's outcome: done, or refused with the server's code, its sentence and the rest of its body (`suggestion`). */
export type WriteOutcome = {ok: true} | Refused;

/** A refused write. */
export interface Refused {
    ok: false;
    code: string;
    message: string;
    body: Record<string, unknown>;
}

/** The outcome a failed write's error stands for (`unreachable` when the server never answered). */
export function refusal(error: unknown): Refused {
    return error instanceof EditorApiError
        ? {ok: false, code: error.code, message: errorText(error), body: error.body}
        : {ok: false, code: "unreachable", message: errorText(error), body: {}};
}
