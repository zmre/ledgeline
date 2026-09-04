import {describe, expect, it} from "vitest";
import {decodeRulesDoc, decodeRulesDraft} from "$lib/api/nativeDecode";
import {checkRulesId, createBlocker, createSaveRequest, defaultRulesId, draftForm, draftLines, NEW_FILE_REVISION} from "./createModel";
import {fieldNames, toForm, withFieldNames, withSetting} from "./model";
import type {RulesDocument} from "./types";

// The wire body a real `POST /api/rules-create` answers with, for the plain
// three-column export in `crates/ledgeline-server/tests/rules_create_endpoints.rs`.
// Written as a literal rather than read from a fixture because it is the
// CONTRACT this file is about: if the engine's shape moves, this has to be the
// thing that fails.
const DRAFT_WIRE = {
    doc: {
        id: "import/2026/bank.csv.rules",
        label: "bank",
        // The create handle. A real revision is always `LEN-HASH` in hex, so
        // this can never collide with one.
        revision: "",
        editable: true,
        newline: "lf",
        settings: {
            skip: {value: 1, itemId: 0},
            dateFormat: {value: "%m/%d/%Y", itemId: 1},
            fields: {names: ["date", "description", "amount"], itemId: 2},
            account1: {value: "", itemId: 3},
            account2: {value: "expenses:unknown", itemId: 4},
        },
        items: [
            {id: 0, line: 1, lines: 1, kind: "directive", name: "skip", value: "1"},
            {id: 1, line: 2, lines: 1, kind: "directive", name: "date-format", value: "%m/%d/%Y"},
            {id: 2, line: 3, lines: 1, kind: "fields", names: ["date", "description", "amount"]},
            {id: 3, line: 4, lines: 1, kind: "assignment", field: "account1", value: ""},
            {id: 4, line: 5, lines: 1, kind: "assignment", field: "account2", value: "expenses:unknown"},
        ],
        warnings: [],
    },
    preview: {
        available: true,
        separator: ",",
        header: ["Posted Date", "Description", "Amount"],
        rows: [["01/02/2026", "COFFEE ROASTERS", "-4.50"]],
        columns: 3,
        truncated: false,
    },
    columns: [
        {index: 0, field: "date", confidence: 0.95},
        {index: 1, field: "description", confidence: 1},
        {index: 2, field: "amount", confidence: 1},
    ],
    warnings: ["These dates could be read more than one way."],
};

function draftDoc(): RulesDocument {
    return decodeRulesDoc(DRAFT_WIRE.doc);
}

describe("defaultRulesId", () => {
    it("names the rules file after the CSV it will be read with", () => {
        // Not a convenience: hledger FINDS a rules file by name, so `bank.csv`
        // is read through `bank.csv.rules` beside it. A different name is a
        // rules file hledger will not use.
        expect(defaultRulesId("import/2026/bank.csv")).toBe("import/2026/bank.csv.rules");
        expect(defaultRulesId("bank.csv")).toBe("bank.csv.rules");
    });

    it("only replaces the final .csv, so a dotted name keeps its stem", () => {
        expect(defaultRulesId("bank.export.csv")).toBe("bank.export.csv.rules");
        expect(defaultRulesId("statement.CSV")).toBe("statement.csv.rules");
    });

    it("still produces a usable id when the destination is empty or odd", () => {
        expect(checkRulesId(defaultRulesId(""))).toBeNull();
        expect(checkRulesId(defaultRulesId("   "))).toBeNull();
        expect(checkRulesId(defaultRulesId("/leading/slash.csv"))).toBeNull();
        expect(checkRulesId(defaultRulesId("no-extension"))).toBeNull();
    });
});

describe("checkRulesId", () => {
    it("accepts what a scan could have produced", () => {
        for (const id of ["bank.csv.rules", "import/2026/bank.csv.rules", "checking.rules"]) {
            expect(checkRulesId(id), id).toBeNull();
        }
    });

    it("names the mistake a person actually makes in a filename field", () => {
        expect(checkRulesId("")).toMatch(/name/i);
        expect(checkRulesId("bank.csv")).toMatch(/\.rules/);
        expect(checkRulesId("/etc/bank.csv.rules")).toMatch(/relative/i);
        expect(checkRulesId("../bank.csv.rules")).toMatch(/\.\./);
        expect(checkRulesId("a//b.csv.rules")).toMatch(/empty folder/i);
        expect(checkRulesId("C:/bank.csv.rules")).toMatch(/cannot contain/i);
        // A hidden name would be written and then never listed, because the
        // scan skips dot entries — so the file would vanish from the UI the
        // moment it was created.
        expect(checkRulesId(".hidden/bank.csv.rules")).toMatch(/hidden/i);
    });
});

