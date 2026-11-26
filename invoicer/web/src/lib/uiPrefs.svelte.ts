// The UI prefs the invoicer shares with Ledgeline (SETTINGS-PLAN §2, D2, D4):
// the theme and text size, and the three payment-matching settings. They live
// in the shared `ui-prefs.toml`, or in the open books' `ledgeline.toml` for a
// key pinned to them ("Use for these books only").
//
// - The theme saves at once, the text size after a short pause (zoom keys
//   come in bursts), one save at a time and in order, so an older scale never
//   lands after a newer one. A save still waiting goes out as the page hides.
// - The matching settings are part of the Settings page's form: they change
//   only on Save (`saveMatching`).
// - `pin` keeps a key for these books alone (`POST /api/ui-prefs/pin`), or
//   stops; the answer is the effective prefs, adopted as they come back.
//
// The layout hands this store to `useAppearance`, so the zoom keys and the
// View menu step the saved scale.
import {authHeaders} from "#ui/api/client.js";
import {DEFAULT_THEME_PREF} from "#ui/theme/catalog.js";
import {DEFAULT_SCALE} from "#ui/theme/scale.js";
import type {AppearancePrefs} from "#ui/theme/appearance.svelte.js";
import {send} from "./editorApi.js";
import {clampMatching, defaultMatching, MATCHING_KEYS, type Matching, type MatchingKey} from "./matching.js";

export {MATCHING_KEYS, type Matching, type MatchingKey};

/** Every key this app may pin to the books. */
export type PinnableKey = "theme" | "uiScale" | MatchingKey;

/** `GET`, `PATCH /api/ui-prefs` and the pin's answer. */
export interface UiPrefsAnswer {
    prefs: Record<string, unknown>;
    /** `browser`: the prefs live in the browser, so nothing can be pinned. */
    storage?: "files" | "browser";
    /** The keys the open books' own file sets. */
    localKeys?: string[];
    issues?: unknown[];
}

/** The requests the store makes; the test passes its own. */
export interface UiPrefsApi {
    load: () => Promise<UiPrefsAnswer>;
    patch: (values: Record<string, unknown>) => Promise<UiPrefsAnswer>;
    pin: (key: PinnableKey, pinned: boolean) => Promise<UiPrefsAnswer>;
    /** A last save that outlives the page (`keepalive`). */
    beacon: (values: Record<string, unknown>) => void;
}

const isAnswer = (body: unknown): boolean => typeof body === "object" && body !== null && "prefs" in body;

/** A scale as a whole percent (the server keeps a hand edit's 50–300; the UI steps between 70 and 200). */
const wholeScale = (percent: number): number => (Number.isFinite(percent) ? Math.round(percent) : DEFAULT_SCALE);

/** The server's API. */
export const httpUiPrefsApi: UiPrefsApi = {
    load: () => send<UiPrefsAnswer>("GET", "/api/ui-prefs", isAnswer),
    patch: (values) => send<UiPrefsAnswer>("PATCH", "/api/ui-prefs", isAnswer, values),
    pin: (key, pinned) => send<UiPrefsAnswer>("POST", "/api/ui-prefs/pin", isAnswer, {key, pinned}),
    beacon: (values) => {
        const headers = authHeaders({"Content-Type": "application/json"}, "/api/ui-prefs");
        void fetch("/api/ui-prefs", {method: "PATCH", headers, body: JSON.stringify(values), keepalive: true}).catch(() => undefined);
    },
};

/** How long the text size waits for the next zoom step before it is saved. */
export const SCALE_SAVE_MS = 400;

export class UiPrefs implements AppearancePrefs {
    #theme = $state<string>(DEFAULT_THEME_PREF);
    #uiScale = $state(DEFAULT_SCALE);
    #ready = $state(false);
    #storage = $state<"files" | "browser" | null>(null);
    #localKeys = $state<string[]>([]);
    /** The matching settings as saved. */
    matching = $state<Matching>(defaultMatching());
    /** Why the last save or pin didn't go through, in words; null when it did. */
    failure = $state<string | null>(null);

    readonly #api: UiPrefsApi;
    readonly #debounceMs: number;
    #pending: Record<string, unknown> = {};
    #timer: ReturnType<typeof setTimeout> | undefined;
    #saving: Promise<void> = Promise.resolve();

    constructor(api: UiPrefsApi = httpUiPrefsApi, debounceMs = SCALE_SAVE_MS) {
        this.#api = api;
        this.#debounceMs = debounceMs;
    }

