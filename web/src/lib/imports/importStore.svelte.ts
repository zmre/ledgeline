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

import {classify, type EditFailure, type EditResult} from "#lib/api/editFailure.js";
import {LedgelineApi, type ImportRunBody} from "#lib/api/native.js";
import {decodeRulePreview} from "#lib/api/importReviewDecode.js";
import {
    decodeCommitResult,
    decodeConfWritten,
    decodeDryRun,
    decodeImportCapabilities,
    decodePrefs,
    decodeRulesDoc,
    decodeRulesDraft,
    decodeSortResult,
    decodeStagedFile,
} from "#lib/api/nativeDecode.js";
import {createSaveRequest, draftForm, newRulesId} from "./createModel";
import type {RulesDraft} from "./types";
import type {FormItem, RulesForm} from "./model";
import {baseName, importPrefill} from "./ruleBuilder";
import {appendRuleTo} from "./ruleSave";
import {dataView, type DataView} from "#lib/stores/loadState.js";
import {createResource} from "#lib/stores/resource.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
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
import type {ReplayReport, RulePreview, RuleSource, RuleSpec, StagedOverride, StagedRef, StagedSink, StagedTxn} from "./reviewTypes";
import {createStagedEdits} from "./stagedEdits.svelte";

/** What `staged` is loaded FOR: the file itself, plus a nonce so re-dropping the same file refetches. */
export interface StageQuery {
    readonly file: File;
    readonly attempt: number;
    /** Which account of a multi-account upload to read — an index into `StagedFile.statements`. 0 for every other file. */
    readonly statement: number;
}

export const capabilities = createResource<string, ImportCapabilities>(async (serverUrl) =>
    decodeImportCapabilities(await new LedgelineApi(serverUrl).importCapabilities())
);

export const staged = createResource<StageQuery, StagedFile>(async (serverUrl, query) => {
    // `arrayBuffer()` is the only place the file's bytes exist in this process;
    // they are handed straight to `fetch` and never held in a rune.
    const bytes = await query.file.arrayBuffer();
    // The header is only sent for a later account, so every ordinary upload is
    // the request it always was.
    const statement = query.statement === 0 ? undefined : query.statement;
    return decodeStagedFile(await new LedgelineApi(serverUrl).stageImport(headerFilename(query.file.name), bytes, statement));
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
    // Both responses decode as a `CommitResult`: save-csv sends `{csvWritten}`
    // and the decoder already reads an absent `journalWritten` as "no journal
    // was touched", which is exactly what happened.
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

// --- creating a rules file (WP-16 Phase 2) --------------------------------
//
// Reached from the candidate list when NOTHING scored against the drop, which
// is the one state where the New Transactions tab has nothing else to offer.
// It lives in this store rather than its own because it is keyed on the staged
// upload — the whole point is that the user has already dropped the file, so
// asking them to upload it again to describe it would be asking twice.

/** The panel is open. Closed is the resting state; nothing is fetched until it opens. */
let creating = $state(false);
/** The engine's draft, or null before the first one arrives. */
let draft = $state<RulesDraft | null>(null);
/**
 * The editable form built from the draft.
 *
 * A `$state` object, deliberately: the mapping and account panels write into it
 * through the same `withFieldNames`/`withSetting` the Edit Rules tab uses, and
 * those return new arrays that this field is reassigned to.
 */
let createForm = $state<RulesForm | null>(null);
/** The new file's id. Seeded from the CSV destination, then the user's to change. */
let createId = $state("");
let createDrafting = $state(false);
let createSaving = $state(false);
let createError = $state<string | null>(null);
/** The id that was written, once one has been. Null until then. */
let createdId = $state<string | null>(null);

/** A file refused before it was uploaded (a `.pdf`, an extension the engine does not read). */
let rejection = $state<string | null>(null);
/** `Test Import` / `Save CSV` has been pressed at least once for the current staging. */
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

// --- review before writing (plan 32 D23–D26) -------------------------------
//
// The review overlay's edits to the staged transactions are OVERRIDES, held
// here by ordinal and sent with every dry run and the commit, so the preview
// shows exactly the bytes that will be appended. Its edits to similar journal
// transactions are queued too, and land in the commit's own write (D24).
// Closing the overlay keeps them all; Write changes is never blocked (D26).

const edits = createStagedEdits();
/** What the last replay did, so the panel can say "2 edits were covered by your rule". Null until one ran. */
let lastReplay = $state.raw<ReplayReport | null>(null);

let capabilitiesKey: string | null = null;
let stageAttempt = 0;
/**
 * The file the user last offered, so Retry can re-upload one whose FIRST upload
 * failed. `staged.query` is written on success only, so it is null exactly when
 * a retry is most wanted.
 */
let offeredFile: File | null = null;
/** Which account of `offeredFile` was asked for, so Retry and a rules-file save re-read the same one. */
let offeredStatement = 0;

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
    closeCreate();
    createdId = null;
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
    // A new staging is a different import: the review edits belong to the old one.
    edits.clear();
    lastReplay = null;
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
        // Only when there are some: a body with no review edits is exactly the
        // one sent before review existed.
        ...withOverrides(),
    };
}

