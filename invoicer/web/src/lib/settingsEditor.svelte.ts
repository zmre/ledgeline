// The "These books" form's state and Save (SETTINGS-PLAN §3, §4, D1, D5).
//
// Save is the explicit action, and asks first when it must:
// 1. Problems the page can see itself keep Save off. The server's check
//    (`POST /api/settings/check`, live while editing and again on Save) marks
//    the rest by setting.
// 2. A change refused outright (D5: deposit payments in cash books while a
//    deposit waits for its final invoice) opens the refusal, listing them.
// 3. A change that affects new bookings opens a confirmation with each
//    effect's sentence; Save goes on with them confirmed.
// 4. Books with no settings file ask before one is created.
// 5. The settings file is written (`PUT /api/settings`), then the shared
//    matching settings (`PATCH /api/ui-prefs`).
//
// Live reload: after a change on disk the page refetches. With nothing
// unsaved it takes the file as it is; with edits it keeps them and says the
// file changed (the conflict notice), and Save is refused until the user
// reloads or keeps theirs over it. This page's own save, coming back, isn't
// a change.
import {EditorApiError} from "./editorApi.js";
import {settingsMessage} from "./labels.js";
import type {Matching} from "./matching.js";
import type {Effect, Refusal, SettingKey, SettingName, SettingProblem, SettingsChange, SettingsCheck, SettingsPage, SettingsSaved} from "./settings.js";
import {effectsOf, problemsOf, refusalOf} from "./settingsApi.js";
import {
    baselineOf,
    changedKeys,
    changesOf,
    formOf,
    isEmpty,
    localProblems,
    withDefault,
    withValue,
    type Baseline,
    type Changes,
    type Form,
} from "./settingsForm.js";
import type {SaveOutcome} from "./unsavedGuard.svelte.js";

/** The requests the form makes; the test passes its own. */
export interface SettingsEditorApi {
    fetch: () => Promise<SettingsPage>;
    check: (change: SettingsChange) => Promise<SettingsCheck>;
    save: (change: SettingsChange) => Promise<SettingsSaved>;
    /** The shared matching settings as saved now. */
    matching: () => Matching;
    saveMatching: (values: Partial<Matching>) => Promise<void>;
}

/** What Save is waiting on the user for. */
export type Question = {kind: "create"} | {kind: "confirm"; effects: Effect[]} | {kind: "refused"; refusal: Refusal};

/** A caught error in the page's words. */
const messageOf = (error: unknown): string => settingsMessage(error instanceof Error ? error.message : String(error));

/** How long live validation waits for typing to settle. */
export const CHECK_MS = 400;

export class SettingsEditor {
    page = $state<SettingsPage | null>(null);
    base = $state<Baseline | null>(null);
    form = $state<Form | null>(null);
    loadError = $state<string | null>(null);
    saving = $state(false);
    saveError = $state<string | null>(null);
    /** The server's problems with the values as edited, by setting. */
    serverProblems = $state<SettingProblem[]>([]);
    /** The question Save waits on; its dialog is open while set. */
    question = $state<Question | null>(null);
    /** The settings file changed on disk under unsaved edits: its revision now. */
    conflict = $state<string | null>(null);
    /** "Saved." after a save, until the next edit. */
    savedNote = $state<string | null>(null);

    readonly #api: SettingsEditorApi;
    readonly #checkMs: number;
    #checkTimer: ReturnType<typeof setTimeout> | undefined;
    /** The revision this page last wrote: its own save coming back isn't a conflict. */
    #lastSaved: string | null = null;

    constructor(api: SettingsEditorApi, checkMs = CHECK_MS) {
        this.#api = api;
        this.#checkMs = checkMs;
    }

