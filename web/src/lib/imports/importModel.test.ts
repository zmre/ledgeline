// The New Transactions flow, tested where it is testable.
//
// There is no component renderer in this repo — `vite.config.ts` declares one
// `node` vitest project and excludes `*.svelte.test.ts`, and Chromium cannot
// launch in the build environment — so the whole screen was built with its
// decisions in `importModel.ts` and its wire knowledge in `nativeDecode.ts`
// precisely so that this file can cover them.
//
// The wire fixtures below are LITERAL JSON, copied from "The lane E wire
// contract" in plans/11-enhanced-import.md rather than round-tripped through
// our own encoder. That is the point: the Rust half of lane E is being built
// against the same document concurrently, so a decoder that agreed with itself
// would prove nothing. If the contract changes, these fail.

import {describe, expect, it} from "vitest";
import {ApiShapeError} from "$lib/api/client";
import {decodeCommitResult, decodeDryRun, decodeImportCapabilities, decodePrefs, decodeSortResult, decodeStagedFile} from "$lib/api/nativeDecode";
import {dataView} from "$lib/stores/loadState";
import {
    acceptAttribute,
    actionBlocker,
    actionLabel,
    actionRunsDryRun,
    balanceVerdict,
    candidateById,
    candidateCards,
    canWrite,
    csvPathForRules,
    defaultBalanceAccount,
    defaultJournalId,
    deriveCsvPath,
    fileExtension,
    formatList,
    formatScore,
    formIsBusy,
    gitBlockMessage,
    headerFilename,
    hledgerBannerCopy,
    importAction,
    isInFlight,
    journalOptionLabel,
    noCandidates,
    noteIsWarning,
    noteText,
    previewSummary,
    refuseFile,
    reorderOffer,
    sameRunRequest,
    sameWriteRequest,
    scoreTone,
    shows,
    signalLines,
    skippedWarning,
    statementFacts,
    validateCsvPath,
    visibleSections,
    writtenLines,
    type ImportFlowState,
} from "./importModel";
import type {CommitResult, ImportCapabilities, JournalTarget, StagedFile} from "./importTypes";

// ---------------------------------------------------------------------------
// Wire fixtures — the contract's own JSON, verbatim
// ---------------------------------------------------------------------------

const CAPABILITIES_JSON = {
    hledger: {available: true, version: "1.52"},
    formats: ["csv", "tsv", "ssv", "ofx", "qfx", "xls", "xlsx", "xlsm", "xlsb", "ods"],
    journals: [{id: "2026/2026.journal", label: "2026.journal", txnCount: 412, lastTxnDate: "2026-08-01", isRoot: false, writable: true}],
    git: {available: true, autocommit: true},
    editable: true,
};

const STAGE_JSON = {
    stageId: "opaque-token",
    format: "ofx",
    preview: {header: ["date", "amount", "description"], rows: [["2026-06-24", "-12.34", "GROCERY STORE"]], rowCount: 26, truncated: false},
    statement: {accountHint: "7777", currency: "USD", ledgerBalance: "-3238.65", balanceAsOf: "2026-08-12"},
    notes: [{kind: "preambleSkipped", lines: 4}],
    candidates: [
        {
            id: "import/2026/bank.csv.rules",
            label: "bank",
            score: 0.98,
            signals: {txns: 26, postings: 52, amountlessPostings: 0, bareCommodityAmounts: 0, unknownAccounts: 0},
            sample: [{date: "2026-06-24", description: "GROCERY STORE", postings: ["assets:bank:checking  $-12.34", "expenses:groceries"]}],
            account1: "assets:bank:checking",
            account2: "expenses:unknown",
        },
    ],
    defaults: {csvPath: "import/2026/bank.csv", journalId: "2026/2026.journal"},
};

const DRY_RUN_JSON = {
    ok: true,
    entries: "2026-02-01 GROCERY STORE\n    expenses:groceries  $12.34\n",
    count: 3,
    status: "would import 3 new transactions from bank.csv:",
    skipped: {olderThan: "2026-02-05", count: 1},
    balance: {statement: "$2945.05", computed: "$2945.05", matches: true, difference: "$0.00"},
    blockedByGit: ["2026/2026.journal"],
};

const COMMIT_JSON = {
    csvWritten: "import/2026/bank.csv",
    journalWritten: "2026/2026.journal",
    imported: 3,
    ordering: {inOrder: false, moves: [{date: "2026-01-20", description: "BACK DATED", fromLine: 812, toLine: 540}]},
    git: {committed: true, paths: ["import/2026/bank.csv", "2026/2026.journal"], skipped: []},
};

/** The staged file as the SPA holds it — decoded once, reused by the model tests. */
const staged = (): StagedFile => decodeStagedFile(STAGE_JSON);

const capabilities = (): ImportCapabilities => decodeImportCapabilities(CAPABILITIES_JSON);

const journal = (over: Partial<JournalTarget> = {}): JournalTarget => ({
    id: "2026/2026.journal",
    label: "2026.journal",
    txnCount: 412,
    lastTxnDate: "2026-08-01",
    isRoot: false,
    writable: true,
    ...over,
});

// ---------------------------------------------------------------------------
// Decoders
// ---------------------------------------------------------------------------

