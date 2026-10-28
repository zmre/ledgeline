import {describe, expect, it} from "vitest";
import {dec} from "#lib/domain/money.js";
import {NO_ACCOUNT_CLASSES, type AccountClasses} from "#lib/domain/accountClass.js";
import {classesOf} from "#lib/domain/accountClassFixtures.js";
import {
    blankAutoRule,
    ruleSample,
    accountOrNull,
    blankAssetLine,
    blankLine,
    boundsLockedReason,
    cloneScenario,
    duplicateRow,
    emptyScenario,
    freshId,
    gatherRules,
    isRevenueAccount,
    logicalRows,
    magnitudeOf,
    MAX_NOTE_BYTES,
    mergeStep,
    moveEvent,
    moveEventTo,
    moveRow,
    moveRowTo,
    noteIsStale,
    noteProblem,
    percentFromRate,
    periodRaw,
    projectable,
    projectionId,
    rateFromPercent,
    removeSegment,
    resolvedSection,
    rowNames,
    ruleSize,
    runBody,
    scenarioToWire,
    sectionOfLine,
    sectionPrefix,
    slugFilename,
    splitStep,
    takenIds,
    trimDec,
    withAccount,
    withCounter,
    withGains,
    withMagnitude,
    withBounds,
    withInterval,
    withNote,
} from "./scenarioModel";
import type {Scenario, ScenarioLine} from "./types";

/**
 * A journal that declares its own types rather than relying on hledger's name
 * regexes — [[account-type-not-name]]: `wages` is revenue here because it is
 * DECLARED revenue, which is the case name matching gets wrong.
 */
const DECLARED: AccountClasses = classesOf(["income", "revenue"], ["wages", "revenue"], ["expenses", "expense"], ["assets", "asset"]);

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
        note: "",
        counter: null,
        gains: null,
        source: "journal",
        ...overrides,
    };
}

function scenario(lines: ScenarioLine[]): Scenario {
    return {...emptyScenario(), lines};
}

// ---------------------------------------------------------------------------

describe("UNIT scenarioModel — magnitudes in, the engine signs", () => {
    // The sign rule itself — revenue negative, by the account's resolved TYPE —
    // is the engine's (`account_types::signed_entry`, locked to the TypeScript it
    // replaced by `scenario_signs_golden.rs`). This side only keeps the number a
    // user typed and says that it is one.

    it("the table shows a magnitude whichever way the wire signed it", () => {
        expect(magnitudeOf(line({amount: {commodity: "$", quantity: dec(-518833, 2), precision: 2}}))).toEqual(dec(518833, 2));
        expect(magnitudeOf(line({amount: {commodity: "$", quantity: dec(187500, 2), precision: 2}}))).toEqual(dec(187500, 2));
    });

    it("a typed amount becomes the line's ENTRY, a magnitude, and the shown figure", () => {
        const edited = withMagnitude(line({account: "income:salary"}), dec(-900000, 2));
        expect(edited.entry).toEqual(dec(900000, 2));
        expect(magnitudeOf(edited)).toEqual(dec(900000, 2));
    });

    it("moving a row to another account re-enters what it held, so the engine re-signs it", () => {
        const moved = withAccount(line({amount: {commodity: "$", quantity: dec(-420000, 2), precision: 2}}), "expenses:rent");
        expect(moved.account).toBe("expenses:rent");
        expect(moved.entry).toEqual(dec(420000, 2));
    });

    it("display precision is RAISED to fit a typed amount and never lowered", () => {
        const seeded = line({amount: {commodity: "$", quantity: dec(187500, 2), precision: 2}});
        // Retyping "1875" must not quietly move this line's growth onto whole dollars.
        expect(withMagnitude(seeded, dec(1875, 0)).amount.precision).toBe(2);
        // A third decimal place has to be representable or the growth walk rounds it away.
        expect(withMagnitude(seeded, dec(1875125, 3)).amount.precision).toBe(3);
    });

    it("the preview classification honours the engine's declared type over the name", () => {
        // Nothing about "wages" matches hledger's revenue regex; the declaration
        // is what makes it revenue.
        expect(isRevenueAccount("wages:contract", DECLARED)).toBe(true);
        expect(isRevenueAccount("wages:contract", NO_ACCOUNT_CLASSES)).toBe(false);
        expect(isRevenueAccount("income:salary", NO_ACCOUNT_CLASSES)).toBe(true);
    });
});

