// Bank sync store (plan 43): the engine's sync status, polled, and every
// action the screens take — Sync now, connect, link, the drawer's edits.
//
// # Polling (contract WP-W)
//
// `GET /api/sync/status` every 60 s while the window is visible (the shared
// `pollWhileVisible`), and every 2 s while the engine says a run is going, so
// rows update as each connection finishes and the spinner stops when it ends.
//
// # An engine without sync
//
// The server half (WP-S) lands after this. An engine that 404s the status
// route is `unsupported`: the header dot hides, the Accounts tab says so, and
// polling stops until the next connection — never a crash, never a toast.
//
// # Dev-only fixtures
//
// With `localStorage["ledgeline.dev.syncFixtures"] = "1"` in a DEV build, the
// store talks to `devMock.ts` (in-memory fixtures) instead of the engine, so
// the screens can be worked on before the server exists. Both the flag check
// and the import sit behind `import.meta.env.DEV`, which a production build
// replaces with `false` and drops along with the fixtures.

import {classify} from "#lib/api/editFailure.js";
import {NativeApiUnavailableError, SyncProblemError} from "#lib/api/native.js";
import {pollWhileVisible} from "#lib/stores/visiblePoll.js";
import {liveSyncApi, type SyncApi} from "./api";
import {placedProblems, worstSeverity, type PlacedProblem, type ProblemEffect} from "./problemAction";
import type {
    Severity,
    WireAccountPatch,
    WireConnectResult,
    WireLinkPreview,
    WireLinkRequest,
    WireManualAccount,
    WireManualUpdate,
    WireProblem,
    WireRemoteAccount,
    WireSharePreview,
    WireSyncRun,
    WireSyncStatus,
} from "./types";

/** Poll cadence while idle and while a run is going (contract WP-W). */
export const POLL_IDLE_MS = 60_000;
export const POLL_RUNNING_MS = 2_000;

/** The DEV-only localStorage flag that swaps in the fixtures. */
export const DEV_FIXTURES_FLAG = "ledgeline.dev.syncFixtures";

/** A failed action: the sentence to show, and the structured problem when the engine sent one. */
export interface SyncFailure {
    message: string;
    problem: WireProblem | null;
}

export type SyncOutcome<T> = {ok: true; value: T} | {ok: false; failure: SyncFailure};

/** The header dot's state (plan 43 §4.1). */
export type DotState = "hidden" | "idle" | "syncing" | "attention" | "broken";

/** Something a screen asked the Accounts tab to do once it is showing (the Problems drawer, the header dot). */
export type PanelIntent = Extract<ProblemEffect, {type: "reconnect" | "review-held" | "choose-file" | "connect"}>;

let status = $state.raw<WireSyncStatus | null>(null);
let unsupported = $state(false);
let pollError = $state<string | null>(null);
let manual = $state.raw<WireManualAccount[] | null>(null);
let manualError = $state<string | null>(null);
let syncing = $state(false);
let lastRun = $state.raw<WireSyncRun | null>(null);
let lastRunError = $state.raw<SyncFailure | null>(null);
let intent = $state.raw<PanelIntent | null>(null);

const placed = $derived(status === null || !status.available ? [] : placedProblems(status));
const worst = $derived(worstSeverity(placed.map((p) => p.problem)));

let api: SyncApi | null = null;
/** Bumped by every start/stop, so an answer from a previous connection is dropped. */
let epoch = 0;
let fastTimer: ReturnType<typeof setTimeout> | null = null;
let inFlight: Promise<void> | null = null;
/** Tests hand in an API; `start` then uses it instead of the engine's. */
let override: ((serverUrl: string) => SyncApi) | null = null;

function failure(error: unknown): SyncFailure {
    if (error instanceof SyncProblemError) return {message: error.problem.message, problem: error.problem};
    return {message: classify(error).message, problem: null};
}

async function attempt<T>(run: (api: SyncApi) => Promise<T>): Promise<SyncOutcome<T>> {
    const current = api;
    if (current === null) return {ok: false, failure: {message: "Ledgeline isn't connected to its engine.", problem: null}};
    try {
        return {ok: true, value: await run(current)};
    } catch (error) {
        return {ok: false, failure: failure(error)};
    }
}

/** Write actions answer with the new status; take it, and keep polling fast if a run started. */
function adopt(next: WireSyncStatus): void {
    status = next;
    unsupported = false;
    scheduleFast();
}

function clearFast(): void {
    if (fastTimer !== null) {
        clearTimeout(fastTimer);
        fastTimer = null;
    }
}