    /** What Save would send. */
    get changes(): Changes | null {
        return this.base === null || this.form === null ? null : changesOf(this.base, this.form, this.#api.matching());
    }

    get dirty(): boolean {
        const changes = this.changes;
        return changes !== null && !isEmpty(changes);
    }

    /** The settings with unsaved changes (the rail's dots). */
    get changed(): Set<string> {
        const changes = this.changes;
        return changedKeys(changes ?? {set: {}, unset: [], matching: {}});
    }

    /** Nothing may be saved: the file is unusable, or its settings don't read. */
    get readOnly(): boolean {
        return this.page !== null && (this.page.readOnly || this.page.file === "unusable");
    }

    /** Every problem by setting: the page's own, then the server's. */
    get problems(): Record<string, string> {
        const own = this.form === null ? {} : localProblems(this.form);
        const all: Record<string, string> = {...own};
        for (const p of this.serverProblems) if (p.key !== null && !(p.key in all)) all[p.key] = p.message;
        return all;
    }

    /** The page's own problems keep Save off. */
    get canSave(): boolean {
        return this.dirty && !this.saving && !this.readOnly && this.form !== null && Object.keys(localProblems(this.form)).length === 0;
    }

    async load(): Promise<void> {
        this.loadError = null;
        try {
            this.#adopt(await this.#api.fetch());
        } catch (e) {
            this.loadError = messageOf(e);
        }
    }

    /** After a change on disk: take it when nothing is unsaved; else say so. */
    async refresh(): Promise<void> {
        if (this.saving) return;
        try {
            const next = await this.#api.fetch();
            if (!this.dirty) {
                this.#adopt(next);
                return;
            }
            const revision = this.page?.revision ?? null;
            if (next.revision !== revision && next.revision !== this.#lastSaved) this.conflict = next.revision ?? "";
        } catch (e) {
            console.warn("settings:", e);
        }
    }

    /** Edit one setting. */
    set(key: string, value: unknown): void {
        if (this.form === null) return;
        this.form = withValue(this.form, key, value);
        this.#edited();
    }

    /** "Use the default". */
    useDefault(key: string): void {
        if (this.form === null || this.base === null) return;
        this.form = withDefault(this.form, this.base, key);
        this.#edited();
    }

    /** Undo the change to one setting: as the file has it. */
    revert(key: string): void {
        if (this.form === null || this.base === null) return;
        const name = key as SettingName;
        this.form = {
            ...this.form,
            values: {...this.form.values, [name]: this.base.values[name]},
            reset: this.form.reset.filter((k) => k !== name),
            matching: Object.fromEntries(Object.entries(this.form.matching).filter(([k]) => k !== key)),
        };
        this.#edited();
    }

    /** Discard changes: the form as the file is. */
    discard(): void {
        if (this.base !== null) this.form = formOf(this.base);
        this.serverProblems = [];
        this.saveError = null;
        this.question = null;
    }

    /** Reload theirs (the conflict notice): the file as it is now, edits dropped. */
    async reloadTheirs(): Promise<void> {
        this.conflict = null;
        await this.load();
    }

    /** Keep mine: save over the file as it is now. */
    async keepMine(): Promise<SaveOutcome> {
        const over = this.conflict;
        this.conflict = null;
        return this.save({over: over ?? undefined});
    }

    /**
     * Save: check, then ask what must be asked, then write. "asked" while a
     * question waits (answer it with `answer`), "failed" with `saveError` or
     * the marked problems, "saved" when everything went out.
     */
    async save(options: {over?: string; confirmed?: SettingKey[]; createFile?: boolean} = {}): Promise<SaveOutcome> {
        const changes = this.changes;
        if (changes === null || this.page === null || this.readOnly) return "failed";
        if (isEmpty(changes)) return "saved";
        if (this.conflict !== null && options.over === undefined) {
            this.saveError = "The settings file changed outside the app. Reload it, or keep your changes and save over it.";
            return "failed";
        }
        this.#cancelCheck();
        this.saving = true;
        this.saveError = null;
        this.savedNote = null;
        try {
            const fileChanged = Object.keys(changes.set).length > 0 || changes.unset.length > 0;
            if (fileChanged) {
                const outcome = await this.#writeFile(changes, options);
                if (outcome !== "saved") return outcome;
            }
            if (Object.keys(changes.matching).length > 0) await this.#api.saveMatching(changes.matching);
            this.#adopt(await this.#api.fetch());
            this.savedNote = "Saved.";
            return "saved";
        } catch (e) {
            this.saveError = messageOf(e);
            return "failed";
        } finally {
            this.saving = false;
        }
    }

    /** The answer to the question Save asked: go on, or (false) stay as it is. */
    async answer(goOn: boolean): Promise<SaveOutcome> {
        const asked = this.question;
        this.question = null;
        if (asked === null || !goOn || asked.kind === "refused") return "failed";
        if (asked.kind === "create") return this.save({createFile: true, confirmed: this.#confirmed});
        this.#confirmed = asked.effects.map((e) => e.key);
        return this.save({confirmed: this.#confirmed, over: this.#over});
    }

    /** The effects confirmed for this save, and the revision it saves over. */
    #confirmed: SettingKey[] = [];
    #over: string | undefined;

    async #writeFile(changes: Changes, options: {over?: string; confirmed?: SettingKey[]; createFile?: boolean}): Promise<SaveOutcome> {
        const page = this.page as SettingsPage;
        const change: SettingsChange = {revision: options.over ?? page.revision, set: changes.set, unset: changes.unset};
        this.#over = options.over;
        const confirmed = options.confirmed ?? [];
        if (confirmed.length === 0) {
            // Before anything is written: what is wrong, refused or to be confirmed.
            const check = await this.#api.check(change);
            this.serverProblems = check.problems;
            if (check.problems.length > 0) {
                this.saveError = "Some settings need a change first; they're marked below.";
                return "failed";
            }
            if (check.refused !== null) return this.#ask({kind: "refused", refusal: check.refused});
            if (check.effects.length > 0) return this.#ask({kind: "confirm", effects: check.effects});
        }
        if (page.file === "missing" && options.createFile !== true) return this.#ask({kind: "create"});
        try {
            const saved = await this.#api.save({...change, createFile: options.createFile === true, confirmed});
            this.#lastSaved = saved.revision;
            this.#confirmed = [];
            this.#over = undefined;
            return "saved";
        } catch (e) {
            if (!(e instanceof EditorApiError)) throw e;
            switch (e.code) {
                case "changed":
                    this.conflict = e.revision ?? "";
                    this.saveError = "The settings file changed outside the app. Reload it, or keep your changes and save over it.";
                    return "failed";
                case "no-file":
                    return this.#ask({kind: "create"});
                case "confirm":
                    return this.#ask({kind: "confirm", effects: effectsOf(e)});
                case "refused": {
                    const refusal = refusalOf(e);
                    if (refusal !== null) return this.#ask({kind: "refused", refusal});
                    throw e;
                }
                case "invalid":
                    this.serverProblems = problemsOf(e);
                    this.saveError = "Some settings need a change first; they're marked below.";
                    return "failed";
                case "unusable":
                    // The page turns read-only and says why.
                    this.page = await this.#api.fetch();
                    throw e;
                default:
                    throw e;
            }
        }
    }

    #ask(question: Question): SaveOutcome {
        this.question = question;
        return "asked";
    }

    #adopt(page: SettingsPage): void {
        this.page = page;
        this.base = baselineOf(page);
        this.form = formOf(this.base);
        this.conflict = null;
        this.serverProblems = [];
        this.saveError = null;
        this.question = null;
    }

    #edited(): void {
        this.savedNote = null;
        this.saveError = null;
        this.#scheduleCheck();
    }

    #cancelCheck(): void {
        if (this.#checkTimer !== undefined) clearTimeout(this.#checkTimer);
        this.#checkTimer = undefined;
    }

    /** Live validation: ask the server about the values once typing settles. */
    #scheduleCheck(): void {
        this.#cancelCheck();
        if (this.readOnly) return;
        this.#checkTimer = setTimeout(() => {
            this.#checkTimer = undefined;
            const changes = this.changes;
            if (changes === null || this.page === null || (Object.keys(changes.set).length === 0 && changes.unset.length === 0)) {
                this.serverProblems = [];
                return;
            }
            const sent = JSON.stringify(changes);
            this.#api
                .check({revision: this.page.revision, set: changes.set, unset: changes.unset})
                .then((check) => {
                    // Only for the values it was asked about.
                    if (JSON.stringify(this.changes) === sent) this.serverProblems = check.problems;
                })
                .catch((e: unknown) => console.warn("settings check:", e));
        }, this.#checkMs);
    }
}