describe("UNIT scenarioModel — sections", () => {
    it("revenue goes to Income; everything else, including an asset transfer, to Expenses", () => {
        expect(sectionOfLine(line({account: "income:salary"}), DECLARED)).toBe("income");
        expect(sectionOfLine(line({account: "expenses:rent"}), DECLARED)).toBe("expense");
        expect(sectionOfLine(line({account: "assets:savings"}), DECLARED)).toBe("expense");
    });

    it("a single-date `~` rule is shown among the one-offs, though the engine keeps it a LINE", () => {
        const dated = line({period: {raw: "2027-03-01", simple: null, base: null, from: null, to: null}});
        expect(sectionOfLine(dated, DECLARED)).toBe("oneoff");
    });

    it("a bounded-but-irregular period is NOT a one-off — its raw is not a date", () => {
        const weird = line({period: {raw: "every weekday", simple: null, base: null, from: null, to: null}});
        expect(sectionOfLine(weird, DECLARED)).toBe("expense");
    });

    it("groups segments into one logical row, earliest span first, keeping the rows' own order", () => {
        const lines = [
            line({id: "b", group: "g-b", account: "expenses:software"}),
            line({id: "a", group: "g-a2", period: withBounds({raw: "", simple: "monthly", base: null, from: null, to: null}, "2027-04-01", null)}),
            line({id: "a", group: "g-a1", period: withBounds({raw: "", simple: "monthly", base: null, from: null, to: null}, null, "2027-04-01")}),
        ];
        const rows = logicalRows(lines, DECLARED, "expense");
        expect(rows.map((r) => r.id)).toEqual(["b", "a"]);
        expect(rows[1].segments.map((s) => s.group)).toEqual(["g-a1", "g-a2"]);
    });

    it("a new row opens on a prefix of the right TYPE, learned from the journal", () => {
        const names = ["revenues:salary", "revenues:consulting", "spending:rent"];
        const declared = classesOf(["revenues", "revenue"], ["spending", "expense"]);
        expect(sectionPrefix("income", names, declared)).toBe("revenues:");
        expect(sectionPrefix("expense", names, declared)).toBe("spending:");
    });

    it("falls back to the hledger defaults when the journal has nothing of that type yet", () => {
        expect(sectionPrefix("income", [], NO_ACCOUNT_CLASSES)).toBe("income:");
        expect(sectionPrefix("expense", [], NO_ACCOUNT_CLASSES)).toBe("expenses:");
        expect(sectionPrefix("asset", [], NO_ACCOUNT_CLASSES)).toBe("assets:");
    });

    it("an ASSET section learns its prefix the same way the other two do", () => {
        const names = ["patrimonio:corretaje", "gastos:alquiler"];
        const declared = classesOf(["patrimonio", "asset"], ["gastos", "expense"]);
        expect(sectionPrefix("asset", names, declared)).toBe("patrimonio:");
    });
});

describe("UNIT scenarioModel — the ASSET section (plan 23, Phase 3)", () => {
    // A row's section is its ROLE, ahead of its period and ahead of its
    // account. That is the whole reason this section needs no hold: nothing a
    // user can type into the row changes `role`.

    it("an asset row is an asset row whatever its account resolves to", () => {
        for (const account of ["assets:brokerage", "expenses:rent", "income:salary", "", "a"]) {
            expect(sectionOfLine(line({role: "asset", account}), DECLARED)).toBe("asset");
        }
    });

    it("a FLOW row posting to an asset account is still an outflow — the role decides, not the account", () => {
        expect(sectionOfLine(line({role: "flow", account: "assets:savings"}), DECLARED)).toBe("expense");
    });

    it("`asset` beats a single-date period, which `oneoff` would otherwise claim", () => {
        // A balance with a dated contribution is still a balance, and the
        // one-off table has no Growth column to show its rate in.
        const dated = line({role: "asset", account: "assets:house", period: {raw: "2027-03-01", simple: null, base: null, from: null, to: null}});
        expect(sectionOfLine(dated, DECLARED)).toBe("asset");
        expect(resolvedSection(dated, DECLARED)).toBe("asset");
    });

    it("`asset` beats a stale HOLD, which only the two flow sections can use", () => {
        expect(sectionOfLine(line({role: "asset", account: "assets:house", section: "income"}), DECLARED)).toBe("asset");
    });

    it("`logicalRows` gathers the asset section like any other", () => {
        const lines = [line({id: "rent"}), line({id: "brok", role: "asset", account: "assets:brokerage"})];
        expect(logicalRows(lines, DECLARED, "asset").map((r) => r.id)).toEqual(["brok"]);
        expect(logicalRows(lines, DECLARED, "expense").map((r) => r.id)).toEqual(["rent"]);
    });

    it("`blankAssetLine` is an asset with a ZERO contribution and the journal's balance", () => {
        const asset = blankAssetLine("line-9", "$");
        expect(asset.role).toBe("asset");
        // Zero is the `$0` posting the file writes for a balance that only
        // compounds, and a null `opening` is "use the journal's".
        expect(asset.amount.quantity).toEqual({m: 0n, p: 2});
        expect(asset.opening).toBeNull();
        expect(asset.growth).toBeNull();
    });

    it("`blankLine` still makes a FLOW, and must — an asset row is created deliberately", () => {
        expect(blankLine("line-1", "$").role).toBe("flow");
    });
});

