// The New Transactions flow's decisions, all of them, as pure functions.
//
// Not a style preference. `model.ts` (the rules editor's twin of this file)
// says the same thing at its head for the same reason.
//
// It used to be an absolute constraint: `web/vite.config.ts` declared ONE vitest
// project, `node`, which excluded `*.svelte.test.ts`, so a decision written
// inside a `.svelte` file was a decision that could not be tested at all. There
// is now a second, `components`, that mounts them in jsdom — so the constraint
// is softer, and the discipline is not. A decision in here is tested by naming
// its inputs and reading its answer; the same decision in a template is tested
// only by constructing a whole screen that reaches it, which costs more and
// covers less. Spend the `components` project on what genuinely needs a DOM —
// what a component was HANDED, and what it does when it is mounted.
//
// What that means in practice: the components under `ui/` may read a value and
// place it on the screen, and may not decide anything. Which sections exist,
// which CSV path is proposed, whether a candidate list counts as empty, what a
// `ConvertNote` says in English, whether a balance reconciles, and what an
// unreadable file should be told to the user all live here, with a test each.
//
// No Svelte, no DOM, no `fetch` — `importStore.svelte.ts` owns all three.

import type {ImportCommitBody, ImportRunBody, ImportSaveCsvBody} from "$lib/api/native";
import type {LoadStatus} from "$lib/stores/loadState";
import type {
    BalanceCheck,
    CandidateSignals,
    CommitResult,
    ConvertNote,
    DryRunResult,
    ImportCapabilities,
    JournalTarget,
    RulesCandidate,
    SkippedRows,
    StagedFile,
    StageDefaults,
    StatementMeta,
} from "./importTypes";

// ---------------------------------------------------------------------------
// The state machine: which sections exist
// ---------------------------------------------------------------------------

/** Every section the New Transactions screen can show, in the order it shows them. */
export type ImportSection =
    "hledgerBanner" | "readOnlyBanner" | "drop" | "preview" | "candidates" | "destinations" | "balance" | "actions" | "dryRun" | "result";

/** The facts the section machine reads. Deliberately booleans: it decides nothing about payloads. */
export interface ImportFlowState {
    /** The capabilities probe has answered. Before that the screen is a spinner, not a form. */
    readonly capabilitiesLoaded: boolean;
    readonly hledgerAvailable: boolean;
    /** `capabilities.editable` — false means no journal is bound, so nothing can be written. */
    readonly editable: boolean;
    /**
     * A staged file has landed, or an attempt to stage one failed — either way
     * the staged section has something to render. NOT "an upload is running":
     * the drop target owns that, and every section this unlocks sits inside a
     * branch that needs the payload anyway.
     */
    readonly staged: boolean;
    /** `Save and Import` was pressed, so a dry run exists or is running. */
    readonly dryRunRequested: boolean;
    /** `Write changes` was pressed. */
    readonly committed: boolean;
}

/**
 * Which sections to render, in order.
 *
 * Two gates are EXCLUSIVE and come first, because both are states where every
 * later section is a lie:
 *
 *   - No usable hledger. Offering a drop target invites the user to convert a
 *     file, choose a rules file and press an Import button that cannot run. The
 *     plan calls this "the state a new user hits", and it is fixed in one place
 *     — the banner's path control — so the banner IS the screen until it is.
 *   - `editable: false`. There is no journal bound to an editor, so there is no
 *     include root, so `csvPath` has nothing to be relative to.
 *
 * Everything after that is additive, which is what "reveals sections as they
 * resolve" means: staging a file never hides the drop target, and a dry run
 * never hides the destinations it was run against.
 */
export function visibleSections(state: ImportFlowState): ImportSection[] {
    if (!state.capabilitiesLoaded) return [];
    if (!state.hledgerAvailable) return ["hledgerBanner"];
    if (!state.editable) return ["readOnlyBanner"];
    const sections: ImportSection[] = ["drop"];
    if (state.staged) sections.push("preview", "candidates", "destinations", "balance", "actions");
    if (state.dryRunRequested) sections.push("dryRun");
    if (state.committed) sections.push("result");
    return sections;
}

