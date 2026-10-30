import {describe, expect, it} from "vitest";
import {dec} from "#lib/domain/money.js";
import {accountTypeOf, NO_ACCOUNT_CLASSES} from "#lib/domain/accountClass.js";
import {classesOf} from "#lib/domain/accountClassFixtures.js";
import {
    activeScenario,
    commentToWire,
    withLayout,
    blankAutoRule,
    blankEvent,
    renderBody,
    ruleSample,
    accountOrNull,
    blankAssetLine,
    blankLine,
    boundsLockedReason,
    cloneScenario,
    emptyScenario,
    freshId,
    MAX_NOTE_BYTES,
    mergeStep,
    noteIsStale,
    noteProblem,
    percentFromRate,
    periodRaw,
    projectable,
    projectionId,
    rateFromPercent,
    runBody,
    scenarioClasses,
    linesBalance,
    eventBalances,
    scenarioToWire,
    sectionPrefix,
    slugFilename,
    splitStep,
    takenIds,
    trimDec,
    parseSignedInput,
    withQuantity,
    withBounds,
    withInterval,
} from "./scenarioModel";
import type {Scenario, ScenarioLine} from "./types";

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
        comment: "",
        ruleComment: "",
        ...overrides,
    };
}

function scenario(lines: ScenarioLine[]): Scenario {
    return {...emptyScenario(), lines};
}

// ---------------------------------------------------------------------------

describe("UNIT scenarioModel — amounts are signed as the file writes them", () => {
    // Plan 31 amendment C3-29: what is typed is what the file holds. Nothing
    // here, and nothing in the engine on this path, signs by account type.

    it("a typed figure is the line's signed amount, minus kept", () => {
        const salary = line({account: "income:salary", amount: {commodity: "$", quantity: dec(-518833, 2), precision: 2}});
        expect(withQuantity(salary, dec(-600000, 2)).amount.quantity).toEqual(dec(-600000, 2));
        // A positive figure on revenue stays positive: a refund, or a mistake the "Writes:" box shows.
        expect(withQuantity(salary, dec(600000, 2)).amount.quantity).toEqual(dec(600000, 2));
    });

    it("the capex rule's checking leg keeps its minus, so the rule still balances", () => {
        const capex = [
            line({id: "c", account: "assets:checking", amount: {commodity: "$", quantity: dec(-300000, 2), precision: 2}}),
            line({id: "f", account: "assets:fixed:computers", amount: {commodity: "$", quantity: dec(300000, 2), precision: 2}}),
        ];
        const edited = [withQuantity(capex[0], dec(-3500, 0)), withQuantity(capex[1], dec(3500, 0))];
        expect(edited[0].amount.quantity).toEqual(dec(-3500, 0));
        expect(linesBalance(edited)).toBe(true);
        const wire = scenarioToWire(scenario(edited));
        expect(wire.lines.map((wired) => wired.amount.quantity)).toEqual([
            {mantissa: "-3500", places: 0},
            {mantissa: "3500", places: 0},
        ]);
    });

    it("reads a leading minus, and the true minus the list draws, as a minus", () => {
        expect(parseSignedInput("-3,500")).toEqual(dec(-3500, 0));
        expect(parseSignedInput("\u22123500.00")).toEqual(dec(-350000, 2));
        expect(parseSignedInput("12000")).toEqual(dec(12000, 0));
        expect(parseSignedInput("−")).toBeNull();
        expect(parseSignedInput("abc")).toBeNull();
    });

    it("display precision is RAISED to fit a typed amount and never lowered", () => {
        const seeded = line({amount: {commodity: "$", quantity: dec(187500, 2), precision: 2}});
        // Retyping "1875" must not quietly move this line's growth onto whole dollars.
        expect(withQuantity(seeded, dec(1875, 0)).amount.precision).toBe(2);
        // A third decimal place has to be representable or the growth walk rounds it away.
        expect(withQuantity(seeded, dec(-1875125, 3)).amount.precision).toBe(3);
    });

    it("remembers the served figure once, through any number of edits", () => {
        const seeded = line({amount: {commodity: "$", quantity: dec(-518833, 2), precision: 2}});
        const twice = withQuantity(withQuantity(seeded, dec(-1, 0)), dec(-2, 0));
        expect(twice.served).toEqual(dec(-518833, 2));
        expect(twice.amount.quantity).toEqual(dec(-2, 0));
    });
});

