// Entries' state: the list for the URL's query, the selection, and every
// write, each followed by a fresh list (ids shift when lines move, so the
// page never patches what it holds). Only the latest request's answer is
// shown: a slow answer for an older query never replaces a newer list, and
// a list already on screen stays through a failed reload.
import {SvelteSet} from "svelte/reactivity";
import {fetchEntries, refusal, type EntryList, type WriteOutcome} from "../api/entries.js";
import {peopleOf} from "./group.js";

export type {WriteOutcome};

/** The codes after which what's on screen is stale: the list is fetched again even though the write failed. */
export const STALE: readonly string[] = ["conflict", "external-change", "locked", "not-found"];

export class EntriesStore {
    list = $state<EntryList | null>(null);
    error = $state<Error | null>(null);
    /** The ids picked for a bulk action. */
    readonly selected = new SvelteSet<string>();
    /** Everyone seen in any list this session: the person filter's choices survive a filter by person. */
    readonly people = new SvelteSet<string>();
    #query = "";
    #latest = 0;
    readonly #fetch: typeof fetchEntries;

    constructor(fetcher: typeof fetchEntries = fetchEntries) {
        this.#fetch = fetcher;
    }

    /** Loading, the list, or a first load that failed. */
    get view(): "loading" | "data" | "error" {
        return this.error !== null ? "error" : this.list !== null ? "data" : "loading";
    }

    /** Fetch the list for `query` (the API's query string). */
    load(query: string = this.#query): Promise<void> {
        if (query !== this.#query) this.selected.clear();
        this.#query = query;
        const mine = ++this.#latest;
        return this.#fetch(query).then(
            (list) => {
                if (mine !== this.#latest) return;
                this.list = list;
                this.error = null;
                for (const person of peopleOf(list.entries)) this.people.add(person);
                // A selected line that left the list can't be acted on.
                const ids = list.entries.map((e) => e.id);
                for (const id of [...this.selected]) if (!ids.includes(id)) this.selected.delete(id);
            },
            (e: unknown) => {
                if (mine !== this.#latest) return;
                if (this.list === null) this.error = e instanceof Error ? e : new Error(String(e));
                else console.warn("time entries:", e);
            }
        );
    }

    /** Start over after a failed first load. */
    retry(): Promise<void> {
        this.error = null;
        this.list = null;
        return this.load();
    }

    /**
     * Run a write, then fetch the list again (ids shift after any write, so
     * the selection is dropped). A refusal comes back with its code; a stale
     * view (a conflict, a line that moved) is reloaded too.
     */
    async write(op: () => Promise<unknown>): Promise<WriteOutcome> {
        try {
            await op();
        } catch (e) {
            const outcome = refusal(e);
            if (STALE.includes(outcome.code)) await this.load();
            return outcome;
        }
        this.selected.clear();
        await this.load();
        return {ok: true};
    }

    /** Pick or drop one line. */
    toggle(id: string): void {
        if (this.selected.has(id)) this.selected.delete(id);
        else this.selected.add(id);
    }

    /** Pick every line in `ids` (shift-click's range), or drop them all when every one is already picked. */
    toggleRange(ids: readonly string[]): void {
        const all = ids.every((id) => this.selected.has(id));
        for (const id of ids) {
            if (all) this.selected.delete(id);
            else this.selected.add(id);
        }
    }
}