/** Convenience for a template: `{#if shows(sections, "preview")}`. */
export function shows(sections: readonly ImportSection[], section: ImportSection): boolean {
    return sections.includes(section);
}

// ---------------------------------------------------------------------------
// In flight — a DIFFERENT question from the one `dataView` answers
// ---------------------------------------------------------------------------

/**
 * Whether a request is genuinely running right now.
 *
 * This exists because `dataView` cannot answer it and must never be asked. That
 * function maps `idle` — nothing requested, nothing to wait for — onto
 * "loading", which is CORRECT everywhere else in the app: every other surface
 * fetches on mount, so idle is a sub-frame gap before the first request and a
 * spinner is what belongs in it. All three of this screen's resources are the
 * other kind. `staged` waits for a file to be dropped, `dryRun` and `committed`
 * wait for a button, and each may sit idle forever.
 *
 * Reading `view === "loading"` as "busy" is what shipped: the drop target span
 * "Reading the file…" before any file existed, the destination and balance
 * fields were disabled from mount, and the action button wore a spinner nobody
 * had earned and could not be pressed. All four were one expression.
 *
 * So: `dataView` decides which BRANCH to render for a request that exists.
 * `isInFlight` decides whether a request exists and has not answered yet. A
 * surface that disables, freezes or spins asks this one.
 */
export function isInFlight(status: LoadStatus): boolean {
    return status === "loading";
}

/**
 * Whether the destinations, the balance and the action button are frozen.
 *
 * The dry run and the write are the only two things that freeze them, and only
 * while they are actually running: both write to disk on the user's behalf, and
 * a field edited mid-flight would describe a request other than the one being
 * answered. Neither `dryRunRequested` nor a held payload freezes anything — the
 * form is how you change your mind after seeing a dry run.
 */
export function formIsBusy(dryRunStatus: LoadStatus, writeStatus: LoadStatus): boolean {
    return isInFlight(dryRunStatus) || isInFlight(writeStatus);
}

// ---------------------------------------------------------------------------
// The file the user dropped
// ---------------------------------------------------------------------------

/** Lowercase extension with no dot, or "" when the name has none. `Bank.CSV` → `csv`. */
export function fileExtension(name: string): string {
    const bare = bareName(name);
    const dot = bare.lastIndexOf(".");
    // A leading dot is a hidden file, not an extension: `.gitignore` has none.
    if (dot <= 0 || dot === bare.length - 1) return "";
    return bare.slice(dot + 1).toLowerCase();
}

/** The final path component, whichever separator was used. `C:\Users\x\bank.csv` → `bank.csv`. */
function bareName(name: string): string {
    const parts = name.split(/[\\/]/);
    return parts[parts.length - 1] ?? "";
}

/**
 * The value for `X-Ledgeline-Filename`.
 *
 * Three jobs, and the third is the one that surprises:
 *
 *  1. Reduce to a bare name — no separators, no `..`. The engine sanitises again
 *     (a client is never a check), but sending something path-shaped that the
 *     server then rejects is a worse error message than not sending it.
 *  2. Refuse the empty result, so the header is never present-but-blank.
 *  3. Replace every byte outside printable ASCII. An HTTP header value is a byte
 *     string: `fetch` throws a bare `TypeError` on a name containing `é`, which
 *     would surface as "network failure" for a file that is perfectly fine. The
 *     engine only uses this name for format detection and the CSV default, so a
 *     mangled non-ASCII stem costs a nicer default and nothing else.
 */
export function headerFilename(name: string): string {
    const bare = bareName(name).replace(/^\.+/, "");
    // Printable ASCII only. A CR or LF here would be a header-splitting
    // primitive, and anything above 0x7e is what makes `fetch` throw.
    const ascii = bare.replace(/[^\x20-\x7e]/g, "_");
    return ascii.trim() === "" ? "statement" : ascii;
}

/** `accept` for the hidden `<input type="file">`, built from what the engine says it reads. */
export function acceptAttribute(formats: readonly string[]): string {
    return formats.map((format) => `.${format}`).join(",");
}