describe("UNIT scenarioModel — account types and balance", () => {
    it("the scenario file's own types win over the journal's, for the account and its undeclared descendants", () => {
        const journal = classesOf({name: "hmrc", type: null, typeDeclared: false}, {name: "hmrc:paye", type: null, typeDeclared: false}, [
            "hmrc:paye:refund",
            "asset",
        ]);
        const classes = scenarioClasses(journal, {accountTypes: {hmrc: "liability"}});
        expect(accountTypeOf("hmrc", classes)).toBe("liability");
        expect(accountTypeOf("hmrc:paye", classes)).toBe("liability");
        // A journal declaration of its own is the nearer one; it stands.
        expect(accountTypeOf("hmrc:paye:refund", classes)).toBe("asset");
        // A name nobody has seen inherits from the declared ancestor.
        expect(accountTypeOf("hmrc:vat", classes)).toBe("liability");
        expect(scenarioClasses(journal, {})).toBe(journal);
    });

    it("a rule that states both legs balances; one that leaves its cash leg to a counter does not", () => {
        const capex = [
            line({account: "assets:checking", amount: {commodity: "$", quantity: dec(-300000, 2), precision: 2}}),
            line({account: "assets:fixed:computers", amount: {commodity: "$", quantity: dec(300000, 2), precision: 2}}),
        ];
        expect(linesBalance(capex)).toBe(true);
        expect(linesBalance([line()])).toBe(false);
        expect(linesBalance([])).toBe(false);
    });

    it("balance is read from the amounts as written, whatever the accounts' types", () => {
        // Revenue is typed negative, so it balances a liability going up by
        // the same figure (Patrick's deferred revenue).
        const usd = (cents: number) => ({commodity: "$", quantity: dec(cents, 2), precision: 2});
        const deferred = [line({account: "liabilities:deferred-revenue", amount: usd(500000)}), line({account: "income:sales", amount: usd(-500000)})];
        expect(linesBalance(deferred)).toBe(true);
        // Two positive figures do not, though one account is revenue: nothing re-signs it.
        expect(linesBalance([deferred[0], {...deferred[1], amount: usd(500000)}])).toBe(false);
        // Dollars against euros balance nothing.
        const fx = [line(), line({account: "assets:eur", amount: {commodity: "€", quantity: dec(-420000, 2), precision: 2}})];
        expect(linesBalance(fx)).toBe(false);
    });

    it("a one-off balances when its own postings sum to zero", () => {
        const posting = (account: string, cents: number) => ({account, amount: {commodity: "$", quantity: dec(cents, 2), precision: 2}, comment: ""});
        const move = {...blankEvent("move", "2027-01-01", "$"), postings: [posting("assets:checking", 10000), posting("assets:savings", -10000)]};
        expect(eventBalances(move)).toBe(true);
        expect(eventBalances({...move, postings: [posting("expenses:legal", 10000)]})).toBe(false);
    });
});

