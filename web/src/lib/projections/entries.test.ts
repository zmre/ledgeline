import {describe, expect, it} from "vitest";
import {dec} from "#lib/domain/money.js";
import {classesOf} from "#lib/domain/accountClassFixtures.js";
import {
    addEntry,
    addStep,
    attachedDisplay,
    attachedText,
    copyEntry,
    duplicateAsAlternative,
    duplicateEntry,
    entriesOf,
    entryMatches,
    entryVisible,
    filterCounts,
    insertEntry,
    mergeSteps,
    moveEntry,
    moveEntryBy,
    removeEntry,
    setAttached,
    setComment,
    setDisabled,
    splitBlock,
    stepProblem,
    toggleDisabled,
} from "./entries";
import {blankEvent, emptyScenario, scenarioToWire} from "./scenarioModel";
import type {LayoutItem, Scenario, ScenarioLine} from "./types";

const DECLARED = classesOf(["income", "revenue"], ["expenses", "expense"], ["assets", "asset"]);

function line(overrides: Partial<ScenarioLine> = {}): ScenarioLine {
    return {
        id: "rent",
        group: "rule:0",
        role: "flow",
        account: "expenses:rent",
        amount: {commodity: "$", quantity: dec(420000, 2), precision: 2},
        period: {raw: "monthly", simple: "monthly", base: null, from: null, to: null},
        growth: null,
        opening: null,
        note: "rent",
        counter: "assets:checking",
        gains: null,
        source: "journal",
        comment: "",
        ruleComment: "",
        ...overrides,
    };
}

const item = (kind: LayoutItem["kind"], key: string, overrides: Partial<LayoutItem> = {}): LayoutItem => ({
    kind,
    key,
    origin: null,
    disabled: false,
    attached: "",
    raw: "",
    ...overrides,
});

/** A file as read: a section, a salary rule with an attached comment, a disabled alternative, a step pair, a one-off, an `=` rule, a note and file text. */
function file(): Scenario {
    const monthly = (from: string | null, to: string | null) => ({
        raw: `monthly${from === null ? "" : ` from ${from}`}${to === null ? "" : ` to ${to}`}`,
        simple: "monthly" as const,
        base: null,
        from,
        to,
    });
    return {
        ...emptyScenario(),
        lines: [
            line({
                id: "rule:0:0",
                group: "rule:0",
                account: "income:salary",
                amount: {commodity: "$", quantity: dec(-600000, 2), precision: 2},
                note: "salary",
            }),
            line({id: "rule:0:1", group: "rule:0", account: "expenses:rent", note: "salary"}),
            line({id: "off:0:0", group: "off:0", account: "income:acme", amount: {commodity: "$", quantity: dec(-4000000, 2), precision: 2}, note: "acme"}),
            line({id: "payroll", group: "rule:1", account: "expenses:payroll", period: monthly(null, "2027-02-01"), note: "payroll"}),
            line({id: "payroll", group: "rule:2", account: "expenses:payroll", period: monthly("2027-02-01", null), note: "payroll"}),
        ],
        events: [
            {
                ...blankEvent("rule:3", "2027-03-01", "$"),
                description: "Series A",
                postings: [{account: "assets:checking", amount: {commodity: "$", quantity: dec(2000000, 0), precision: 0}, comment: ""}],
            },
        ],
        autoRules: [{raw: "= expenses:rent\n    (x)  *0.1\n", query: "expenses:rent", unsupported: null, form: null, block: 0, key: "auto:0"}],
        comments: [
            {key: "comment:0", style: "section", raw: ";;;;;;\n;; Income\n", text: "Income"},
            {key: "comment:1", style: "note", raw: "; Assumes no rises.\n", text: "Assumes no rises."},
        ],
        fixed: [{key: "fixed:0", raw: "account equity:x  ; type: E\n"}],
        layout: [
            item("fixed", "fixed:0", {origin: 0}),
            item("comment", "comment:0", {origin: 1}),
            item("rule", "rule:0", {origin: 2, attached: "; Paid monthly.\n"}),
            item("rule", "off:0", {origin: 3, disabled: true}),
            item("rule", "rule:1", {origin: 4}),
            item("rule", "rule:2", {origin: 5}),
            item("comment", "comment:1", {origin: 6}),
            item("event", "rule:3", {origin: 7}),
            item("auto", "auto:0", {origin: 8}),
        ],
    };
}

const keys = (scenario: Scenario): string[] => scenario.layout.map((candidate) => candidate.key);

