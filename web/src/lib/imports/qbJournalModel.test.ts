import {describe, expect, it} from "vitest";
import {
    canCommitQbJournal,
    dateFormatNotice,
    defaultAliasTargetFile,
    filesNeedingSort,
    hasMappingsToSave,
    isQuickbooksJournalFormat,
    isQuickbooksJournalStage,
    mappingDraft,
    mappingEdits,
    mappingProblems,
    qbIdMatchesSummary,
    qbReorderOffer,
} from "./qbJournalModel";
import type {AliasFile, QbFileOrdering, QbIdMatches, QbOrdering, QbPreview, StagedFile} from "./importTypes";

// Literal domain objects, not built through the decoder — the same discipline
// `aliasModel.test.ts` follows (`alias()`), so a test cannot pass because a
// bug in this file and a bug in the decoder cancel each other out.

function staged(format: string): StagedFile {
    return {
        stageId: "s1",
        format,
        preview: {header: null, rows: [], rowCount: 0, truncated: false},
        statement: null,
        notes: [],
        unknownNoteCount: 0,
        candidates: [],
        defaults: {csvPath: "", journalId: null},
    };
}

function aliasFile(over: Partial<AliasFile> = {}): AliasFile {
    return {journalId: "main.journal", label: "main.journal", revision: "rev-1", writable: true, aliases: [], ...over};
}

function idMatches(over: Partial<QbIdMatches> = {}): QbIdMatches {
    return {new: 0, unchanged: 0, conflicting: [], conflictingTotal: 0, ...over};
}

function fileOrdering(over: Partial<QbFileOrdering> = {}): QbFileOrdering {
    return {journalId: "2026/2026.journal", inOrder: true, moves: [], ...over};
}

describe("UNIT qbJournalModel — the one branch point", () => {
    it("recognises the exact format string the engine sends for this pipeline", () => {
        expect(isQuickbooksJournalFormat("quickbooks-journal")).toBe(true);
    });

    it("treats every other format as the ordinary CSV/spreadsheet path", () => {
        expect(isQuickbooksJournalFormat("csv")).toBe(false);
        expect(isQuickbooksJournalFormat("ofx")).toBe(false);
        expect(isQuickbooksJournalFormat("")).toBe(false);
    });

    it("routes a staged file by its format alone", () => {
        expect(isQuickbooksJournalStage(staged("quickbooks-journal"))).toBe(true);
        expect(isQuickbooksJournalStage(staged("csv"))).toBe(false);
    });

    it("is false before anything is staged", () => {
        expect(isQuickbooksJournalStage(null)).toBe(false);
    });
});

describe("UNIT qbJournalModel — the date-format ambiguity affordance", () => {
    it("says nothing when the export gave enough evidence", () => {
        expect(dateFormatNotice({format: "%m/%d/%Y", ambiguous: false})).toBeNull();
    });

    it("names the format and asks the user to check the sample when ambiguous", () => {
        const notice = dateFormatNotice({format: "%m/%d/%Y", ambiguous: true});
        expect(notice).not.toBeNull();
        expect(notice).toContain("%m/%d/%Y");
        expect(notice).toContain("sample below");
    });
});