function scheduleFast(): void {
    clearFast();
    if (status?.running === true || syncing) fastTimer = setTimeout(() => void refresh(), POLL_RUNNING_MS);
}

/** Re-read the manual-downloads list when a screen has loaded it: it leaves out linked accounts. */
async function reloadManualIfShown(): Promise<void> {
    if (manual === null) return;
    const outcome = await attempt((a) => a.manual());
    if (outcome.ok) manual = outcome.value;
}

async function refresh(): Promise<void> {
    if (api === null || unsupported) return;
    if (inFlight !== null) return inFlight;
    const mine = epoch;
    const current = api;
    inFlight = (async () => {
        try {
            const next = await current.status();
            if (mine !== epoch) return;
            status = next;
            pollError = null;
        } catch (error) {
            if (mine !== epoch) return;
            if (error instanceof NativeApiUnavailableError) {
                unsupported = true;
                status = null;
            } else {
                // Stale beats blank for a status line: keep the last answer, say why it is old.
                pollError = classify(error).message;
            }
        } finally {
            inFlight = null;
            if (mine === epoch) scheduleFast();
        }
    })();
    return inFlight;
}

async function devApi(serverUrl: string): Promise<SyncApi> {
    if (import.meta.env.DEV && typeof localStorage !== "undefined" && localStorage.getItem(DEV_FIXTURES_FLAG) === "1") {
        const {devMockApi} = await import("./devMock");
        return devMockApi();
    }
    return liveSyncApi(serverUrl);
}

