// The Budget tab's data layer: the report the bars are drawn from, the goal
// listing the editor rewrites, the per-account history strip, and the two
// dispatchers that write.
//
// The shapes are borrowed from `aliasStore.svelte.ts` rather than invented, and
// for the same reasons — `createResource` for each read (so FE-1's "payload and
// the question it answers are ONE value" and the stale-response token come for
// free) and a `run()`-style dispatcher for each write.
//
// # Why a save refetches the journal AND the report
//
// A budget goal lives IN the journal, so writing one changes a file the engine
// has parsed and the watcher is watching. Two things go stale at once: the
// listing (every file's revision, and every goal's index — a delete renumbers
// everything below it) and the report (the bars are drawn against the goals that
// just moved). Reloading one and not the other leaves a screen whose top half
// disagrees with its bottom half about what the budget is.

import {classify, type EditFailure} from "$lib/api/editFailure";
import {LedgelineApi, NativeApiUnavailableError, type BudgetChange, type SaveBudgetLinesBody} from "$lib/api/native";
import {decodeAccountReference, decodeBudgetFileResponse, decodeBudgetListing, decodeBudgetReport, decodeCreatedBudgetFile} from "$lib/api/nativeDecode";
import type {BudgetReport} from "$lib/reports/types";
import {createResource} from "$lib/stores/resource.svelte";
import {settings} from "$lib/stores/settings.svelte";
import type {AccountReference, BudgetFile, BudgetListing, BudgetPeriod, CreatedBudgetFile} from "./types";

/** The exact query the budget report is fetched for — always monthly buckets. */
export interface BudgetReportQuery {
    /** Carried so `sameBudgetQuery` can tell one span from another; the fetch uses `end`+`count`. */
    from: string;
    end: string;
    count: number;
    depth: number;
}

/** The history strip's query. */
export interface ReferenceQuery {
    account: string;
    interval: BudgetPeriod;
    count: number;
}

/**
 * How many periods the history strip asks for: four complete ones plus the one
 * now running.
 *
 * "What did I spend on groceries over the last few months, and how am I doing
 * this month" is the question the strip exists to answer, and it is the same
 * shape for an annual income goal. Four rather than three because the strip now
 * shows an average too, and three points is the fewest a pattern can be claimed
 * from at all — the fourth is what turns "those two were high" into "this is
 * what it usually is".
 */
export const REFERENCE_PERIODS = 5;

const report = createResource<BudgetReportQuery, BudgetReport>(async (serverUrl, query) =>
    decodeBudgetReport(await new LedgelineApi(serverUrl).budget({end: query.end, interval: "monthly", count: query.count, depth: query.depth}))
);

const listing = createResource<string, BudgetListing>(async (serverUrl) => decodeBudgetListing(await new LedgelineApi(serverUrl).listBudgetLines()));

const reference = createResource<ReferenceQuery, AccountReference>(async (serverUrl, query) =>
    decodeAccountReference(await new LedgelineApi(serverUrl).budgetReference({account: query.account, interval: query.interval, count: query.count}))
);

/** Whether two report queries ask for exactly the same bars (gates the export, per FE-1). */
export function sameBudgetQuery(a: BudgetReportQuery, b: BudgetReportQuery): boolean {
    return a.from === b.from && a.end === b.end && a.count === b.count && a.depth === b.depth;
}

let available = $state(true);
let saving = $state(false);
/** Set by a 409: the file moved under us, so any open form is a stale base for a save. */
let conflict = $state(false);
/** The last listing key, so revisiting the tab after a prefetch does not re-read the tree. */
let listingKey: string | null = null;

/** A write either lands (handing back what the engine wrote) or fails with a classified reason. */
export type BudgetSaveOutcome = {ok: true; file: BudgetFile} | {ok: false; failure: EditFailure};
export type CreateFileOutcome = {ok: true; created: CreatedBudgetFile} | {ok: false; failure: EditFailure};

export const budgetStore = {
    /** The bars' report and the span it answers. */
    get report() {
        return report;
    },
    /** The goal listing the editor rewrites. */
    get listing() {
        return listing;
    },
    /** The history strip beside the amount box. */
    get reference() {
        return reference;
    },
    /** False once the engine has answered 404 for `/api/budget/lines` — an older engine. */
    get available(): boolean {
        return available;
    },
    get saving(): boolean {
        return saving;
    },
    /** A file changed on disk under an edit; the page offers reload-and-discard until cleared. */
    get conflict(): boolean {
        return conflict;
    },
    clearConflict(): void {
        conflict = false;
    },

    /** Load the goal listing once per (server, reconnect), and never twice for the same one. */
    async ensureListing(serverUrl: string, nonce: number): Promise<void> {
        const key = `${nonce}|${serverUrl}`;
        if (key === listingKey) return;
        listingKey = key;
        await this.reloadListing(serverUrl);
    },

    /** Re-read the goal listing unconditionally (after a write, or a Retry). */
    async reloadListing(serverUrl: string): Promise<void> {
        conflict = false;
        await listing.load(serverUrl, serverUrl);
        available = !(listing.error instanceof NativeApiUnavailableError);
    },

    /** Fetch the history strip for one account. */
    async loadReference(serverUrl: string, query: ReferenceQuery): Promise<void> {
        await reference.load(serverUrl, query);
    },

    /**
     * Apply one change to one file's goals.
     *
     * On success the engine answers with the file it actually wrote, at a fresh
     * revision. The whole listing is reloaded anyway rather than patched from
     * that: a goal index is a scan ordinal and is explicitly not stable across
     * saves (removing one renumbers every goal below it in the same file), so
     * keeping the old listing would make the next edit address a different line.
     */
    async save(journalId: string, revision: string, change: BudgetChange, reportQuery: BudgetReportQuery | null): Promise<BudgetSaveOutcome> {
        const url = settings.serverUrl;
        if (url === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
        const body: SaveBudgetLinesBody = {revision, change};
        saving = true;
        try {
            const file = decodeBudgetFileResponse(await new LedgelineApi(url).saveBudgetLines(journalId, body));
            conflict = false;
            await this.afterWrite(url, reportQuery);
            return {ok: true, file};
        } catch (error) {
            const failure = classify(error);
            if (failure.kind === "conflict") conflict = true;
            // `available` is deliberately NOT cleared here: a 501 on this route
            // means "no journal is bound to an editor", which the listing already
            // reports as `editable: false`. Conflating the two would make one
            // read-only server remove the whole screen.
            return {ok: false, failure};
        } finally {
            saving = false;
        }
    },

    /** Create a `budget.journal` and include it from the main journal. */
    async createFile(reportQuery: BudgetReportQuery | null): Promise<CreateFileOutcome> {
        const url = settings.serverUrl;
        if (url === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
        saving = true;
        try {
            const created = decodeCreatedBudgetFile(await new LedgelineApi(url).createBudgetFile());
            conflict = false;
            await this.afterWrite(url, reportQuery);
            return {ok: true, created};
        } catch (error) {
            const failure = classify(error);
            if (failure.kind === "conflict") conflict = true;
            return {ok: false, failure};
        } finally {
            saving = false;
        }
    },

    /**
     * Re-read everything a write invalidates.
     *
     * Both, always, and awaited: the bars are drawn from the goals the listing
     * describes, so returning while one of them still shows the old journal is
     * how a screen comes to disagree with itself.
     */
    async afterWrite(serverUrl: string, reportQuery: BudgetReportQuery | null): Promise<void> {
        await Promise.all([this.reloadListing(serverUrl), reportQuery === null ? Promise.resolve() : report.load(serverUrl, reportQuery)]);
    },
};