describe("UNIT scenarioModel — a HELD section", () => {
    // The model half of the section-stability rule. The component owns when a
    // hold is taken and dropped; what a hold MEANS is here.

    it("REGRESSION: a held row does not move on a half-typed account", () => {
        // `r` is not a revenue account and neither is ``, so both of these read
        // as "expense" by type. That re-derivation on every keystroke is what
        // threw a row out of Income on its first letter.
        expect(sectionOfLine(line({account: "r", section: "income"}), DECLARED)).toBe("income");
        expect(sectionOfLine(line({account: "", section: "income"}), DECLARED)).toBe("income");
    });

    it("holds a row on the WRONG side too, for as long as the hold lasts", () => {
        // Deliberate: the point is that nothing moves mid-edit, not that the
        // hold is ever right. Reconciling is the release's job.
        expect(sectionOfLine(line({account: "income:salary", section: "expense"}), DECLARED)).toBe("expense");
    });

    it("hands the row back to its account's TYPE once the hold is gone", () => {
        expect(sectionOfLine(line({account: "wages:contract"}), DECLARED)).toBe("income");
    });

    it("never lets a hold beat `oneoff`, which is the PERIOD talking", () => {
        const dated = line({account: "expenses:rent", section: "income", period: {raw: "2027-03-01", simple: null, base: null, from: null, to: null}});
        expect(sectionOfLine(dated, DECLARED)).toBe("oneoff");
    });

    it("`resolvedSection` ignores the hold — it is what a release reconciles against", () => {
        expect(resolvedSection(line({account: "income:salary", section: "expense"}), DECLARED)).toBe("income");
        expect(resolvedSection(line({account: "r", section: "income"}), DECLARED)).toBe("expense");
    });

    it("keeps a held row out of the section its account would file it under", () => {
        // The tear this prevents: `logicalRows` gathers per section, so a row
        // listed in neither or both is a row that vanished or was duplicated.
        const held = line({id: "new", group: "g-new", account: "r", section: "income"});
        expect(logicalRows([held], DECLARED, "income").map((r) => r.id)).toEqual(["new"]);
        expect(logicalRows([held], DECLARED, "expense")).toEqual([]);
    });

    it("keeps both segments of a step change together when both are held", () => {
        const first = line({
            id: "pay",
            group: "g-1",
            account: "inc",
            section: "income",
            period: withBounds({raw: "", simple: "monthly", base: null, from: null, to: null}, null, "2027-04-01"),
        });
        const second = line({
            id: "pay",
            group: "g-2",
            account: "inc",
            section: "income",
            period: withBounds({raw: "", simple: "monthly", base: null, from: null, to: null}, "2027-04-01", null),
        });
        const rows = logicalRows([first, second], DECLARED, "income");
        expect(rows).toHaveLength(1);
        expect(rows[0].segments.map((s) => s.group)).toEqual(["g-1", "g-2"]);
    });

    it("a duplicated row does NOT inherit the hold", () => {
        // Nobody is editing a row that did not exist a moment ago, and nothing
        // releases a hold on a row that is never focused.
        const held = line({id: "pay", group: "g-1", account: "income:salary", section: "expense"});
        const copies = duplicateRow([held], "pay", takenIds(scenario([held])));
        expect(copies[1].section).toBeUndefined();
        expect(sectionOfLine(copies[1], DECLARED)).toBe("income");
    });

    it("names every row of a section distinctly, blank or duplicated", () => {
        // Defect 2's other half: a delete you cannot address is not a delete.
        const rows = logicalRows(
            [
                line({id: "a", group: "g-a", account: "income:salary"}),
                line({id: "b", group: "g-b", account: "income:"}),
                line({id: "c", group: "g-c", account: "income:"}),
                line({id: "d", group: "g-d", account: "", section: "income"}),
            ],
            DECLARED,
            "income"
        );
        expect(rowNames(rows, "Income")).toEqual(["income:salary", "income: (row 2 of Income)", "income: (row 3 of Income)", "row 4 of Income"]);
    });

    it("never puts a hold on the wire", () => {
        // UI-only, like a period's derived fields. `scenarioToWire` names its
        // fields one by one, and a file round trip must not carry this one.
        const wired = scenarioToWire(scenario([line({account: "income:salary", section: "expense"})]));
        expect(wired.lines[0]).not.toHaveProperty("section");
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
        const bounded = withBounds({raw: "monthly", simple: "monthly", base: null, from: null, to: null}, "2027-04-01", null);
        expect(withInterval(bounded, "yearly")).toEqual({raw: "yearly from 2027-04-01", simple: "yearly", base: null, from: "2027-04-01", to: null});
    });

    it("leaves a period it cannot rebuild exactly as it arrived", () => {
        const weird = {raw: "every weekday", simple: null, base: null, from: null, to: null} as const;
        expect(withBounds(weird, "2027-01-01", null)).toBe(weird);
        expect(boundsLockedReason(weird)).toContain("every weekday");
    });

    it("re-bounds a period it cannot spell around the words the engine handed back", () => {
        const anchored = {raw: "every 15th day of month from 2026-11-01", simple: null, base: "every 15th day of month", from: "2026-11-01", to: null} as const;
        expect(boundsLockedReason(anchored)).toBeNull();
        expect(withBounds(anchored, null, "2027-06-01")).toEqual({
            raw: "every 15th day of month to 2027-06-01",
            simple: null,
            base: "every 15th day of month",
            from: null,
            to: "2027-06-01",
        });
    });
});

