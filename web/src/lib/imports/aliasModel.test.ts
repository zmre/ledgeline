import {describe, expect, it} from "vitest";
import {
    ALIAS_EXPLAINER,
    aliasBadges,
    aliasNotice,
    aliasPatternText,
    aliasText,
    blankRow,
    isDirty,
    plainAliasMatches,
    relevantAliases,
    renameText,
    toEdits,
    toForm,
    toSaveRequest,
    validateForm,
    validateRow,
} from "./aliasModel";
import type {AliasDraft, AliasForm} from "./aliasModel";
import type {AliasEffect, AliasEntry, AliasFile} from "./importTypes";

// The fixtures below are literal wire shapes, written out rather than built by
// the app's own encoder — the same rule `importModel.test.ts` follows, so a test
// cannot pass because two of our own bugs cancel.

function alias(over: Partial<AliasEntry> = {}): AliasEntry {
    return {
        journalId: "main.journal",
        index: 0,
        line: 1,
        pattern: "PW Roth IRA - 3077",
        replacement: "assets:morganstanley:pw-roth-ira",
        regex: false,
        forwarded: true,
        refusal: null,
        refusalMessage: null,
        editable: true,
        lock: null,
        lockMessage: null,
        ...over,
    };
}

function effect(over: Partial<AliasEffect> = {}): AliasEffect {
    return {forwarded: 1, renames: [{from: "PW Roth IRA - 3077:cash", to: "assets:morganstanley:pw-roth-ira:cash"}], ...over};
}

describe("UNIT aliasModel — hledger's plain-alias matching rule", () => {
    // `alias a = b` rewrites `a` and `a:sub` and leaves `abc` alone. A naive
    // `startsWith` gets the third one wrong, and getting it wrong would put a
    // rename beside the wrong line of the user's journal.
    it("matches an exact name and a prefix at a colon boundary, and nothing else", () => {
        expect(plainAliasMatches("a", "a")).toBe(true);
        expect(plainAliasMatches("a", "a:sub")).toBe(true);
        expect(plainAliasMatches("a", "a:sub:deeper")).toBe(true);
        expect(plainAliasMatches("a", "abc")).toBe(false);
        expect(plainAliasMatches("a", "b:a")).toBe(false);
        expect(plainAliasMatches("a:b", "a")).toBe(false);
    });

    it("handles the bank-speak shape the feature exists for", () => {
        expect(plainAliasMatches("PW Roth IRA - 3077", "PW Roth IRA - 3077:cash")).toBe(true);
        expect(plainAliasMatches("PW Roth IRA - 3077", "PW Roth IRA - 30771")).toBe(false);
    });
});

describe("UNIT aliasModel — which aliases are relevant to the staged data", () => {
    // The requirement in one test: quiet unless an alias actually did something.
    it("says nothing when no alias is in force", () => {
        expect(relevantAliases([alias()], null)).toEqual([]);
        expect(aliasNotice(null)).toBeNull();
    });

    it("says nothing when the aliases matched nothing in this statement", () => {
        const nothing = effect({renames: []});
        expect(relevantAliases([alias()], nothing)).toEqual([]);
        expect(aliasNotice(nothing)).toBeNull();
    });

    it("attributes a rename to the plain alias that provably explains it", () => {
        const relevant = relevantAliases([alias()], effect());
        expect(relevant).toHaveLength(1);
        expect(relevant[0].alias.pattern).toBe("PW Roth IRA - 3077");
        expect(relevant[0].attributable).toBe(true);
        expect(relevant[0].renames).toEqual([{from: "PW Roth IRA - 3077:cash", to: "assets:morganstanley:pw-roth-ira:cash"}]);
    });

    it("leaves out an alias that explains nothing here", () => {
        const unrelated = alias({pattern: "CHK 8842", replacement: "assets:bank:checking", line: 2});
        const relevant = relevantAliases([alias(), unrelated], effect());
        expect(relevant.map((entry) => entry.alias.pattern)).toEqual(["PW Roth IRA - 3077"]);
    });

    // A regex alias cannot be attributed without running hledger's regex
    // dialect, which this codebase deliberately does not reimplement. So it is
    // offered as a possible explanation and flagged as unproven, rather than
    // asserted.
    it("offers a regex alias for what no plain alias explains, and marks it unproven", () => {
        const regex = alias({pattern: "^CC (.+)$", regex: true, line: 3});
        const renamed = effect({forwarded: 2, renames: [{from: "CC PLATINUM", to: "liabilities:PLATINUM"}]});
        const relevant = relevantAliases([alias(), regex], renamed);
        expect(relevant).toHaveLength(1);
        expect(relevant[0].alias.regex).toBe(true);
        expect(relevant[0].attributable).toBe(false);
    });

    it("does not offer a regex alias for a rename a plain one already explains", () => {
        const regex = alias({pattern: "^PW", regex: true, line: 3});
        const relevant = relevantAliases([alias(), regex], effect({forwarded: 2}));
        expect(relevant.map((entry) => entry.alias.regex)).toEqual([false]);
    });

    // An alias the engine refused is not in force, so it cannot have caused
    // anything and must not be presented as if it had.
    it("ignores an alias the engine did not forward", () => {
        const scoped = alias({forwarded: false, refusal: "scoped"});
        expect(relevantAliases([scoped], effect())).toEqual([]);
    });

    it("orders the relevant aliases by their line in the journal", () => {
        const first = alias({pattern: "A", replacement: "x:a", line: 9});
        const second = alias({pattern: "B", replacement: "x:b", line: 2});
        const both = effect({
            forwarded: 2,
            renames: [
                {from: "A", to: "x:a"},
                {from: "B", to: "x:b"},
            ],
        });
        expect(relevantAliases([first, second], both).map((entry) => entry.alias.line)).toEqual([2, 9]);
    });

    it("counts the renames in its headline, singular and plural", () => {
        expect(aliasNotice(effect())).toBe("Your journal's aliases rewrite 1 account name in this import.");
        expect(
            aliasNotice(
                effect({
                    renames: [
                        {from: "a", to: "b"},
                        {from: "c", to: "d"},
                    ],
                })
            )
        ).toBe("Your journal's aliases rewrite 2 account names in this import.");
    });
});