/**
 * Why we are refusing this file before uploading it, or null to let the engine decide.
 *
 * Deliberately conservative. The engine sniffs CONTENT first and the extension
 * second (a `.qfx` that is really OFX 2.x XML must not be parsed as SGML), so
 * this may only refuse things the engine could not possibly accept — a named
 * `.pdf`, and an extension that is not in the list the engine itself published.
 * A file with NO extension is always let through: sniffing is exactly the case
 * it exists for.
 */
export function refuseFile(name: string, formats: readonly string[]): string | null {
    const ext = fileExtension(name);
    if (ext === "pdf") {
        return "Ledgeline can't read statements out of a PDF yet — the numbers in one aren't reliably where they look like they are. Export CSV, OFX/QFX or a spreadsheet from your bank instead.";
    }
    if (ext === "") return null;
    if (formats.includes(ext)) return null;
    return `Ledgeline doesn't read .${ext} files. It reads ${formatList(formats)}.`;
}

/** "csv, tsv and ofx" — an Oxford-comma list for a sentence. */
export function formatList(formats: readonly string[]): string {
    const names = formats.map((format) => `.${format}`);
    if (names.length === 0) return "nothing on this server";
    if (names.length === 1) return names[0]!;
    return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]!}`;
}

// ---------------------------------------------------------------------------
// Destinations
// ---------------------------------------------------------------------------

/**
 * The CSV a rules file describes: its own id minus `.rules`.
 *
 * hledger's convention is that `FILE.rules` sits beside `FILE`, so
 * `import/2026/bank.csv.rules` describes `import/2026/bank.csv`. A rules file
 * named without the data file's extension (`bank.rules`) leaves a stem with no
 * extension, and `.csv` is what the converter writes, so that is what it gets.
 * Anything not ending in `.rules` yields null — there is no convention to apply.
 */
export function csvPathForRules(rulesId: string): string | null {
    if (!rulesId.endsWith(".rules")) return null;
    const stem = rulesId.slice(0, -".rules".length);
    if (stem === "") return null;
    const lastSlash = stem.lastIndexOf("/");
    const name = stem.slice(lastSlash + 1);
    return name.includes(".") ? stem : `${stem}.csv`;
}

/**
 * The CSV path to show.
 *
 * The chosen rules file wins over the server's default, because the user
 * changing candidate is them saying "read it as THIS instead", and leaving the
 * previous candidate's file name in the box is how a credit-card statement ends
 * up written over `checking.csv`. The server default is the fallback for the
 * no-candidate case and for a rules id that carries no convention.
 */
export function deriveCsvPath(defaults: StageDefaults, selectedRulesId: string | null): string {
    if (selectedRulesId === null) return defaults.csvPath;
    return csvPathForRules(selectedRulesId) ?? defaults.csvPath;
}

/** Complaints about a CSV destination, in the order a user would fix them. Empty = fine. */
export function validateCsvPath(path: string): string[] {
    const trimmed = path.trim();
    const problems: string[] = [];
    if (trimmed === "") problems.push("Give the CSV a name.");
    if (trimmed.startsWith("/") || /^[A-Za-z]:[\\/]/.test(trimmed)) {
        problems.push("The path is relative to the folder your journal is in, so it can't start at the root of the disk.");
    }
    if (trimmed.split("/").includes("..")) problems.push("The path can't step outside the folder your journal is in.");
    return problems;
}

/** The journal the screen should preselect: the staged default when it is offered, else the engine's ranking. */
export function defaultJournalId(defaults: StageDefaults, journals: readonly JournalTarget[]): string | null {
    const offered = journals.filter((journal) => journal.writable);
    if (defaults.journalId !== null && offered.some((journal) => journal.id === defaults.journalId)) return defaults.journalId;
    return offered[0]?.id ?? null;
}

/**
 * How a journal reads in the select: name, how much is in it, and how recent.
 *
 * The counts and the date are shown precisely so the engine's ranking is
 * legible — a file at the top of the list with 412 transactions ending last week
 * explains itself, and `accounts.journal` sitting at the bottom with none
 * explains that too. No ranking decision anywhere reads the label.
 */
export function journalOptionLabel(journal: JournalTarget): string {
    const parts: string[] = [journal.label];
    parts.push(journal.txnCount === 0 ? "no transactions" : `${journal.txnCount.toLocaleString()} transaction${journal.txnCount === 1 ? "" : "s"}`);
    if (journal.lastTxnDate !== null) parts.push(`latest ${journal.lastTxnDate}`);
    if (journal.isRoot) parts.push("main file");
    if (!journal.writable) parts.push("read-only");
    return `${parts[0]!} — ${parts.slice(1).join(", ")}`;
}

// ---------------------------------------------------------------------------
// Candidates
// ---------------------------------------------------------------------------

/**
 * "No rules file fits this data."
 *
 * A predicate rather than `candidates.length === 0` at the call site because the
 * two states it must NOT be confused with are both also "nothing to show": a
 * file that has not been staged yet, and one still uploading. Saying "no rules
 * file fits" while the upload is in flight is a lie the user acts on.
 */
export function noCandidates(staged: StagedFile | null): boolean {
    return staged !== null && staged.candidates.length === 0;
}

/** `0.98` → `"98%"`. Clamped, because a score outside 0..1 is a bug that must not render as `-4000%`. */
export function formatScore(score: number): string {
    if (!Number.isFinite(score)) return "—";
    const clamped = Math.min(1, Math.max(0, score));
    return `${Math.round(clamped * 100)}%`;
}

/** daisyUI tone for a score badge. The thresholds are display only; ranking is the engine's. */
export function scoreTone(score: number): "success" | "warning" | "error" {
    if (!Number.isFinite(score) || score < 0.5) return "error";
    return score >= 0.8 ? "success" : "warning";
}

/** One readable line about a signal, plus whether it is a complaint. */
export interface SignalLine {
    readonly text: string;
    readonly bad: boolean;
}

/**
 * The counts behind a score, in English.
 *
 * The three "bad" lines are fact 4 from the plan: a mismatched rules file
 * frequently PARSES, exits 0 and produces garbage, so parse success is not a
 * matching signal and the user needs the specific symptoms named. The
 * bare-commodity line spells out its consequence because the symptom
 * ("the import succeeded but my balance didn't move") is otherwise unattributable.
 */
export function signalLines(signals: CandidateSignals): SignalLine[] {
    const lines: SignalLine[] = [
        {text: `${signals.txns} transaction${signals.txns === 1 ? "" : "s"} from ${signals.postings} posting${signals.postings === 1 ? "" : "s"}`, bad: false},
    ];
    if (signals.amountlessPostings > 0) {
        lines.push({text: `${signals.amountlessPostings} posting${signals.amountlessPostings === 1 ? "" : "s"} with no amount at all`, bad: true});
    }
    if (signals.bareCommodityAmounts > 0) {
        lines.push({
            text: `${signals.bareCommodityAmounts} amount${signals.bareCommodityAmounts === 1 ? "" : "s"} with no currency — these form a separate commodity, so your balance would not move`,
            bad: true,
        });
    }
    if (signals.unknownAccounts > 0) {
        lines.push({text: `${signals.unknownAccounts} posting${signals.unknownAccounts === 1 ? "" : "s"} fell through to an :unknown account`, bad: true});
    }
    if (signals.emptyDescriptions !== null && signals.emptyDescriptions > 0) {
        lines.push({text: `${signals.emptyDescriptions} transaction${signals.emptyDescriptions === 1 ? "" : "s"} with an empty description`, bad: true});
    }
    if (signals.columnCountMatches === false) lines.push({text: "the rules file names a different number of columns than the data has", bad: true});
    if (signals.headerMatchesSource === true) lines.push({text: "its column names match this file's header", bad: false});
    return lines;
}

/**
 * The account a balance assertion should default to: the chosen rules file's `account1`.
 *
 * Read off the candidate itself. `account1` is the account every imported
 * posting lands in, so it is the only account a statement balance could
 * sensibly be asserted against — and the engine already knew it when it ranked
 * the file. This used to join the candidate's id against the `/api/rules`
 * listing, which was a second round trip whose failure mode was a silently empty
 * field; the candidate now carries the answer.
 */
export function defaultBalanceAccount(candidate: RulesCandidate | null): string {
    return candidate?.account1 ?? "";
}

/** The candidate with this id, or null — the store's lookup, here so it is tested. */
export function candidateById(staged: StagedFile | null, id: string | null): RulesCandidate | null {
    if (staged === null || id === null) return null;
    return staged.candidates.find((candidate) => candidate.id === id) ?? null;
}

// ---------------------------------------------------------------------------
// Preview notes
// ---------------------------------------------------------------------------

/** Names for the delimiters a sniffer reports, so a note does not read "delimiter guessed:  ". */
const DELIMITER_NAMES: Record<string, string> = {",": "comma", ";": "semicolon", "\t": "tab", "|": "pipe", " ": "space"};

/**
 * A `ConvertNote` as a sentence.
 *
 * These are judgement calls the conversion made — a sheet it picked, an encoding
 * it guessed, rows it threw away — and every one of them can be the reason an
 * import looks wrong later. The plan's own examples ("sheet 2 of 3 used", "4
 * preamble rows skipped", "encoding guessed: windows-1252") are the register.
 */
export function noteText(note: ConvertNote): string {
    switch (note.kind) {
        case "sheetChosen":
            return note.of <= 1 ? `Read the sheet "${note.name}".` : `Read the sheet "${note.name}" — the workbook has ${note.of}.`;
        case "statementChosen":
            return `This file holds ${note.of} statements, one per account. Only the first was read — import the others by downloading them separately.`;
        case "datesFromSerial":
            return `${note.count} date${note.count === 1 ? " was" : "s were"} stored as spreadsheet serial numbers and ${note.count === 1 ? "was" : "were"} read as dates.`;
        case "encodingGuessed":
            return `Encoding guessed: ${note.label}. The file didn't declare one, so accented characters are worth a look in the preview.`;
        case "delimiterSniffed": {
            const name = DELIMITER_NAMES[note.delimiter];
            return `Delimiter guessed: ${name === undefined ? JSON.stringify(note.delimiter) : name}.`;
        }
        case "preambleSkipped":
            return `${note.lines} row${note.lines === 1 ? "" : "s"} of preamble above the header ${note.lines === 1 ? "was" : "were"} skipped.`;
        case "trailerSkipped":
            // Named loudly: a statement export can end in pages of disclaimer,
            // and "we ignored the last 26 rows of your file" is the kind of
            // helpfulness that must never be silent.
            return `${note.lines} row${note.lines === 1 ? "" : "s"} below the last transaction ${note.lines === 1 ? "was" : "were"} skipped — a footer or disclaimer, not data.`;
        case "blankRowsDropped":
            return `${note.count} blank row${note.count === 1 ? " was" : "s were"} dropped. hledger abandons a whole file on one unreadable record, so an empty row cannot be passed through.`;
        case "raggedRows":
            return `${note.count} row${note.count === 1 ? " has" : "s have"} a different number of columns than the header.`;
        case "balanceMismatch":
            return `The file's own arithmetic doesn't add up: it says ${note.expected}, its rows total ${note.computed}. Something was misread — check the preview before importing.`;
    }
}