describe("UNIT scenarioModel — step split and merge", () => {
    const base = scenario([line({id: "payroll", group: "rule:0", account: "expenses:payroll"})]);

    it("splitting makes TWO bounded segments of ONE logical row, meeting on the date", () => {
        const split = splitStep(base.lines, "payroll", "2027-04-01", takenIds(base));
        expect(split).toHaveLength(2);
        expect(split.map((s) => s.id)).toEqual(["payroll", "payroll"]);
        expect(split[0].period).toEqual({raw: "monthly to 2027-04-01", simple: "monthly", base: null, from: null, to: "2027-04-01"});
        expect(split[1].period).toEqual({raw: "monthly from 2027-04-01", simple: "monthly", base: null, from: "2027-04-01", to: null});
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
        const bounded = scenario([
            line({id: "payroll", period: withBounds({raw: "", simple: "monthly", base: null, from: null, to: null}, "2027-01-01", "2029-01-01")}),
        ]);
        const split = splitStep(bounded.lines, "payroll", "2028-01-01", takenIds(bounded));
        expect(mergeStep(split, "payroll")[0].period).toEqual({
            raw: "monthly from 2027-01-01 to 2029-01-01",
            simple: "monthly",
            base: null,
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
        const bounded = scenario([
            line({id: "payroll", period: withBounds({raw: "", simple: "monthly", base: null, from: null, to: null}, "2027-01-01", "2028-01-01")}),
        ]);
        expect(splitStep(bounded.lines, "payroll", "2026-01-01", new Set())).toHaveLength(1);
        expect(splitStep(bounded.lines, "nope", "2027-06-01", new Set())).toHaveLength(1);
        expect(splitStep(bounded.lines, "payroll", "not-a-date", new Set())).toHaveLength(1);

        const weird = scenario([line({id: "payroll", period: {raw: "every weekday", simple: null, base: null, from: null, to: null}})]);
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
        const wire = scenarioToWire(scenario([line({period: {raw: "monthly from 2027-04-01", simple: "monthly", base: null, from: "2027-04-01", to: null}})]));
        expect(wire.lines[0].period).toEqual({raw: "monthly from 2027-04-01"});
    });

    it("sends the journal's sign, the display precision, and an explicit null for an absent growth", () => {
        const wire = scenarioToWire(scenario([line({account: "income:salary", amount: {commodity: "$", quantity: dec(-518833, 2), precision: 2}})]));
        expect(wire.lines[0].amount).toEqual({commodity: "$", quantity: {mantissa: "-518833", places: 2}, precision: 2});
        expect(wire.lines[0].growth).toBeNull();
        expect(wire.created).toBeNull();
    });

    it("sends an edited line's ENTRY for the engine to sign, and no entry for an untouched one", () => {
        const untouched = line({account: "income:salary", amount: {commodity: "$", quantity: dec(-518833, 2), precision: 2}});
        const edited = withMagnitude(untouched, dec(600000, 2));
        const wire = scenarioToWire(scenario([untouched, {...edited, id: "b", group: "rule:1"}]));
        expect(wire.lines[0].entry).toBeUndefined();
        expect(wire.lines[1].entry).toEqual({mantissa: "600000", places: 2});
    });

    it("sends a growth rate as the fraction it is", () => {
        const wire = scenarioToWire(scenario([line({growth: {rate: dec(3, 2), unit: "year"}})]));
        expect(wire.lines[0].growth).toEqual({rate: {mantissa: "3", places: 2}, unit: "year"});
    });

    it("drops half-typed rows rather than making the whole request a 400", () => {
        const half = {
            ...emptyScenario(),
            lines: [line(), blankLine("line-1", "$")],
            events: [
                {
                    id: "e1",
                    date: "2027-03-01",
                    description: "raise",
                    postings: [{account: "assets:cash", amount: {commodity: "$", quantity: dec(200, 0), precision: 0}}],
                    counter: null,
                },
                {
                    id: "e2",
                    date: "2027-03-01",
                    description: "half typed",
                    postings: [{account: "", amount: {commodity: "$", quantity: dec(0, 0), precision: 0}}],
                    counter: null,
                },
                {
                    id: "e3",
                    date: "",
                    description: "no date",
                    postings: [{account: "assets:cash", amount: {commodity: "$", quantity: dec(1, 0), precision: 0}}],
                    counter: null,
                },
            ],
        };
        const kept = projectable(half);
        expect(kept.lines.map((l) => l.account)).toEqual(["expenses:rent"]);
        expect(kept.events.map((e) => e.id)).toEqual(["e1"]);
    });

    it("sends no `asOf`: the engine's today is the clock every other report on the page is read against", () => {
        const body = runBody(scenario([line()]), {interval: "monthly", count: 24, lookback: 2, depth: 2});
        expect(body.asOf).toBeUndefined();
        expect(body).toMatchObject({interval: "monthly", count: 24, lookback: 2, depth: 2});
    });

    it("asks for the Accounts and Journal readings only when the window does", () => {
        const plain = runBody(scenario([line()]), {interval: "monthly", count: 24, lookback: 2, depth: 2, accounts: null, journal: false});
        expect("accounts" in plain).toBe(false);
        expect("journal" in plain).toBe(false);
        const asked = runBody(scenario([line()]), {
            interval: "monthly",
            count: 24,
            lookback: 2,
            depth: 2,
            accounts: {mode: "balance", empty: true},
            journal: true,
        });
        expect(asked.accounts).toEqual({mode: "balance", empty: true});
        expect(asked.journal).toBe(true);
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
            autoRules: Object.freeze([
                Object.freeze({raw: "= expenses:rent\n    (budget:rent)  *-1\n", query: "expenses:rent", unsupported: null, form: null, block: 0}),
                Object.freeze({
                    raw: "= expenses:rent\n    expenses:rent-tax  *0.1\n",
                    query: "expenses:rent",
                    unsupported: null,
                    block: 1,
                    form: Object.freeze({
                        matchAccount: "expenses:rent",
                        extraQuery: "",
                        postings: Object.freeze([
                            Object.freeze({account: "expenses:rent-tax", kind: "percent" as const, percent: Object.freeze(dec(10, 0)), amount: null}),
                        ]),
                        counter: null,
                        comment: "",
                    }),
                }),
            ]),
        }) as Scenario;

        const clone = cloneScenario(decoded);
        expect(clone).toEqual(decoded);
        // Every level is writable, which is what the editor's `$state` needs.
        for (const target of [
            clone,
            clone.lines[0],
            clone.lines[0].amount,
            clone.lines[0].amount.quantity,
            clone.events[0],
            clone.events[0].postings[0],
            clone.autoRules,
            clone.autoRules[0],
            clone.autoRules[1]!.form,
            clone.autoRules[1]!.form!.postings[0],
            clone.autoRules[1]!.form!.postings[0]!.percent,
        ]) {
            expect(Object.isFrozen(target)).toBe(false);
        }
        clone.lines[0].account = "expenses:other";
        expect(decoded.lines[0].account).toBe("expenses:rent");
    });

    it("deep-clones an asset row's balance OVERRIDE, which the decoder also freezes", () => {
        const frozen = Object.freeze({
            ...emptyScenario(),
            lines: Object.freeze([
                Object.freeze({
                    ...line({role: "asset", account: "assets:house"}),
                    opening: Object.freeze({commodity: "$", quantity: Object.freeze(dec(64000000, 2)), precision: 2}),
                }),
            ]),
        }) as Scenario;

        const clone = cloneScenario(frozen);
        expect(clone.lines[0].opening).toEqual(frozen.lines[0].opening);
        expect(Object.isFrozen(clone.lines[0].opening)).toBe(false);
        expect(Object.isFrozen(clone.lines[0].opening?.quantity)).toBe(false);
    });
});