describe("UNIT import wire decoders", () => {
    it("decodes the contract's capabilities body", () => {
        const caps = decodeImportCapabilities(CAPABILITIES_JSON);
        expect(caps.hledger).toEqual({available: true, version: "1.52", reason: null, message: null});
        expect(caps.formats).toContain("qfx");
        expect(caps.journals[0]).toEqual({
            id: "2026/2026.journal",
            label: "2026.journal",
            txnCount: 412,
            lastTxnDate: "2026-08-01",
            isRoot: false,
            writable: true,
        });
        expect(caps.git).toEqual({available: true, autocommit: true});
        expect(caps.editable).toBe(true);
    });

    it("decodes the unavailable-hledger variant, keeping the engine's own message", () => {
        const caps = decodeImportCapabilities({
            ...CAPABILITIES_JSON,
            hledger: {available: false, reason: "tooOld", message: "hledger 1.31 is older than 1.40"},
        });
        expect(caps.hledger.available).toBe(false);
        expect(caps.hledger.reason).toBe("tooOld");
        expect(caps.hledger.message).toBe("hledger 1.31 is older than 1.40");
    });

    it("keeps the message when the reason is one this build has never heard of", () => {
        // The whole point: refusing to decode would replace an actionable banner
        // with a decode failure, on precisely the screen that exists to fix it.
        const caps = decodeImportCapabilities({...CAPABILITIES_JSON, hledger: {available: false, reason: "quantumFlux", message: "something new"}});
        expect(caps.hledger.reason).toBeNull();
        expect(caps.hledger.message).toBe("something new");
    });

    it("refuses to read a journal target as writable when the wire did not say so", () => {
        const caps = decodeImportCapabilities({...CAPABILITIES_JSON, journals: [{id: "a.journal", label: "a", txnCount: 0, isRoot: false}]});
        expect(caps.journals[0]!.writable).toBe(false);
        expect(caps.journals[0]!.lastTxnDate).toBeNull();
    });

    it("throws when capabilities carries no journals array", () => {
        expect(() => decodeImportCapabilities({hledger: {available: true}})).toThrow(ApiShapeError);
    });

    it("decodes the contract's stage body", () => {
        const file = staged();
        expect(file.stageId).toBe("opaque-token");
        expect(file.format).toBe("ofx");
        expect(file.preview.rowCount).toBe(26);
        expect(file.preview.header).toEqual(["date", "amount", "description"]);
        expect(file.statement?.ledgerBalance).toBe("-3238.65");
        expect(file.notes).toEqual([{kind: "preambleSkipped", lines: 4}]);
        expect(file.candidates[0]!.signals.amountlessPostings).toBe(0);
        expect(file.candidates[0]!.sample[0]!.postings).toHaveLength(2);
        expect(file.defaults).toEqual({csvPath: "import/2026/bank.csv", journalId: "2026/2026.journal"});
    });

    it("decodes every ConvertNote variant the engine can emit", () => {
        const file = decodeStagedFile({
            ...STAGE_JSON,
            notes: [
                {kind: "sheetChosen", name: "Statement", of: 3},
                {kind: "datesFromSerial", count: 12},
                {kind: "encodingGuessed", label: "windows-1252"},
                {kind: "delimiterSniffed", delimiter: ";"},
                {kind: "preambleSkipped", lines: 4},
                {kind: "raggedRows", count: 2},
                {kind: "balanceMismatch", expected: "100.00", computed: "99.00"},
            ],
        });
        expect(file.notes.map((note) => note.kind)).toEqual([
            "sheetChosen",
            "datesFromSerial",
            "encodingGuessed",
            "delimiterSniffed",
            "preambleSkipped",
            "raggedRows",
            "balanceMismatch",
        ]);
        expect(file.unknownNoteCount).toBe(0);
    });

    it("counts a note kind it does not know instead of losing the whole import", () => {
        const file = decodeStagedFile({
            ...STAGE_JSON,
            notes: [
                {kind: "somethingNewer", count: 1},
                {kind: "raggedRows", count: 2},
            ],
        });
        expect(file.notes).toEqual([{kind: "raggedRows", count: 2}]);
        expect(file.unknownNoteCount).toBe(1);
    });

    it("reads an absent statement as null rather than an empty one", () => {
        expect(decodeStagedFile({...STAGE_JSON, statement: null}).statement).toBeNull();
    });

    it("reads the three Signals fields the contract omits as null, not as zero", () => {
        // `0` is a measurement and `false` is a verdict; neither is what "this
        // engine did not send it" means, and the card must not claim either.
        const signals = staged().candidates[0]!.signals;
        expect(signals.emptyDescriptions).toBeNull();
        expect(signals.columnCountMatches).toBeNull();
        expect(signals.headerMatchesSource).toBeNull();
    });

    it("decodes the extra Signals fields when an engine does send them", () => {
        const file = decodeStagedFile({
            ...STAGE_JSON,
            candidates: [
                {
                    ...STAGE_JSON.candidates[0],
                    signals: {...STAGE_JSON.candidates[0]!.signals, emptyDescriptions: 2, columnCountMatches: false, headerMatchesSource: true},
                },
            ],
        });
        expect(file.candidates[0]!.signals).toMatchObject({emptyDescriptions: 2, columnCountMatches: false, headerMatchesSource: true});
    });

    it("throws when a candidate's signals are missing entirely", () => {
        expect(() => decodeStagedFile({...STAGE_JSON, candidates: [{id: "a", label: "a", score: 1}]})).toThrow(ApiShapeError);
    });

    it("decodes the contract's successful dry run", () => {
        const run = decodeDryRun(DRY_RUN_JSON);
        expect(run.ok).toBe(true);
        if (!run.ok) throw new Error("unreachable");
        expect(run.count).toBe(3);
        expect(run.status).toBe("would import 3 new transactions from bank.csv:");
        expect(run.skipped).toEqual({olderThan: "2026-02-05", count: 1});
        // ONE representation: the engine renders all three amounts in the
        // commodity it computed the balance in, so `statement` and `computed`
        // are comparable at a glance. They used to be `2945.05` (what the user
        // typed) beside `$2945.05` (what hledger answered), which reads as a
        // mismatch above a badge saying "match".
        expect(run.balance).toEqual({statement: "$2945.05", computed: "$2945.05", matches: true, difference: "$0.00"});
        expect(run.blockedByGit).toEqual(["2026/2026.journal"]);
    });

    it("decodes a failed dry run as a VALUE carrying stderr verbatim", () => {
        const stderr = 'hledger: Error: could not parse date "13/40/2026"\n  record: 13/40/2026,-12.34,GROCERY\n';
        const run = decodeDryRun({ok: false, stderr});
        expect(run.ok).toBe(false);
        if (run.ok) throw new Error("unreachable");
        // Byte for byte — the `record:` echo is the useful half.
        expect(run.stderr).toBe(stderr);
    });

    it("reads a null skipped and a null balance as 'nothing to report'", () => {
        const run = decodeDryRun({...DRY_RUN_JSON, skipped: null, balance: null, blockedByGit: []});
        if (!run.ok) throw new Error("unreachable");
        expect(run.skipped).toBeNull();
        expect(run.balance).toBeNull();
        expect(run.blockedByGit).toEqual([]);
    });

    it("throws when a dry run carries no ok flag", () => {
        expect(() => decodeDryRun({entries: "", count: 0})).toThrow(ApiShapeError);
    });

    it("decodes the contract's commit body", () => {
        const commit = decodeCommitResult(COMMIT_JSON);
        expect(commit.csvWritten).toBe("import/2026/bank.csv");
        expect(commit.journalWritten).toBe("2026/2026.journal");
        expect(commit.imported).toBe(3);
        expect(commit.ordering.inOrder).toBe(false);
        expect(commit.ordering.moves[0]).toEqual({date: "2026-01-20", description: "BACK DATED", fromLine: 812, toLine: 540});
        expect(commit.git).toEqual({committed: true, paths: ["import/2026/bank.csv", "2026/2026.journal"], skipped: []});
    });

    it("decodes the Save-CSV-only commit, where no journal was touched", () => {
        // The contract does not spell this shape out; the decoder treats an
        // absent journal/ordering as "nothing was imported", which is the only
        // reading that is not a lie.
        const commit = decodeCommitResult({csvWritten: "import/2026/bank.csv", journalWritten: null, git: null});
        expect(commit.journalWritten).toBeNull();
        expect(commit.imported).toBe(0);
        expect(commit.ordering).toEqual({inOrder: true, moves: []});
        expect(commit.git).toBeNull();
    });

    it("decodes the sort result", () => {
        expect(decodeSortResult({moved: 3})).toEqual({moved: 3});
    });

    it("decodes prefs, keeping gitAutocommit's three states apart", () => {
        expect(decodePrefs({hledgerPath: null, gitAutocommit: null})).toEqual({hledgerPath: null, gitAutocommit: null});
        expect(decodePrefs({hledgerPath: "/usr/bin/hledger", gitAutocommit: false})).toEqual({hledgerPath: "/usr/bin/hledger", gitAutocommit: false});
        expect(decodePrefs({})).toEqual({hledgerPath: null, gitAutocommit: null});
    });
});

