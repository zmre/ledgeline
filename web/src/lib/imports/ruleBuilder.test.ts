// "Remember as a rule" (plan 32 D25): the pattern a rule is prefilled with,
// the field it matches, what it sets, and how its live check's rows read.

import {describe, expect, it} from "vitest";
import {cardChange, reviewFormOf} from "#lib/api/editMapping.js";
import {decodeCsvStagedTxn} from "#lib/api/importReviewDecode.js";
import {decodeQueueItem} from "#lib/api/reviewDecode.js";
import {queueItemWire, stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {
    baseName,
    changedLines,
    defaultMatchField,
    descriptionPrefill,
    escapeRegex,
    fieldReferences,
    importPrefill,
    keyTokens,
    matchFields,
    rulePattern,
    ruleSpec,
    ruleTargets,
    rulesFileLabel,
    sourceText,
} from "./ruleBuilder";
import type {StagedRecords} from "./reviewTypes";

const RECORDS: StagedRecords = {
    aligned: true,
    header: ["Date", "Description", "Amount"],
    fields: ["date", "description", "amount"],
    rows: [
        ["01/15/2024", "ACME PAYROLL", "3000.00"],
        ["01/19/2024", "HARDWARE STORE #0042", "-62.13"],
    ],
};

describe("UNIT ruleBuilder — rulesFileLabel", () => {
    it("names a rules file by its file name when that is unique among the candidates", () => {
        expect(rulesFileLabel("import/citi-2026.csv.rules", ["import/citi-2025.csv.rules", "import/citi-2026.csv.rules"])).toBe("citi-2026.csv.rules");
        expect(rulesFileLabel("import/citi.csv.rules", ["import/citi.csv.rules"])).toBe("citi.csv.rules");
    });

    it("adds the relative path when another candidate has the same file name", () => {
        const all = ["2025/citi.csv.rules", "2026/citi.csv.rules", "import/amex.csv.rules"];
        expect(rulesFileLabel("2025/citi.csv.rules", all)).toBe("2025/citi.csv.rules");
        expect(rulesFileLabel("2026/citi.csv.rules", all)).toBe("2026/citi.csv.rules");
        expect(rulesFileLabel("import/amex.csv.rules", all)).toBe("amex.csv.rules");
    });
});

describe("UNIT ruleBuilder — keyTokens", () => {
    it("keeps what the engine's similarity key keeps (D10)", () => {
        expect(keyTokens("SQ *BLUE BOTTLE 4412 OAKLAND")).toEqual(["sq", "blue", "bottle", "oakland"]);
        expect(keyTokens("Peet's Coffee | latte")).toEqual(["peet", "s", "coffee"]);
        expect(keyTokens("POS0923 10/12 X1234")).toEqual([]);
        expect(keyTokens("ＡＣＭＥ Payroll")).toEqual(["acme", "payroll"]);
    });
});

describe("UNIT ruleBuilder — rulePattern", () => {
    it("joins the key's tokens with .* when the raw text has them in order", () => {
        expect(rulePattern("hardware store", "HARDWARE STORE #0042")).toBe("hardware.*store");
    });

    it("falls back to the raw text's own words when a rule renamed the description", () => {
        // The generated description (and so its key) is a rename; the bank's
        // text is what the new rule will be tested against.
        expect(rulePattern("home depot", "THD #6612 ATLANTA GA")).toBe("thd.*atlanta.*ga");
    });

    it("uses the key alone when there is no raw text (records not aligned)", () => {
        expect(rulePattern("sq blue bottle oakland", null)).toBe("sq.*blue.*bottle.*oakland");
    });

    it("is empty when there is nothing to build from", () => {
        expect(rulePattern(null, null)).toBe("");
        expect(rulePattern(null, "12345")).toBe("");
    });

    it("escapes regex metacharacters", () => {
        expect(escapeRegex("a.b*c(d)[e]|f+g?h^i$j{k}\\")).toBe("a\\.b\\*c\\(d\\)\\[e\\]\\|f\\+g\\?h\\^i\\$j\\{k\\}\\\\");
        expect(new RegExp(escapeRegex("1+1=(2)")).test("1+1=(2)")).toBe(true);
    });
});

describe("UNIT ruleBuilder — the field and the source text", () => {
    it("matches the description field when records line up and the rules file names it", () => {
        expect(defaultMatchField(RECORDS)).toBe("description");
    });

    it("matches the whole record when records do not line up, or there is no description field", () => {
        expect(defaultMatchField({...RECORDS, aligned: false, rows: []})).toBeNull();
        expect(defaultMatchField({...RECORDS, fields: ["date", "payee", "amount"]})).toBeNull();
        expect(defaultMatchField({...RECORDS, fields: null})).toBeNull();
    });

    it("offers the rules file's named fields, once each", () => {
        expect(matchFields({...RECORDS, fields: ["date", "", "description", "description", " amount "]})).toEqual(["date", "description", "amount"]);
        expect(matchFields({...RECORDS, fields: null})).toEqual([]);
    });

    it("reads the raw cell a field's matcher sees", () => {
        expect(sourceText(RECORDS, 1, "description")).toBe("HARDWARE STORE #0042");
        expect(sourceText(RECORDS, 5, "description")).toBeNull();
        expect(sourceText(RECORDS, 1, "payee")).toBeNull();
        expect(sourceText({...RECORDS, aligned: false}, 1, "description")).toBeNull();
    });
});

describe("UNIT ruleBuilder — what the rule sets", () => {
    const original = txnFixture(2, "HARDWARE STORE #0042", "expenses:home");
    // txnFixture's first posting is the given account, the second liabilities:visa.
    const unknownSecond = {...txnFixture(2, "HARDWARE STORE #0042"), postings: [...txnFixture(2, "x").postings].reverse()};

    it("sets the account slot of the posting that moved, checked", () => {
        const form = reviewFormOf(unknownSecond);
        const at = form.postings.findIndex((row) => row.account === "expenses:unknown");
        (form.postings[at] as {account: string}).account = "expenses:home:supplies";
        const change = cardChange(unknownSecond, form);
        if (change === null) throw new Error("expected a change");
        expect(ruleTargets(unknownSecond, change)).toEqual([{field: `account${at + 1}`, value: "expenses:home:supplies", defaultOn: true}]);
    });

    it("on a journal card, counts slots from the posting booked to the rules file's account1 (D27)", () => {
        // Written by hand with the expense first: the rules file still generates its account1 first.
        const form = reviewFormOf(original);
        (form.postings[0] as {account: string}).account = "expenses:home:supplies";
        const change = cardChange(original, form);
        if (change === null) throw new Error("expected a change");
        expect(ruleTargets(original, change)).toEqual([{field: "account1", value: "expenses:home:supplies", defaultOn: true}]);
        expect(ruleTargets(original, change, "liabilities:visa")).toEqual([{field: "account2", value: "expenses:home:supplies", defaultOn: true}]);
        // An account1 no posting holds leaves the order as written.
        expect(ruleTargets(original, change, "assets:elsewhere")).toEqual([{field: "account1", value: "expenses:home:supplies", defaultOn: true}]);
    });

    it("offers the new description, checked only when it is the whole change", () => {
        const renamed = reviewFormOf(original);
        renamed.description = "Hardware Store";
        const onlyRename = cardChange(original, renamed);
        if (onlyRename === null) throw new Error("expected a change");
        expect(ruleTargets(original, onlyRename)).toEqual([{field: "description", value: "Hardware Store", defaultOn: true}]);

        (renamed.postings[1] as {account: string}).account = "assets:checking";
        const both = cardChange(original, renamed);
        if (both === null) throw new Error("expected a change");
        expect(ruleTargets(original, both)).toEqual([
            {field: "account2", value: "assets:checking", defaultOn: true},
            {field: "description", value: "Hardware Store", defaultOn: false},
        ]);
    });

    it("builds the rule only with a pattern and something checked", () => {
        const targets = [
            {field: "account2", value: "expenses:home", defaultOn: true},
            {field: "description", value: "Hardware", defaultOn: false},
        ];
        expect(ruleSpec("description", " hardware.*store ", targets, new Set(["account2"]))).toEqual({
            matchers: [{field: "description", pattern: "hardware.*store"}],
            assignments: [{field: "account2", value: "expenses:home"}],
        });
        expect(ruleSpec(null, "hardware", targets, new Set(["account2", "description"]))?.assignments).toHaveLength(2);
        expect(ruleSpec("description", "  ", targets, new Set(["account2"]))).toBeNull();
        expect(ruleSpec("description", "hardware", targets, new Set())).toBeNull();
    });
});

describe("UNIT ruleBuilder — field references in a value", () => {
    it("finds what hledger reads as a field reference", () => {
        expect(fieldReferences("50%off")).toEqual(["%off"]);
        expect(fieldReferences("Store %1 and %description")).toEqual(["%1", "%description"]);
        expect(fieldReferences("%_x %-x %Ü")).toEqual(["%_x", "%-x", "%Ü"]);
        // `%%` is no escape: the second `%` still starts a reference.
        expect(fieldReferences("%%desc")).toEqual(["%desc"]);
    });

    it("leaves a % that hledger keeps literal", () => {
        expect(fieldReferences("50% off")).toEqual([]);
        expect(fieldReferences("100%")).toEqual([]);
        expect(fieldReferences("a %.x %(desc) %{desc}")).toEqual([]);
        expect(fieldReferences("Blue Bottle Coffee")).toEqual([]);
    });
});

describe("UNIT ruleBuilder — the live check's rows", () => {
    it("reads a recategorisation as the line taken away and the line added", () => {
        const before = "2024-01-19 HARDWARE STORE\n    assets:bank:checking  $-62.13\n    expenses:unknown\n";
        const after = "2024-01-19 HARDWARE STORE\n    assets:bank:checking  $-62.13\n    expenses:home:supplies\n";
        expect(changedLines(before, after)).toEqual({
            title: "2024-01-19 HARDWARE STORE",
            removed: ["expenses:unknown"],
            added: ["expenses:home:supplies"],
        });
    });

    it("ignores a line hledger only re-aligned (the engine's own output)", () => {
        const before = "2024-01-19 HARDWARE STORE\n    assets:bank:checking         $-62.13\n    expenses:unknown              $62.13";
        const after = "2024-01-19 HARDWARE STORE\n    assets:bank:checking           $-62.13\n    expenses:home:supplies          $62.13";
        expect(changedLines(before, after)).toEqual({
            title: "2024-01-19 HARDWARE STORE",
            removed: ["expenses:unknown $62.13"],
            added: ["expenses:home:supplies $62.13"],
        });
    });

    it("titles a rename with the new header", () => {
        expect(changedLines("2024-01-19 THD\n    a", "2024-01-19 Home Depot\n    a").title).toBe("2024-01-19 Home Depot");
    });

    it("names a file by its last path part", () => {
        expect(baseName("imports/2026/checking.csv")).toBe("checking.csv");
        expect(baseName("checking.csv.rules")).toBe("checking.csv.rules");
        expect(baseName("")).toBe("");
    });
});

describe("UNIT ruleBuilder — prefills (D25 import, D27 journal)", () => {
    const records = {
        aligned: true,
        header: ["Date", "Description", "Amount"],
        fields: ["date", "description", "amount"],
        rows: [["01/19/2024", "HARDWARE STORE #0042", "-62.13"]],
    };

    it("an import's starts on its description field and patterns on the card's own CSV cell", () => {
        const prefill = importPrefill(records);
        expect(prefill.fields).toEqual(["date", "description", "amount"]);
        expect(prefill.field).toBe("description");
        const staged = decodeCsvStagedTxn(stagedTxnWire(txnFixture(1, "Hardware Store"), {ordinal: 0, key: "hardware store"}), "card");
        expect(prefill.pattern(staged, "description")).toBe("hardware.*store");
        // A whole-record match is prefilled from the description's cell too.
        expect(prefill.pattern(staged, null)).toBe("hardware.*store");
        expect(importPrefill({...records, aligned: false}).field).toBeNull();
    });

    it("a journal card's patterns on its own description, scoped to the candidate's match field", () => {
        const prefill = descriptionPrefill({fields: ["date", "description", "amount"], matchField: "description"});
        expect(prefill.fields).toEqual(["date", "description", "amount"]);
        expect(prefill.field).toBe("description");
        const item = decodeQueueItem(queueItemWire(txnFixture(4, "BLUE BOTTLE COFFEE 4412"), {key: "blue bottle coffee"}), "card");
        expect(prefill.pattern(item, "description")).toBe("blue.*bottle.*coffee");
        const whole = descriptionPrefill({fields: null, matchField: null});
        expect(whole.fields).toEqual([]);
        expect(whole.field).toBeNull();
        expect(whole.pattern(item, null)).toBe("blue.*bottle.*coffee");
    });
});