describe("UNIT scenarioModel — the name and the file it becomes", () => {
    // A MIRROR of the engine's `projections::slug_filename`, and these cases are
    // the same ones `crates/ledgeline-core/src/projections/discovery.rs` pins.
    // The dialog shows the resulting path before it writes, so the two agreeing
    // is what keeps that preview from being a lie.
    describe("slugFilename", () => {
        it("lowercases, collapses runs of non-alphanumerics, and trims the ends", () => {
            expect(slugFilename("Series A with a hiring ramp")).toBe("projection-series-a-with-a-hiring-ramp.journal");
            expect(slugFilename("  ***Plan B!!! 2027  ")).toBe("projection-plan-b-2027.journal");
            expect(slugFilename("UPPER CASE")).toBe("projection-upper-case.journal");
        });

        it("slugs a traversal away rather than refusing it", () => {
            // Every separator and every dot is a non-alphanumeric, so there is
            // nothing left that could name a parent directory. The engine's
            // `resolve_new` refuses one anyway; this is the first of the two.
            expect(slugFilename("a/../../etc/passwd")).toBe("projection-a-etc-passwd.journal");
            expect(slugFilename("..")).toBeNull();
        });

        it("is null when nothing survives, rather than `projection-.journal`", () => {
            // A name nobody chose, and one the scan would then list.
            expect(slugFilename("")).toBeNull();
            expect(slugFilename("   ")).toBeNull();
            expect(slugFilename("—")).toBeNull();
            // ASCII only, deliberately: a slug is a filename on somebody else's
            // filesystem too, and a Unicode class would normalize differently
            // on two machines.
            expect(slugFilename("日本語")).toBeNull();
        });

        it("never ends in a separator, however long the name is", () => {
            const long = `${"a".repeat(199)} b`;
            const slug = slugFilename(long);
            expect(slug).not.toBeNull();
            expect(slug?.endsWith("-.journal")).toBe(false);
        });
    });

    describe("projectionId", () => {
        it("joins the chosen folder to the slugged file name", () => {
            expect(projectionId("plans", "Series A")).toBe("plans/projection-series-a.journal");
            expect(projectionId("", "Series A")).toBe("projection-series-a.journal");
        });

        it("never leaves a leading or doubled separator", () => {
            expect(projectionId("/plans/", "Series A")).toBe("plans/projection-series-a.journal");
        });

        it("is null when the name has no file name in it", () => {
            expect(projectionId("plans", "!!!")).toBeNull();
        });
    });
});

