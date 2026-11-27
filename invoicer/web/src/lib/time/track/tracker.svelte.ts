// The tracker's shared state (plan 46 T-slots, T-light, T-running): one store
// that Track, the sidebar lamp and the app-wide time keys all read, so a
// press anywhere lights the same lamp.
//
// - `refresh` reads `GET /api/time/state`; the layout calls it on start,
//   after every change event (the watcher follows the time files) and when
//   the window comes back into view (the long-running guard's "on wake").
// - A toggle or stop is optimistic (the < 50 ms budget): the state changes at
//   once, the write follows, and its answer's state is adopted. Writes go out
//   one at a time, in order. A refused write puts the state back (when no
//   later press has moved it on) with a message; a 409 the books moved under
//   it (`external-change`, `conflict`) re-reads the state instead.
// - The live elapsed time counts from when the state arrived, on the
//   browser's monotonic-ish clock, so a server clock skew never shows.
import {EditorApiError} from "#lib/editorApi.js";
import {errorText} from "#lib/labels.js";
import {httpTrackApi, type DayLine, type SlotTarget, type TrackApi, type TrackWrite} from "#lib/time/api/track.js";
import type {Guard, Slot, TrackState} from "#lib/time/types.js";
import {optimisticStop, optimisticToggle} from "./format.js";

/** The slots' pins as five places (`""` empty). */
export const PIN_COUNT = 5;

/** The words for a failure: the server's sentence, never a code or a field key. */
export function failureText(error: unknown): string {
    return errorText(error);
}

/** Codes that mean "the books moved under you": read again rather than revert. */
const MOVED = new Set(["external-change", "conflict"]);

export class Tracker {
    /** The last state read (or predicted); null until the first read. */
    state = $state<TrackState | null>(null);
    /** Why the state couldn't be read; null when it could. */
    loadError = $state<string | null>(null);
    /** Why the last write didn't go through, in words; cleared by the next. */
    message = $state<string | null>(null);
    /** The clock, in ms, ticking each second while a timer runs. */
    now = $state(0);

    readonly #api: TrackApi;
    readonly #clock: () => number;
    /** When `state.running.elapsed` was true (ms on `#clock`). */
    #at = 0;
    #writes: Promise<void> = Promise.resolve();
    #pending = 0;
    /** Bumped on every local change, so a stale answer can tell. */
    #version = 0;
    /** The guard was answered "Keep all" for this start. */
    #keptSince = $state<string | null>(null);
    #reading = 0;

    constructor(api: TrackApi = httpTrackApi, clock: () => number = () => Date.now()) {
        this.#api = api;
        this.#clock = clock;
        this.now = clock();
    }

    get api(): TrackApi {
        return this.#api;
    }

    /** Whether the time files exist. Null until known. */
    get configured(): boolean | null {
        return this.state === null ? null : this.state.configured;
    }

