// The Projections tab's data layer: the scenario being edited, the seed it
// starts from, and the projection it produces.
//
// Shapes borrowed from `budgetStore.svelte.ts` rather than invented —
// `createResource` per read, so FE-1's "the payload and the question it answers
// are ONE value" and the stale-response token come for free.
//
// # Why the scenario is `$state` and the projection is a resource
//
// The scenario is the only thing on this page the user owns, and it is not
// fetched: Phase 2 seeds it once from the engine and then never reads it back.
// The PROJECTION is fetched, from a body built out of that scenario, so the
// invariant that matters is that a projection on screen answers the table above
// it — which is what `revision` in the query key is for. Every edit bumps it;
// a projection whose `query.revision` is not the live one is a stale answer and
// the page says so rather than charting it.
//
// # Why the recompute is debounced
//
// Typing an amount is several edits, and each one is a `POST` carrying the whole
// scenario. 250 ms is the same debounce `searchMirror` uses on the URL, for the
// same reason: it is under the threshold where a person notices a pause, and
// over the one where a keystroke costs a request.
//
// # Phase 3 seams
//
// Loading and saving files are Phase 3's. What they need from here is already
// here: [`adopt`] replaces the whole scenario and resets `dirty` (a load), and
// [`markSaved`] clears `dirty` without touching the scenario (a save). Nothing
// else writes `dirty`, so "is there unsaved work" has exactly one answer.

import {LedgelineApi} from "#lib/api/native.js";
import {decodeProjection, decodeScenario} from "#lib/api/nativeDecode.js";
import type {PeriodInterval} from "#lib/reports/ui/params.js";
import {dataView, type DataView, type LoadStatus} from "#lib/stores/loadState.js";
import {createResource} from "#lib/stores/resource.svelte.js";
import {cloneScenario, emptyScenario, runBody} from "./scenarioModel";
import type {Projection, Scenario} from "./types";

/** How many trailing months the seed's unbudgeted averages are taken over. The engine's own default, stated so the request is readable. */
const SEED_COUNT = 12;

/** The same 250 ms the URL mirror uses, and for the same reason. */
const DEBOUNCE_MS = 250;

/** The window every reading shares. */
export interface ProjectionWindow {
    interval: PeriodInterval;
    count: number;
    /** Full actual periods the statements show before the forecast. */
    lookback: number;
    depth: number;
}

/** The exact question a held projection answers. */
export interface ProjectionQuery extends ProjectionWindow {
    /**
     * The scenario version this was computed for.
     *
     * The scenario itself is a mutable `$state` tree and so cannot be compared
     * by identity; a monotonic counter bumped by every edit can, and is what
     * stops an old projection rendering under an edited table.
     */
    revision: number;
}

/**
 * Which branch the what-if table renders.
 *
 * Gated on whether a scenario has been ADOPTED, not on the seed resource alone.
 * Reopening the remembered file on mount (decision 10) deliberately never reads
 * the seed, so a table gated on the seed's status saw it `idle` forever and
 * drew a spinner over a loaded scenario — every reload of the tab with a file
 * open, with no way out but a fresh origin (a relaunch, on the desktop's
 * ephemeral port). Once anything is adopted the table is the user's, and a seed
 * failure behind it is no reason to hide it.
 */
export function scenarioTableView(adoptedScenario: boolean, seedStatus: LoadStatus, hasSeed: boolean): DataView {
    return adoptedScenario ? "data" : dataView(seedStatus, hasSeed);
}

/** Whether two queries ask for exactly the same projection (the FE-1 gate). */
export function sameProjectionQuery(a: ProjectionQuery, b: ProjectionQuery): boolean {
    return a.revision === b.revision && a.interval === b.interval && a.count === b.count && a.lookback === b.lookback && a.depth === b.depth;
}

let scenario = $state<Scenario>(emptyScenario());
let revision = $state(0);
let dirty = $state(false);
/** Whether ANY scenario — a seed or a file — has been adopted since the page loaded. */
let adopted = $state(false);
/**
 * The depth the scenario on screen was SEEDED at, or null when it came from a
 * file (or nothing has been adopted). What "Re-seed at this depth" compares
 * the page's slider against.
 */
let seedDepth = $state<number | null>(null);
/** The last (nonce, server) a seed was requested for, so revisiting the tab does not re-read the journal. */
let seedKey: string | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;

/**
 * The seed, keyed by the DEPTH it names its unbudgeted categories at.
 *
 * The page's depth slider, not a constant (plan 28, follow-ups 2026-10-26): the
 * slider is where a user says how finely they want to plan, and the seed is the
 * starting point of that plan. Only the starting point — once seeded, rows sit
 * at whatever level the user puts them, leaves beside parents, and moving the
 * slider never re-seeds by itself (see `reseed`).
 */