describe("UNIT scenarioModel — counter-accounts, one per rule (plan 28)", () => {
    // One `~` rule of three lines, the shape a budget.journal rule seeds as.
    const rule = (): ScenarioLine[] => [
        line({id: "food", group: "rule:0", account: "expenses:food", counter: "assets:checking"}),
        line({id: "rent", group: "rule:0", account: "expenses:rent", counter: "assets:checking"}),
        line({id: "fun", group: "rule:0", account: "expenses:fun", counter: "assets:checking"}),
    ];
    const taken = (lines: ScenarioLine[]) => takenIds(scenario(lines));

    it("moves a line whose counter now DIFFERS into a rule of its own, so the file stays writable", () => {
        const lines = rule();
        const next = withCounter(lines, "rent", "rule:0", "liabilities:card", taken(lines));
        const rent = next.find((l) => l.id === "rent")!;
        expect(rent.counter).toBe("liabilities:card");
        expect(rent.group).not.toBe("rule:0");
        expect(taken(lines).has(rent.group)).toBe(false);
        // The rest of the rule is untouched, and still agrees with itself.
        expect(next.filter((l) => l.group === "rule:0").map((l) => [l.id, l.counter])).toEqual([
            ["food", "assets:checking"],
            ["fun", "assets:checking"],
        ]);
        // Every rule now names ONE counter — what the serializer requires.
        const byGroup = new Map<string, Set<string | null>>();
        for (const l of next) byGroup.set(l.group, (byGroup.get(l.group) ?? new Set()).add(l.counter));
        expect([...byGroup.values()].every((counters) => counters.size === 1)).toBe(true);
    });

    it("keeps the rule when the new counter agrees with every rule-mate", () => {
        const lines = rule();
        expect(withCounter(lines, "rent", "rule:0", "assets:checking", taken(lines))).toEqual(lines);
    });

    it("keeps the rule for a line alone in it, whatever it is set to", () => {
        const lines = [line({counter: null})];
        const next = withCounter(lines, "rent", "rule:0", "assets:savings", taken(lines));
        expect(next).toEqual([{...lines[0], counter: "assets:savings"}]);
    });

    it("clearing one line's counter splits it too: a counter-less line beside a countered one would be written virtual", () => {
        const lines = rule();
        const next = withCounter(lines, "food", "rule:0", "  ", taken(lines));
        const food = next.find((l) => l.id === "food")!;
        expect(food.counter).toBeNull();
        expect(food.group).not.toBe("rule:0");
    });

    it("touches only the segment named, not the other segments of a step change", () => {
        const lines = [line({id: "pay", group: "g:a", counter: null}), line({id: "pay", group: "g:b", counter: null})];
        const next = withCounter(lines, "pay", "g:b", "assets:savings", taken(lines));
        expect(next.map((l) => [l.group, l.counter])).toEqual([
            ["g:a", null],
            ["g:b", "assets:savings"],
        ]);
    });

    it("trims a typed account, and blank means the engine's default", () => {
        expect(accountOrNull("  assets:checking ")).toBe("assets:checking");
        expect(accountOrNull("   ")).toBeNull();
        expect(accountOrNull(null)).toBeNull();
    });

    it("sets an asset row's gains account without touching its rule", () => {
        const lines = [line({id: "house", group: "rule:9", role: "asset", account: "assets:house"}), line()];
        const next = withGains(lines, "rule:9", " equity:home gains ");
        expect(next[0]).toEqual({...lines[0], gains: "equity:home gains"});
        // Never on a flow line — the wire refuses one there.
        expect(withGains(lines, "rule:0", "equity:x")[1].gains).toBeNull();
        expect(withGains(next, "rule:9", "")[0].gains).toBeNull();
    });

    it("sends trimmed accounts, and blank as null, so ' ' is never an account name on the wire", () => {
        const wire = scenarioToWire({
            ...scenario([line({counter: " assets:checking ", gains: ""})]),
            cash: "  ",
            events: [{id: "e", date: "2027-01-01", description: "", postings: [], counter: ""}],
        });
        expect(wire.lines[0].counter).toBe("assets:checking");
        expect(wire.lines[0].gains).toBeNull();
        expect(wire.cash).toBeNull();
        expect(wire.events[0].counter).toBeNull();
    });
});

// ---------------------------------------------------------------------------
// Order, and descriptions (plan 28, follow-ups 2026-10-26)
// ---------------------------------------------------------------------------