describe("UNIT scenarioModel — sections", () => {
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

    it("sends an edited line's amount signed as typed, and never an `entry` for the engine to sign", () => {
        const untouched = line({account: "income:salary", amount: {commodity: "$", quantity: dec(-518833, 2), precision: 2}});
        const edited = withQuantity(untouched, dec(-600000, 2));
        const wire = scenarioToWire(scenario([untouched, {...edited, id: "b", group: "rule:1"}]));
        expect(wire.lines.map((wired) => "entry" in wired)).toEqual([false, false]);
        expect(wire.lines.map((wired) => "served" in wired)).toEqual([false, false]);
        expect(wire.lines[1].amount.quantity).toEqual({mantissa: "-600000", places: 2});
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
                    postings: [{account: "assets:cash", amount: {commodity: "$", quantity: dec(200, 0), precision: 0}, comment: ""}],
                    counter: null,
                    comment: "",
                },
                {
                    id: "e2",
                    date: "2027-03-01",
                    description: "half typed",
                    postings: [{account: "", amount: {commodity: "$", quantity: dec(0, 0), precision: 0}, comment: ""}],
                    counter: null,
                    comment: "",
                },
                {
                    id: "e3",
                    date: "",
                    description: "no date",
                    postings: [{account: "assets:cash", amount: {commodity: "$", quantity: dec(1, 0), precision: 0}, comment: ""}],
                    counter: null,
                    comment: "",
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
            accounts: {empty: true},
            journal: true,
        });
        expect(asked.accounts).toEqual({empty: true});
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
                        Object.freeze({
                            account: "assets:cash",
                            amount: Object.freeze({commodity: "$", quantity: Object.freeze(dec(200, 0)), precision: 0}),
                            comment: "a",
                        }),
                    ]),
                    comment: "the raise",
                }),
            ]),
            layout: Object.freeze([
                Object.freeze({kind: "rule" as const, key: "rule:0", origin: 0, disabled: false, attached: "; rent\n", raw: "; rent\n~ monthly\n"}),
            ]),
            comments: Object.freeze([Object.freeze({key: "comment:0", style: "note" as const, raw: "; hi\n", text: "hi"})]),
            fixed: Object.freeze([Object.freeze({key: "fixed:0", raw: "account x\n"})]),
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
                        forecastOnly: true,
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
            clone.layout,
            clone.layout[0],
            clone.comments[0],
            clone.fixed[0],
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