describe("UNIT entries — the file's pieces as a list (plan 31 C1)", () => {
    it("lists every piece in FILE order, each kind as itself", () => {
        const entries = entriesOf(file(), DECLARED);
        expect(entries.map((entry) => [entry.kind, entry.key])).toEqual([
            ["fixed", "fixed:0"],
            ["comment", "comment:0"],
            ["rule", "rule:0"],
            ["rule", "off:0"],
            ["rule", "rule:1"],
            ["rule", "rule:2"],
            ["comment", "comment:1"],
            ["event", "rule:3"],
            ["auto", "auto:0"],
        ]);
    });

    it("badges a rule for every section its postings fall in, in a fixed order", () => {
        const [, , salary, acme] = entriesOf(file(), DECLARED);
        expect(salary.badges).toEqual(["income", "expense"]);
        expect(acme.badges).toEqual(["income"]);
        expect(acme.disabled).toBe(true);
    });

    it("a single-date rule badges as a one-off, and an asset rule as an asset", () => {
        const dated = {...emptyScenario(), lines: [line({period: {raw: "2027-03-01", simple: null, base: null, from: null, to: null}})]};
        expect(entriesOf(dated, DECLARED)[0].badges).toEqual(["oneoff"]);
        const asset = {...emptyScenario(), lines: [line({role: "asset", account: "assets:house"})]};
        expect(entriesOf(asset, DECLARED)[0].badges).toEqual(["asset"]);
    });

    it("finds a step change's segments by their shared posting id, in span order", () => {
        const entries = entriesOf(file(), DECLARED);
        const first = entries.find((entry) => entry.key === "rule:1");
        const second = entries.find((entry) => entry.key === "rule:2");
        expect(first?.kind === "rule" && first.step).toEqual({index: 1, of: 2, groups: ["rule:1", "rule:2"]});
        expect(second?.kind === "rule" && second.step).toEqual({index: 2, of: 2, groups: ["rule:1", "rule:2"]});
        expect(entries.find((entry) => entry.key === "rule:0")).toMatchObject({step: null});
    });

    it("names every entry uniquely, adding its position only where two would share a name", () => {
        const names = entriesOf(file(), DECLARED).map((entry) => entry.name);
        expect(names).toContain("salary");
        expect(names).toContain("payroll (entry 5)");
        expect(names).toContain("payroll (entry 6)");
        expect(names).toContain("section Income");
        expect(names).toContain("automatic posting = expenses:rent");
        expect(new Set(names).size).toBe(names.length);
    });

    it("derives a layout for a scenario that has none, and skips an item naming nothing", () => {
        const plain = {...emptyScenario(), lines: [line()]};
        expect(entriesOf(plain, DECLARED).map((entry) => entry.key)).toEqual(["rule:0"]);
        const stray = {...plain, layout: [item("rule", "rule:0"), item("event", "gone")]};
        expect(entriesOf(stray, DECLARED).map((entry) => entry.key)).toEqual(["rule:0"]);
    });

    it("reads an attached comment as display text, and an edited one as edited", () => {
        expect(attachedDisplay("; one\n;two\n# three\n")).toBe("one\ntwo\nthree");
        expect(attachedText(item("rule", "k", {attached: "; Paid.\n"}))).toBe("Paid.");
        expect(attachedText(item("rule", "k", {attached: "; Paid.\n", attachedText: "Paid late."}))).toBe("Paid late.");
    });
});

describe("UNIT entries — filters (plan 31 C12)", () => {
    it("counts each chip, headings only under All and Notes", () => {
        expect(filterCounts(entriesOf(file(), DECLARED))).toEqual({all: 9, income: 2, expense: 3, asset: 0, oneoff: 1, auto: 1, off: 1, notes: 2});
    });

    it("keeps section headings visible under every filter, so the context survives", () => {
        const entries = entriesOf(file(), DECLARED);
        const shown = entries.filter((entry) => entryVisible(entry, "oneoff")).map((entry) => entry.key);
        expect(shown).toEqual(["comment:0", "rule:3"]);
        const section = entries[1];
        expect(entryMatches(section, "oneoff")).toBe(false);
    });
});