describe("UNIT qbJournalModel — resolving unmapped accounts", () => {
    it("builds a draft with the account as a FIXED pattern and the typed text as the replacement", () => {
        const draft = mappingDraft("3000 Member Equity", "equity:opening");
        expect(draft).toEqual({index: null, pattern: "3000 Member Equity", replacement: "equity:opening", regex: false, deleted: false, locked: false});
    });

    it("has no problems for a well-formed replacement", () => {
        expect(mappingProblems("3000 Member Equity", "equity:opening")).toEqual([]);
    });

    it("reuses the alias editor's own validation — an empty replacement is refused the same way", () => {
        const problems = mappingProblems("3000 Member Equity", "");
        expect(problems.length).toBeGreaterThan(0);
        expect(problems.some((p) => p.includes("cannot be empty"))).toBe(true);
    });

    it("builds one append edit per account with a valid, non-blank typed replacement", () => {
        const edits = mappingEdits(["a", "b", "c"], {a: "assets:a", b: "  ", c: "assets:c"});
        expect(edits).toEqual([
            {kind: "append", pattern: "a", replacement: "assets:a", regex: false},
            {kind: "append", pattern: "c", replacement: "assets:c", regex: false},
        ]);
    });

    it("skips a row whose typed replacement fails the engine's own rules rather than refusing the whole submit", () => {
        const edits = mappingEdits(["a", "b"], {a: "assets:a", b: "has;semicolon"});
        expect(edits).toEqual([{kind: "append", pattern: "a", replacement: "assets:a", regex: false}]);
    });

    it("has nothing to save when every row is blank or untyped", () => {
        expect(hasMappingsToSave(["a", "b"], {})).toBe(false);
        expect(hasMappingsToSave(["a"], {a: "   "})).toBe(false);
    });

    it("has something to save once at least one row is valid", () => {
        expect(hasMappingsToSave(["a", "b"], {a: "assets:a"})).toBe(true);
    });

    it("picks the first WRITABLE alias file, in listing order", () => {
        const files = [aliasFile({journalId: "readonly.journal", writable: false}), aliasFile({journalId: "main.journal", writable: true})];
        expect(defaultAliasTargetFile(files)?.journalId).toBe("main.journal");
    });

    it("is null when nothing here can be written to", () => {
        expect(defaultAliasTargetFile([aliasFile({writable: false})])).toBeNull();
        expect(defaultAliasTargetFile([])).toBeNull();
    });
});

describe("UNIT qbJournalModel — the commit gate", () => {
    const preview = (unmapped: readonly string[]): QbPreview => ({
        stageId: "s1",
        transactionCount: 1,
        postingCount: 2,
        dateFormat: {format: "%m/%d/%Y", ambiguous: false},
        unmappedAccounts: unmapped,
        sample: [],
        idMatches: unmapped.length === 0 ? idMatches() : null,
    });

    it("is blocked before a preview has answered", () => {
        expect(canCommitQbJournal(null)).toBe(false);
    });

    it("is blocked while any account is unmapped", () => {
        expect(canCommitQbJournal(preview(["3000 Member Equity"]))).toBe(false);
    });

    it("is open once every account resolves", () => {
        expect(canCommitQbJournal(preview([]))).toBe(true);
    });
});

describe("UNIT qbJournalModel — id-match summary", () => {
    it("says nothing when there is nothing conflicting", () => {
        expect(qbIdMatchesSummary(null)).toBeNull();
        expect(qbIdMatchesSummary(idMatches({new: 3, unchanged: 1}))).toBeNull();
    });

    it("names a conflict rather than staying quiet, even alone", () => {
        const summary = qbIdMatchesSummary(idMatches({conflictingTotal: 1}));
        expect(summary).toContain("1 row");
        expect(summary).toContain("left untouched");
    });

    it("pluralises for more than one conflict", () => {
        expect(qbIdMatchesSummary(idMatches({conflictingTotal: 2}))).toContain("2 rows");
    });
});

describe("UNIT qbJournalModel — per-file ordering after a commit", () => {
    it("offers nothing for a file already in order", () => {
        expect(qbReorderOffer(fileOrdering({inOrder: true}))).toBeNull();
    });

    it("names the file and how many transactions would move", () => {
        const offer = qbReorderOffer(
            fileOrdering({
                journalId: "2026/2026.journal",
                inOrder: false,
                moves: [{date: "2026-01-01", description: "a", fromLine: 1, toLine: 2}],
            })
        );
        expect(offer).toContain("2026/2026.journal");
        expect(offer).toContain("1 transaction");
    });

    it("filters an ordering report down to the out-of-order files", () => {
        const ordering: QbOrdering = {
            inOrder: false,
            files: [fileOrdering({journalId: "a", inOrder: true}), fileOrdering({journalId: "b", inOrder: false})],
        };
        expect(filesNeedingSort(ordering).map((f) => f.journalId)).toEqual(["b"]);
    });

    it("is empty when every touched file is already in order", () => {
        const ordering: QbOrdering = {inOrder: true, files: [fileOrdering({journalId: "a", inOrder: true})]};
        expect(filesNeedingSort(ordering)).toEqual([]);
    });
});
