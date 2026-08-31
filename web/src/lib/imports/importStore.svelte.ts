// The New Transactions screen's data layer: four async payloads, one form, and
// the preferences blob the hledger banner writes.
//
// Shaped like `rulesStore.svelte.ts` on purpose — `createResource` for anything
// the screen WAITS on, a plain dispatcher for anything it PRESSES — so the two
// stale-response rules (a superseded response is dropped; a payload and the
// question it answers are one value) are inherited rather than re-derived. This
// screen needs the second one more than any other in the app: `dryRun` and
// `commit` are keyed on a stage id, a rules file and a destination, and showing
// a dry run computed for the file the user dropped BEFORE this one is not a
// cosmetic bug, it is the wrong journal getting the wrong transactions.
//
// Everything with a decision in it lives in `importModel.ts`. This file wires,
// sequences and holds; it does not choose.
//
// # No mock layer
//
// The fetch layer is testable because every request goes through `LedgelineApi`,
// which calls the global `fetch` — so a test stubs `fetch` and drives the whole
// store with literal wire JSON, exactly as `native.test.ts` already does. There
// is no injected-transport seam because there does not need to be one, and a
// second implementation of these calls would be a second thing to keep true.

import {classify, type EditFailure} from "$lib/api/editFailure";
import {LedgelineApi, type ImportRunBody} from "$lib/api/native";
import {
    decodeCommitResult,
    decodeConfWritten,
    decodeDryRun,
    decodeImportCapabilities,
    decodePrefs,
    decodeSortResult,
    decodeStagedFile,
} from "$lib/api/nativeDecode";
import {dataView, type DataView} from "$lib/stores/loadState";
import {createResource} from "$lib/stores/resource.svelte";
import {settings} from "$lib/stores/settings.svelte";
import {
    candidateById,
    defaultBalanceAccount,
    defaultJournalId,
    deriveCsvPath,
    formIsBusy,
    headerFilename,
    isInFlight,
    refuseFile,
    sameRunRequest,
    sameWriteRequest,
    type WriteRequest,
} from "./importModel";
import type {CommitResult, ConfWritten, DryRunResult, ImportCapabilities, JournalTarget, Prefs, StagedFile} from "./importTypes";

/** What `staged` is loaded FOR: the file itself, plus a nonce so re-dropping the same file refetches. */
export interface StageQuery {
    readonly file: File;
    readonly attempt: number;
}

export const capabilities = createResource<string, ImportCapabilities>(async (serverUrl) =>
    decodeImportCapabilities(await new LedgelineApi(serverUrl).importCapabilities())
);

export const staged = createResource<StageQuery, StagedFile>(async (serverUrl, query) => {
    // `arrayBuffer()` is the only place the file's bytes exist in this process;
    // they are handed straight to `fetch` and never held in a rune.
    const bytes = await query.file.arrayBuffer();
    return decodeStagedFile(await new LedgelineApi(serverUrl).stageImport(headerFilename(query.file.name), bytes));
});

export const dryRun = createResource<ImportRunBody, DryRunResult>(async (serverUrl, body) =>
    decodeDryRun(await new LedgelineApi(serverUrl).importDryRun(body))
);

/**
 * The write, whichever of the two it is.
 *
 * One resource over a discriminated request rather than two, because the screen
 * has ONE result panel and one "is what I am looking at still the answer to the
 * question on screen" test. Two resources would need a third thing to decide
 * which of them the panel is showing.
 */
export const committed = createResource<WriteRequest, CommitResult>(async (serverUrl, request) => {
    const api = new LedgelineApi(serverUrl);
    const raw = request.kind === "import" ? await api.importCommit(request.body) : await api.importSaveCsv(request.body);
    // Both responses decode as a `CommitResult`: save-csv sends `{csvWritten,
    // git}` and the decoder already reads an absent `journalWritten` as "no
    // journal was touched", which is exactly what happened.
    return decodeCommitResult(raw);
});

// --- the form -------------------------------------------------------------

let selectedRulesId = $state<string | null>(null);
let csvPath = $state("");
/**
 * The user has typed in the CSV field, so changing candidate must not overwrite
 * it. Without this, picking a different rules file silently discards a
 * hand-chosen destination — and the destination is the thing that decides which
 * `.latest` state file the next import will read.
 */