// ---------------------------------------------------------------------------
// The state machine
// ---------------------------------------------------------------------------

const flow = (over: Partial<ImportFlowState> = {}): ImportFlowState => ({
    capabilitiesLoaded: true,
    hledgerAvailable: true,
    editable: true,
    staged: false,
    dryRunRequested: false,
    committed: false,
    ...over,
});

describe("UNIT visibleSections", () => {
    it("shows nothing at all before the capabilities probe answers", () => {
        expect(visibleSections(flow({capabilitiesLoaded: false}))).toEqual([]);
    });

    it("shows ONLY the hledger banner when hledger cannot be run", () => {
        // It gates everything: with no hledger, every affordance below is an
        // invitation to press a button that cannot work.
        expect(visibleSections(flow({hledgerAvailable: false}))).toEqual(["hledgerBanner"]);
    });

    it("keeps the hledger banner exclusive even once a file has been staged", () => {
        expect(visibleSections(flow({hledgerAvailable: false, staged: true, dryRunRequested: true, committed: true}))).toEqual(["hledgerBanner"]);
    });

    it("shows only the read-only banner when no journal is bound", () => {
        expect(visibleSections(flow({editable: false, staged: true}))).toEqual(["readOnlyBanner"]);
    });

    it("shows the drop target and nothing else before a file arrives", () => {
        expect(visibleSections(flow())).toEqual(["drop"]);
    });

    it("reveals the form once a file is staged, without hiding the drop target", () => {
        expect(visibleSections(flow({staged: true}))).toEqual(["drop", "preview", "candidates", "destinations", "balance", "actions"]);
    });

    it("adds the dry run, then the result, as each is requested", () => {
        expect(visibleSections(flow({staged: true, dryRunRequested: true}))).toContain("dryRun");
        const all = visibleSections(flow({staged: true, dryRunRequested: true, committed: true}));
        expect(all.slice(-2)).toEqual(["dryRun", "result"]);
    });

    it("`shows` answers membership", () => {
        expect(shows(visibleSections(flow({staged: true})), "balance")).toBe(true);
        expect(shows(visibleSections(flow()), "balance")).toBe(false);
    });
});