export const syncStore = {
    /** The last status the engine answered, or null before the first (and on an engine without sync). */
    get status(): WireSyncStatus | null {
        return status;
    },
    /** The engine has no sync routes (it predates plan 43). */
    get unsupported(): boolean {
        return unsupported;
    },
    /** Why the last poll failed, when it did — the status shown is then the one before. */
    get pollError(): string | null {
        return pollError;
    },
    /** A Sync now (or Finish, or I've fixed it) is in flight from this window. */
    get syncing(): boolean {
        return syncing;
    },
    /** A run is going: this window's, or one the engine started (auto-sync). */
    get running(): boolean {
        return syncing || status?.running === true;
    },
    /** The last run this window started, for the toast. */
    get lastRun(): WireSyncRun | null {
        return lastRun;
    },
    get lastRunError(): SyncFailure | null {
        return lastRunError;
    },
    dismissRun(): void {
        lastRun = null;
        lastRunError = null;
    },

    /** Every problem worth a look, with where it is (the Problems drawer's "Bank sync" group). */
    get problems(): PlacedProblem[] {
        return placed;
    },
    get maxSeverity(): Severity | null {
        return worst;
    },
    /** Hidden with nothing linked; else idle, syncing, amber for attention, red for a broken connection. */
    get dot(): DotState {
        if (status === null || !status.available || status.accounts.length === 0) return "hidden";
        if (this.running) return "syncing";
        if (status.connections.some((c) => c.state === "broken") || worst === "error") return "broken";
        if (status.connections.some((c) => c.state === "attention") || worst === "warning") return "attention";
        return "idle";
    },

    /**
     * Point the store at `serverUrl` and poll; the returned function stops.
     * Called by the layout once per (server, reconnect).
     */
    start(serverUrl: string): () => void {
        const mine = ++epoch;
        unsupported = false;
        pollError = null;
        let stopPoll: (() => void) | null = null;
        let stopped = false;
        void (async () => {
            const chosen = override !== null ? override(serverUrl) : await devApi(serverUrl);
            if (stopped || mine !== epoch) return;
            api = chosen;
            await refresh();
            if (stopped || mine !== epoch || unsupported) return;
            stopPoll = pollWhileVisible(() => void refresh(), POLL_IDLE_MS);
        })();
        return () => {
            stopped = true;
            stopPoll?.();
            if (mine === epoch) {
                epoch += 1;
                clearFast();
                api = null;
            }
        };
    },

    /** Re-read the status now (after a write elsewhere, or a Retry). */
    refresh,

    /** Sync now: everything, one connection, or one account. Writes on click (plan 43 decision #1). */
    async syncNow(target: {connectionId?: string; accountKey?: string} = {}): Promise<SyncOutcome<WireSyncRun>> {
        if (syncing) return {ok: false, failure: {message: "A sync is already running.", problem: null}};
        syncing = true;
        lastRun = null;
        lastRunError = null;
        scheduleFast();
        try {
            const outcome = await attempt((a) => a.now(target));
            if (outcome.ok) lastRun = outcome.value;
            else lastRunError = outcome.failure;
            return outcome;
        } finally {
            syncing = false;
            await refresh();
        }
    },

    /** Claim a setup token (the wizard's step 2), or replace an existing connection's. */
    async connect(setupToken: string, replacing: string | null = null): Promise<SyncOutcome<WireConnectResult>> {
        const outcome = await attempt((a) => (replacing === null ? a.connect(setupToken) : a.replaceToken(replacing, setupToken)));
        if (outcome.ok) void refresh();
        return outcome;
    },
    /** Re-check a connection's accounts (finds banks newly linked at SimpleFIN). Costs one request. */
    async recheckAccounts(connectionId: string): Promise<SyncOutcome<WireRemoteAccount[]>> {
        const outcome = await attempt((a) => a.accounts(connectionId));
        if (outcome.ok) void refresh();
        return outcome;
    },
    async disconnect(connectionId: string): Promise<SyncOutcome<WireSyncStatus>> {
        const outcome = await attempt((a) => a.disconnect(connectionId));
        if (outcome.ok) adopt(outcome.value);
        return outcome;
    },
    diagnostics(connectionId: string): Promise<SyncOutcome<string>> {
        return attempt((a) => a.diagnostics(connectionId));
    },
    sharePreview(manualRulesId: string, journalAccount: string): Promise<SyncOutcome<WireSharePreview>> {
        return attempt((a) => a.sharePreview(manualRulesId, journalAccount));
    },
    linkPreview(links: readonly WireLinkRequest[]): Promise<SyncOutcome<WireLinkPreview[]>> {
        return attempt((a) => a.linkPreview(links));
    },
    /** The wizard's Finish: write the links and run the first sync. */
    async link(links: readonly WireLinkRequest[]): Promise<SyncOutcome<WireSyncRun>> {
        syncing = true;
        try {
            return await attempt((a) => a.link(links));
        } finally {
            syncing = false;
            await refresh();
            // A linked account leaves the manual list.
            await reloadManualIfShown();
        }
    },
    async patchAccount(key: string, patch: WireAccountPatch): Promise<SyncOutcome<WireSyncStatus>> {
        const outcome = await attempt((a) => a.patchAccount(key, patch));
        if (outcome.ok) adopt(outcome.value);
        return outcome;
    },
    async stopAccount(key: string): Promise<SyncOutcome<WireSyncStatus>> {
        const outcome = await attempt((a) => a.stopAccount(key));
        if (outcome.ok) {
            adopt(outcome.value);
            // An unlinked account rejoins the manual list.
            await reloadManualIfShown();
        }
        return outcome;
    },
    async resolveHeld(key: string, heldId: string, action: "import" | "duplicate"): Promise<SyncOutcome<WireSyncStatus>> {
        const outcome = await attempt((a) => a.resolveHeld(key, heldId, action));
        if (outcome.ok) adopt(outcome.value);
        return outcome;
    },

    /** Manual downloads: the journal's accounts imported by hand. */
    get manual(): WireManualAccount[] | null {
        return manual;
    },
    get manualError(): string | null {
        return manualError;
    },
    async loadManual(): Promise<void> {
        const outcome = await attempt((a) => a.manual());
        if (outcome.ok) {
            manual = outcome.value;
            manualError = null;
        } else {
            manualError = outcome.failure.message;
        }
    },
    async putManual(body: WireManualUpdate): Promise<SyncOutcome<WireManualAccount[]>> {
        const outcome = await attempt((a) => a.putManual(body));
        if (outcome.ok) manual = outcome.value;
        return outcome;
    },

    /** Ask the Accounts tab to open something once it shows (a Problems drawer button on another page). */
    get intent(): PanelIntent | null {
        return intent;
    },
    request(next: PanelIntent): void {
        intent = next;
    },
    takeIntent(): PanelIntent | null {
        const taken = intent;
        intent = null;
        return taken;
    },

    /** Tests: use `factory` instead of the engine (or the dev fixtures) from the next `start`. Null restores. */
    useApi(factory: ((serverUrl: string) => SyncApi) | null): void {
        override = factory;
    },
    /** Tests: forget everything. */
    reset(): void {
        epoch += 1;
        clearFast();
        api = null;
        status = null;
        unsupported = false;
        pollError = null;
        manual = null;
        manualError = null;
        syncing = false;
        lastRun = null;
        lastRunError = null;
        intent = null;
        inFlight = null;
    },
    /** Tests and the dev mock: show this status as if the engine had answered it. */
    setStatus(next: WireSyncStatus | null): void {
        status = next;
    },
};
