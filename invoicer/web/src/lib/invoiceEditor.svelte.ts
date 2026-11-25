// The invoice editor's state (invoicer PLAN 4a): the form being edited, the
// revision it was read at, the server's amounts for it, saving, and what to do
// when the file changes on disk meanwhile. The page draws it; the tests drive
// it with a mocked server.
//
// - Every edit asks the server for the amounts (debounced, ~120 ms) and, less
//   often (~500 ms), for what it would book; an answer for an older form is
//   dropped. The preview page is fed the unsaved text (~400 ms).
// - Saving is conditional on the revision read (409 `changed` otherwise). A
//   sent invoice's save asks first, since it re-books (409 `confirm-rebook`).
// - A change on disk to this file: our own save coming back is ignored (same
//   revision); with nothing unsaved here the file is shown; with unsaved
//   edits it is a conflict for the user to settle. Nothing is clobbered.
import {computeTable, type LocalTable} from "./arithmetic.js";
import {
    allTotalWords,
    copyForm,
    DEFAULT_WORDS,
    formsDiffer,
    onDiskChange,
    totalWords,
    type Computed,
    type EditorForm,
    type EditorOptions,
    type EditorState,
    type WouldBook,
} from "./editor.js";
import {errorText} from "./labels.js";
import {computeForm, draftPreview, EditorApiError, fetchForm, saveForm} from "./editorApi.js";

/** Debounces, in milliseconds. */
export const DELAYS = {amounts: 120, books: 500, preview: 400};

const key = (form: EditorForm): string => JSON.stringify(form);

export class InvoiceEditor {
    number = $state("");
    path = $state("");
    title = $state("");
    status = $state("draft");
    revision = $state("");
    form = $state<EditorForm | null>(null);
    /** The form as saved: what `dirty` compares against. */
    baseline = $state<EditorForm | null>(null);
    computed = $state<Computed | null>(null);
    /** The form `computed` answers for. */
    computedFor = $state("");
    books = $state<WouldBook | null>(null);
    options = $state<EditorOptions | null>(null);
    loadError = $state<Error | null>(null);
    saveError = $state<string | null>(null);
    saving = $state(false);
    /** The file on disk, when it changed under unsaved edits. */
    conflict = $state<EditorState | null>(null);
    /** A save waiting for the user's yes: a sent invoice is re-booked. */
    confirmRebook = $state(false);
    previewUrl = $state<string | null>(null);
    /** When the last save landed (for "Saved" feedback). */
    savedAt = $state<number | null>(null);

    readonly dirty = $derived(this.form !== null && this.baseline !== null && formsDiffer(this.form, this.baseline));
    /** The amounts shown aren't the server's for this form yet. */
    readonly pending = $derived(this.form !== null && this.computedFor !== key(this.form));
    /** The display-only amounts for the form as it is. */
    readonly local = $derived<LocalTable>(
        this.form?.table == null
            ? {rows: [], total: 0n}
            : computeTable(
                  this.form.table.columns.map((c) => c.role),
                  this.form.table.rows,
                  allTotalWords(totalWords(this.options, this.options?.words ?? DEFAULT_WORDS))
              )
    );

    // Pending debounces: bookkeeping, never shown, so not reactive on purpose.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    #timers = new Map<string, ReturnType<typeof setTimeout>>();

    /** Load invoice `number` (the URL's). */
    async load(number: string): Promise<void> {
        this.loadError = null;
        try {
            this.adopt(await fetchForm(number));
        } catch (error) {
            this.loadError = error instanceof Error ? error : new Error(String(error));
        }
    }

    /** Show `state` as the file is: the form, its revision, its amounts. */
    adopt(state: EditorState): void {
        this.number = state.number;
        this.path = state.path;
        this.title = state.title;
        this.status = state.status;
        this.revision = state.revision;
        this.form = copyForm(state.form);
        this.baseline = copyForm(state.form);
        this.computed = state.computed;
        this.computedFor = key(state.form);
        this.conflict = null;
        this.confirmRebook = false;
        this.saveError = null;
        this.schedule("books", DELAYS.books, () => this.compute(true));
        this.schedule("preview", 0, () => this.preview());
    }

