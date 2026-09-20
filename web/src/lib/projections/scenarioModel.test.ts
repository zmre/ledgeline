import {describe, expect, it} from "vitest";
import {dec} from "$lib/domain/money";
import type {AccountType} from "$lib/domain/accountTypes";
import {
    amountFor,
    blankLine,
    cloneScenario,
    duplicateRow,
    emptyScenario,
    freshId,
    isRevenueAccount,
    logicalRows,
    magnitudeOf,
    mergeStep,
    percentFromRate,
    periodRaw,
    projectable,
    rateFromPercent,
    removeSegment,
    runBody,
    scenarioToWire,
    sectionOfLine,
    sectionPrefix,
    signedQuantity,
    splitStep,
    takenIds,
    trimDec,
    withAccount,
    withBounds,
    withInterval,
    withMagnitude,
} from "./scenarioModel";
import type {Scenario, ScenarioLine} from "./types";

/**
 * A journal that declares its own types rather than relying on hledger's name
 * regexes — [[account-type-not-name]]: `wages` is revenue here because it is
 * DECLARED revenue, which is the case name matching gets wrong.
 */
const DECLARED: ReadonlyMap<string, AccountType> = new Map<string, AccountType>([
    ["income", "revenue"],
    ["wages", "revenue"],
    ["expenses", "expense"],
    ["assets", "asset"],
]);

function line(overrides: Partial<ScenarioLine> = {}): ScenarioLine {
    return {
        id: "rent",
        group: "rule:0",
        account: "expenses:rent",
        amount: {commodity: "$", quantity: dec(420000, 2), precision: 2},
        period: {raw: "monthly", simple: "monthly", from: null, to: null},
        growth: null,
        note: "",
        source: "journal",
        ...overrides,
    };
}

function scenario(lines: ScenarioLine[]): Scenario {
    return {...emptyScenario(), lines};
}

// ---------------------------------------------------------------------------

describe("UNIT scenarioModel — the sign flip", () => {
    it("a revenue line carries the journal's NEGATIVE amount; everything else stays positive", () => {
        expect(signedQuantity(dec(500000, 2), "income:salary", DECLARED)).toEqual(dec(-500000, 2));
        expect(signedQuantity(dec(420000, 2), "expenses:rent", DECLARED)).toEqual(dec(420000, 2));
        expect(signedQuantity(dec(100000, 2), "assets:savings", DECLARED)).toEqual(dec(100000, 2));
    });

    it("decides by DECLARED type, not by the name", () => {
        // Nothing about "wages" matches hledger's revenue regex; the declaration
        // is what makes it revenue.
        expect(isRevenueAccount("wages:contract", DECLARED)).toBe(true);
        expect(isRevenueAccount("wages:contract", new Map())).toBe(false);
    });

    it("falls back to name inference when nothing is declared", () => {
        expect(isRevenueAccount("income:salary", new Map())).toBe(true);
        expect(isRevenueAccount("expenses:rent", new Map())).toBe(false);
    });

    it("a magnitude the user typed is idempotent under the flip — a minus sign is not a second negation", () => {
        expect(signedQuantity(dec(-500000, 2), "income:salary", DECLARED)).toEqual(dec(-500000, 2));
        expect(signedQuantity(dec(-420000, 2), "expenses:rent", DECLARED)).toEqual(dec(420000, 2));
    });

    it("the table shows a magnitude whichever way the wire signed it", () => {
        expect(magnitudeOf({commodity: "$", quantity: dec(-518833, 2), precision: 2})).toEqual(dec(518833, 2));
        expect(magnitudeOf({commodity: "$", quantity: dec(187500, 2), precision: 2})).toEqual(dec(187500, 2));
    });

    it("moving a row from an expense account to a revenue one RE-SIGNS what it already held", () => {
        const moved = withAccount(line(), "income:consulting", DECLARED);
        expect(moved.amount.quantity).toEqual(dec(-420000, 2));
        // …and back again.
        expect(withAccount(moved, "expenses:rent", DECLARED).amount.quantity).toEqual(dec(420000, 2));
    });

    it("editing an amount signs it for the account the row already has", () => {
        const income = withMagnitude(line({account: "income:salary"}), dec(900000, 2), DECLARED);
        expect(income.amount.quantity).toEqual(dec(-900000, 2));
    });

    it("display precision is RAISED to fit a typed amount and never lowered", () => {
        const seeded = {commodity: "$", quantity: dec(187500, 2), precision: 2};
        // Retyping "1875" must not quietly move this line's growth onto whole dollars.
        expect(amountFor(seeded, dec(1875, 0), "expenses:housing", DECLARED).precision).toBe(2);
        // A third decimal place has to be representable or the growth walk rounds it away.
        expect(amountFor(seeded, dec(1875125, 3), "expenses:housing", DECLARED).precision).toBe(3);
    });
});