describe("UNIT aliasModel — display", () => {
    it("writes a regex pattern with its slashes and a plain one without", () => {
        expect(aliasPatternText(alias())).toBe("PW Roth IRA - 3077");
        expect(aliasPatternText(alias({pattern: "^CC", regex: true}))).toBe("/^CC/");
        expect(aliasText(alias())).toBe("PW Roth IRA - 3077 → assets:morganstanley:pw-roth-ira");
        expect(renameText({from: "a", to: "b"})).toBe("a → b");
    });

    it("badges only what is unusual, so a working row is undecorated", () => {
        expect(aliasBadges(alias())).toEqual([]);
        expect(aliasBadges(alias({forwarded: false})).map((badge) => badge.text)).toEqual(["not used for imports"]);
        expect(aliasBadges(alias({editable: false})).map((badge) => badge.text)).toEqual(["read-only"]);
        expect(aliasBadges(alias({regex: true})).map((badge) => badge.text)).toEqual(["regular expression"]);
    });

    // The explainer is the whole mitigation for a real divergence: Ledgeline
    // reads aliases but does not apply them to the journal it shows you. If it
    // ever stops saying so, that is a silent behaviour change.
    it("tells the user that Ledgeline does not apply aliases itself", () => {
        expect(ALIAS_EXPLAINER).toContain("does not rewrite the account names shown elsewhere");
        expect(ALIAS_EXPLAINER).toContain("hledger applies these itself");
    });
});