describe("UNIT entries — moving (any entry, anywhere the file can store it)", () => {
    it("moves an entry to an index, past file text and across kinds", () => {
        expect(keys(moveEntry(file(), "auto:0", 0))).toEqual(["auto:0", "fixed:0", "comment:0", "rule:0", "off:0", "rule:1", "rule:2", "comment:1", "rule:3"]);
        expect(keys(moveEntry(file(), "comment:0", 8)).at(-1)).toBe("comment:0");
    });

    it("never moves file text, and a no-op move is the same scenario", () => {
        const scenario = file();
        expect(moveEntry(scenario, "fixed:0", 3)).toBe(scenario);
        expect(moveEntry(scenario, "rule:0", 2)).toBe(scenario);
        expect(moveEntryBy(scenario, "fixed:0", 1)).toBe(scenario);
    });

    it("moves one place up and down, and the ends go nowhere", () => {
        expect(keys(moveEntryBy(file(), "rule:2", -1)).slice(4, 6)).toEqual(["rule:2", "rule:1"]);
        const last = file();
        expect(moveEntryBy(last, "auto:0", 1)).toBe(last);
    });

    it("steps move independently: hledger does not care about their order", () => {
        const moved = moveEntry(file(), "rule:2", 1);
        expect(keys(moved).indexOf("rule:2")).toBeLessThan(keys(moved).indexOf("rule:1"));
        expect(entriesOf(moved, DECLARED).find((entry) => entry.key === "rule:2")).toMatchObject({step: {index: 2, of: 2}});
    });
});

describe("UNIT entries — delete, turn off, insert", () => {
    it("deletes an entry and everything it holds; file text never", () => {
        const scenario = file();
        expect(removeEntry(scenario, "rule:0").lines.some((l) => l.group === "rule:0")).toBe(false);
        expect(keys(removeEntry(scenario, "comment:1"))).not.toContain("comment:1");
        expect(removeEntry(scenario, "comment:1").comments.map((c) => c.key)).toEqual(["comment:0"]);
        expect(removeEntry(scenario, "fixed:0")).toBe(scenario);
    });

    it("turns a rule, a one-off and an `=` rule off and back on; never a comment", () => {
        const off = toggleDisabled(file(), "rule:3");
        expect(off.layout.find((candidate) => candidate.key === "rule:3")?.disabled).toBe(true);
        expect(toggleDisabled(off, "rule:3").layout.find((candidate) => candidate.key === "rule:3")?.disabled).toBe(false);
        const scenario = file();
        expect(toggleDisabled(scenario, "comment:0")).toBe(scenario);
        expect(setDisabled(scenario, "off:0", true)).toBe(scenario);
    });

    it("turning off changes the run body and nothing else", () => {
        const before = scenarioToWire(file());
        const after = scenarioToWire(toggleDisabled(file(), "rule:0"));
        expect(after.lines).toEqual(before.lines);
        expect(after.layout?.find((candidate) => candidate.key === "rule:0")?.disabled).toBe(true);
    });

    it("inserts directly after a key, or at the end", () => {
        const payload = {kind: "comment" as const, comment: {key: "comment-9", style: "note" as const, raw: "", text: "x", edited: true}};
        expect(keys(insertEntry(file(), payload, "rule:0")).slice(2, 4)).toEqual(["rule:0", "comment-9"]);
        expect(keys(insertEntry(file(), payload, null)).at(-1)).toBe("comment-9");
    });

    it("an attached comment edited back to the file's own text is the file's again", () => {
        const edited = setAttached(file(), "rule:0", "Paid weekly.");
        expect(edited.layout[2].attachedText).toBe("Paid weekly.");
        expect("attachedText" in setAttached(edited, "rule:0", "Paid monthly.").layout[2]).toBe(false);
    });

    it("an edited comment is marked edited, so a save sends its text", () => {
        const edited = setComment(file(), "comment:1", "Assumes 2% rises.", "note");
        expect(edited.comments[1]).toMatchObject({text: "Assumes 2% rises.", edited: true});
        expect(scenarioToWire(edited).comments?.[1]).toEqual({key: "comment:1", style: "note", text: "Assumes 2% rises.", previous: "; Assumes no rises.\n"});
    });
});