// ---------------------------------------------------------------------------
// The dropped file
// ---------------------------------------------------------------------------

describe("UNIT the dropped file", () => {
    it.each([
        ["bank.csv", "csv"],
        ["BANK.CSV", "csv"],
        ["statement.2026.xlsx", "xlsx"],
        ["/home/me/Downloads/bank.qfx", "qfx"],
        ["C:\\Users\\me\\bank.OFX", "ofx"],
        ["noextension", ""],
        [".gitignore", ""],
        ["trailingdot.", ""],
    ])("reads the extension of %s as %s", (name, expected) => {
        expect(fileExtension(name)).toBe(expected);
    });

    it("reduces a path-shaped name to a bare one for the filename header", () => {
        expect(headerFilename("/etc/../home/me/bank.csv")).toBe("bank.csv");
        expect(headerFilename("C:\\Users\\me\\bank.csv")).toBe("bank.csv");
        expect(headerFilename("../../bank.csv")).toBe("bank.csv");
    });

    it("replaces bytes an HTTP header cannot carry", () => {
        // `fetch` throws a bare TypeError on a header value outside latin-1, and
        // that would surface to the user as "network failure" for a file that is
        // perfectly readable.
        expect(headerFilename("relevé-café.csv")).toBe("relev_-caf_.csv");
        expect(headerFilename("bank\r\nX-Evil: 1.csv")).toBe("bank__X-Evil: 1.csv");
    });

    it("never produces an empty filename header", () => {
        expect(headerFilename("")).toBe("statement");
        expect(headerFilename("...")).toBe("statement");
        expect(headerFilename("   ")).toBe("statement");
    });

    it("builds the file picker's accept list from what the engine says it reads", () => {
        expect(acceptAttribute(["csv", "ofx"])).toBe(".csv,.ofx");
        expect(acceptAttribute([])).toBe("");
    });

    it("refuses a PDF by name, with its own message", () => {
        expect(refuseFile("statement.pdf", ["csv"])).toMatch(/PDF/);
    });

    it("refuses an extension this engine does not read, naming the ones it does", () => {
        const message = refuseFile("notes.docx", ["csv", "ofx"]);
        expect(message).toContain(".docx");
        expect(message).toContain(".csv and .ofx");
    });

    it("lets a file with no extension through, because sniffing is what that case is for", () => {
        expect(refuseFile("statement", ["csv"])).toBeNull();
    });

    it("lets a supported extension through regardless of case", () => {
        expect(refuseFile("BANK.CSV", ["csv"])).toBeNull();
    });

    it("lists formats as a sentence", () => {
        expect(formatList([])).toBe("nothing on this server");
        expect(formatList(["csv"])).toBe(".csv");
        expect(formatList(["csv", "ofx", "xlsx"])).toBe(".csv, .ofx and .xlsx");
    });
});

// ---------------------------------------------------------------------------
// Destinations
// ---------------------------------------------------------------------------

describe("UNIT destination derivation", () => {
    it.each([
        ["import/2026/bank.csv.rules", "import/2026/bank.csv"],
        ["bank.csv.rules", "bank.csv"],
        ["import/bank.rules", "import/bank.csv"],
        ["bank.tsv.rules", "bank.tsv"],
    ])("derives %s → %s", (rulesId, expected) => {
        expect(csvPathForRules(rulesId)).toBe(expected);
    });

    it("declines to derive from an id that is not a rules file", () => {
        expect(csvPathForRules("import/bank.csv")).toBeNull();
        expect(csvPathForRules(".rules")).toBeNull();
    });

    it("lets the CHOSEN rules file win over the server's default", () => {
        // Switching candidate is the user saying "read it as this instead"; the
        // previous candidate's file name staying in the box is how a credit-card
        // statement gets written over checking.csv.
        const defaults = {csvPath: "import/2026/bank.csv", journalId: "2026/2026.journal"};
        expect(deriveCsvPath(defaults, "import/2026/creditcard.csv.rules")).toBe("import/2026/creditcard.csv");
    });

    it("falls back to the server default with no rules file, or an id carrying no convention", () => {
        const defaults = {csvPath: "import/2026/bank.csv", journalId: null};
        expect(deriveCsvPath(defaults, null)).toBe("import/2026/bank.csv");
        expect(deriveCsvPath(defaults, "weird-handle")).toBe("import/2026/bank.csv");
    });

    it("rejects a destination that leaves the include root", () => {
        expect(validateCsvPath("")).toHaveLength(1);
        expect(validateCsvPath("   ")[0]).toMatch(/name/);
        expect(validateCsvPath("/etc/passwd")[0]).toMatch(/root of the disk/);
        expect(validateCsvPath("C:\\Windows\\x.csv")[0]).toMatch(/root of the disk/);
        expect(validateCsvPath("../outside.csv")[0]).toMatch(/outside/);
        expect(validateCsvPath("import/2026/bank.csv")).toEqual([]);
    });

    it("prefers the staged default journal when it is one that can be written", () => {
        const journals = [journal({id: "a.journal"}), journal({id: "2026/2026.journal"})];
        expect(defaultJournalId({csvPath: "x.csv", journalId: "2026/2026.journal"}, journals)).toBe("2026/2026.journal");
    });

    it("falls back to the engine's top-ranked writable journal", () => {
        const journals = [journal({id: "prices.journal", writable: false}), journal({id: "a.journal"})];
        expect(defaultJournalId({csvPath: "x.csv", journalId: "gone.journal"}, journals)).toBe("a.journal");
        expect(defaultJournalId({csvPath: "x.csv", journalId: null}, journals)).toBe("a.journal");
    });

    it("has no default when nothing offered can be written", () => {
        expect(defaultJournalId({csvPath: "x.csv", journalId: null}, [journal({writable: false})])).toBeNull();
    });

    it("makes the engine's ranking legible in the option label", () => {
        expect(journalOptionLabel(journal())).toBe("2026.journal — 412 transactions, latest 2026-08-01");
        expect(journalOptionLabel(journal({label: "accounts.journal", txnCount: 0, lastTxnDate: null}))).toBe("accounts.journal — no transactions");
        expect(journalOptionLabel(journal({txnCount: 1, isRoot: true, writable: false}))).toBe(
            "2026.journal — 1 transaction, latest 2026-08-01, main file, read-only"
        );
    });
});