let csvPathTouched = $state(false);
let journalId = $state<string | null>(null);
let balance = $state("");
let balanceAccount = $state("");
/**
 * The user has aimed the assertion themselves, so choosing a different rules
 * file must not re-aim it.
 *
 * ONE field's latch, and named for it. It used to be `balanceTouched` and both
 * setters set it, which made the amount and the account inseparable: typing the
 * closing balance off a paper statement — the ordinary thing to do, and the
 * field the eye lands on first — permanently blocked the account from ever
 * being seeded, and the form then refused to submit because the account it had
 * just stopped filling in was empty.
 *
 * There is deliberately no `balanceTouched` twin. The amount has exactly one
 * seeding moment, `seedFrom`, which runs for a newly staged file and resets
 * every other field in the same breath, so nothing exists that could overwrite
 * what the user typed. A latch with no seeding path to guard would be a flag
 * that is written and never read. If the amount ever gains a second source,
 * that source needs its own latch — not this one.
 */
let balanceAccountTouched = $state(false);
let writeAssertion = $state(true);

/** A file refused before it was uploaded (a `.pdf`, an extension the engine does not read). */
let rejection = $state<string | null>(null);
/** `Save and Import` / `Save CSV` has been pressed at least once for the current staging. */
let dryRunRequested = $state(false);
let writeRequested = $state(false);

// --- prefs + sort (pressed, not waited on) --------------------------------

let prefs = $state<Prefs | null>(null);
let prefsSaving = $state(false);
let prefsError = $state<string | null>(null);
let sorting = $state(false);
let sortMoved = $state<number | null>(null);
let sortError = $state<string | null>(null);

/** The command-line-parity fix: a pressed action with a one-line outcome. */
let confWriting = $state(false);
let confWritten = $state<ConfWritten | null>(null);
let confError = $state<string | null>(null);

let capabilitiesKey: string | null = null;
let stageAttempt = 0;
/**
 * The file the user last offered, so Retry can re-upload one whose FIRST upload
 * failed. `staged.query` is written on success only, so it is null exactly when
 * a retry is most wanted.
 */
let offeredFile: File | null = null;

/** The engine, or null when no server is configured yet. */
function api(): LedgelineApi | null {
    const url = settings.serverUrl;
    return url === null ? null : new LedgelineApi(url);
}

function failureMessage(error: unknown): string {
    const failure: EditFailure = classify(error);
    return failure.message;
}

/** Forget everything downstream of the staged file — a new file invalidates all of it. */
function resetFlow(): void {
    selectedRulesId = null;
    csvPath = "";
    csvPathTouched = false;
    balance = "";
    balanceAccount = "";
    balanceAccountTouched = false;
    writeAssertion = true;
    dryRunRequested = false;
    writeRequested = false;
    sortMoved = null;
    sortError = null;
    confWritten = null;
    confError = null;
}

/**
 * Seed the form from a freshly staged file.
 *
 * Called once per successful stage rather than from an `$effect`, for the same
 * reason `EditRulesPanel` latches its own seeding on document identity: an
 * effect that writes the fields it reads overwrites what the user is typing on
 * every unrelated reactive tick.
 */
function seedFrom(file: StagedFile, journals: readonly JournalTarget[]): void {
    selectedRulesId = file.candidates[0]?.id ?? null;
    csvPath = deriveCsvPath(file.defaults, selectedRulesId);
    csvPathTouched = false;
    journalId = defaultJournalId(file.defaults, journals);
    balance = file.statement?.ledgerBalance ?? "";
    balanceAccountTouched = false;
    seedBalanceAccount();
}

/**
 * Default the assertion's account to the chosen rules file's `account1`.
 *
 * Read straight off the chosen candidate, which carries it. Skipped once the
 * user has typed AN ACCOUNT: an assertion is theirs to aim. Editing the amount
 * is not aiming it and does not stop this.
 */
function seedBalanceAccount(): void {
    if (balanceAccountTouched) return;
    balanceAccount = defaultBalanceAccount(candidateById(staged.value, selectedRulesId));
}

/**
 * The dry-run/commit request as the form currently reads.
 *
 * Null when there is nothing to import: no file staged, or no rules file chosen.
 * The second is not an error state — it is the Save-CSV path, which is
 * `saveCsvBody` and a different route — and returning null here is what makes it
 * impossible to send a dry-run that has nothing to run.
 */