/** Which notes are a warning rather than an aside. Both are real failures, not guesses. */
export function noteIsWarning(note: ConvertNote): boolean {
    return note.kind === "raggedRows" || note.kind === "balanceMismatch";
}

/** "26 rows of ofx" — the one-line summary above the preview table. */
export function previewSummary(staged: StagedFile): string {
    const {rowCount, rows, truncated} = staged.preview;
    const shown = truncated || rows.length < rowCount ? `, showing the first ${rows.length}` : "";
    return `${staged.format.toUpperCase()} — ${rowCount} row${rowCount === 1 ? "" : "s"}${shown}`;
}

/** What the format volunteered about the statement, as label/value pairs. Empty when it volunteered nothing. */
export function statementFacts(statement: StatementMeta | null): {label: string; value: string}[] {
    if (statement === null) return [];
    const facts: {label: string; value: string}[] = [];
    if (statement.accountHint !== null) facts.push({label: "Account", value: `…${statement.accountHint}`});
    if (statement.currency !== null) facts.push({label: "Currency", value: statement.currency});
    if (statement.ledgerBalance !== null) facts.push({label: "Statement balance", value: statement.ledgerBalance});
    if (statement.balanceAsOf !== null) facts.push({label: "as of", value: statement.balanceAsOf});
    return facts;
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/** `Save CSV` alone when no rules file is chosen; `Save and Import` when one is. */
export type ImportAction = "saveCsv" | "saveAndImport";

export function importAction(selectedRulesId: string | null): ImportAction {
    return selectedRulesId === null ? "saveCsv" : "saveAndImport";
}

export function actionLabel(action: ImportAction): string {
    return action === "saveCsv" ? "Save CSV" : "Save and Import";
}

/**
 * Whether pressing the action runs a dry run first.
 *
 * Only the import does. `Save CSV` writes one converted file and touches no
 * journal, so there is nothing for hledger to propose and nothing to reconcile —
 * interposing an empty confirmation panel would be ceremony, not safety.
 */
export function actionRunsDryRun(action: ImportAction): boolean {
    return action === "saveAndImport";
}

/** The destinations as the user has them. Everything is text; nothing here is a number. */
export interface DestinationDraft {
    readonly csvPath: string;
    readonly journalId: string | null;
    readonly balance: string;
    readonly balanceAccount: string;
}

/**
 * Why the action button is disabled, or null when it is not.
 *
 * A sentence rather than a boolean: a disabled button with no explanation is the
 * single most common way a form dead-ends, and every one of these is fixable in
 * the field right above it.
 *
 * Every blocker must describe the request the button ACTUALLY sends. `Save CSV`
 * goes to its own route with a two-field body — `{stageId, csvPath}` — carrying
 * neither the journal nor the balance nor the account, so only the CSV path can
 * stop it. Refusing to press it over a balance it will not send was a dead end
 * with no way out of it: an OFX volunteers its closing balance, so the field is
 * prefilled, and the Save-CSV path is by definition the one where NO rules file
 * matched, so there is no `account1` to default the account from either. The
 * user's only exits were to delete a balance they had not typed or to type an
 * account that would be thrown away.
 */
export function actionBlocker(action: ImportAction, draft: DestinationDraft): string | null {
    const csvProblems = validateCsvPath(draft.csvPath);
    if (csvProblems.length > 0) return csvProblems[0]!;
    if (action === "saveCsv") return null;
    if (draft.journalId === null) return "Choose the journal to import into.";
    if (draft.balance.trim() !== "" && draft.balanceAccount.trim() === "") {
        return "A statement balance needs the account it is a balance of.";
    }
    return null;
}

// ---------------------------------------------------------------------------
// Dry run
// ---------------------------------------------------------------------------

/**
 * The back-dated-row warning, or null when nothing was skipped.
 *
 * This is a definition-of-done item and the reason it is worded at length:
 * hledger drops these rows with NO output at all. State lives in `.latest.<name>`
 * beside the data file, keyed to its name, and holds the newest date already
 * imported; a row older than that simply is not in the dry run, and a user
 * reconciling by count would never find out why.
 */
export function skippedWarning(skipped: SkippedRows | null): string | null {
    if (skipped === null || skipped.count === 0) return null;
    const rows = `${skipped.count} row${skipped.count === 1 ? "" : "s"}`;
    return `${rows} dated on or before ${skipped.olderThan} ${skipped.count === 1 ? "is" : "are"} NOT in this import. hledger keeps the newest imported date beside your CSV and silently drops anything older, so ${skipped.count === 1 ? "that row" : "those rows"} will not appear in your journal.`;
}

/** How the balance reconciliation reads: a verdict, a sentence, and the tone to show it in. */
export interface BalanceVerdict {
    readonly matches: boolean;
    readonly headline: string;
    readonly detail: string;
    readonly tone: "success" | "error";
}

/**
 * The statement-vs-computed comparison, formatted.
 *
 * Every value here is the ENGINE's decimal text, passed through untouched — the
 * verdict is `matches` and the gap is `difference`, both computed server-side by
 * concatenating the journal and the proposed entries (fact 3: two `-f` flags
 * silently give the wrong combined balance). Re-deriving either from these
 * strings on this side would mean parsing money in the browser, which convention
 * #1 forbids, and would mean disagreeing with the engine when it mattered.
 */
export function balanceVerdict(balance: BalanceCheck): BalanceVerdict {
    if (balance.matches) {
        return {
            matches: true,
            headline: "The balance reconciles.",
            detail: `Your statement says ${balance.statement} and the journal plus these transactions computes to ${balance.computed}.`,
            tone: "success",
        };
    }
    return {
        matches: false,
        // A null difference is a multi-commodity balance, where there is no one
        // number to be off BY. The mismatch is still stated — losing the
        // headline entirely because the gap cannot be summarised would hide the
        // fact that matters.
        headline: balance.difference === null ? "The balance doesn't match." : `Off by ${balance.difference}.`,
        detail: `Your statement says ${balance.statement}; the journal plus these transactions computes to ${balance.computed}. Importing will write an assertion that fails.`,
        tone: "error",
    };
}

/** The git-blocked panel's sentence, or null when nothing blocks. */
export function gitBlockMessage(blockedByGit: readonly string[]): string | null {
    if (blockedByGit.length === 0) return null;
    const files = blockedByGit.length === 1 ? "one file this import writes has" : `${blockedByGit.length} files this import writes have`;
    return `Commit first: ${files} uncommitted changes. An import rewrites ${blockedByGit.length === 1 ? "it" : "them"} in place, and \`git diff\` is how you would undo that — which only works if what is there now is committed.`;
}

/** Whether `Write changes` may be offered at all. Git blocks it, and so does a failed dry run. */
export function canWrite(dryRun: DryRunResult | null): boolean {
    return dryRun !== null && dryRun.ok && dryRun.blockedByGit.length === 0;
}

/**
 * Whether two run requests ask the same question — the `matchesRequest` half of
 * `dataView`, and the reason a stale dry run cannot render.
 *
 * FE-1 in its most expensive form: `DryRunResult` and `CommitResult` carry no
 * field naming the file, rules file or destination they were computed for, so a
 * held payload's own shape CANNOT say which request it belongs to and no type
 * error is possible when they are mixed up. Dropping a second statement, or just
 * switching candidate, has to make the previous answer stop rendering — showing
 * a credit card's proposed transactions under a checking account's destination
 * is one click away from importing them there.
 */
export function sameRunRequest(a: ImportRunBody | null, b: ImportRunBody | null): boolean {
    if (a === null || b === null) return a === b;
    return (
        a.stageId === b.stageId &&
        a.rulesId === b.rulesId &&
        a.csvPath === b.csvPath &&
        a.journalId === b.journalId &&
        a.balance === b.balance &&
        a.balanceAccount === b.balanceAccount
    );
}

/**
 * What pressing the write button sends: an import, or a bare CSV save.
 *
 * A discriminated union rather than one body with nullable handles, mirroring
 * the two engine routes. The two requests are not the same question with a field
 * missing — one appends transactions to a journal and one writes a single file —
 * and a shape that cannot express "import with no rules file" is what keeps the
 * store from ever asking for it.
 */
export type WriteRequest = {readonly kind: "import"; readonly body: ImportCommitBody} | {readonly kind: "saveCsv"; readonly body: ImportSaveCsvBody};

/**
 * Whether two write requests ask the same question — [`sameRunRequest`] for the
 * result panel, which holds a `CommitResult` that names neither its rules file
 * nor its journal.
 *
 * Switching between the two kinds always counts as a change: "saved bank.csv"
 * must not stay on screen once a rules file has been chosen, because the next
 * press writes to a journal as well.
 */
export function sameWriteRequest(a: WriteRequest | null, b: WriteRequest | null): boolean {
    if (a === null || b === null) return a === b;
    if (a.kind !== b.kind) return false;
    if (a.kind === "saveCsv" && b.kind === "saveCsv") {
        return a.body.stageId === b.body.stageId && a.body.csvPath === b.body.csvPath;
    }
    if (a.kind === "import" && b.kind === "import") {
        return sameRunRequest(a.body, b.body) && a.body.writeAssertion === b.body.writeAssertion;
    }
    return false;
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

/** What was written, as sentences — one per file actually touched. */
export function writtenLines(result: CommitResult): string[] {
    const lines = [`Wrote ${result.csvWritten}.`];
    if (result.journalWritten !== null) {
        lines.push(`Imported ${result.imported} transaction${result.imported === 1 ? "" : "s"} into ${result.journalWritten}.`);
    }
    if (result.git !== null) {
        if (result.git.committed) lines.push(`Committed ${result.git.paths.join(", ")}.`);
        if (result.git.skipped.length > 0) lines.push(`Not committed: ${result.git.skipped.join(", ")}.`);
    }
    return lines;
}

/** The re-sort offer, or null when the journal came out in date order. */
export function reorderOffer(result: CommitResult): string | null {
    if (result.ordering.inOrder) return null;
    const moves = result.ordering.moves.length;
    return `${result.journalWritten ?? "The journal"} is no longer in date order — ${moves} transaction${moves === 1 ? "" : "s"} would move. Ledgeline can re-sort it in place, leaving every directive, include and comment exactly where it is.`;
}

// ---------------------------------------------------------------------------
// hledger banner
// ---------------------------------------------------------------------------

/** Headline, explanation and whether a path control helps — the whole banner, decided. */
export interface HledgerBannerCopy {
    readonly headline: string;
    readonly detail: string;
    /** False for `tooOld`, where pointing at the same binary again changes nothing on its own. */
    readonly offersPath: boolean;
}

/**
 * What to say when hledger cannot be used.
 *
 * `message` is the engine's own sentence and is always shown beside this; these
 * add the remedy, which the engine has no opinion about. `tooOld` still offers
 * the path control — a user with two hledgers installed fixes it by naming the
 * newer one — but leads with the upgrade, because that is the usual answer.
 */
export function hledgerBannerCopy(capabilities: ImportCapabilities): HledgerBannerCopy {
    switch (capabilities.hledger.reason) {
        case "notFound":
            return {
                headline: "Ledgeline can't find hledger.",
                detail: "Importing shells out to hledger, so it needs the path to the binary. Install it, or type where it lives below.",
                offersPath: true,
            };
        case "tooOld":
            return {
                headline: `hledger ${capabilities.hledger.version ?? "on this machine"} is too old to import.`,
                detail: "Ledgeline needs 1.40 or newer — that is the release where `--rules-file` became `--rules`. Upgrade it, or point Ledgeline at a newer one below.",
                offersPath: true,
            };
        case "timedOut":
            return {
                headline: "hledger didn't answer.",
                detail: "Ledgeline ran it to read its version and gave up waiting. If it is on a slow or disconnected network share, name a local one below.",
                offersPath: true,
            };
        case "unrunnable":
        case null:
            return {
                headline: "Ledgeline can't run hledger.",
                detail: "The file is there but would not start — most often it is not executable, or is built for a different architecture. Naming a different one below is the quickest test.",
                offersPath: true,
            };
    }
}

// ---------------------------------------------------------------------------
// Candidate ordering (display only)
// ---------------------------------------------------------------------------

/**
 * The candidates as the list renders them.
 *
 * The engine ranks (`score DESC, mtime DESC`) and this preserves that order
 * exactly — it only trims the sample to what a card shows, so a rules file that
 * happens to produce forty sample transactions cannot push the next card off the
 * screen. Re-sorting here would silently override a ranking the engine
 * documented, so it does not.
 */
export function candidateCards(candidates: readonly RulesCandidate[], sampleSize = 2): {candidate: RulesCandidate; sample: RulesCandidate["sample"]}[] {
    return candidates.map((candidate) => ({candidate, sample: candidate.sample.slice(0, sampleSize)}));
}