describe("UNIT scenarioModel — moving rows", () => {
    const expense = (id: string, group: string, account = `expenses:${id}`) => line({id, group, account});
    const accounts = (lines: readonly ScenarioLine[]) => logicalRows(lines, DECLARED, "expense").map((row) => row.segments[0].account);

    it("swaps two single-rule rows, and the ends go nowhere", () => {
        const lines = [expense("a", "g-a"), expense("b", "g-b"), expense("c", "g-c")];
        expect(accounts(moveRow(lines, DECLARED, "expense", "b", -1))).toEqual(["expenses:b", "expenses:a", "expenses:c"]);
        expect(accounts(moveRow(lines, DECLARED, "expense", "b", 1))).toEqual(["expenses:a", "expenses:c", "expenses:b"]);
        expect(accounts(moveRow(lines, DECLARED, "expense", "a", -1))).toEqual(["expenses:a", "expenses:b", "expenses:c"]);
        expect(accounts(moveRow(lines, DECLARED, "expense", "c", 1))).toEqual(["expenses:a", "expenses:b", "expenses:c"]);
    });

    it("two postings of ONE rule swap inside it — the posting order is what persists", () => {
        const lines = [expense("a", "rule:0"), expense("b", "rule:0"), expense("c", "rule:1")];
        const moved = moveRow(lines, DECLARED, "expense", "b", -1);
        expect(moved.map((l) => [l.id, l.group])).toEqual([
            ["b", "rule:0"],
            ["a", "rule:0"],
            ["c", "rule:1"],
        ]);
    });

    it("past a row of another rule, a row carries its whole rule with it", () => {
        // `a` and `b` are one budget rule; `c` is a seeded row of its own. Moving
        // `c` above `b` can only be stored as `c`'s rule above `a`+`b`'s.
        const lines = [expense("a", "rule:0"), expense("b", "rule:0"), expense("c", "gap:c")];
        expect(accounts(moveRow(lines, DECLARED, "expense", "c", -1))).toEqual(["expenses:c", "expenses:a", "expenses:b"]);
        // …and `b` moving down past `c` carries `a` along, for the same reason.
        expect(accounts(moveRow(lines, DECLARED, "expense", "b", 1))).toEqual(["expenses:c", "expenses:a", "expenses:b"]);
    });

    it("a step change's segments move together, as one row", () => {
        const lines = [
            expense("a", "g-a"),
            line({
                id: "pay",
                group: "g-1",
                account: "expenses:pay",
                period: {raw: "monthly to 2027-04-01", simple: "monthly", base: null, from: null, to: "2027-04-01"},
            }),
            line({
                id: "pay",
                group: "g-2",
                account: "expenses:pay",
                period: {raw: "monthly from 2027-04-01", simple: "monthly", base: null, from: "2027-04-01", to: null},
            }),
        ];
        const moved = moveRow(lines, DECLARED, "expense", "pay", -1);
        expect(moved.map((l) => l.group)).toEqual(["g-1", "g-2", "g-a"]);
        expect(logicalRows(moved, DECLARED, "expense")[0].segments).toHaveLength(2);
    });

    it("moves only within the section: an income row in between is not a neighbour", () => {
        const lines = [expense("a", "g-a"), line({id: "pay", group: "g-p", account: "income:salary"}), expense("b", "g-b")];
        const moved = moveRow(lines, DECLARED, "expense", "b", -1);
        expect(accounts(moved)).toEqual(["expenses:b", "expenses:a"]);
        expect(logicalRows(moved, DECLARED, "income").map((row) => row.id)).toEqual(["pay"]);
    });

    it("never changes a row's section or its hold", () => {
        const held = line({id: "x", group: "g-x", account: "income:half-typed", section: "expense"});
        const lines = [expense("a", "g-a"), held];
        const moved = moveRow(lines, DECLARED, "expense", "x", -1);
        expect(moved[0]).toEqual(held);
        expect(sectionOfLine(moved[0], DECLARED)).toBe("expense");
    });

    it("a drag lands the row where the target was, through the same moves", () => {
        const lines = ["a", "b", "c", "d"].map((id) => expense(id, `g-${id}`));
        expect(accounts(moveRowTo(lines, DECLARED, "expense", "d", "a"))).toEqual(["expenses:d", "expenses:a", "expenses:b", "expenses:c"]);
        expect(accounts(moveRowTo(lines, DECLARED, "expense", "a", "c"))).toEqual(["expenses:b", "expenses:c", "expenses:a", "expenses:d"]);
        expect(accounts(moveRowTo(lines, DECLARED, "expense", "b", "b"))).toEqual(["expenses:a", "expenses:b", "expenses:c", "expenses:d"]);
    });

    it("gathers a rule whose lines were apart, which is what the file would do", () => {
        const lines = [expense("a", "rule:0"), expense("b", "rule:1"), expense("c", "rule:0")];
        expect(gatherRules(lines).map((l) => l.id)).toEqual(["a", "c", "b"]);
    });

    it("events reorder freely", () => {
        const ev = (id: string) => ({id, date: "2027-01-01", description: id, postings: [], counter: null});
        const events = [ev("x"), ev("y"), ev("z")];
        expect(moveEvent(events, "z", -1).map((e) => e.id)).toEqual(["x", "z", "y"]);
        expect(moveEvent(events, "x", -1).map((e) => e.id)).toEqual(["x", "y", "z"]);
        expect(moveEventTo(events, "z", "x").map((e) => e.id)).toEqual(["z", "x", "y"]);
        expect(moveEventTo(events, "x", "z").map((e) => e.id)).toEqual(["y", "z", "x"]);
    });
});