describe("UNIT entries — duplicate, and duplicate as alternative (plan 31 C11)", () => {
    it("a copy takes fresh keys and fresh posting ids, so it is never a step of the original", () => {
        const copy = copyEntry(file(), "rule:1");
        expect(copy?.payload.kind).toBe("rule");
        if (copy?.payload.kind !== "rule") return;
        expect(copy.payload.lines[0].group).not.toBe("rule:1");
        expect(copy.payload.lines[0].id).not.toBe("payroll");
        expect(copy.item).toMatchObject({origin: null, disabled: false, raw: ""});
    });

    it("a copy of an estimated rule is AUTHORED, and carries the attached comment", () => {
        const scenario = file();
        scenario.lines[0] = {...scenario.lines[0], source: "unbudgeted"};
        const copy = copyEntry(scenario, "rule:0");
        expect(copy?.payload.kind === "rule" && copy.payload.lines.every((l) => l.source === "journal")).toBe(true);
        expect(copy?.item.attached).toBe("; Paid monthly.\n");
    });

    it("duplicate inserts the copy directly below", () => {
        const done = duplicateEntry(file(), "rule:3");
        expect(done).not.toBeNull();
        if (done === null) return;
        const at = keys(done.scenario).indexOf("rule:3");
        expect(keys(done.scenario)[at + 1]).toBe(done.key);
        expect(done.scenario.events).toHaveLength(2);
    });

    it("duplicate as alternative also turns the original off, in the same scenario", () => {
        const done = duplicateAsAlternative(file(), "auto:0");
        expect(done?.scenario.layout.find((candidate) => candidate.key === "auto:0")?.disabled).toBe(true);
        expect(done?.scenario.layout.find((candidate) => candidate.key === done.key)?.disabled).toBe(false);
        expect(done?.scenario.autoRules.find((rule) => rule.key === done.key)?.block).toBeNull();
    });

    it("comments duplicate too; file text does not", () => {
        expect(duplicateEntry(file(), "comment:1")?.scenario.comments).toHaveLength(3);
        expect(duplicateEntry(file(), "fixed:0")).toBeNull();
    });
});

describe("UNIT entries — add", () => {
    const context = {commodity: "$", date: "2027-06-01", assetPrefix: "assets:", cash: "assets:checking"};

    it("adds each kind blank, after the given entry", () => {
        const flow = addEntry(file(), "flow", "rule:0", context);
        expect(keys(flow.scenario)[3]).toBe(flow.key);
        const asset = addEntry(file(), "asset", null, context);
        expect(asset.scenario.lines.at(-1)).toMatchObject({role: "asset", account: "assets:"});
        const oneoff = addEntry(file(), "oneoff", null, context);
        expect(oneoff.scenario.events.at(-1)?.date).toBe("2027-06-01");
        const auto = addEntry(file(), "auto", null, context);
        expect(auto.scenario.autoRules.at(-1)?.form?.counter).toBe("assets:checking");
        const section = addEntry(file(), "section", null, context);
        expect(section.scenario.comments.at(-1)).toMatchObject({style: "section", raw: "", edited: true});
    });
});

describe("UNIT entries — step changes (plan 31 C2)", () => {
    it("splits the WHOLE block at a date: every posting bounded, a new block from it", () => {
        const split = splitBlock(file(), "rule:0", "2027-06-01");
        expect(split?.before.map((l) => l.period.raw)).toEqual(["monthly to 2027-06-01", "monthly to 2027-06-01"]);
        expect(split?.after.map((l) => [l.id, l.period.raw])).toEqual([
            ["rule:0:0", "monthly from 2027-06-01"],
            ["rule:0:1", "monthly from 2027-06-01"],
        ]);
        expect(new Set(split?.after.map((l) => l.group)).size).toBe(1);
        expect(split?.after[0].group).not.toBe("rule:0");
    });

    it("adds the step directly below, and the pair reads as a step", () => {
        const done = addStep(file(), "rule:0", "2027-06-01");
        expect(done).not.toBeNull();
        if (done === null) return;
        expect(keys(done.scenario)[3]).toBe(done.key);
        expect(entriesOf(done.scenario, DECLARED).find((entry) => entry.key === done.key)).toMatchObject({step: {index: 2, of: 2}});
    });

    it("refuses an asset rule, a date outside the span, and a period it cannot rebuild", () => {
        expect(stepProblem([line({role: "asset"})], "2027-01-01")).toMatch(/asset/);
        expect(
            stepProblem([line({period: {raw: "monthly to 2027-01-01", simple: "monthly", base: null, from: null, to: "2027-01-01"}})], "2027-06-01")
        ).toMatch(/inside/);
        expect(stepProblem([line({period: {raw: "every weekday", simple: null, base: null, from: null, to: null}})], "2027-06-01")).toMatch(/split/);
        expect(addStep(file(), "rule:3", "2027-06-01")).toBeNull();
    });

    it("merging gives back one rule spanning both, in the earlier segment's place", () => {
        const merged = mergeSteps(file(), "rule:2");
        expect(merged?.key).toBe("rule:1");
        expect(keys(merged?.scenario ?? file())).not.toContain("rule:2");
        expect(merged?.scenario.lines.find((l) => l.group === "rule:1")?.period.raw).toBe("monthly");
        expect(mergeSteps(file(), "rule:0")).toBeNull();
    });
});