// ---------------------------------------------------------------------------
// Candidates
// ---------------------------------------------------------------------------

describe("UNIT candidates", () => {
    it("says 'no rules file fits' only once an answer exists", () => {
        // The two states this must NOT be confused with are both also "nothing
        // to show": nothing staged, and an upload still in flight.
        expect(noCandidates(null)).toBe(false);
        expect(noCandidates(staged())).toBe(false);
        expect(noCandidates(decodeStagedFile({...STAGE_JSON, candidates: []}))).toBe(true);
    });

    it("formats a score as a whole percentage, clamped", () => {
        expect(formatScore(0.98)).toBe("98%");
        expect(formatScore(0)).toBe("0%");
        expect(formatScore(1)).toBe("100%");
        expect(formatScore(-4)).toBe("0%");
        expect(formatScore(Number.NaN)).toBe("—");
    });

    it("tones a score", () => {
        expect(scoreTone(0.98)).toBe("success");
        expect(scoreTone(0.65)).toBe("warning");
        expect(scoreTone(0.2)).toBe("error");
    });

    it("names fact 4's symptoms explicitly, and only when they occurred", () => {
        const clean = signalLines({
            txns: 26,
            postings: 52,
            amountlessPostings: 0,
            bareCommodityAmounts: 0,
            unknownAccounts: 0,
            emptyDescriptions: null,
            columnCountMatches: null,
            headerMatchesSource: null,
        });
        expect(clean).toHaveLength(1);
        expect(clean[0]!.text).toBe("26 transactions from 52 postings");
        expect(clean[0]!.bad).toBe(false);

        const broken = signalLines({
            txns: 26,
            postings: 52,
            amountlessPostings: 4,
            bareCommodityAmounts: 12,
            unknownAccounts: 3,
            emptyDescriptions: 1,
            columnCountMatches: false,
            headerMatchesSource: true,
        });
        expect(broken.filter((line) => line.bad)).toHaveLength(5);
        // The bare-commodity line has to spell out its consequence: the symptom
        // is "the import succeeded but my balance didn't move".
        expect(broken.some((line) => line.text.includes("balance would not move"))).toBe(true);
    });

    it("keeps the engine's ranking and only trims the sample", () => {
        const file = decodeStagedFile({
            ...STAGE_JSON,
            candidates: [
                {...STAGE_JSON.candidates[0], id: "a.rules", score: 0.4},
                {...STAGE_JSON.candidates[0], id: "b.rules", score: 0.9},
            ],
        });
        // Ranking is `score DESC, mtime DESC` and is the ENGINE's; re-sorting
        // here would silently override a documented decision using half its input.
        expect(candidateCards(file.candidates).map((card) => card.candidate.id)).toEqual(["a.rules", "b.rules"]);
        expect(candidateCards(file.candidates, 1)[0]!.sample).toHaveLength(1);
    });

    it("defaults the balance account to the chosen candidate's own account1", () => {
        // Read off the candidate, which carries it. The previous shape joined
        // the candidate id against a separately-fetched `/api/rules` listing —
        // a second round trip whose failure mode was a silently empty field.
        const file = staged();
        expect(defaultBalanceAccount(candidateById(file, "import/2026/bank.csv.rules"))).toBe("assets:bank:checking");
        expect(defaultBalanceAccount(candidateById(file, null))).toBe("");
        expect(defaultBalanceAccount(candidateById(file, "unlisted.rules"))).toBe("");
        expect(defaultBalanceAccount(candidateById(null, "import/2026/bank.csv.rules"))).toBe("");
    });

    it("has no balance account to default to when the rules file declares none", () => {
        const file = decodeStagedFile({
            ...STAGE_JSON,
            // `account1`/`account2` are omitted by the engine when the file
            // declares none, and an absent key must read as "none" rather than
            // throwing the whole screen away.
            candidates: [{...STAGE_JSON.candidates[0], account1: undefined, account2: undefined}],
        });
        expect(file.candidates[0]!.account1).toBeNull();
        expect(defaultBalanceAccount(candidateById(file, "import/2026/bank.csv.rules"))).toBe("");
    });
});

// ---------------------------------------------------------------------------
// Notes and preview copy
// ---------------------------------------------------------------------------