describe("UNIT scenarioModel — descriptions", () => {
    it("a description belongs to the RULE: every line of it takes the edit", () => {
        const lines = [line({id: "a", group: "rule:0"}), line({id: "b", group: "rule:0"}), line({id: "c", group: "rule:1", note: "kept"})];
        const next = withNote(lines, "rule:0", "rent and utilities");
        expect(next.map((l) => l.note)).toEqual(["rent and utilities", "rent and utilities", "kept"]);
        expect(ruleSize(lines, "rule:0")).toBe(2);
        expect(ruleSize(lines, "rule:1")).toBe(1);
    });

    it("refuses exactly what the file writer refuses", () => {
        expect(noteProblem("rent, after the move")).toBeNull();
        expect(noteProblem("")).toBeNull();
        expect(noteProblem("rent ; utilities")).toMatch(/comment/);
        expect(noteProblem("rent # 2")).toMatch(/comment/);
        expect(noteProblem("two\nlines")).toMatch(/line break/);
        expect(noteProblem("x".repeat(MAX_NOTE_BYTES))).toBeNull();
        expect(noteProblem("é".repeat(MAX_NOTE_BYTES / 2 + 1))).toMatch(/at most/);
    });

    it("flags a seeded description as stale once the amount it describes changes", () => {
        const seeded = line({source: "unbudgeted", note: "unbudgeted — average over 2025-11-01 to 2026-10-22"});
        expect(noteIsStale(seeded)).toBe(false);
        expect(noteIsStale(withMagnitude(seeded, dec(420000, 2)))).toBe(false);
        expect(noteIsStale(withMagnitude(seeded, dec(5000, 0)))).toBe(true);
        // A description the user wrote is theirs, whatever the amount does.
        expect(noteIsStale(withMagnitude({...seeded, note: "rent"}, dec(5000, 0)))).toBe(false);
    });
});

describe("UNIT scenarioModel — automatic posting rules (plan 30 Phase 4)", () => {
    const form = () => ({
        matchAccount: "expenses:rent",
        extraQuery: "",
        postings: [{account: "expenses:rent-tax", kind: "percent" as const, percent: dec(10, 0), amount: null}],
        counter: "assets:checking",
        comment: "",
    });
    const read = (block: number) => ({
        raw: `= expenses:rent\n    expenses:rent-tax  *0.${block + 1}\n`,
        query: "expenses:rent",
        unsupported: null,
        form: form(),
        block,
    });

    it("sends an untouched rule as its text and an edited or new one as its form, each with its block", () => {
        const locked = {...read(2), form: null, edited: true};
        const wire = scenarioToWire({...emptyScenario(), autoRules: [read(0), {...read(1), edited: true}, blankAutoRule("assets:cash"), locked]});
        expect(wire.autoRules).toEqual([
            {block: 0, raw: read(0).raw},
            {
                block: 1,
                form: {
                    matchAccount: "expenses:rent",
                    extraQuery: "",
                    postings: [{account: "expenses:rent-tax", kind: "percent", percent: {mantissa: "10", places: 0}, amount: null}],
                    counter: "assets:checking",
                    comment: "",
                },
            },
            {
                block: null,
                form: {
                    matchAccount: "",
                    extraQuery: "",
                    postings: [{account: "", kind: "percent", percent: null, amount: null}],
                    counter: "assets:cash",
                    comment: "",
                },
            },
            // A rule the form cannot state always goes back as its text.
            {block: 2, raw: locked.raw},
        ]);
    });

    it("leaves a half-typed rule out of a run, as it does a line with no account", () => {
        const kept = projectable({...emptyScenario(), autoRules: [read(0), blankAutoRule(null), {...read(1), edited: true}]});
        expect(kept.autoRules.map((rule) => rule.block)).toEqual([0, 1]);
    });

    it("finds a typical amount on the matched account or under it, for the preview", () => {
        const sales = line({id: "s", account: "revenue:sales:acme", amount: {commodity: "$", quantity: dec(-4000000, 2), precision: 2}});
        const withEvent: Scenario = {
            ...scenario([line(), sales]),
            events: [
                {
                    id: "e",
                    date: "2027-01-01",
                    description: "",
                    counter: null,
                    postings: [{account: "income:bonus", amount: {commodity: "$", quantity: dec(500, 0), precision: 0}}],
                },
            ],
        };
        expect(ruleSample(withEvent, "revenue:sales")).toEqual(sales.amount);
        expect(ruleSample(withEvent, "revenue:sale")).toBeNull();
        expect(ruleSample(withEvent, "income")?.quantity).toEqual(dec(500, 0));
        expect(ruleSample(withEvent, " ")).toBeNull();
    });
});