function runBody(): ImportRunBody | null {
    const file = staged.value;
    if (file === null || selectedRulesId === null || journalId === null) return null;
    const trimmedBalance = balance.trim();
    return {
        stageId: file.stageId,
        rulesId: selectedRulesId,
        csvPath: csvPath.trim(),
        journalId,
        balance: trimmedBalance === "" ? null : trimmedBalance,
        balanceAccount: trimmedBalance === "" ? null : balanceAccount.trim(),
    };
}

/** What the write button will send, as the form currently reads. Null when nothing is staged. */
function writeRequest(): WriteRequest | null {
    const file = staged.value;
    if (file === null) return null;
    const run = runBody();
    if (run === null) return {kind: "saveCsv", body: {stageId: file.stageId, csvPath: csvPath.trim()}};
    return {kind: "import", body: {...run, writeAssertion}};
}

export const importStore = {
    // --- capabilities -----------------------------------------------------
    get capabilities(): ImportCapabilities | null {
        return capabilities.value;
    },
    get capabilitiesView(): DataView {
        return capabilities.view;
    },
    get capabilitiesError(): Error | null {
        return capabilities.error;
    },

    /** Probe once per (server, reconnect); the tab host calls this on `onServerReady`. */
    async ensureCapabilities(serverUrl: string, nonce: number): Promise<void> {
        const key = `${nonce}|${serverUrl}`;
        if (key === capabilitiesKey) return;
        capabilitiesKey = key;
        await this.reloadCapabilities(serverUrl);
    },

    /** Re-probe unconditionally — after saving an hledger path, or a Retry. */
    async reloadCapabilities(serverUrl: string): Promise<void> {
        await capabilities.load(serverUrl, serverUrl);
    },

    // --- staging ----------------------------------------------------------
    get staged(): StagedFile | null {
        return staged.value;
    },
    get stagedError(): Error | null {
        return staged.error;
    },
    /**
     * An upload is running right now.
     *
     * The drop target's spinner, and NOTHING else, hangs off this. It was
     * `stagedView === "loading"`, which is true at rest as well — so the panel
     * said "Reading the file…" to a user who had not dropped anything yet, and
     * then, once a file finally landed, went back to "Drop a statement here"
     * exactly when there was something to show. Backwards, both halves.
     */
    get stagingInFlight(): boolean {
        return isInFlight(staged.status);
    },
    /**
     * Whether the staged section has anything to say: a converted file, or a
     * failure to report.
     *
     * NOT "a file has been offered". While the FIRST upload is in flight there
     * is neither, and the drop target's own spinner is the feedback — a second
     * one below it would be the same news twice. It stays true once anything has
     * settled, which is what makes a failed first upload visible at all: the
     * payload is still null there, so gating on `staged.value` (or on
     * `staged.query`, which is written on success only) silently stopped the
     * spinner and showed nothing.
     */
    get hasStagedOutcome(): boolean {
        return staged.query !== null || staged.error !== null;
    },
    get stagedView(): DataView {
        // Matched on the request, not just the status: a second drop must not
        // render the FIRST file's preview while its own upload is in flight.
        // Only ever read once `hasStagedOutcome`, so its idle reading — which
        // `dataView` reports as "loading" — is never on screen.
        return dataView(staged.status, staged.value !== null, staged.query?.attempt === stageAttempt);
    },
    /** A file refused locally (a `.pdf`, an unread extension) — never uploaded. */
    get rejection(): string | null {
        return rejection;
    },

    /**
     * Take a dropped or picked file: refuse what the engine cannot read, upload
     * the rest, and seed the form from the answer.
     */
    async offerFile(file: File): Promise<void> {
        const url = settings.serverUrl;
        if (url === null) return;
        const refusal = refuseFile(file.name, capabilities.value?.formats ?? []);
        rejection = refusal;
        if (refusal !== null) return;
        resetFlow();
        offeredFile = file;
        stageAttempt += 1;
        const attempt = stageAttempt;
        await staged.load(url, {file, attempt});
        const result = staged.value;
        if (result === null || staged.query?.attempt !== attempt) return;
        seedFrom(result, capabilities.value?.journals ?? []);
    },

    /**
     * Re-upload the file already offered (the staged section's Retry).
     *
     * Reads the file the USER offered, not `staged.query.file`: the query is
     * written on success, so a Retry offered after the only interesting failure
     * — the first one — had nothing to retry with and did nothing at all.
     */
    async retryStage(): Promise<void> {
        const file = offeredFile;
        if (file !== null) await this.offerFile(file);
    },

    // --- the form ---------------------------------------------------------
    get selectedRulesId(): string | null {
        return selectedRulesId;
    },
    get csvPath(): string {
        return csvPath;
    },
    get journalId(): string | null {
        return journalId;
    },
    get balance(): string {
        return balance;
    },
    get balanceAccount(): string {
        return balanceAccount;
    },
    get writeAssertion(): boolean {
        return writeAssertion;
    },

    /** Choose a rules file (or none). The CSV destination follows unless the user has set one. */
    selectCandidate(id: string | null): void {
        selectedRulesId = id;
        const file = staged.value;
        if (file !== null && !csvPathTouched) csvPath = deriveCsvPath(file.defaults, id);
        seedBalanceAccount();
        this.invalidateRun();
    },
    setCsvPath(value: string): void {
        csvPath = value;
        csvPathTouched = true;
        this.invalidateRun();
    },
    setJournalId(value: string | null): void {
        journalId = value;
        this.invalidateRun();
    },
    /** Type the closing balance. Deliberately latches nothing — see `balanceAccountTouched`. */
    setBalance(value: string): void {
        balance = value;
        this.invalidateRun();
    },
    setBalanceAccount(value: string): void {
        balanceAccount = value;
        balanceAccountTouched = true;
        this.invalidateRun();
    },
    setWriteAssertion(value: boolean): void {
        writeAssertion = value;
        // Invalidating too: the assertion is written INTO the journal, so a
        // result computed with it on does not describe a run with it off.
        this.invalidateRun();
    },
    /** Any destination change makes an existing dry run answer a question nobody is asking. */
    invalidateRun(): void {
        dryRunRequested = false;
        writeRequested = false;
        sortMoved = null;
        sortError = null;
        // The config-write outcome is reported inside the dry-run panel and
        // carries that run's revision, so it goes stale with the run it belongs
        // to. Leaving it up would show "added 2 aliases" beside a measurement
        // taken before they existed.
        confWritten = null;
        confError = null;
    },

    // --- dry run ----------------------------------------------------------
    get dryRunRequested(): boolean {
        return dryRunRequested;
    },
    get dryRun(): DryRunResult | null {
        return dryRun.value;
    },
    get dryRunError(): Error | null {
        return dryRun.error;
    },
    /** The dry run is running right now. Never `dryRunView === "loading"` — see `isInFlight`. */
    get dryRunInFlight(): boolean {
        return isInFlight(dryRun.status);
    },
    /**
     * Which branch the dry-run panel renders.
     *
     * Only read under `dryRunRequested`, i.e. once a dry run has been asked for.
     * Asked before that it answers "loading" — `dataView` has no idle branch to
     * give — which is why nothing here decides whether a control is BUSY.
     */
    get dryRunView(): DataView {
        return dataView(dryRun.status, dryRun.value !== null, sameRunRequest(dryRun.query, runBody()));
    },

    /** Run the dry run for the form as it stands. */
    async runDryRun(): Promise<void> {
        const url = settings.serverUrl;
        const body = runBody();
        if (url === null || body === null) return;
        writeRequested = false;
        dryRunRequested = true;
        await dryRun.load(url, body);
    },

    // --- commit -----------------------------------------------------------
    get writeRequested(): boolean {
        return writeRequested;
    },
    get committed(): CommitResult | null {
        return committed.value;
    },
    get committedError(): Error | null {
        return committed.error;
    },
    /** The write is running right now: the one thing `Write changes` must not be pressed twice during. */
    get writeInFlight(): boolean {
        return isInFlight(committed.status);
    },
    /** Which branch the result panel renders. Only read under `writeRequested` — see `dryRunView`. */
    get committedView(): DataView {
        return dataView(committed.status, committed.value !== null, sameWriteRequest(committed.query, writeRequest()));
    },

    /**
     * The destinations, the balance and the action button are frozen.
     *
     * Lives here rather than in the panel so it is a tested expression: it was
     * `dryRunView === "loading" || committedView === "loading"` in a `.svelte`
     * file, which was true from mount — nothing had been requested, so neither
     * view had a payload, so both read "loading" — and froze the whole form
     * before the user had done anything. A `.svelte` file is the one place in
     * this repo where that could not have a test on it.
     */
    get formBusy(): boolean {
        return formIsBusy(dryRun.status, committed.status);
    },

    /**
     * Write.
     *
     * With a rules file chosen that is the CSV plus the real import; without
     * one it is the CSV alone, on its own route. The engine refuses either when
     * git blocks; so does the button.
     */
    async writeChanges(): Promise<void> {
        const url = settings.serverUrl;
        const request = writeRequest();
        if (url === null || request === null) return;
        writeRequested = true;
        sortMoved = null;
        sortError = null;
        await committed.load(url, request);
    },

    // --- the post-import re-sort -----------------------------------------
    get sorting(): boolean {
        return sorting;
    },
    get sortMoved(): number | null {
        return sortMoved;
    },
    get sortError(): string | null {
        return sortError;
    },

    /**
     * Apply the confirmed re-sort. A pressed action with a one-line outcome, so
     * it reports inline rather than replacing the result panel with a spinner.
     */
    async resort(): Promise<void> {
        const client = api();
        const target = committed.value?.journalWritten;
        if (client === null || target === undefined || target === null) return;
        sorting = true;
        sortError = null;
        try {
            const sorted = decodeSortResult(await client.importSort(target));
            sortMoved = sorted.moved;
            // A re-sort git did not take leaves the journal rewritten and
            // uncommitted — which dismantles the safety net the import's own
            // commit established moments earlier: `git revert` of the import no
            // longer restores the file. Reported through the same line a failed
            // sort uses, because what the user has to do about it is the same.
            if (sorted.git !== null && !sorted.git.committed) {
                sortError = `The journal was re-sorted, but git did not commit the change. Commit ${target} yourself — until you do, this import can no longer be undone with \`git revert\`.`;
            }
        } catch (error) {
            sortError = failureMessage(error);
        } finally {
            sorting = false;
        }
    },

    // --- the command-line-parity fix --------------------------------------
    get confWriting(): boolean {
        return confWriting;
    },
    get confWritten(): ConfWritten | null {
        return confWritten;
    },
    get confError(): string | null {
        return confError;
    },

    /**
     * Install the journal's aliases into an `hledger.conf` beside it.
     *
     * The revision is the one the dry run reported, so a config file edited in a
     * terminal since this page loaded produces a 409 rather than a silent
     * clobber. Re-runs the dry run afterwards, because the whole point is that
     * the notice should now be gone — and a notice that stays up after its own
     * fix worked teaches the user to ignore it.
     */
    async installConfAliases(revision: string): Promise<void> {
        const url = settings.serverUrl;
        if (url === null) return;
        confWriting = true;
        confError = null;
        try {
            confWritten = decodeConfWritten(await new LedgelineApi(url).writeHledgerConf(revision));
            await this.runDryRun();
        } catch (error) {
            confError = failureMessage(error);
        } finally {
            confWriting = false;
        }
    },

    // --- preferences ------------------------------------------------------
    get prefs(): Prefs | null {
        return prefs;
    },
    get prefsSaving(): boolean {
        return prefsSaving;
    },
    get prefsError(): string | null {
        return prefsError;
    },

    /** Read the preferences blob so the banner's path field starts where the user left it. */
    async loadPrefs(): Promise<void> {
        const client = api();
        if (client === null) return;
        try {
            prefs = decodePrefs(await client.getPrefs());
            prefsError = null;
        } catch (error) {
            // Non-fatal: the banner still works, the field just starts empty.
            prefsError = failureMessage(error);
        }
    },

    /**
     * Store the hledger path and re-probe.
     *
     * The engine validates the path at store time (a non-executable is a 400,
     * not a persisted value that fails on the next import), so a rejection here
     * is the server's own sentence and is shown verbatim.
     */
    async saveHledgerPath(path: string): Promise<boolean> {
        const url = settings.serverUrl;
        if (url === null) return false;
        const trimmed = path.trim();
        prefsSaving = true;
        prefsError = null;
        try {
            prefs = decodePrefs(
                await new LedgelineApi(url).putPrefs({hledgerPath: trimmed === "" ? null : trimmed, gitAutocommit: prefs?.gitAutocommit ?? null})
            );
            await this.reloadCapabilities(url);
            return true;
        } catch (error) {
            prefsError = failureMessage(error);
            return false;
        } finally {
            prefsSaving = false;
        }
    },
};