describe("UNIT ConvertNote rendering", () => {
    it.each([
        [{kind: "sheetChosen", name: "Statement", of: 3} as const, 'Read the sheet "Statement" — the workbook has 3.'],
        [{kind: "sheetChosen", name: "Sheet1", of: 1} as const, 'Read the sheet "Sheet1".'],
        [{kind: "datesFromSerial", count: 12} as const, "12 dates were stored as spreadsheet serial numbers and were read as dates."],
        [{kind: "preambleSkipped", lines: 4} as const, "4 rows of preamble above the header were skipped."],
        [{kind: "preambleSkipped", lines: 1} as const, "1 row of preamble above the header was skipped."],
        [{kind: "raggedRows", count: 1} as const, "1 row has a different number of columns than the header."],
    ])("renders %j", (note, expected) => {
        expect(noteText(note)).toBe(expected);
    });

    it("names the delimiter it guessed rather than printing the character bare", () => {
        expect(noteText({kind: "delimiterSniffed", delimiter: ";"})).toBe("Delimiter guessed: semicolon.");
        expect(noteText({kind: "delimiterSniffed", delimiter: "\t"})).toBe("Delimiter guessed: tab.");
        expect(noteText({kind: "delimiterSniffed", delimiter: "~"})).toBe('Delimiter guessed: "~".');
    });

    it("says what a guessed encoding means for the preview", () => {
        expect(noteText({kind: "encodingGuessed", label: "windows-1252"})).toContain("windows-1252");
    });

    it("makes a failed arithmetic check read as the loud failure it is", () => {
        const note = {kind: "balanceMismatch", expected: "100.00", computed: "99.00"} as const;
        expect(noteText(note)).toContain("100.00");
        expect(noteText(note)).toContain("99.00");
        expect(noteIsWarning(note)).toBe(true);
        expect(noteIsWarning({kind: "raggedRows", count: 1})).toBe(true);
        expect(noteIsWarning({kind: "preambleSkipped", lines: 1})).toBe(false);
    });

    it("summarises the preview, saying when it is only the first rows", () => {
        expect(previewSummary(staged())).toBe("OFX — 26 rows, showing the first 1");
        const whole = decodeStagedFile({...STAGE_JSON, preview: {...STAGE_JSON.preview, rowCount: 1}});
        expect(previewSummary(whole)).toBe("OFX — 1 row");
    });

    it("lists only the statement facts the format actually volunteered", () => {
        expect(statementFacts(null)).toEqual([]);
        expect(statementFacts(staged().statement)).toEqual([
            {label: "Account", value: "…7777"},
            {label: "Currency", value: "USD"},
            {label: "Statement balance", value: "-3238.65"},
            {label: "as of", value: "2026-08-12"},
        ]);
        expect(statementFacts({accountHint: null, currency: null, ledgerBalance: "1.00", balanceAsOf: null})).toEqual([
            {label: "Statement balance", value: "1.00"},
        ]);
    });
});

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

describe("UNIT actions", () => {
    it("offers Save CSV alone with no rules file, and Save and Import with one", () => {
        expect(importAction(null)).toBe("saveCsv");
        expect(actionLabel(importAction(null))).toBe("Save CSV");
        expect(importAction("bank.csv.rules")).toBe("saveAndImport");
        expect(actionLabel(importAction("bank.csv.rules"))).toBe("Save and Import");
    });

    it("dry-runs only the import — Save CSV has nothing to propose", () => {
        expect(actionRunsDryRun("saveAndImport")).toBe(true);
        expect(actionRunsDryRun("saveCsv")).toBe(false);
    });

    it("names what blocks the action, one fixable thing at a time", () => {
        const ok = {csvPath: "import/bank.csv", journalId: "2026.journal", balance: "", balanceAccount: ""};
        expect(actionBlocker("saveAndImport", ok)).toBeNull();
        expect(actionBlocker("saveAndImport", {...ok, csvPath: ""})).toMatch(/name/);
        expect(actionBlocker("saveAndImport", {...ok, journalId: null})).toMatch(/journal/);
        // Save CSV needs no journal at all.
        expect(actionBlocker("saveCsv", {...ok, journalId: null})).toBeNull();
    });

    it("refuses a balance with no account to assert it against", () => {
        const draft = {csvPath: "bank.csv", journalId: "j", balance: "2945.05", balanceAccount: ""};
        expect(actionBlocker("saveAndImport", draft)).toMatch(/account/);
        expect(actionBlocker("saveAndImport", {...draft, balanceAccount: "assets:bank"})).toBeNull();
        // A blank balance needs no account.
        expect(actionBlocker("saveAndImport", {...draft, balance: "   "})).toBeNull();
    });

    it("does not fire the balance blocker once the account has been seeded from the rules file", () => {
        // The ordinary path: an OFX volunteers its closing balance and the
        // chosen candidate's `account1` fills the account in. Nothing is
        // missing, so nothing may be in the way of the button.
        const seeded = {
            csvPath: "import/2026/bank.csv",
            journalId: "2026/2026.journal",
            balance: "-3238.65",
            balanceAccount: defaultBalanceAccount(candidateById(decodeStagedFile(STAGE_JSON), "import/2026/bank.csv.rules")),
        };
        expect(seeded.balanceAccount).toBe("assets:bank:checking");
        expect(actionBlocker("saveAndImport", seeded)).toBeNull();
    });

    it("never blocks Save CSV over a balance its request cannot carry", () => {
        // `Save CSV` is the no-rules-file path, so there is no `account1` to
        // seed the account from — and its route takes `{stageId, csvPath}` and
        // nothing else. Blocking it over the prefilled balance dead-ended the
        // only button that path has.
        const prefilled = {csvPath: "import/2026/bank.csv", journalId: null, balance: "-3238.65", balanceAccount: ""};
        expect(actionBlocker("saveCsv", prefilled)).toBeNull();
        // The CSV path is the one thing that request DOES carry, so it still speaks up.
        expect(actionBlocker("saveCsv", {...prefilled, csvPath: "  "})).toMatch(/name/);
    });
});

