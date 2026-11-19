// The accrual recognition helpers' store (plan 45, Phase 3): the gate and the
// badge counts, the Recognition overlay (Due · Needs a schedule · Invoices),
// the settlements page, and every accrual write.
//
// The STATUS (`/api/accrual`) is read once per (server, generation, today,
// needs-lookback): the journal toolbar keeps it in step with the journal, and
// everything else — the overlay, the editor's Accrual section, the journal's
// chip, the settings — asks `accrual.enabled` before showing anything. With
// the helpers off (`{enabled: false}`, D-gate) `status` stays null and no
// accrual UI appears anywhere.
//
// Writes go through `editing.write`, so the journal is re-read on success and
// a 409 raises the page's conflict banner, as for every other edit; then the
// status and whatever the overlay or page holds are read again at the new
// generation. Nothing is optimistic.

import {LedgelineApi, type RecognitionSaveBody, type WireScheduleSpecIn} from "#lib/api/native.js";
import type {EditFailure} from "#lib/api/editFailure.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {localToday} from "#lib/stores/filters.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {
    decodeAccrualStatus,
    decodeClosedThrough,
    decodeCreatedFile,
    decodeDue,
    decodeNeeds,
    decodePostResult,
    decodeSavePlan,
    decodeSettleLink,
    decodeSettleLinkAll,
    decodeSettlements,
    decodeTransactionSchedules,
} from "./decode";
import {NEEDS_LOOKBACK_DEFAULT, postThroughDate} from "./model";
import type {
    AccrualRole,
    AccrualStatus,
    DueEntry,
    DueView,
    NeedsView,
    PostResult,
    SavePlan,
    SettlementsView,
    SettleProposal,
    TransactionSchedules,
} from "./types";

/** What an accrual write came to: done, or why not. A conflict also raised the page's banner. */
export type AccrualOutcome<T> = {ok: true; value: T} | {ok: false; failure: EditFailure};

export type RecognitionTab = "due" | "needs" | "invoices";

/** The note a write leaves for the toast (D6: "Posted 37 recognition entries through Nov 17"). */
export interface AccrualNotice {
    message: string;
    /** The month whose group row the toast links to, or null. */
    month: string | null;
}

let status = $state.raw<AccrualStatus | null>(null);
let statusKey: string | null = null;
let statusError = $state<string | null>(null);
let inflight: {key: string; promise: Promise<void>} | null = null;

let needsLookback = $state(NEEDS_LOOKBACK_DEFAULT);

let open = $state(false);
let tab = $state<RecognitionTab>("due");
let due = $state.raw<DueView | null>(null);
let needs = $state.raw<NeedsView | null>(null);
let settlements = $state.raw<SettlementsView | null>(null);
let viewError = $state<string | null>(null);
let loading = $state(false);
let busy = $state(false);
let notice = $state.raw<AccrualNotice | null>(null);
/** Bumped per overlay/page read, so a stale answer is dropped. */
let viewSeq = 0;
/** Whether the settlements page (Q9) is showing, so writes re-read its list too. */
let pageOpen = false;

function api(serverUrl: string): LedgelineApi {
    return new LedgelineApi(serverUrl);
}

function message(cause: unknown): string {
    return cause instanceof Error ? cause.message : String(cause);
}

function keyOf(serverUrl: string, generation: string): string {
    return `${serverUrl}|${generation}|${localToday()}|${needsLookback}|${settings.settlementLookbackDays}|${settings.settlementToleranceBps}|${settings.settlementLeadDays}`;
}

/** The generation the showing lists were read at, or null. */
function viewsGeneration(): string | null {
    return settlements?.generation ?? due?.generation ?? needs?.generation ?? null;
}

async function loadStatus(serverUrl: string, key: string): Promise<void> {
    try {
        const next = decodeAccrualStatus(
            await api(serverUrl).accrualStatus({
                today: localToday(),
                lookback: needsLookback,
                settlementLookback: settings.settlementLookbackDays,
                settlementTolerance: settings.settlementToleranceBps,
                settlementLead: settings.settlementLeadDays,
            })
        );
        if (inflight?.key !== key) return;
        status = next;
        statusKey = key;
        statusError = null;
        // The journal moved on (an outside edit, the watcher's reload after a
        // write, auto-recognize): whatever list is showing was read at an older
        // generation, and its fingerprints may no longer be the editor's. Read
        // it again rather than leave every Link waiting on a stale list.
        if ((open || pageOpen) && next !== null && viewsGeneration() !== null && viewsGeneration() !== next.generation) void loadViews();
    } catch (cause) {
        if (inflight?.key !== key) return;
        // Keep what we had: the badge is an aid, and a blip must not hide the helpers.
        statusError = message(cause);
        statusKey = key;
    } finally {
        if (inflight?.key === key) inflight = null;
    }
}