function withOverrides(): Pick<ImportRunBody, "overrides"> {
    const overrides = edits.csvOverrides();
    return overrides.length === 0 ? {} : {overrides};
}

/** What the write button will send, as the form currently reads. Null when nothing is staged. */
function writeRequest(): WriteRequest | null {
    const file = staged.value;
    if (file === null) return null;
    const run = runBody();
    if (run === null) return {kind: "saveCsv", body: {stageId: file.stageId, csvPath: csvPath.trim()}};
    const journalEdits = edits.journalEdits;
    return {kind: "import", body: {...run, writeAssertion, ...(journalEdits.length === 0 ? {} : {journalEdits: [...journalEdits]})}};
}

/** Drop the draft and everything derived from it. */
function closeCreate(): void {
    creating = false;
    draft = null;
    createForm = null;
    createId = "";
    createError = null;
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
    async offerFile(file: File, statement = 0): Promise<void> {
        const url = settings.serverUrl;
        if (url === null) return;
        const refusal = refuseFile(file.name, capabilities.value?.formats ?? []);
        rejection = refusal;
        if (refusal !== null) return;
        resetFlow();
        offeredFile = file;
        offeredStatement = statement;
        stageAttempt += 1;
        const attempt = stageAttempt;
        await staged.load(url, {file, attempt, statement});
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
        if (file !== null) await this.offerFile(file, offeredStatement);
    },

    /**
     * Read another account of a multi-account upload.
     *
     * A fresh staging of the same file, not a filter over the old one: each
     * account is its own import, with its own rules file, CSV and balance, so
     * everything downstream of the old staging is reset exactly as a new drop
     * resets it.
     */
    async chooseStatement(statement: number): Promise<void> {
        const file = offeredFile;
        if (file === null || statement === offeredStatement) return;
        await this.offerFile(file, statement);
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

    /**
     * Run the dry run for the form as it stands, review overrides included.
     *
     * A 409 means an override no longer matches what the rules generate (a
     * rule was saved, a rules file changed). Then the import is regenerated
     * WITHOUT overrides, they are replayed against it (D25: covered by a rule
     * → dropped, unaffected → kept, changed underneath → kept and flagged), and
     * the dry run is run again with what is left. The panel never shows a run
     * carrying edits the user did not make.
     *
     * That 409 is EXPECTED, not a failure: saving a rule from the review card
     * always changes the text the card's own override was fingerprinted
     * against. So it is resolved inside one `dryRun.run` and never reaches the
     * panel. Loading it into `dryRun` first used to flash the conflict as an
     * error under the review overlay until the replayed run landed (Patrick,
     * 2026-10-31). Only a failure of the final run is shown.
     */
    async runDryRun(): Promise<void> {
        const url = settings.serverUrl;
        const body = runBody();
        if (url === null || body === null) return;
        writeRequested = false;
        dryRunRequested = true;
        const api = new LedgelineApi(url);
        await dryRun.run(async () => {
            try {
                return {query: body, value: decodeDryRun(await api.importDryRun(body))};
            } catch (error) {
                if (body.overrides === undefined || classify(error).kind !== "conflict") throw error;
            }
            const bare = decodeDryRun(await api.importDryRun({...body, overrides: undefined}));
            // hledger refused the rules file outright: there is nothing to
            // replay against, and that refusal is the answer to show.
            if (!bare.ok) return {query: {...body, overrides: undefined}, value: bare};
            lastReplay = edits.replay(bare.txns);
            const replayed = runBody() ?? body;
            return {query: replayed, value: decodeDryRun(await api.importDryRun(replayed))};
        });
    },

    /** Re-run the dry run — after the review overlay closed, or a rule was saved (D25). Same as `runDryRun`. */
    async refreshDryRun(): Promise<void> {
        await this.runDryRun();
    },

    // --- review edits (plan 32 D23–D25) -----------------------------------
    /** The staged transactions of the last successful dry run, as review cards. Empty before one. */
    get stagedTxns(): readonly StagedTxn[] {
        const run = dryRun.value;
        return run !== null && run.ok ? run.txns : [];
    },
    /** Every held override, in the order first made. */
    get overrides(): readonly StagedOverride[] {
        return edits.overrides;
    },
    /** "N reviewed edits" beside Write changes. */
    get overrideCount(): number {
        return edits.overrideCount;
    },
    /** Similar journal transactions the commit will also edit (D24). */
    get journalEditCount(): number {
        return edits.journalEditCount;
    },
    /** Overrides a replay found changed underneath them. */
    get flaggedOverrides(): readonly StagedOverride[] {
        return edits.flagged;
    },
    /** What the last 409-triggered replay did, or null. */
    get lastReplay(): ReplayReport | null {
        return lastReplay;
    },
    /** The review store's staged sink: saves become overrides and queued journal edits, not writes. */
    reviewSink: {stage: (save) => edits.stage(save)} satisfies StagedSink as StagedSink,

    /**
     * The rule builder's live check (D25): this import with `rule` appended to
     * a temporary copy of the chosen rules file. Null when the form names no
     * rules file or journal. Pass `signal` to cancel a superseded check (it
     * then rejects with the abort).
     */
    async rulePreview(rule: RuleSpec, signal?: AbortSignal): Promise<RulePreview | null> {
        const url = settings.serverUrl;
        const run = runBody();
        if (url === null || run === null) return null;
        const body = {
            stageId: run.stageId,
            rulesId: run.rulesId,
            csvPath: run.csvPath,
            journalId: run.journalId,
            rule: {
                matchers: rule.matchers.map((matcher) => ({field: matcher.field, pattern: matcher.pattern})),
                assignments: rule.assignments.map((assignment) => ({field: assignment.field, value: assignment.value})),
            },
        };
        return decodeRulePreview(await new LedgelineApi(url, {signal}).importRulePreview(body));
    },

    /**
     * "Remember as a rule" (D25): append `rule` to the import's rules file
     * through the rules editor's own PUT, pinned to the revision just read, at
     * the END (the last match wins). Writes only the rules file; the caller
     * stages the card's override and then `refreshDryRun()`s, and the commit
     * picks the edited rules file up by itself (4a amendment 5).
     */
    async saveReviewRule(rule: RuleSpec): Promise<EditResult> {
        const url = settings.serverUrl;
        const id = runBody()?.rulesId ?? null;
        if (url === null || id === null) return {ok: false, failure: {kind: "unavailable", message: "This import has no rules file to add a rule to."}};
        return appendRuleTo(url, id, rule);
    },

    /** The review overlay's rule builder's view of this import (D25), or null when nothing is staged for a dry run. */
    ruleSource(): RuleSource | null {
        const run = dryRun.value;
        const body = runBody();
        if (run === null || !run.ok || body === null) return null;
        return {
            prefill: importPrefill(run.records),
            fileLabel: baseName(body.csvPath) || "this file",
            rulesLabel: baseName(body.rulesId),
            preview: (rule, signal) => importStore.rulePreview(rule, signal),
            save: (rule) => importStore.saveReviewRule(rule),
            refresh: async () => {
                await importStore.refreshDryRun();
                const error = importStore.dryRunError;
                if (error !== null) return error.message;
                const run = importStore.dryRun;
                return run !== null && !run.ok ? run.stderr.trim() || "hledger could not regenerate the import." : null;
            },
        };
    },

    /** Let go of held overrides (the panel's "Discard" on flagged edits), then show the import without them. */
    async discardOverrides(refs: readonly StagedRef[]): Promise<void> {
        for (const ref of refs) edits.discard(ref);
        await this.runDryRun();
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
     * one it is the CSV alone, on its own route.
     */
    async writeChanges(): Promise<void> {
        const url = settings.serverUrl;
        const request = writeRequest();
        if (url === null || request === null) return;
        writeRequested = true;
        sortMoved = null;
        sortError = null;
        await committed.load(url, request);
        if (committed.error === null && committed.query === request) {
            // Landed: the edits are in the journal now, and belong to no import.
            if (request.kind === "import") edits.clear();
        } else if (
            committed.error !== null &&
            classify(committed.error).kind === "conflict" &&
            request.kind === "import" &&
            request.body.overrides !== undefined
        ) {
            // A review edit went stale between the preview and the press:
            // regenerate and replay, so the next press sends what is current.
            // The refusal stays on screen: the re-run must not hide it.
            await this.runDryRun();
            writeRequested = true;
        }
    },

    // --- creating a rules file --------------------------------------------
    get creating(): boolean {
        return creating;
    },
    get draft(): RulesDraft | null {
        return draft;
    },
    get createForm(): RulesForm | null {
        return createForm;
    },
    get createId(): string {
        return createId;
    },
    get createDrafting(): boolean {
        return createDrafting;
    },
    get createSaving(): boolean {
        return createSaving;
    },
    get createError(): string | null {
        return createError;
    },
    /** The rules file this flow wrote, or null. Survives `closeCreate` so the panel can say so. */
    get createdId(): string | null {
        return createdId;
    },

    /**
     * Open the Create panel and ask the engine for a draft.
     *
     * The id is defaulted from the CSV destination — a rules file is found by
     * being named after its data file, so that default is the convention rather
     * than a convenience — and the account starts empty for the form to fill.
     */
    async openCreate(): Promise<void> {
        const file = staged.value;
        if (file === null) return;
        creating = true;
        createdId = null;
        createId = newRulesId(
            csvPath,
            file.defaults.uploadCsvPath,
            file.candidates.map((candidate) => candidate.id)
        );
        await this.redraft();
    },

    closeCreate(): void {
        closeCreate();
    },

    /** Rename the file being created. Local only — the engine re-checks on save. */
    setCreateId(value: string): void {
        createId = value;
        // A name that was rejected is a name the user is now changing, so the
        // stale refusal must not sit under the field they are fixing.
        createError = null;
    },

    /** Replace the working form after a panel edited it. */
    setCreateItems(items: FormItem[]): void {
        if (createForm !== null) createForm = {...createForm, items};
    },

    /**
     * Ask the engine to draft (or re-draft) a rules file for the staged upload.
     *
     * Called once when the panel opens. It is NOT called again as the user
     * edits: the draft is a starting point, and everything after it is an edit
     * of the returned document through the same typed vocabulary the Edit Rules
     * tab uses — so a re-draft would discard the user's corrections, which is
     * the opposite of what a "give a starting point, then show the preview"
     * flow is for.
     */
    async redraft(): Promise<void> {
        const url = settings.serverUrl;
        const file = staged.value;
        if (url === null || file === null) return;
        createDrafting = true;
        createError = null;
        try {
            const answer = decodeRulesDraft(await new LedgelineApi(url).createRules({stageId: file.stageId, id: createId.trim(), account1: ""}));
            draft = answer;
            createForm = draftForm(answer.doc);
        } catch (error) {
            draft = null;
            createForm = null;
            createError = failureMessage(error);
        } finally {
            createDrafting = false;
        }
    },

    /**
     * Write the drafted file, then re-stage so the candidate list can score it.
     *
     * The re-stage is the point of the whole flow: the user dropped a file that
     * nothing could read, and the answer is that it can be read NOW. Re-uploading
     * the same `File` is how the candidate list learns that — the engine scores
     * candidates at stage time, and there is no cheaper way to ask it again.
     */
    async saveCreate(): Promise<boolean> {
        const url = settings.serverUrl;
        const form = createForm;
        if (url === null || form === null) return false;
        const id = createId.trim();
        createSaving = true;
        createError = null;
        try {
            const saved = decodeRulesDoc(await new LedgelineApi(url).saveRules(id, createSaveRequest(form)));
            createdId = saved.id;
            closeCreate();
            // Re-upload, which re-scores every candidate against the new file
            // and re-seeds the form. `offerFile` resets `createdId`, so it is
            // restored afterwards — the banner saying what was written outlives
            // the staging it came from.
            const written = saved.id;
            await this.retryStage();
            createdId = written;
            // The file was made FOR this statement, so it is the one to read it
            // with — even when an older file scores as well or better (which is
            // the usual reason to make a new one).
            if (staged.value?.candidates.some((candidate) => candidate.id === written) === true) this.selectCandidate(written);
            return true;
        } catch (error) {
            createError = failureMessage(error);
            return false;
        } finally {
            createSaving = false;
        }
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
            sortMoved = decodeSortResult(await client.importSort(target)).moved;
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
            prefs = decodePrefs(await new LedgelineApi(url).putPrefs({hledgerPath: trimmed === "" ? null : trimmed}));
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
