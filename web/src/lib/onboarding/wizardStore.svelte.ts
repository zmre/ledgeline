// File → New wizard (plan 39 Phase 6): the wizard's state and the four calls
// it makes. The pure rules live in `model.ts`; navigation is the page's job,
// so every action here returns what happened rather than going anywhere.
//
// One module-level store, like every other screen's: the wizard is one page,
// and a "Start from another app" run must still hold the chosen FILE after the
// engine has switched journals underneath it (see `handOffToImport`).

import {classify} from "#lib/api/editFailure.js";
import {LedgelineApi} from "#lib/api/native.js";
import {decodeImportCapabilities} from "#lib/api/nativeDecode.js";
import {importStore} from "#lib/imports/importStore.svelte.js";
import {refreshEverything} from "#lib/stores/refreshAll.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {decodeCreatedLedger, decodeNewLedgerStatus, decodePickedFolder, decodeTemplate} from "./decode";
import {
    accountProblems,
    commodityProblem,
    createBody,
    CURRENCY_PRESETS,
    CUSTOM_CURRENCY,
    defaultTitle,
    nextStep,
    previousStep,
    removeAccount,
    renameAccount,
    stepsFor,
    titleProblem,
} from "./model";
import type {CreatedLedger, CurrencyStyle, LedgerKind, NewLedgerStatus, PickedFolder, SourceApp, TemplateAccount, WizardPath, WizardStep} from "./types";

/** The folder picker waits on a person; give them an hour rather than the usual request deadline. */
const PICKER_TIMEOUT_MS = 60 * 60 * 1000;

export type WizardBusy = "status" | "template" | "folder" | "create" | "sample" | null;

export type WizardOutcome = {ok: true; created: CreatedLedger} | {ok: false; message: string};

let status = $state<NewLedgerStatus | null>(null);
let step = $state<WizardStep>("start");
let path = $state<WizardPath | null>(null);
let kind = $state<LedgerKind>("individual");
let currencyPreset = $state<string>("usd");
let customCode = $state("");
let customDecimals = $state(2);
let decimalComma = $state(false);
let fiscalYearStart = $state(1);
let title = $state("");
let titleEdited = $state(false);
let accounts = $state.raw<readonly TemplateAccount[]>([]);
let templateKind: LedgerKind | null = null;
let folder = $state<PickedFolder | null>(null);
let source = $state<SourceApp | null>(null);
// `raw`: a File is a host object, and a deep proxy over it would break `arrayBuffer()`.
let sourceFile = $state.raw<File | null>(null);
let readableFormats = $state.raw<readonly string[] | null>(null);
let busy = $state<WizardBusy>(null);
let error = $state<string | null>(null);

function api(timeoutMs?: number): LedgelineApi | null {
    const url = settings.serverUrl;
    return url === null ? null : new LedgelineApi(url, timeoutMs === undefined ? undefined : {timeoutMs});
}

function resetAnswers(): void {
    step = "start";
    path = null;
    kind = "individual";
    currencyPreset = "usd";
    customCode = "";
    customDecimals = 2;
    decimalComma = false;
    fiscalYearStart = 1;
    title = "";
    titleEdited = false;
    accounts = [];
    templateKind = null;
    folder = null;
    source = null;
    sourceFile = null;
    error = null;
}

/** Run `job` as `what`, turning a failure into `error`'s sentence. */
async function run<T>(what: Exclude<WizardBusy, null>, job: (client: LedgelineApi) => Promise<T>, client = api()): Promise<T | null> {
    if (client === null) {
        error = "No Ledgeline engine is connected.";
        return null;
    }
    busy = what;
    error = null;
    try {
        return await job(client);
    } catch (cause) {
        error = classify(cause).message;
        return null;
    } finally {
        busy = null;
    }
}

function currentCurrency(): CurrencyStyle {
    if (currencyPreset === CUSTOM_CURRENCY) {
        return {commodity: customCode.trim(), side: "right", spaced: true, decimals: customDecimals, decimalMark: decimalComma ? "comma" : "period"};
    }
    const preset = CURRENCY_PRESETS.find((entry) => entry.id === currencyPreset) ?? CURRENCY_PRESETS[0];
    return {...preset.style, decimalMark: decimalComma ? "comma" : "period"};
}