describe("UNIT aliasModel — the editor's diff", () => {
    const file: AliasFile = {
        journalId: "main.journal",
        label: "main.journal",
        revision: "2a-00ff",
        writable: true,
        aliases: [alias({index: 0, line: 1}), alias({index: 1, line: 2, pattern: "CHK 8842", replacement: "assets:bank:checking"})],
    };

    const edit = (form: AliasForm, at: number, over: Partial<AliasDraft>): AliasForm => ({
        ...form,
        rows: form.rows.map((row, i) => (i === at ? {...row, ...over} : row)),
    });

    it("produces no edits for an untouched form, so a save touches nothing", () => {
        const form = toForm(file);
        expect(toEdits(form, form)).toEqual([]);
        expect(isDirty(form, form)).toBe(false);
        expect(toSaveRequest(form, form)).toEqual({revision: "2a-00ff", edits: []});
    });

    it("names only the row that changed", () => {
        const base = toForm(file);
        const draft = edit(base, 1, {replacement: "assets:bank:everyday"});
        expect(toEdits(base, draft)).toEqual([{kind: "replace", index: 1, pattern: "CHK 8842", replacement: "assets:bank:everyday", regex: false}]);
        expect(isDirty(base, draft)).toBe(true);
    });

    it("turns a deleted row into a delete and an added row into an append", () => {
        const base = toForm(file);
        const draft: AliasForm = {
            ...base,
            rows: [{...base.rows[0], deleted: true}, base.rows[1], {...blankRow(), pattern: "SAV 1", replacement: "assets:bank:savings", regex: true}],
        };
        expect(toEdits(base, draft)).toEqual([
            {kind: "delete", index: 0},
            {kind: "append", pattern: "SAV 1", replacement: "assets:bank:savings", regex: true},
        ]);
    });

    // A blank row the user added and then abandoned must not become an append of
    // an empty alias the engine would refuse.
    it("ignores a row that was added and then emptied or removed", () => {
        const base = toForm(file);
        expect(toEdits(base, {...base, rows: [...base.rows, blankRow()]})).toEqual([]);
        expect(toEdits(base, {...base, rows: [...base.rows, {...blankRow(), pattern: "x", deleted: true}]})).toEqual([]);
    });

    // The engine refuses to rewrite a locked line. A UI that can build a request
    // the engine will refuse is a UI that eventually sends one.
    it("never produces a replace for a locked row", () => {
        const locked: AliasFile = {...file, aliases: [alias({index: 0, editable: false, lock: "commentLike"})]};
        const base = toForm(locked);
        expect(base.rows[0].locked).toBe(true);
        expect(toEdits(base, edit(base, 0, {replacement: "something:else"}))).toEqual([]);
    });

    it("still allows a locked row to be deleted", () => {
        const locked: AliasFile = {...file, aliases: [alias({index: 0, editable: false, lock: "commentLike"})]};
        const base = toForm(locked);
        expect(toEdits(base, edit(base, 0, {deleted: true}))).toEqual([{kind: "delete", index: 0}]);
    });

    it("carries the revision it was planned against, never a fresh one", () => {
        const base = toForm(file);
        const draft = {...edit(base, 0, {replacement: "z"}), revision: "somebody-elses"};
        expect(toSaveRequest(base, draft).revision).toBe("2a-00ff");
    });
});

describe("UNIT aliasModel — validation mirrors the engine's refusals", () => {
    const row = (over: Partial<AliasDraft> = {}): AliasDraft => ({...blankRow(), pattern: "a", replacement: "b:c", ...over});

    it("accepts an ordinary alias in both forms", () => {
        expect(validateRow(row())).toEqual([]);
        expect(validateRow(row({pattern: "^PW (.+)$", replacement: "assets:\\1", regex: true}))).toEqual([]);
    });

    it("refuses a value that would be written and then read back differently", () => {
        // Each of these is a rule `ledgeline_core::aliases` enforces; the form
        // states them so the user hears it while typing, not after a round trip.
        const cases: [Partial<AliasDraft>, string][] = [
            [{pattern: ""}, "pattern cannot be empty"],
            [{replacement: ""}, "replacement cannot be empty"],
            [{replacement: "b ; note"}, "not as a comment"],
            [{replacement: " b"}, "cannot begin or end with a space"],
            [{pattern: "a=b"}, "splits the line at the first one"],
            [{pattern: "/a/"}, "tick the box instead"],
            [{pattern: "a/b", regex: true}, "unescaped"],
            [{replacement: "b\nalias x = y"}, "control character"],
        ];
        for (const [over, needle] of cases) {
            const problems = validateRow(row(over));
            expect(problems.join(" "), JSON.stringify(over)).toContain(needle);
        }
    });

    it("enforces the engine's length caps", () => {
        expect(validateRow(row({pattern: "x".repeat(257)})).join(" ")).toContain("longer than 256 bytes");
        expect(validateRow(row({replacement: "x".repeat(513)})).join(" ")).toContain("longer than 512 bytes");
        // Bytes, not characters: a cap counted in UTF-16 units would let a
        // multi-byte name through and then be refused by the engine.
        expect(validateRow(row({pattern: "é".repeat(129)})).join(" ")).toContain("longer than 256 bytes");
    });

    it("says nothing about a row the user deleted", () => {
        expect(validateRow(row({pattern: "", deleted: true}))).toEqual([]);
    });

    it("numbers the row a problem is in", () => {
        const form: AliasForm = {journalId: "main.journal", label: "m", revision: "r", writable: true, rows: [row(), row({replacement: ""})]};
        expect(validateForm(form)).toEqual(["Alias 2: The replacement cannot be empty."]);
    });
});
