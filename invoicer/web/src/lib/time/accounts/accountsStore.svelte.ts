// The Accounts page's state: the tree for a month, which branches are
// folded, and every write, each followed by a fresh tree (a write edits the
// accounts file, and a rename every time file). Latest request wins; a tree
// on screen stays through a failed reload.
import {SvelteSet} from "svelte/reactivity";
import {fetchAccounts, type AccountsView} from "../api/accounts.js";
import {refusal, type WriteOutcome} from "../api/entries.js";
import {STALE} from "../entries/entriesStore.svelte.js";

export class AccountsStore {
    view = $state<AccountsView | null>(null);
    error = $state<Error | null>(null);
    /** Folded branches, by account. */
    readonly folded = new SvelteSet<string>();
    #month: string | undefined;
    #latest = 0;
    readonly #fetch: typeof fetchAccounts;

    constructor(fetcher: typeof fetchAccounts = fetchAccounts) {
        this.#fetch = fetcher;
    }

    get state(): "loading" | "data" | "error" {
        return this.error !== null ? "error" : this.view !== null ? "data" : "loading";
    }

    /** Fetch the tree with `month` (`YYYY-MM`) for the month column. */
    load(month: string | undefined = this.#month): Promise<void> {
        this.#month = month;
        const mine = ++this.#latest;
        return this.#fetch(month).then(
            (view) => {
                if (mine !== this.#latest) return;
                this.view = view;
                this.error = null;
            },
            (e: unknown) => {
                if (mine !== this.#latest) return;
                if (this.view === null) this.error = e instanceof Error ? e : new Error(String(e));
                else console.warn("time accounts:", e);
            }
        );
    }

    retry(): Promise<void> {
        this.error = null;
        this.view = null;
        return this.load();
    }

    /** Run a write, then fetch the tree again; a refusal comes back with its code. */
    async write(op: () => Promise<unknown>): Promise<WriteOutcome> {
        try {
            await op();
        } catch (e) {
            const outcome = refusal(e);
            if (STALE.includes(outcome.code)) await this.load();
            return outcome;
        }
        await this.load();
        return {ok: true};
    }

    /** Fold or unfold `account`'s branch. */
    toggleFold(account: string): void {
        if (this.folded.has(account)) this.folded.delete(account);
        else this.folded.add(account);
    }
}