describe("UNIT scenarioModel — sections", () => {
    it("revenue goes to Income; everything else, including an asset transfer, to Expenses", () => {
        expect(sectionOfLine(line({account: "income:salary"}), DECLARED)).toBe("income");
        expect(sectionOfLine(line({account: "expenses:rent"}), DECLARED)).toBe("expense");
        expect(sectionOfLine(line({account: "assets:savings"}), DECLARED)).toBe("expense");
    });

    it("a single-date `~` rule is shown among the one-offs, though the engine keeps it a LINE", () => {
        const dated = line({period: {raw: "2027-03-01", simple: null, from: null, to: null}});
        expect(sectionOfLine(dated, DECLARED)).toBe("oneoff");
    });

    it("a bounded-but-irregular period is NOT a one-off — its raw is not a date", () => {
        const weird = line({period: {raw: "every weekday", simple: null, from: null, to: null}});
        expect(sectionOfLine(weird, DECLARED)).toBe("expense");
    });

    it("groups segments into one logical row, earliest span first, keeping the rows' own order", () => {
        const lines = [
            line({id: "b", group: "g-b", account: "expenses:software"}),
            line({id: "a", group: "g-a2", period: withBounds({raw: "", simple: "monthly", from: null, to: null}, "2027-04-01", null)}),
            line({id: "a", group: "g-a1", period: withBounds({raw: "", simple: "monthly", from: null, to: null}, null, "2027-04-01")}),
        ];
        const rows = logicalRows(lines, DECLARED, "expense");
        expect(rows.map((r) => r.id)).toEqual(["b", "a"]);
        expect(rows[1].segments.map((s) => s.group)).toEqual(["g-a1", "g-a2"]);
    });

    it("a new row opens on a prefix of the right TYPE, learned from the journal", () => {
        const names = ["revenues:salary", "revenues:consulting", "spending:rent"];
        const declared = new Map<string, AccountType>([
            ["revenues", "revenue"],
            ["spending", "expense"],
        ]);
        expect(sectionPrefix("income", names, declared)).toBe("revenues:");
        expect(sectionPrefix("expense", names, declared)).toBe("spending:");
    });

    it("falls back to the hledger defaults when the journal has nothing of that type yet", () => {
        expect(sectionPrefix("income", [], new Map())).toBe("income:");
        expect(sectionPrefix("expense", [], new Map())).toBe("expenses:");
    });
});

describe("UNIT scenarioModel — periods", () => {
    it("spells a period the way a journal would, with `to` exclusive", () => {
        expect(periodRaw("monthly", null, null)).toBe("monthly");
        expect(periodRaw("monthly", "2027-04-01", null)).toBe("monthly from 2027-04-01");
        expect(periodRaw("monthly", null, "2027-04-01")).toBe("monthly to 2027-04-01");
        expect(periodRaw("quarterly", "2027-01-01", "2028-01-01")).toBe("quarterly from 2027-01-01 to 2028-01-01");
    });

    it("changing the interval regenerates `raw`, which is the only field the engine reads", () => {
        const bounded = withBounds({raw: "monthly", simple: "monthly", from: null, to: null}, "2027-04-01", null);
        expect(withInterval(bounded, "yearly")).toEqual({raw: "yearly from 2027-04-01", simple: "yearly", from: "2027-04-01", to: null});
    });

    it("leaves a period it cannot rebuild exactly as it arrived", () => {
        const weird = {raw: "every weekday", simple: null, from: null, to: null} as const;
        expect(withBounds(weird, "2027-01-01", null)).toBe(weird);
    });
});