    /** The user edited the form. */
    edit(form: EditorForm): void {
        this.form = form;
        this.schedule("amounts", DELAYS.amounts, () => this.compute(false));
        this.schedule("books", DELAYS.books, () => this.compute(true));
        this.schedule("preview", DELAYS.preview, () => this.preview());
    }

    /** Ask the server for the form's amounts (and with `books`, what it would book). */
    async compute(books: boolean): Promise<void> {
        const form = this.form;
        if (form === null) return;
        const asked = key(form);
        try {
            const answer = await computeForm(this.number, form, books);
            if (this.form === null || key(this.form) !== asked) return; // edited since: a newer answer is coming
            this.computed = answer;
            this.computedFor = asked;
            if (books) this.books = answer.books;
        } catch (error) {
            if (books)
                this.books = {
                    basis: "accrual",
                    would_book: null,
                    booked: null,
                    in_step: null,
                    receivable: null,
                    error: errorText(error),
                };
        }
    }

    /** Feed the preview page the unsaved text. */
    async preview(): Promise<void> {
        const text = this.computed?.text;
        if (text === undefined || this.number === "") return;
        try {
            this.previewUrl = (await draftPreview(this.number, text, this.form?.template ?? null)).url;
        } catch (error) {
            console.warn("preview:", error);
        }
    }

    /**
     * Save over the revision read. Returns the invoice's number afterwards
     * (a renumbered draft has a new one), or null when it didn't save
     * (a conflict, a refusal, or a confirmation to ask for first).
     */
    async save(options: {rebook?: boolean} = {}): Promise<string | null> {
        const form = this.form;
        if (form === null || this.saving) return null;
        this.saving = true;
        this.saveError = null;
        try {
            const saved = await saveForm(this.number, form, this.revision, options);
            this.revision = saved.revision;
            this.status = saved.status;
            this.path = saved.path;
            this.number = saved.number;
            this.baseline = copyForm(form);
            this.confirmRebook = false;
            this.savedAt = Date.now();
            this.schedule("books", 0, () => this.compute(true));
            return saved.number;
        } catch (error) {
            if (error instanceof EditorApiError && error.code === "changed") {
                await this.showConflict();
            } else if (error instanceof EditorApiError && error.code === "confirm-rebook") {
                this.confirmRebook = true;
            } else {
                this.saveError = errorText(error);
                this.confirmRebook = false;
            }
            return null;
        } finally {
            this.saving = false;
        }
    }

    async showConflict(): Promise<void> {
        try {
            this.conflict = await fetchForm(this.number);
        } catch (error) {
            this.saveError = errorText(error);
        }
    }

    /** Files changed on disk (`paths` `[]`: anything may have). */
    async diskChanged(paths: string[]): Promise<void> {
        if (this.form === null || (paths.length > 0 && !paths.includes(this.path))) return;
        let theirs: EditorState;
        try {
            theirs = await fetchForm(this.number);
        } catch (error) {
            console.warn("reload:", error);
            return;
        }
        switch (onDiskChange(this.revision, theirs.revision, this.dirty)) {
            case "ignore":
                return;
            case "replace":
                this.adopt(theirs);
                return;
            case "conflict":
                this.conflict = theirs;
        }
    }

    /** Drop the unsaved edits: the form as last saved (or read), its amounts asked for again. */
    discard(): void {
        if (this.baseline === null || !this.dirty) return;
        this.edit(copyForm(this.baseline));
        this.saveError = null;
        this.confirmRebook = false;
    }

    /** Settle a conflict with their file: the edits here are dropped. */
    reloadTheirs(): void {
        if (this.conflict !== null) this.adopt(this.conflict);
    }

    /** Settle a conflict with these edits, saved over their file (its revision is now the one expected). */
    async keepMine(): Promise<string | null> {
        if (this.conflict === null) return null;
        this.revision = this.conflict.revision;
        this.conflict = null;
        return this.save();
    }

    /** Stop every pending timer (the page is going). */
    dispose(): void {
        for (const timer of this.#timers.values()) clearTimeout(timer);
        this.#timers.clear();
    }

    private schedule(name: string, delay: number, run: () => Promise<void>): void {
        const old = this.#timers.get(name);
        if (old !== undefined) clearTimeout(old);
        this.#timers.set(
            name,
            setTimeout(() => {
                this.#timers.delete(name);
                void run();
            }, delay)
        );
    }
}