const seed = createResource<number, Scenario>(async (serverUrl, depth) =>
    decodeScenario(await new LedgelineApi(serverUrl).projectionSeed({count: SEED_COUNT, depth}))
);

const projection = createResource<ProjectionQuery, Projection>(async (serverUrl, query) =>
    // The body is built HERE rather than carried in the query so that it is
    // built from the scenario as it stands when the debounce fires — the point
    // of coalescing edits is that only the last one is sent.
    decodeProjection(await new LedgelineApi(serverUrl).runProjection(runBody(scenario, query)))
);

export const scenarioStore = {
    /** The scenario being edited. Mutate its fields directly, then call `touch()`. */
    get scenario(): Scenario {
        return scenario;
    },
    /** Bumped by every edit; the projection query's identity. */
    get revision(): number {
        return revision;
    },
    /** Whether the scenario has been edited since it was loaded, seeded or saved. */
    get dirty(): boolean {
        return dirty;
    },
    /** Whether a scenario has been adopted, from a seed OR a file — the table's gate (see {@link scenarioTableView}). */
    get adopted(): boolean {
        return adopted;
    },
    /** The depth the scenario on screen was seeded at, or null for a file. */
    get seedDepth(): number | null {
        return seedDepth;
    },
    /** The seeded starting scenario, for its status and its error. */
    get seed() {
        return seed;
    },
    /** The projection and the question it answers. */
    get projection() {
        return projection;
    },
    /**
     * Record that the scenario changed.
     *
     * Every edit path calls this and only this. Splitting it into per-field
     * notifications would mean a new column could be added that moved a number
     * without moving `revision`, which is the one way a stale projection could
     * render as a current one.
     */
    touch(): void {
        revision += 1;
        dirty = true;
    },

    /**
     * Replace the whole scenario — a seed, or (Phase 3) a loaded file.
     *
     * Cloned on the way in: `decodeScenario` freezes what it returns, as every
     * decoder here does, and the editor has to be able to write into what it
     * holds.
     */
    adopt(next: Scenario, seededAt: number | null = null): void {
        scenario = cloneScenario(next);
        revision += 1;
        dirty = false;
        adopted = true;
        seedDepth = seededAt;
    },

    /** Phase 3: the scenario was written to disk unchanged, so there is no unsaved work. */
    markSaved(): void {
        dirty = false;
    },

    /**
     * Seed the table once per (server, reconnect), and never over unsaved work.
     *
     * The `dirty` guard is what makes a reconnect safe: the engine restarting
     * under a half-edited scenario must not silently replace it with an average
     * of the user's history.
     */
    async ensureSeeded(serverUrl: string, nonce: number, depth: number): Promise<void> {
        // Not keyed on `depth`: a seed is a starting point, and moving the
        // depth slider afterwards is a question about the statements, not an
        // instruction to throw the table away.
        const key = `${nonce}|${serverUrl}`;
        if (key === seedKey) return;
        seedKey = key;
        await seed.load(serverUrl, depth);
        if (seed.value !== null && !dirty) this.adopt(seed.value, depth);
    },

    /**
     * Re-read the seed at `depth` unconditionally, discarding the current
     * table — a Retry, or the page's "Re-seed at depth N", which it only
     * offers over an untouched seed.
     */
    async reseed(serverUrl: string, depth: number): Promise<void> {
        await seed.load(serverUrl, depth);
        if (seed.value !== null && seed.query === depth) this.adopt(seed.value, depth);
    },

    /**
     * Recompute, after `DEBOUNCE_MS` of quiet.
     *
     * A pending run is cancelled by the next call, so a burst of edits costs one
     * request. `createResource`'s own token then drops any response that a newer
     * request has already superseded, which covers the case where the debounce
     * fired but the network was slow.
     */
    schedule(serverUrl: string, window: ProjectionWindow): void {
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(() => {
            timer = null;
            void projection.load(serverUrl, {...window, revision});
        }, DEBOUNCE_MS);
    },

    /** Recompute now, skipping the debounce (a Retry button). */
    async runNow(serverUrl: string, window: ProjectionWindow): Promise<void> {
        if (timer !== null) clearTimeout(timer);
        timer = null;
        await projection.load(serverUrl, {...window, revision});
    },

    /** Cancel a pending recompute. Works directly as an `onMount` cleanup. */
    stop(): void {
        if (timer !== null) clearTimeout(timer);
        timer = null;
    },
};