describe("UNIT scenarioModel — step split and merge", () => {
    const base = scenario([line({id: "payroll", group: "rule:0", account: "expenses:payroll"})]);

    it("splitting makes TWO bounded segments of ONE logical row, meeting on the date", () => {
        const split = splitStep(base.lines, "payroll", "2027-04-01", takenIds(base));
        expect(split).toHaveLength(2);
        expect(split.map((s) => s.id)).toEqual(["payroll", "payroll"]);
        expect(split[0].period).toEqual({raw: "monthly to 2027-04-01", simple: "monthly", from: null, to: "2027-04-01"});
        expect(split[1].period).toEqual({raw: "monthly from 2027-04-01", simple: "monthly", from: "2027-04-01", to: null});
    });

    it("the two halves are two `~` RULES, so they take distinct groups", () => {
        const split = splitStep(base.lines, "payroll", "2027-04-01", takenIds(base));
        // The engine's cash and net-worth guards are scoped per group; two rules
        // sharing one would have their derived legs counted against each other.
        expect(split[0].group).not.toBe(split[1].group);
    });

    it("the later half starts from the SAME base, which is what the user then edits", () => {
        const split = splitStep(base.lines, "payroll", "2027-04-01", takenIds(base));
        expect(split[1].amount).toEqual(split[0].amount);
        expect(split[1].account).toBe("expenses:payroll");
    });

    it("merging gives back exactly the line that was split", () => {
        const split = splitStep(base.lines, "payroll", "2027-04-01", takenIds(base));
        const merged = mergeStep(split, "payroll");
        expect(merged).toHaveLength(1);
        expect(merged[0].period).toEqual(base.lines[0].period);
        expect(merged[0].amount).toEqual(base.lines[0].amount);
    });

    it("merging spans the UNION, so a bounded row keeps its own bounds", () => {
        const bounded = scenario([line({id: "payroll", period: withBounds({raw: "", simple: "monthly", from: null, to: null}, "2027-01-01", "2029-01-01")})]);
        const split = splitStep(bounded.lines, "payroll", "2028-01-01", takenIds(bounded));
        expect(mergeStep(split, "payroll")[0].period).toEqual({
            raw: "monthly from 2027-01-01 to 2029-01-01",
            simple: "monthly",
            from: "2027-01-01",
            to: "2029-01-01",
        });
    });

    it("merging keeps the row where it was, rather than moving it to the end", () => {
        const many = scenario([line({id: "a", group: "g-a"}), line({id: "payroll", group: "g-p"}), line({id: "z", group: "g-z"})]);
        const split = splitStep(many.lines, "payroll", "2027-04-01", takenIds(many));
        expect(mergeStep(split, "payroll").map((l) => l.id)).toEqual(["a", "payroll", "z"]);
    });

    it("splitting refuses a date outside the span, a row that is not there, and a period it cannot rebuild", () => {
        const bounded = scenario([line({id: "payroll", period: withBounds({raw: "", simple: "monthly", from: null, to: null}, "2027-01-01", "2028-01-01")})]);
        expect(splitStep(bounded.lines, "payroll", "2026-01-01", new Set())).toHaveLength(1);
        expect(splitStep(bounded.lines, "nope", "2027-06-01", new Set())).toHaveLength(1);
        expect(splitStep(bounded.lines, "payroll", "not-a-date", new Set())).toHaveLength(1);

        const weird = scenario([line({id: "payroll", period: {raw: "every weekday", simple: null, from: null, to: null}})]);
        expect(splitStep(weird.lines, "payroll", "2027-06-01", new Set())).toHaveLength(1);
    });

    it("merging a row that is not a step leaves it alone", () => {
        expect(mergeStep(base.lines, "payroll")).toEqual(base.lines);
    });

    it("deleting ONE segment of a step leaves the other, still a whole row", () => {
        const split = splitStep(base.lines, "payroll", "2027-04-01", takenIds(base));
        const left = removeSegment(split, split[1].group);
        expect(left).toHaveLength(1);
        expect(left[0].period.to).toBe("2027-04-01");
        expect(logicalRows(left, DECLARED, "expense")).toHaveLength(1);
    });

    it("duplicating copies every segment under one NEW id, immediately after the original", () => {
        const split = splitStep(base.lines, "payroll", "2027-04-01", takenIds(base));
        const copied = duplicateRow(split, "payroll", new Set(split.flatMap((s) => [s.id, s.group])));
        expect(copied).toHaveLength(4);
        const ids = new Set(copied.slice(2).map((s) => s.id));
        expect(ids.size).toBe(1);
        expect([...ids][0]).not.toBe("payroll");
        // Distinct groups, or the copy's derived cash leg would be guarded by the original's postings.
        expect(new Set(copied.map((s) => s.group)).size).toBe(4);
    });

    it("a duplicate of an estimated row is AUTHORED — it is no longer an average of anything", () => {
        const seeded = scenario([line({id: "gap", group: "gap", source: "unbudgeted"})]);
        const copied = duplicateRow(seeded.lines, "gap", takenIds(seeded));
        expect(copied[1].source).toBe("journal");
    });

    it("fresh ids avoid everything in use, ids and groups alike", () => {
        expect(freshId("line", new Set(["line-1", "line-2"]))).toBe("line-3");
        const s = scenario([line({id: "line-1", group: "line-2"})]);
        expect(takenIds(s)).toEqual(new Set(["line-1", "line-2"]));
    });
});