describe("createSaveRequest", () => {
    it("sends every item as a NEW one, against the no-file-yet revision", () => {
        // The structural difference from an edit. `{kind:"keep", id}` tells the
        // engine to re-emit that item's original BYTES — and a file that does
        // not exist has none, so a create carrying one could not mean anything.
        const form = draftForm(draftDoc());
        const body = createSaveRequest(form);

        expect(body.revision).toBe(NEW_FILE_REVISION);
        expect(body.delete).toEqual([]);
        expect(body.items).toHaveLength(5);
        for (const item of body.items) {
            expect(item.kind).not.toBe("keep");
            expect(item.id).toBeUndefined();
        }
        expect(body.items[0]).toEqual({kind: "directive", name: "skip", value: "1"});
        expect(body.items[2]).toEqual({kind: "fields", names: ["date", "description", "amount"]});
    });

    it("carries the account the user typed", () => {
        const form = draftForm(draftDoc());
        const edited = {...form, items: withSetting(form.items, "account1", "assets:bank:checking")};
        const body = createSaveRequest(edited);
        expect(body.items).toContainEqual({kind: "assignment", field: "account1", value: "assets:bank:checking"});
    });

    it("carries a corrected column mapping", () => {
        // The failure mode the whole screen exists for: a mis-detected column.
        // Correcting it has to reach the engine, and it does so as the ordinary
        // `fields` item rather than as anything create-specific.
        const form = draftForm(draftDoc());
        const items = [...form.items];
        const at = items.findIndex((item) => item.kind === "fields");
        items[at] = {kind: "fields", id: null, names: ["date", "description", "amount-out"]};
        const body = createSaveRequest({...form, items});
        expect(body.items).toContainEqual({kind: "fields", names: ["date", "description", "amount-out"]});
    });

    it("refuses to silently drop an item it cannot express", () => {
        // A drafted document never contains one — the engine asserts that in
        // `every_drafted_item_can_be_written_back` — but omitting it quietly
        // would write a file missing a line the user was shown, so this throws
        // rather than filtering.
        const doc = decodeRulesDoc({
            ...DRAFT_WIRE.doc,
            items: [...DRAFT_WIRE.doc.items, {id: 5, line: 6, lines: 1, kind: "trivia", text: "# hand-written\n", truncated: false}],
        });
        expect(() => createSaveRequest(toForm(doc))).toThrow(/trivia/);
    });
});

describe("createBlocker", () => {
    it("asks for the one thing no CSV can supply", () => {
        const form = draftForm(draftDoc());
        expect(createBlocker("bank.csv.rules", form)).toMatch(/which account/i);
    });

    it("clears once the account is set", () => {
        const form = draftForm(draftDoc());
        const ready = {...form, items: withSetting(form.items, "account1", "assets:bank:checking")};
        expect(createBlocker("bank.csv.rules", ready)).toBeNull();
    });

    it("reports a bad name before it reports a missing account", () => {
        // The name is the field above, and fixing the account first would leave
        // the button disabled for a reason that had moved.
        const form = draftForm(draftDoc());
        expect(createBlocker("bank.csv", form)).toMatch(/\.rules/);
    });

    it("is satisfied by a column mapped to account1, with no top-level default set", () => {
        // The multi-account case: a QuickBooks-style export names a different
        // account per row rather than one for the whole statement, and the
        // idiomatic hledger fix is to map a column straight onto `account1` in
        // `fields` — a column carries account1's value per row exactly as
        // `AccountsPanel`'s text field carries one fixed value for the whole
        // file. Both are "account1 is covered"; only one used to be checked.
        const form = draftForm(draftDoc());
        const names = fieldNames(form.items) ?? [];
        const mapped = {...form, items: withFieldNames(form.items, [names[0]!, "account1", names[2]!])};
        expect(createBlocker("bank.csv.rules", mapped)).toBeNull();
    });

    it("still defers to the shared form validation", () => {
        const form = draftForm(draftDoc());
        const items = withSetting(form.items, "account1", "assets:bank:checking");
        const withBadFields = items.map((item) => (item.kind === "fields" ? {...item, names: ["date", "not a name"]} : item));
        expect(createBlocker("bank.csv.rules", {...form, items: withBadFields})).toMatch(/field name/i);
    });
});

describe("draftLines", () => {
    it("puts a currency the user adds after the settings already there", () => {
        // Pins the LINE ORDER of a created file, which is not obvious and which
        // the e2e spec asserts byte-for-byte: `withSetting` inserts after the
        // last setting in the document, and `account2` is the last one a draft
        // carries — so a currency typed into the panel lands at the end rather
        // than beside the other directives. Harmless to hledger (only `skip` is
        // positional), and worth pinning so the e2e's expected bytes are a
        // measurement rather than a guess.
        const form = draftForm(draftDoc());
        const withCurrency = withSetting(form.items, "currency", "$");
        expect(draftLines(withCurrency)).toEqual([
            "skip 1",
            "date-format %m/%d/%Y",
            "fields date, description, amount",
            "account1",
            "account2 expenses:unknown",
            "currency $",
        ]);
    });

    it("renders the file the way it will read", () => {
        const form = draftForm(draftDoc());
        expect(draftLines(form.items)).toEqual([
            "skip 1",
            "date-format %m/%d/%Y",
            "fields date, description, amount",
            // A bare field name is how hledger spells "assign the empty
            // string", and it is what an unfilled account1 will write.
            "account1",
            "account2 expenses:unknown",
        ]);
    });
});

describe("decodeRulesDraft", () => {
    it("decodes the engine's own shape", () => {
        const draft = decodeRulesDraft(DRAFT_WIRE);
        expect(draft.doc.id).toBe("import/2026/bank.csv.rules");
        expect(draft.doc.revision).toBe("");
        expect(draft.preview.header).toEqual(["Posted Date", "Description", "Amount"]);
        expect(draft.columns).toEqual([
            {index: 0, field: "date", confidence: 0.95},
            {index: 1, field: "description", confidence: 1},
            {index: 2, field: "amount", confidence: 1},
        ]);
        expect(draft.warnings).toHaveLength(1);
    });

    it("reads an absent field as the engine declining to map the column", () => {
        // Not a missing key: the engine omits `field` precisely when it will not
        // claim a column, and a decoder that threw would refuse the honest
        // answer.
        const draft = decodeRulesDraft({
            ...DRAFT_WIRE,
            columns: [{index: 0, confidence: 0}],
        });
        expect(draft.columns[0]?.field).toBeNull();
    });
});