// ---------------------------------------------------------------------------
// Busy, which is not the question `dataView` answers
// ---------------------------------------------------------------------------

describe("UNIT isInFlight / formIsBusy", () => {
    it("does not call an unrequested resource busy, where `dataView` calls it loading", () => {
        // The whole class of bug, in two lines. `dataView("idle", false)` is
        // "loading" ON PURPOSE — every other surface in this app fetches on
        // mount, so idle is the gap before the first response and a spinner
        // belongs in it. This screen's three resources wait for a drop or a
        // button press and may sit idle forever, so the same reading froze the
        // form, span the drop target and disabled the action button at rest.
        expect(dataView("idle", false)).toBe("loading");
        expect(isInFlight("idle")).toBe(false);
        expect(formIsBusy("idle", "idle")).toBe(false);
    });

    it("is true only while a request is genuinely running", () => {
        expect(isInFlight("loading")).toBe(true);
        expect(isInFlight("ready")).toBe(false);
        expect(isInFlight("error")).toBe(false);
    });

    it("freezes the form for either write, and unfreezes when both have settled", () => {
        expect(formIsBusy("loading", "idle")).toBe(true);
        expect(formIsBusy("idle", "loading")).toBe(true);
        expect(formIsBusy("ready", "ready")).toBe(false);
        // A failed dry run leaves the form editable: fixing a destination is
        // the only way out of one.
        expect(formIsBusy("error", "idle")).toBe(false);
    });
});

// ---------------------------------------------------------------------------
// The dry run
// ---------------------------------------------------------------------------

describe("UNIT dry run rendering", () => {
    it("makes the silently-skipped back-dated rows loud", () => {
        const message = skippedWarning({olderThan: "2026-02-05", count: 1});
        expect(message).toContain("2026-02-05");
        expect(message).toContain("silently");
        expect(skippedWarning({olderThan: "2026-02-05", count: 3})).toContain("3 rows");
    });

    it("says nothing when nothing was skipped", () => {
        expect(skippedWarning(null)).toBeNull();
        expect(skippedWarning({olderThan: "2026-02-05", count: 0})).toBeNull();
    });

    it("reports the engine's balance verdict and its own arithmetic, never ours", () => {
        const matched = balanceVerdict({statement: "2945.05", computed: "2945.05", matches: true, difference: "0.00"});
        expect(matched.tone).toBe("success");
        expect(matched.detail).toContain("2945.05");

        const off = balanceVerdict({statement: "2945.05", computed: "1950.05", matches: false, difference: "995.00"});
        expect(off.tone).toBe("error");
        expect(off.headline).toBe("Off by 995.00.");
        expect(off.detail).toContain("1950.05");
    });

    it("still says the balance is wrong when the engine cannot size the gap", () => {
        // A multi-commodity balance has no single number to be off BY, so the
        // engine sends `difference: null`. Dropping the headline there would
        // hide the one fact that matters.
        const verdict = balanceVerdict({statement: "$10, 2 AAPL", computed: "$10", matches: false, difference: null});
        expect(verdict.headline).toBe("The balance doesn't match.");
        expect(verdict.tone).toBe("error");
    });

    it("decodes a null difference rather than refusing the whole dry run", () => {
        const run = decodeDryRun({...DRY_RUN_JSON, balance: {statement: "a", computed: "b", matches: false, difference: null}});
        if (!run.ok) throw new Error("unreachable");
        expect(run.balance?.difference).toBeNull();
    });

    it("trusts `matches` even when the two strings would compare equal as text", () => {
        // The verdict is the engine's, computed by concatenation (fact 3). If it
        // says they disagree, we say they disagree.
        expect(balanceVerdict({statement: "10.0", computed: "10.0", matches: false, difference: "0.00"}).tone).toBe("error");
    });

    it("names the files git blocks on, and says nothing when clear", () => {
        expect(gitBlockMessage([])).toBeNull();
        expect(gitBlockMessage(["2026/2026.journal"])).toContain("one file");
        expect(gitBlockMessage(["a", "b"])).toContain("2 files");
    });

    it("refuses to offer a write with no dry run, a failed one, or a git block", () => {
        expect(canWrite(null)).toBe(false);
        expect(canWrite({ok: false, stderr: "boom"})).toBe(false);
        expect(canWrite(decodeDryRun(DRY_RUN_JSON))).toBe(false);
        expect(canWrite(decodeDryRun({...DRY_RUN_JSON, blockedByGit: []}))).toBe(true);
    });
});

// ---------------------------------------------------------------------------
// Stale-request matching (FE-1)
// ---------------------------------------------------------------------------