describe("UNIT scenarioModel — growth rates", () => {
    it("the wire carries a FRACTION and the UI shows a percent", () => {
        expect(percentFromRate(dec(3, 2))).toEqual(dec(3, 0)); // 0.03 -> 3
        expect(percentFromRate(dec(25, 3))).toEqual(dec(25, 1)); // 0.025 -> 2.5
        expect(percentFromRate(dec(1, 0))).toEqual(dec(100, 0)); // 1 -> 100
        expect(percentFromRate(dec(-1, 2))).toEqual(dec(-1, 0)); // -0.01 -> -1
    });

    it("a typed percent becomes the fraction the wire carries", () => {
        expect(rateFromPercent(dec(3, 0))).toEqual(dec(3, 2)); // 3 % -> 0.03
        expect(rateFromPercent(dec(25, 1))).toEqual(dec(25, 3)); // 2.5 % -> 0.025
    });

    it("round-trips, so showing a rate and saving it back does not drift", () => {
        for (const rate of [dec(3, 2), dec(25, 3), dec(-1, 2), dec(5, 2)]) {
            expect(rateFromPercent(percentFromRate(rate))).toEqual(trimDec(rate));
        }
    });

    it("trims trailing zeros, so 3.00 % reads as 3 %", () => {
        expect(trimDec(dec(300, 2))).toEqual(dec(3, 0));
        expect(trimDec(dec(250, 2))).toEqual(dec(25, 1));
        expect(trimDec(dec(0, 4))).toEqual(dec(0, 0));
    });
});

describe("UNIT scenarioModel — putting a scenario on the wire", () => {
    it("hands back only `raw` for the period: the derived fields are echoes", () => {
        const wire = scenarioToWire(scenario([line({period: {raw: "monthly from 2027-04-01", simple: "monthly", from: "2027-04-01", to: null}})]));
        expect(wire.lines[0].period).toEqual({raw: "monthly from 2027-04-01"});
    });

    it("sends the journal's sign, the display precision, and an explicit null for an absent growth", () => {
        const wire = scenarioToWire(scenario([line({account: "income:salary", amount: {commodity: "$", quantity: dec(-518833, 2), precision: 2}})]));
        expect(wire.lines[0].amount).toEqual({commodity: "$", quantity: {mantissa: "-518833", places: 2}, precision: 2});
        expect(wire.lines[0].growth).toBeNull();
        expect(wire.created).toBeNull();
    });

    it("sends a growth rate as the fraction it is", () => {
        const wire = scenarioToWire(scenario([line({growth: {rate: dec(3, 2), unit: "year"}})]));
        expect(wire.lines[0].growth).toEqual({rate: {mantissa: "3", places: 2}, unit: "year"});
    });

    it("drops half-typed rows rather than making the whole request a 400", () => {
        const half = {
            ...emptyScenario(),
            lines: [line(), blankLine("line-1", "$", 2)],
            events: [
                {
                    id: "e1",
                    date: "2027-03-01",
                    description: "raise",
                    postings: [{account: "assets:cash", amount: {commodity: "$", quantity: dec(200, 0), precision: 0}}],
                },
                {
                    id: "e2",
                    date: "2027-03-01",
                    description: "half typed",
                    postings: [{account: "", amount: {commodity: "$", quantity: dec(0, 0), precision: 0}}],
                },
                {id: "e3", date: "", description: "no date", postings: [{account: "assets:cash", amount: {commodity: "$", quantity: dec(1, 0), precision: 0}}]},
            ],
        };
        const kept = projectable(half);
        expect(kept.lines.map((l) => l.account)).toEqual(["expenses:rent"]);
        expect(kept.events.map((e) => e.id)).toEqual(["e1"]);
    });

    it("sends no `asOf`: the engine's today is the clock every other report on the page is read against", () => {
        const body = runBody(scenario([line()]), {interval: "monthly", count: 24, depth: 2});
        expect(body.asOf).toBeUndefined();
        expect(body).toMatchObject({interval: "monthly", count: 24, depth: 2});
    });
});

describe("UNIT scenarioModel — cloning", () => {
    it("a clone shares nothing with the frozen payload a decoder returned", () => {
        const decoded: Scenario = Object.freeze({
            name: "",
            created: null,
            updated: null,
            lines: Object.freeze([Object.freeze({...line(), amount: Object.freeze({commodity: "$", quantity: Object.freeze(dec(420000, 2)), precision: 2})})]),
            events: Object.freeze([
                Object.freeze({
                    id: "e1",
                    date: "2027-03-01",
                    description: "raise",
                    postings: Object.freeze([
                        Object.freeze({account: "assets:cash", amount: Object.freeze({commodity: "$", quantity: Object.freeze(dec(200, 0)), precision: 0})}),
                    ]),
                }),
            ]),
        }) as Scenario;

        const clone = cloneScenario(decoded);
        expect(clone).toEqual(decoded);
        // Every level is writable, which is what the editor's `$state` needs.
        for (const target of [clone, clone.lines[0], clone.lines[0].amount, clone.lines[0].amount.quantity, clone.events[0], clone.events[0].postings[0]]) {
            expect(Object.isFrozen(target)).toBe(false);
        }
        clone.lines[0].account = "expenses:other";
        expect(decoded.lines[0].account).toBe("expenses:rent");
    });
});