function settleQuery(): {today: string; lookback: number; tolerance: number; lead: number} {
    return {today: localToday(), lookback: settings.settlementLookbackDays, tolerance: settings.settlementToleranceBps, lead: settings.settlementLeadDays};
}

/** Read the overlay's (or page's) lists that are showing. */
async function loadViews(): Promise<void> {
    const serverUrl = settings.serverUrl;
    if (serverUrl === null) return;
    const seq = ++viewSeq;
    const client = api(serverUrl);
    const today = localToday();
    const wantDue = open;
    const wantNeeds = open;
    const wantSettlements = open || pageOpen;
    loading = true;
    try {
        const [nextDue, nextNeeds, nextSettlements] = await Promise.all([
            wantDue ? client.recognitionDue(today).then(decodeDue) : Promise.resolve(due),
            wantNeeds ? client.recognitionNeeds({today, lookback: needsLookback}).then(decodeNeeds) : Promise.resolve(needs),
            wantSettlements ? client.settlements(settleQuery()).then(decodeSettlements) : Promise.resolve(settlements),
        ]);
        if (seq !== viewSeq) return;
        due = nextDue;
        needs = nextNeeds;
        settlements = nextSettlements;
        viewError = null;
    } catch (cause) {
        if (seq === viewSeq) viewError = message(cause);
    } finally {
        if (seq === viewSeq) loading = false;
    }
}

/** Re-read the status at the journal's current generation, and the lists that are showing. */
function reread(): Promise<void> {
    const serverUrl = settings.serverUrl;
    const generation = journal.generation;
    const reads: Promise<void>[] = [];
    if (serverUrl !== null && generation !== null) reads.push(accrual.sync(serverUrl, generation));
    if (open || pageOpen) reads.push(loadViews());
    return Promise.all(reads).then(() => undefined);
}

/** Run an accrual write through the journal's write path, then read everything again. */
async function write<T>(action: (api: LedgelineApi) => Promise<T>): Promise<AccrualOutcome<T>> {
    busy = true;
    try {
        const result = await editing.write(action);
        await reread();
        return result.ok ? {ok: true, value: result.value} : result;
    } finally {
        busy = false;
    }
}