describe("UNIT scenarioModel — counter-accounts (plan 28)", () => {
    it("trims a typed account, and blank means the engine's default", () => {
        expect(accountOrNull("  assets:checking ")).toBe("assets:checking");
        expect(accountOrNull("   ")).toBeNull();
        expect(accountOrNull(null)).toBeNull();
    });

    it("sends trimmed accounts, and blank as null, so ' ' is never an account name on the wire", () => {
        const wire = scenarioToWire({
            ...scenario([line({counter: " assets:checking ", gains: ""})]),
            cash: "  ",
            events: [{id: "e", date: "2027-01-01", description: "", postings: [], counter: "", comment: ""}],
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

describe("UNIT scenarioModel — descriptions", () => {
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
        expect(noteIsStale(withQuantity(seeded, dec(420000, 2)))).toBe(false);
        expect(noteIsStale(withQuantity(seeded, dec(5000, 0)))).toBe(true);
        // A sign flip is a different figure too.
        expect(noteIsStale(withQuantity(seeded, dec(-420000, 2)))).toBe(true);
        // A description the user wrote is theirs, whatever the amount does.
        expect(noteIsStale(withQuantity({...seeded, note: "rent"}, dec(5000, 0)))).toBe(false);
    });
});

describe("UNIT scenarioModel — automatic posting rules (plan 30 Phase 4)", () => {
    const form = () => ({
        matchAccount: "expenses:rent",
        extraQuery: "",
        postings: [{account: "expenses:rent-tax", kind: "percent" as const, percent: dec(10, 0), amount: null}],
        counter: "assets:checking",
        comment: "",
        forecastOnly: true,
    });
    const read = (block: number) => ({
        raw: `= expenses:rent\n    expenses:rent-tax  *0.${block + 1}\n`,
        query: "expenses:rent",
        unsupported: null,
        form: form(),
        block,
        key: `auto:${block}`,
    });

    it("sends an untouched rule as its text and an edited or new one as its form, each with its block", () => {
        const locked = {...read(2), form: null, edited: true};
        const wire = scenarioToWire({...emptyScenario(), autoRules: [read(0), {...read(1), edited: true}, blankAutoRule("assets:cash"), locked]});
        expect(wire.autoRules).toEqual([
            {block: 0, key: "auto:0", raw: read(0).raw},
            {
                block: 1,
                key: "auto:1",
                form: {
                    matchAccount: "expenses:rent",
                    extraQuery: "",
                    postings: [{account: "expenses:rent-tax", kind: "percent", percent: {mantissa: "10", places: 0}, amount: null}],
                    counter: "assets:checking",
                    comment: "",
                    forecastOnly: true,
                },
            },
            {
                block: null,
                // Keyless, it is given one: the layout names every rule by its key.
                key: "auto:new-1",
                form: {
                    matchAccount: "",
                    extraQuery: "",
                    postings: [{account: "", kind: "percent", percent: null, amount: null}],
                    counter: "assets:cash",
                    comment: "",
                    forecastOnly: true,
                },
            },
            // A rule the form cannot state always goes back as its text.
            {block: 2, key: "auto:2", raw: locked.raw},
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
                    comment: "",
                    postings: [{account: "income:bonus", amount: {commodity: "$", quantity: dec(500, 0), precision: 0}, comment: ""}],
                },
            ],
        };
        expect(ruleSample(withEvent, "revenue:sales")).toEqual(sales.amount);
        expect(ruleSample(withEvent, "revenue:sale")).toBeNull();
        expect(ruleSample(withEvent, "income")?.quantity).toEqual(dec(500, 0));
        expect(ruleSample(withEvent, " ")).toBeNull();
    });
});

describe("UNIT scenarioModel — the file's pieces on the wire (plan 31 Phase C2)", () => {
    /** A scenario as a file serves it: a live rule, a disabled one, a disabled one-off and `=` rule, a note. */
    function served(): Scenario {
        const event = {...blankEvent("off:1", "2027-03-01", "$"), comment: "maybe"};
        return {
            ...emptyScenario(),
            lines: [line({comment: "owner: alex", ruleComment: "employer: acme"}), line({id: "off:0:0", group: "off:0", account: "revenues:bonus"})],
            events: [{...event, postings: [{...event.postings[0], account: "expenses:legal", comment: "retainer"}]}],
            autoRules: [
                {raw: "= expenses:rent\n    (x)  *0.1\n", query: "expenses:rent", unsupported: null, form: null, block: null, key: "auto:off:0"},
                {raw: "= expenses:rent\n    (y)  *0.2\n", query: "expenses:rent", unsupported: null, form: null, block: 0, key: "auto:0"},
            ],
            layout: [
                {kind: "comment", key: "comment:0", origin: 0, disabled: false, attached: "", raw: "; a note\n"},
                {kind: "rule", key: "rule:0", origin: 1, disabled: false, attached: "; rent\n", raw: "; rent\n~ monthly\n"},
                {kind: "rule", key: "off:0", origin: 2, disabled: true, attached: "", raw: "; ~ monthly\n"},
                {kind: "event", key: "off:1", origin: 3, disabled: true, attached: "", raw: "; ~ 2027-03-01\n"},
                {kind: "auto", key: "auto:off:0", origin: 4, disabled: true, attached: "", raw: "; = expenses:rent\n"},
                {kind: "auto", key: "auto:0", origin: 5, disabled: false, attached: "", raw: "= expenses:rent\n"},
            ],
            comments: [{key: "comment:0", style: "note", raw: "; a note\n", text: "a note"}],
            fixed: [],
        };
    }

    it("the active scenario drops every disabled entry and its layout item, as the engine's does", () => {
        const active = activeScenario(served());
        expect(active.lines.map((l) => l.group)).toEqual(["rule:0"]);
        expect(active.events).toEqual([]);
        expect(active.autoRules.map((r) => r.key)).toEqual(["auto:0"]);
        expect(active.layout.map((item) => item.key)).toEqual(["comment:0", "rule:0", "auto:0"]);
        expect(active.comments).toEqual(served().comments);
        // Nothing disabled: the very same scenario.
        expect(activeScenario(active)).toBe(active);
    });

    it("a run never carries a disabled entry, nor its layout item", () => {
        const body = runBody(served(), {interval: "monthly", count: 3, lookback: 0, depth: 2});
        expect(body.scenario.lines.map((l) => l.group)).toEqual(["rule:0"]);
        expect(body.scenario.events).toEqual([]);
        expect(body.scenario.autoRules.map((r) => r.key)).toEqual(["auto:0"]);
        expect(projectable(served()).layout.some((item) => item.disabled)).toBe(false);
    });

    it("a save sends every comment inside a block, each rule's key, and the whole layout in the list's order (plan 31 C3)", () => {
        const wire = scenarioToWire(cloneScenario(served()));
        expect(wire.lines[0]).toMatchObject({comment: "owner: alex", ruleComment: "employer: acme"});
        expect(wire.events[0]).toMatchObject({comment: "maybe"});
        expect(wire.events[0].postings[0].comment).toBe("retainer");
        expect(wire.autoRules.map((rule) => rule.key)).toEqual(["auto:off:0", "auto:0"]);
        // Every item, disabled flags and attached comments as served, `raw` left behind.
        expect(wire.layout?.map((item) => [item.kind, item.key, item.disabled])).toEqual([
            ["comment", "comment:0", false],
            ["rule", "rule:0", false],
            ["rule", "off:0", true],
            ["event", "off:1", true],
            ["auto", "auto:off:0", true],
            ["auto", "auto:0", false],
        ]);
        expect(wire.layout?.[1]).toEqual({kind: "rule", key: "rule:0", origin: 1, disabled: false, attached: "; rent\n"});
        expect(wire.comments).toEqual([{key: "comment:0", raw: "; a note\n"}]);
        expect(wire.fixed).toEqual([]);
    });

    it("an edited attached comment goes as its text, and an edited comment as text with its old raw", () => {
        const scenario = served();
        scenario.layout[1] = {...scenario.layout[1], attachedText: "Rent, revised."};
        const wire = scenarioToWire(scenario);
        expect(wire.layout?.[1]).toEqual({kind: "rule", key: "rule:0", origin: 1, disabled: false, attachedText: "Rent, revised."});
        expect(commentToWire({key: "comment:0", style: "section", raw: ";;;;\n;; Old\n", text: "New", edited: true})).toEqual({
            key: "comment:0",
            style: "section",
            text: "New",
            previous: ";;;;\n;; Old\n",
        });
        // A new comment has nothing to keep the framing of: the engine writes the house style.
        expect(commentToWire({key: "comment-1", style: "section", raw: "", text: "Capital", edited: true})).toEqual({
            key: "comment-1",
            style: "section",
            text: "Capital",
        });
    });

    it("a scenario with no layout is given the derived one, in the engine's legacy order", () => {
        const plain = {
            ...scenario([line({group: "g-b"}), line({id: "x", group: "g-a"}), line({id: "y", group: "g-b"})]),
            events: [blankEvent("event-1", "2027-01-01", "$")],
            autoRules: [blankAutoRule(null)],
        };
        const derived = withLayout(plain);
        expect(derived.layout.map((item) => [item.kind, item.key])).toEqual([
            ["rule", "g-b"],
            ["rule", "g-a"],
            ["event", "event-1"],
            ["auto", "auto:new-1"],
        ]);
        expect(derived.autoRules[0].key).toBe("auto:new-1");
        expect(withLayout(derived)).toBe(derived);
    });

    it("a run prunes the layout with the entries it drops, so the body still names each one exactly once", () => {
        const half = {...served(), lines: [...served().lines, {...blankLine("line-9", "$"), group: "g-new"}]};
        half.layout = [...half.layout, {kind: "rule", key: "g-new", origin: null, disabled: false, attached: "", raw: ""}];
        const body = runBody(half, {interval: "monthly", count: 3, lookback: 0, depth: 2});
        expect(body.scenario.layout?.map((item) => item.key)).toEqual(["comment:0", "rule:0", "auto:0"]);
    });

    it("a render body is the whole scenario with ONE layout item, the entry's own", () => {
        const body = renderBody(served(), "rule:0");
        expect(body.key).toBe("rule:0");
        expect(body.scenario.layout).toEqual([{kind: "rule", key: "rule:0", origin: 1, disabled: false, attached: "; rent\n"}]);
        expect(body.scenario.lines).toHaveLength(2);
        expect(body.scenario.comments).toEqual([{key: "comment:0", raw: "; a note\n"}]);
        // A draft's item overrides the scenario's; an entry with none sends none.
        const draft = {kind: "rule", key: "rule:0", origin: 1, disabled: true, attachedText: "Rent."};
        expect(renderBody(served(), "rule:0", draft).scenario.layout).toEqual([draft]);
        expect(renderBody(emptyScenario(), "new").scenario.layout).toEqual([]);
    });
});