    get theme(): string {
        return this.#theme;
    }
    set theme(id: string) {
        if (id === this.#theme) return;
        this.#theme = id;
        this.#queue({theme: id}, 0);
    }

    get uiScale(): number {
        return this.#uiScale;
    }
    set uiScale(percent: number) {
        const next = wholeScale(percent);
        if (next === this.#uiScale) return;
        this.#uiScale = next;
        this.#queue({uiScale: next}, this.#debounceMs);
    }

    /** False until the saved prefs have loaded (the first-paint cache waits for it). */
    get uiPrefsReady(): boolean {
        return this.#ready;
    }

    /** Whether a setting can be kept for the open books alone (not in browser mode). */
    get canPin(): boolean {
        return this.#storage === "files";
    }

    isPinned(key: PinnableKey): boolean {
        return this.#localKeys.includes(key);
    }

    /** Read the saved prefs (on start, and when a settings file changes on disk). */
    async load(): Promise<void> {
        try {
            this.#adopt(await this.#api.load());
        } catch (error) {
            console.warn("ui prefs:", error);
        } finally {
            this.#ready = true;
        }
    }

    /** Save the matching settings that changed (the Settings page's Save). Throws why not. */
    async saveMatching(values: Partial<Matching>): Promise<void> {
        const changed = Object.fromEntries(MATCHING_KEYS.filter((k) => k in values && values[k] !== this.matching[k]).map((k) => [k, values[k]]));
        if (Object.keys(changed).length === 0) return;
        await this.flush();
        this.#adopt(await this.#api.patch(changed));
    }

    /** Keep `key` for these books only, or stop; false (with `failure` set) when it didn't. */
    async pin(key: PinnableKey, pinned: boolean): Promise<boolean> {
        if (!this.canPin) return false;
        await this.flush();
        try {
            this.#adopt(await this.#api.pin(key, pinned));
            this.failure = null;
            return true;
        } catch (error) {
            this.failure = `This setting couldn't be ${pinned ? "kept for these books" : "shared again"}: ${error instanceof Error ? error.message : String(error)}`;
            return false;
        }
    }

    /** Send what is waiting now, and wait for every save under way. */
    async flush(): Promise<void> {
        this.#cancelTimer();
        this.#send();
        await this.#saving;
    }

    /** As the page hides or unloads, send what is waiting with a request that outlives it. Returns the teardown. */
    flushWhenLeaving(target: Window = window): () => void {
        const leave = (): void => {
            this.#cancelTimer();
            if (Object.keys(this.#pending).length === 0) return;
            this.#api.beacon(this.#pending);
            this.#pending = {};
        };
        const onVisibility = (): void => {
            if (target.document.visibilityState === "hidden") leave();
        };
        target.addEventListener("pagehide", leave);
        target.document.addEventListener("visibilitychange", onVisibility);
        return () => {
            target.removeEventListener("pagehide", leave);
            target.document.removeEventListener("visibilitychange", onVisibility);
        };
    }

    #adopt(answer: UiPrefsAnswer): void {
        const prefs = answer.prefs ?? {};
        // A value still waiting to be saved is newer than the answer.
        if (typeof prefs.theme === "string" && !("theme" in this.#pending)) this.#theme = prefs.theme;
        if (typeof prefs.uiScale === "number" && !("uiScale" in this.#pending)) this.#uiScale = wholeScale(prefs.uiScale);
        this.matching = Object.fromEntries(MATCHING_KEYS.map((k) => [k, k in prefs ? clampMatching(k, prefs[k]) : this.matching[k]])) as Matching;
        if (answer.storage !== undefined) this.#storage = answer.storage;
        if (Array.isArray(answer.localKeys)) this.#localKeys = answer.localKeys.filter((k): k is string => typeof k === "string");
    }

    #queue(values: Record<string, unknown>, delay: number): void {
        this.#pending = {...this.#pending, ...values};
        this.#cancelTimer();
        if (delay === 0) this.#send();
        else this.#timer = setTimeout(() => this.#send(), delay);
    }

    #cancelTimer(): void {
        if (this.#timer !== undefined) clearTimeout(this.#timer);
        this.#timer = undefined;
    }

    #send(): void {
        if (Object.keys(this.#pending).length === 0) return;
        const values = this.#pending;
        this.#pending = {};
        this.#saving = this.#saving.then(async () => {
            try {
                const answer = await this.#api.patch(values);
                if (answer.storage !== undefined) this.#storage = answer.storage;
                if (Array.isArray(answer.localKeys)) this.#localKeys = answer.localKeys.filter((k): k is string => typeof k === "string");
                this.failure = null;
            } catch (error) {
                this.failure = `Your appearance couldn't be saved: ${error instanceof Error ? error.message : String(error)}`;
                console.warn("ui prefs:", error);
            }
        });
    }
}

/** The app's prefs: the layout loads them and applies the appearance from them. */
export const uiPrefs = new UiPrefs();