export const wizard = {
    get status(): NewLedgerStatus | null {
        return status;
    },
    get step(): WizardStep {
        return step;
    },
    get steps(): readonly WizardStep[] {
        return stepsFor(path, kind);
    },
    get path(): WizardPath | null {
        return path;
    },
    get kind(): LedgerKind {
        return kind;
    },
    get currencyPreset(): string {
        return currencyPreset;
    },
    get customCode(): string {
        return customCode;
    },
    get customDecimals(): number {
        return customDecimals;
    },
    get decimalComma(): boolean {
        return decimalComma;
    },
    get currency(): CurrencyStyle {
        return currentCurrency();
    },
    get fiscalYearStart(): number {
        return fiscalYearStart;
    },
    get title(): string {
        return title;
    },
    get accounts(): readonly TemplateAccount[] {
        return accounts;
    },
    get folder(): PickedFolder | null {
        return folder;
    },
    get source(): SourceApp | null {
        return source;
    },
    get sourceFile(): File | null {
        return sourceFile;
    },
    get readableFormats(): readonly string[] | null {
        return readableFormats;
    },
    get busy(): WizardBusy {
        return busy;
    },
    get error(): string | null {
        return error;
    },

    /** Why the current step can't move on, or null when it can. */
    get blocker(): string | null {
        switch (step) {
            case "start":
                return path === null ? "Choose how to start." : null;
            case "kind":
                return busy === "template" ? "Loading the chart of accounts…" : null;
            case "source":
                if (source === null) return "Choose the app you are coming from.";
                return sourceFile === null ? "Choose the file you exported." : null;
            case "details":
                return commodityProblem(currentCurrency().commodity);
            case "accounts": {
                const problems = accountProblems(accounts);
                return problems.size === 0 ? null : `${problems.size} account${problems.size === 1 ? " needs" : "s need"} fixing.`;
            }
            case "create":
                if (folder === null) return "Choose a folder for the ledger.";
                if (!folder.empty) return "That folder isn't empty. Choose an empty folder, or make a new one.";
                return titleProblem(title);
        }
    },

    /** Start over: what the engine can do, and every answer back to its default. */
    async open(): Promise<void> {
        resetAnswers();
        const loaded = await run("status", async (client) => decodeNewLedgerStatus(await client.newLedgerStatus()));
        status = loaded;
    },

    choosePath(next: WizardPath): void {
        path = next;
        if (next === "import") kind = "empty";
        else if (kind === "empty" && templateKind === null) kind = "individual";
    },

    async chooseKind(next: LedgerKind): Promise<void> {
        kind = next;
        await this.loadTemplate();
    },

    /** Fetch the chart for `kind`, unless it is the one already loaded (keeping the user's edits). */
    async loadTemplate(): Promise<void> {
        if (templateKind === kind) return;
        const wanted = kind;
        const loaded = await run("template", async (client) => decodeTemplate(await client.newLedgerTemplate(wanted)));
        if (loaded !== null && kind === wanted) {
            accounts = loaded;
            templateKind = wanted;
        }
    },

    async next(): Promise<void> {
        if (this.blocker !== null) return;
        if (step === "start" && path === "fresh") await this.loadTemplate();
        const target = nextStep(this.steps, step);
        if (target !== null) step = target;
    },

    back(): void {
        const target = previousStep(this.steps, step);
        if (target !== null) step = target;
        error = null;
    },

    setCurrencyPreset(id: string): void {
        currencyPreset = id;
        const preset = CURRENCY_PRESETS.find((entry) => entry.id === id);
        if (preset !== undefined) decimalComma = preset.style.decimalMark === "comma";
    },
    setCustomCode(value: string): void {
        customCode = value;
    },
    setCustomDecimals(value: number): void {
        customDecimals = Math.max(0, Math.min(4, Math.trunc(value)));
    },
    setDecimalComma(value: boolean): void {
        decimalComma = value;
    },
    setFiscalYearStart(month: number): void {
        fiscalYearStart = month;
    },
    setTitle(value: string): void {
        title = value;
        titleEdited = true;
    },

    renameAccount(index: number, name: string): void {
        accounts = renameAccount(accounts, index, name);
    },
    removeAccount(index: number): void {
        accounts = removeAccount(accounts, index);
    },

    async chooseSource(app: SourceApp): Promise<void> {
        source = app;
        if (readableFormats !== null) return;
        const caps = await run("status", async (client) => decodeImportCapabilities(await client.importCapabilities()));
        readableFormats = caps === null ? null : caps.formats;
        // A failed probe is not the user's problem: the import screen says it again.
        error = null;
    },
    chooseFile(file: File | null): void {
        sourceFile = file;
    },

    /** Show the desktop's folder picker. */
    async pickFolder(): Promise<void> {
        const picked = await run("folder", async (client) => decodePickedFolder(await client.pickNewLedgerFolder()), api(PICKER_TIMEOUT_MS));
        if (picked === null && error !== null) return;
        folder = picked;
        if (picked !== null && !titleEdited) title = defaultTitle(picked.name, kind);
    },

    /** Write the ledger. On success the engine is already serving it. */
    async create(): Promise<WizardOutcome> {
        const blocked = this.blocker;
        if (blocked !== null || folder === null) return {ok: false, message: blocked ?? "Choose a folder."};
        const body = createBody({folderId: folder.id, title, currency: currentCurrency(), fiscalYearStart, accounts: path === "import" ? [] : accounts});
        const created = await run("create", async (client) => decodeCreatedLedger(await client.createNewLedger(body)));
        return created === null ? {ok: false, message: error ?? "The ledger could not be created."} : {ok: true, created};
    },

    /** Open a fresh copy of the sample journal. */
    async trySample(): Promise<WizardOutcome> {
        const created = await run("sample", async (client) => decodeCreatedLedger(await client.openSampleLedger()));
        return created === null ? {ok: false, message: error ?? "The sample could not be opened."} : {ok: true, created};
    },

    /**
     * After "Start from another app" created its empty ledger: bring every
     * screen onto the new journal, and stage the chosen export in the import
     * flow, so the Imports page opens on it.
     */
    async handOffToImport(): Promise<void> {
        const url = settings.serverUrl;
        const file = sourceFile;
        if (url === null || file === null) return;
        // The engine switched journals in place, so everything cached for the
        // old one — the journal feed, the rules index, the import
        // capabilities and their destinations — is re-read first.
        await refreshEverything();
        await importStore.reloadCapabilities(url);
        // MERGE SEAM (plan 39 importers): the import flow reads the file the
        // way it reads any dropped statement — `POST /api/import/stage`, which
        // already routes a QuickBooks Journal .xlsx to the QbJournalPanel. The
        // QIF and grouped-CSV importers plug in behind that same stage call,
        // so nothing here changes when they land. What would help: a way to
        // pass `source.id` along (a preset hint for Xero vs GnuCash CSV, or a
        // `?source=` the Imports route reads), so a CSV that matches several
        // presets is not left to detection alone.
        await importStore.offerFile(file);
    },
};