describe("UNIT sameRunRequest", () => {
    const body = {stageId: "s1", rulesId: "bank.rules", csvPath: "bank.csv", journalId: "2026.journal", balance: null, balanceAccount: null};

    it("matches a body against itself", () => {
        expect(sameRunRequest(body, {...body})).toBe(true);
    });

    it.each([
        ["stageId", {stageId: "s2"}],
        ["rulesId", {rulesId: "card.rules"}],
        ["csvPath", {csvPath: "other.csv"}],
        ["journalId", {journalId: "2025.journal"}],
        ["balance", {balance: "1.00"}],
        ["balanceAccount", {balanceAccount: "assets:bank"}],
    ])("treats a different %s as a different question", (_field, over) => {
        // Neither DryRunResult nor CommitResult names the request it answers, so
        // this comparison is the ONLY thing standing between a user and a
        // credit card's proposed transactions shown under a checking
        // destination.
        expect(sameRunRequest(body, {...body, ...over})).toBe(false);
    });

    it("treats null as matching only null", () => {
        expect(sameRunRequest(null, null)).toBe(true);
        expect(sameRunRequest(null, body)).toBe(false);
        expect(sameRunRequest(body, null)).toBe(false);
    });
});

describe("UNIT sameWriteRequest", () => {
    const importBody = {
        stageId: "s1",
        rulesId: "bank.rules",
        csvPath: "bank.csv",
        journalId: "2026.journal",
        balance: null,
        balanceAccount: null,
        writeAssertion: true,
    };
    const anImport = {kind: "import", body: importBody} as const;
    const aSave = {kind: "saveCsv", body: {stageId: "s1", csvPath: "bank.csv"}} as const;

    it("matches a request against itself, of either kind", () => {
        expect(sameWriteRequest(anImport, {...anImport})).toBe(true);
        expect(sameWriteRequest(aSave, {...aSave})).toBe(true);
    });

    it("never matches across kinds", () => {
        // "Saved bank.csv" must stop rendering the moment a rules file is
        // chosen: the next press writes a journal as well, which the result
        // panel on screen says nothing about.
        expect(sameWriteRequest(anImport, aSave)).toBe(false);
        expect(sameWriteRequest(aSave, anImport)).toBe(false);
    });

    it("treats a changed field as a different question", () => {
        expect(sameWriteRequest(anImport, {kind: "import", body: {...importBody, writeAssertion: false}})).toBe(false);
        expect(sameWriteRequest(anImport, {kind: "import", body: {...importBody, csvPath: "other.csv"}})).toBe(false);
        expect(sameWriteRequest(aSave, {kind: "saveCsv", body: {stageId: "s2", csvPath: "bank.csv"}})).toBe(false);
        expect(sameWriteRequest(aSave, {kind: "saveCsv", body: {stageId: "s1", csvPath: "other.csv"}})).toBe(false);
    });

    it("treats null as matching only null", () => {
        expect(sameWriteRequest(null, null)).toBe(true);
        expect(sameWriteRequest(null, aSave)).toBe(false);
        expect(sameWriteRequest(anImport, null)).toBe(false);
    });
});

// ---------------------------------------------------------------------------
// The result
// ---------------------------------------------------------------------------

describe("UNIT result rendering", () => {
    const commit = (): CommitResult => decodeCommitResult(COMMIT_JSON);

    it("says what was written, including what git did", () => {
        expect(writtenLines(commit())).toEqual([
            "Wrote import/2026/bank.csv.",
            "Imported 3 transactions into 2026/2026.journal.",
            "Committed import/2026/bank.csv, 2026/2026.journal.",
        ]);
    });

    it("says nothing about a journal on the Save-CSV-only path", () => {
        expect(writtenLines(decodeCommitResult({csvWritten: "bank.csv"}))).toEqual(["Wrote bank.csv."]);
    });

    it("reports what git declined to commit rather than hiding it", () => {
        const result = decodeCommitResult({...COMMIT_JSON, git: {committed: true, paths: ["a"], skipped: ["b"]}});
        expect(writtenLines(result)).toContain("Not committed: b.");
    });

    it("offers the re-sort only when the journal came out of order", () => {
        expect(reorderOffer(commit())).toContain("1 transaction would move");
        expect(reorderOffer(decodeCommitResult({...COMMIT_JSON, ordering: {inOrder: true, moves: []}}))).toBeNull();
    });
});

// ---------------------------------------------------------------------------
// The hledger banner
// ---------------------------------------------------------------------------

describe("UNIT hledger banner copy", () => {
    const withReason = (reason: string | null, version?: string): ImportCapabilities =>
        decodeImportCapabilities({...CAPABILITIES_JSON, hledger: {available: false, reason, version, message: "engine says so"}});

    it("leads with the install/path remedy when hledger is missing", () => {
        const copy = hledgerBannerCopy(withReason("notFound"));
        expect(copy.headline).toMatch(/can't find hledger/);
        expect(copy.offersPath).toBe(true);
    });

    it("names the version and the 1.40 floor when it is too old", () => {
        const copy = hledgerBannerCopy(withReason("tooOld", "1.31"));
        expect(copy.headline).toContain("1.31");
        expect(copy.detail).toContain("1.40");
    });

    it("distinguishes a hung hledger from a missing one", () => {
        expect(hledgerBannerCopy(withReason("timedOut")).headline).toMatch(/didn't answer/);
    });

    it("still offers a remedy for an unrunnable or unrecognised reason", () => {
        expect(hledgerBannerCopy(withReason("unrunnable")).offersPath).toBe(true);
        expect(hledgerBannerCopy(withReason(null)).offersPath).toBe(true);
    });

    it("always has the engine's own sentence to show beside the copy", () => {
        expect(capabilities().hledger.message).toBeNull();
        expect(withReason("notFound").hledger.message).toBe("engine says so");
    });
});