export const accrual = {
    // -- the gate ------------------------------------------------------------

    /** `/api/accrual`'s answer with the helpers on, or null (off, or not read yet). */
    get status(): AccrualStatus | null {
        return status;
    },
    /** Whether the helpers are on for this ledger: every accrual affordance checks this. */
    get enabled(): boolean {
        return status !== null;
    },
    get statusError(): string | null {
        return statusError;
    },
    /** The accrual role of `account`, or null. */
    roleOf(account: string): AccrualRole | null {
        return status?.roles.get(account)?.role ?? null;
    },
    /** Due entries (the closed ones are listed apart and never auto-posted). */
    get dueCount(): number {
        return status?.counts.due ?? 0;
    },
    /** The toolbar badge: due + needs a schedule + clear invoice matches. */
    get badgeCount(): number {
        const counts = status?.counts;
        return counts === undefined ? 0 : counts.due + counts.needsSchedule + counts.invoices;
    },
    /** The badge's breakdown: "0 due · 11 need a schedule · 6 ready to link". */
    get badgeBreakdown(): string {
        const counts = status?.counts;
        if (counts === undefined) return "";
        return [
            `${counts.due} due`,
            `${counts.needsSchedule} ${counts.needsSchedule === 1 ? "needs" : "need"} a schedule`,
            `${counts.invoices} ready to link`,
        ].join(" · ");
    },
    /** The first tab with something in it — Due, then Needs a schedule, then Invoices — else Due. */
    get firstTab(): RecognitionTab {
        const counts = status?.counts;
        if (counts === undefined || counts.due > 0 || counts.dueClosed > 0) return "due";
        if (counts.needsSchedule > 0) return "needs";
        if (counts.invoices > 0) return "invoices";
        return "due";
    },

    /** Read the status unless it is already for this (server, generation, day, lookback). */
    sync(serverUrl: string, generation: string): Promise<void> {
        const key = keyOf(serverUrl, generation);
        if (inflight?.key === key) return inflight.promise;
        if (statusKey === key) return Promise.resolve();
        const promise = loadStatus(serverUrl, key);
        inflight = {key, promise};
        return promise;
    },

    /** "Needs a schedule"'s lookback in days (the ▾); 0 is everything. Session state. */
    get needsLookback(): number {
        return needsLookback;
    },
    setNeedsLookback(days: number): Promise<void> {
        if (!Number.isSafeInteger(days) || days < 0 || days === needsLookback) return Promise.resolve();
        needsLookback = days;
        return reread();
    },

    // -- the Recognition overlay (D6, Q8) ------------------------------------

    get open(): boolean {
        return open;
    },
    get tab(): RecognitionTab {
        return tab;
    },
    setTab(next: RecognitionTab): void {
        tab = next;
    },
    /** Open the overlay on `which` tab and read its lists. */
    openOverlay(which: RecognitionTab | null = null): void {
        if (status === null) return;
        tab = which ?? accrual.firstTab;
        open = true;
        notice = null;
        void loadViews();
    },
    close(): void {
        open = false;
        viewSeq += 1;
        loading = false;
    },
    get due(): DueView | null {
        return due;
    },
    get needs(): NeedsView | null {
        return needs;
    },
    get settlements(): SettlementsView | null {
        return settlements;
    },
    get loading(): boolean {
        return loading;
    },
    get error(): string | null {
        return viewError;
    },
    /** A write is in flight. */
    get busy(): boolean {
        return busy;
    },
    /** Read the showing lists again (after a failure, or a settings change). */
    reload(): Promise<void> {
        return loadViews();
    },

    // -- the settlements page (Q9) -------------------------------------------

    /** The page mounted (true) or unmounted (false): its list is read, and re-read after writes. */
    setPageOpen(on: boolean): void {
        pageOpen = on;
        if (on) void loadViews();
    },

    // -- writes --------------------------------------------------------------

    /** The last write's note for the toast, or null. */
    get notice(): AccrualNotice | null {
        return notice;
    },
    clearNotice(): void {
        notice = null;
    },

    /**
     * **Post through ‹date›** (D6): exactly `entries` (the checked rows), in
     * one write, against the generation the table was read at. With
     * `includeClosed`, the closed periods' entries become one adjustment per
     * schedule, dated today.
     */
    post(entries: readonly DueEntry[], includeClosed = false): Promise<AccrualOutcome<PostResult>> {
        const listed = due;
        if (listed === null) return Promise.resolve({ok: false, failure: {kind: "validation", message: "The due list isn't loaded."}});
        // A plain list: `postThroughDate` only needs the latest month.
        const months = entries.map((entry) => entry.recognized.slice(0, 7));
        const through = postThroughDate(months, listed.today) ?? undefined;
        return write(async (client) => {
            const result = decodePostResult(
                await client.recognitionPost({
                    generation: listed.generation,
                    through,
                    entries: entries.map(({id, recognized}) => ({id, recognized})),
                    includeClosed,
                    today: localToday(),
                })
            );
            const count = result.posted.length;
            const latest = result.posted.reduce<string | null>((best, entry) => (best === null || entry.recognized > best ? entry.recognized : best), null);
            notice = {
                message:
                    count === 0 && result.adjustments.length === 0
                        ? "Nothing to post: those entries are already in the journal."
                        : `Posted ${count} recognition ${count === 1 ? "entry" : "entries"}${through === undefined ? "" : ` through ${through}`}` +
                          (result.adjustments.length > 0
                              ? ` and ${result.adjustments.length} closed-period adjustment${result.adjustments.length === 1 ? "" : "s"}`
                              : ""),
                month: latest?.slice(0, 7) ?? null,
            };
            return result;
        });
    },

    /** Create the recognition file (D1), after the user agreed. */
    createFile(): Promise<AccrualOutcome<{generation: string; path: string}>> {
        return write(async (client) => decodeCreatedFile(await client.recognitionCreateFile()));
    },

    /** **Close books through…** (Q1): set, or with null clear, `[books] closed-through`. */
    setClosedThrough(date: string | null): Promise<AccrualOutcome<{generation: string; closedThrough: string | null}>> {
        return write(async (client) => decodeClosedThrough(await client.booksClosedThrough(date)));
    },

    /** Link one proposal (D-settle), with the fee edit for a short payment. */
    link(proposal: SettleProposal, feeAccount: string | null = null): Promise<AccrualOutcome<{generation: string; matchId: string}>> {
        const {invoice, payment} = proposal;
        if (invoice.fingerprint === null || payment.fingerprint === null) {
            return Promise.resolve({ok: false, failure: {kind: "unavailable", message: "Editing is not enabled on this server."}});
        }
        return write(async (client) =>
            decodeSettleLink(
                await client.settlementLink({
                    invoice: {index: invoice.index, posting: invoice.posting, fingerprint: invoice.fingerprint as string},
                    payment: {index: payment.index, posting: payment.posting, fingerprint: payment.fingerprint as string},
                    ...(feeAccount === null || feeAccount.trim() === "" ? {} : {feeAccount: feeAccount.trim()}),
                    tolerance: settings.settlementToleranceBps,
                })
            )
        );
    },

    /** Link every clear proposal, against the generation the list was read at. */
    linkAll(): Promise<AccrualOutcome<{generation: string; linked: number; matchIds: readonly string[]}>> {
        const listed = settlements;
        if (listed === null) return Promise.resolve({ok: false, failure: {kind: "validation", message: "The invoices aren't loaded."}});
        return write(async (client) => {
            const result = decodeSettleLinkAll(
                await client.settlementLinkAll({
                    generation: listed.generation,
                    lookback: settings.settlementLookbackDays,
                    tolerance: settings.settlementToleranceBps,
                    lead: settings.settlementLeadDays,
                    today: localToday(),
                })
            );
            notice = {
                message: `Linked ${result.linked} invoice${result.linked === 1 ? "" : "s"} to ${result.linked === 1 ? "its payment" : "their payments"}.`,
                month: null,
            };
            return result;
        });
    },

    // -- the editor's Accrual section (D-editor) -----------------------------

    /** One source transaction's schedules, periods, renewal prefill and party suggestion; null when off. */
    async forTransaction(index: number): Promise<TransactionSchedules | null> {
        const serverUrl = settings.serverUrl;
        if (serverUrl === null || status === null) return null;
        return decodeTransactionSchedules(await api(serverUrl).recognitionFor(index, localToday()));
    },

    /** One transaction's settlement proposals and candidates (an invoice's or a payment's); null when off. */
    async settlementsFor(index: number): Promise<SettlementsView | null> {
        const serverUrl = settings.serverUrl;
        if (serverUrl === null || status === null) return null;
        return decodeSettlements(await api(serverUrl).settlementsFor(index, settleQuery()));
    },

    /** What saving `schedules` (with the source as `source`) would write: the schedules and any restatement. */
    async preview(body: Omit<RecognitionSaveBody, "today">): Promise<SavePlan | null> {
        const serverUrl = settings.serverUrl;
        if (serverUrl === null || status === null) return null;
        return decodeSavePlan(await api(serverUrl).recognitionPreview({...body, today: localToday()}));
    },

    /**
     * Write `schedules`' rule groups (and any restatement) — step 2 of the
     * editor's save (D-editor), after the source transaction with its tags
     * went through the editor's normal save. `createFile` when the user agreed
     * to create the recognition file.
     */
    saveSchedules(schedules: readonly WireScheduleSpecIn[], createFile: boolean): Promise<AccrualOutcome<SavePlan | null>> {
        return write(async (client) => decodeSavePlan(await client.recognitionSave({schedules: [...schedules], createFile, today: localToday()})));
    },
};

/** Test seam: module-level runes state is shared across a test file. */
export function resetAccrual(): void {
    status = null;
    statusKey = null;
    statusError = null;
    inflight = null;
    needsLookback = NEEDS_LOOKBACK_DEFAULT;
    open = false;
    tab = "due";
    due = null;
    needs = null;
    settlements = null;
    viewError = null;
    loading = false;
    busy = false;
    notice = null;
    viewSeq += 1;
    pageOpen = false;
}
