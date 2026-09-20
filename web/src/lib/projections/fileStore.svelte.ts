// Scenario FILES: the picker's list, the load, and the save (plan 22, Phase 3).
//
// Deliberately a SIBLING of `scenarioStore`, not a part of it. That store owns
// the scenario being edited and the projection it produces; this one owns
// everything about where a scenario came from and where it is going. Keeping
// them apart is what makes the seams `scenarioStore` already exposes —
// `adopt`, `markSaved`, `dirty` — the whole of the contract between them:
//
//   load  → `fileStore.open(url, id)`  → `scenarioStore.adopt(file.scenario)`
//   save  → `fileStore.save(...)`      → `scenarioStore.markSaved()`
//
// # The identity of a loaded scenario lives HERE
//
// `currentId` and `revision` are what turn the next save into an UPDATE rather
// than a create, and they move together, always: a save that landed replaces
// both from what the engine wrote. A revision from a re-read rather than from
// the write is exactly how the next save clobbers somebody else silently, and
// the engine is careful about that — dropping the one it returns would throw
// that care away at the last step.
//
// # `available`
//
// False once the engine answers 404 for `/api/projections`, which means the
// engine predates Phase 3. The tab still works — seeding and projecting are
// Phases 1 and 2 — and simply offers no file controls, rather than showing a
// picker that cannot be used.

import {classify, type EditFailure} from "$lib/api/editFailure";
import {LedgelineApi, NativeApiUnavailableError, type SaveScenarioBody} from "$lib/api/native";
import {decodeProjectionIndex, decodeScenarioFile} from "$lib/api/nativeDecode";
import {createResource} from "$lib/stores/resource.svelte";
import {settings} from "$lib/stores/settings.svelte";
import {scenarioToWire} from "./scenarioModel";
import type {ProjectionIndex, Scenario, ScenarioFile} from "./types";

/** `revision: ""` means CREATE — the engine's `NEW_FILE_REVISION`. */
const NEW_FILE_REVISION = "";

const index = createResource<string, ProjectionIndex>(async (serverUrl) => decodeProjectionIndex(await new LedgelineApi(serverUrl).listProjections()));

let available = $state(true);
let saving = $state(false);
let loading = $state(false);
/** The file the scenario on screen came from, or null when it was seeded. */
let currentId = $state<string | null>(null);
/** The optimistic-concurrency token for `currentId`. Empty when there is no file. */
let revision = $state(NEW_FILE_REVISION);
/** Whether `currentId` may be saved over. False for a file the main journal includes. */
let currentWritable = $state(false);
/** The last index load key, so a page visit after a prefetch does not walk the tree twice. */
let indexKey: string | null = null;

/** A load or a save either lands, or fails with a classified reason the page can explain. */
export type FileOutcome = {ok: true; file: ScenarioFile} | {ok: false; failure: EditFailure};

export const fileStore = {
    /** False once the engine has answered 404 for `/api/projections` — an older engine, so hide the file controls. */
    get available(): boolean {
        return available;
    },
    /** The file listing, for its value, its status and its error. */
    get index() {
        return index;
    },
    /** A save is in flight. */
    get saving(): boolean {
        return saving;
    },
    /** A load is in flight. */
    get loading(): boolean {
        return loading;
    },
    /** The id of the file the scenario came from, or null when it was seeded from the journal. */
    get currentId(): string | null {
        return currentId;
    },
    /** Whether the loaded file can be saved over — false for one the main journal includes. */
    get currentWritable(): boolean {
        return currentWritable;
    },
    /** Whether a plain Save (rather than Save As) is possible right now. */
    get canSave(): boolean {
        return currentId !== null && currentWritable;
    },

    /** Read the listing once per (server, reconnect). */
    async ensureIndex(serverUrl: string, nonce: number): Promise<void> {
        const key = `${nonce}|${serverUrl}`;
        if (key === indexKey) return;
        indexKey = key;
        await this.reloadIndex(serverUrl);
    },

    /** Re-read the listing unconditionally (after a save, or a Retry). */
    async reloadIndex(serverUrl: string): Promise<void> {
        await index.load(serverUrl, serverUrl);
        available = !(index.error instanceof NativeApiUnavailableError);
    },

    /**
     * Load one file. The CALLER adopts the scenario, because adopting it is a
     * decision about unsaved work and this store has no business making it.
     */
    async open(serverUrl: string, id: string): Promise<FileOutcome> {
        loading = true;
        try {
            const file = decodeScenarioFile(await new LedgelineApi(serverUrl).getProjection(id));
            this.adopted(file);
            return {ok: true, file};
        } catch (error) {
            return {ok: false, failure: classify(error)};
        } finally {
            loading = false;
        }
    },

    /**
     * Save the scenario over the file it came from.
     *
     * Refused locally when there is no file, or when the file is one the main
     * journal includes — decision 5. The engine refuses it too; doing it here
     * as well is what keeps the button from being offered at all.
     */
    async save(serverUrl: string, scenario: Scenario): Promise<FileOutcome> {
        if (currentId === null || !currentWritable) {
            return {ok: false, failure: {kind: "validation", message: "This scenario has no file of its own yet — use Save as."}};
        }
        return this.write(serverUrl, currentId, {revision, scenario: scenarioToWire(scenario)});
    },

    /**
     * Save the scenario to a NEW file, which the engine creates with `O_EXCL`.
     *
     * A 409 from here means "a file already exists there" and nothing was
     * written — as opposed to a 409 from {@link save}, which means the file
     * changed on disk. The two are told apart by which call was made, not by
     * the status.
     */
    async saveAs(serverUrl: string, id: string, scenario: Scenario): Promise<FileOutcome> {
        return this.write(serverUrl, id, {revision: NEW_FILE_REVISION, scenario: scenarioToWire(scenario)});
    },

    /** The half {@link save} and {@link saveAs} share. */
    async write(serverUrl: string, id: string, body: SaveScenarioBody): Promise<FileOutcome> {
        saving = true;
        try {
            const file = decodeScenarioFile(await new LedgelineApi(serverUrl).saveProjection(id, body));
            this.adopted(file);
            // A create adds a row and a save moves an `updated:` date, so the
            // listing this page is showing is stale either way.
            void this.reloadIndex(serverUrl);
            return {ok: true, file};
        } catch (error) {
            // `available` is deliberately NOT cleared here. A 501 means "this
            // server has no journal bound to an editor", which the index already
            // reports as `editable: false`; conflating it with "no
            // /api/projections at all" would make one read-only server remove
            // the whole file surface.
            return {ok: false, failure: classify(error)};
        } finally {
            saving = false;
        }
    },

    /**
     * Record that `file` is now the scenario's home.
     *
     * The three fields move TOGETHER and only here, so a revision can never
     * belong to a different id than the one it was taken from. Also persists
     * the id, which is what reopens it on the next mount (decision 10 — in
     * `settings`, not `prefs.json`).
     */
    adopted(file: ScenarioFile): void {
        currentId = file.id;
        revision = file.revision;
        currentWritable = file.writable;
        settings.lastProjectionId = file.id;
    },

    /**
     * Forget the file — the scenario on screen is a seed again, or a load
     * failed against an id that no longer names anything.
     *
     * Clears the remembered id too: an id that does not resolve must not keep
     * being retried on every mount, which would put a failure on the screen
     * every time the tab is opened.
     */
    forget(): void {
        currentId = null;
        revision = NEW_FILE_REVISION;
        currentWritable = false;
        settings.lastProjectionId = null;
    },
};