    /** The running timer's seconds, live. */
    get elapsed(): number {
        const running = this.state?.running ?? null;
        if (running === null) return 0;
        return running.elapsed.seconds + Math.max(0, this.now - this.#at) / 1000;
    }

    /** Today's total with the running timer, live. */
    get todaySeconds(): number {
        const state = this.state;
        if (state === null) return 0;
        return state.banked_today.seconds + this.elapsed;
    }

    /** The long-running guard, unless "Keep all" answered it for this start. */
    get guard(): Guard | null {
        const state = this.state;
        if (state === null || state.guard === null || state.running === null) return null;
        return this.#keptSince === state.running.since ? null : state.guard;
    }

    /** A slot by key. */
    slot(key: number): Slot | undefined {
        return this.state?.slots.find((s) => s.key === key);
    }

    /** Advance the clock (the tick, and tests). */
    tick(): void {
        this.now = this.#clock();
    }

    /** Tick each second while mounted; returns the teardown. */
    startClock(intervalMs = 1000): () => void {
        const timer = setInterval(() => {
            if (this.state?.running) this.tick();
        }, intervalMs);
        return () => clearInterval(timer);
    }

    /** Read the state. A failure keeps what is shown (and says why when nothing is). */
    async refresh(): Promise<void> {
        // A write still under way will hand back a newer state itself.
        if (this.#pending > 0) return;
        const mine = ++this.#reading;
        const version = this.#version;
        try {
            const state = await this.#api.state();
            if (mine !== this.#reading || version !== this.#version || this.#pending > 0) return;
            this.#adopt(state);
            this.loadError = null;
        } catch (error) {
            if (mine !== this.#reading) return;
            if (this.state === null) this.loadError = failureText(error);
            else console.warn("time:", error);
        }
    }

    /** Toggle slot `key` (T-slots): stop it if it runs, else switch to it. */
    toggle(key: number): Promise<boolean> {
        return this.#optimistic(
            (state) => optimisticToggle(state, {slot: key}, this.elapsed, this.#clock()),
            () => this.#api.toggle({slot: key}),
            "The timer couldn't be switched"
        );
    }

    /** Toggle an account by name (Other…, the lamp's resume of a slotless account). */
    toggleAccount(account: string, label?: string): Promise<boolean> {
        return this.#optimistic(
            (state) => optimisticToggle(state, {account, label}, this.elapsed, this.#clock()),
            () => this.#api.toggle({account}),
            "The timer couldn't be switched"
        );
    }

    /** Stop (or with `discard`, throw away) the running timer; `at` stops it then instead (*Stop at…*). */
    stop(options: {at?: string; discard?: boolean} = {}): Promise<boolean> {
        if (this.state?.running == null) return Promise.resolve(false);
        const predict = options.at === undefined ? (state: TrackState) => optimisticStop(state, this.elapsed, options.discard ?? false) : () => null;
        return this.#optimistic(predict, () => this.#api.stop(options), "The timer couldn't be stopped");
    }

    /** Keep a long-running timer as it is (the guard's *Keep all*). */
    keepRunning(): void {
        this.#keptSince = this.state?.running?.since ?? null;
    }

    /** Move `target`'s total by `minutes` (on the running one, its start). */
    adjust(target: SlotTarget, minutes: number): Promise<boolean> {
        return this.#optimistic(
            () => null,
            () => this.#api.adjust(target, minutes),
            "The time couldn't be changed"
        );
    }

    /** Snap the running start back to the end of the previous entry. */
    snap(): Promise<boolean> {
        if (this.state?.running == null) return Promise.resolve(false);
        return this.#optimistic(
            () => null,
            () => this.#api.snap(),
            "The start couldn't be moved"
        );
    }

    /** Save today's note on `target`. Throws the server's refusal (the note field shows a `note-tag`'s suggestion). */
    async saveNote(target: SlotTarget, note: string): Promise<void> {
        await this.#serial(() => this.#api.note(target, note));
        this.message = null;
        await this.refresh();
    }

    /** Pin `account` to place `index` (0–4), or clear it with `""`. */
    async savePin(index: number, account: string): Promise<void> {
        const pinned = Array.from({length: PIN_COUNT}, (_, i) => this.state?.pinned[i] ?? "");
        pinned[index] = account;
        // Trailing empty places are left off, as the file keeps them.
        while (pinned.length > 0 && pinned[pinned.length - 1] === "") pinned.pop();
        await this.#serial(() => this.#api.savePrefs({pinned}));
        await this.refresh();
    }

    /** Save who is tracking (P-person); `""` for no one. */
    async savePerson(person: string): Promise<void> {
        const value = person.trim();
        await this.#serial(() => this.#api.savePrefs({person: value === "" ? null : value}));
        await this.refresh();
    }

    /** Today's lines, for the day card. */
    dayLines(): Promise<DayLine[]> {
        const state = this.state;
        if (state === null) return Promise.resolve([]);
        return this.#api.dayLines(state.today, state.person);
    }

    #adopt(state: TrackState): void {
        this.state = state;
        this.#at = this.#clock();
        this.now = this.#at;
    }

    /** Run `request` after every write before it. */
    #serial<T>(request: () => Promise<T>): Promise<T> {
        this.#pending += 1;
        const run = this.#writes.then(request);
        this.#writes = run.then(
            () => undefined,
            () => undefined
        );
        return run.finally(() => {
            this.#pending -= 1;
        });
    }

    async #optimistic(predict: (state: TrackState) => TrackState | null, request: () => Promise<TrackWrite>, failure: string): Promise<boolean> {
        const before = this.state;
        const beforeAt = this.#at;
        if (before !== null) {
            const predicted = predict(before);
            if (predicted !== null) {
                this.#version += 1;
                this.#adopt(predicted);
            }
        }
        const version = this.#version;
        this.message = null;
        try {
            const answer = await this.#serial(request);
            // A later press has its own answer coming: that one is adopted.
            if (this.#pending === 0 && answer.state !== undefined) {
                this.#version += 1;
                this.#adopt(answer.state);
            } else if (this.#pending === 0) {
                await this.refresh();
            }
            return true;
        } catch (error) {
            this.message = `${failure}. ${failureText(error)}`;
            const moved = error instanceof EditorApiError && MOVED.has(error.code);
            if (!moved && version === this.#version && before !== null) {
                this.state = before;
                this.#at = beforeAt;
                this.tick();
            }
            if (this.#pending === 0) await this.refresh();
            return false;
        }
    }
}

/** The app's tracker: the layout reads it on start and after every change. */
export const tracker = new Tracker();
