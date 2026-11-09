import {describe, expect, it} from "vitest";
import {dec} from "#lib/domain/money.js";
import {compareFixture, usd} from "#lib/testing/projectionFixture.js";
import {comparisonWarnings, compareRunwayStatement, dayLabel, untouchedSeedLines} from "./compareView";
import {blankLine, emptyScenario, withQuantity} from "./scenarioModel";
import type {Scenario} from "./types";

const format = (n: number): string => `$${n.toLocaleString("en-US", {maximumFractionDigits: 2})}`;

/** Words the plan rules out for a gap between actual and projected. */
const JUDGEMENTS = ["miss", "error", "good", "bad"];

describe("UNIT projections/compareView", () => {
    it("labels the day the projection is made on", () => {
        expect(dayLabel("2026-07-01")).toBe("1 Jul 2026");
        expect(dayLabel("2026-12-25")).toBe("25 Dec 2026");
        expect(dayLabel("someday")).toBe("someday");
    });

    it("says how the last actual period's cash compares, in words, without judging it", () => {
        const above = compareRunwayStatement(compareFixture(), "$", format);
        expect(above.headline).toBe("Cash at the end of Jul 2026 was $10,000,");
        expect(above.detail).toBe("$500 (5.3%) more than projected on 1 Jun 2026.");
        const below = compareRunwayStatement(
            compareFixture({cash: {opening: usd(0), actual: [usd(1), usd(31850)], projected: [usd(1), usd(33700)]}}),
            "$",
            format
        );
        expect(below.detail).toBe("$1,850 (5.5%) less than projected on 1 Jun 2026.");
        const same = compareRunwayStatement(compareFixture({cash: {opening: usd(0), actual: [usd(1), usd(5)], projected: [usd(2), usd(5)]}}), "$", format);
        expect(same.detail).toBe("the same as projected on 1 Jun 2026.");
        for (const statement of [above, below, same]) {
            const text = `${statement.headline} ${statement.detail}`.toLowerCase();
            for (const word of JUDGEMENTS) expect(text).not.toContain(word);
        }
    });

    it("prefixes the engine's lines and adds the seed lookahead line for untouched averages", () => {
        // As the seed serves it: never edited, so no `served` kept.
        const seeded = {
            ...withQuantity({...blankLine("avg", "$"), account: "expenses:food"}, dec(50000, 2)),
            group: "rule:0",
            source: "unbudgeted" as const,
            served: undefined,
        };
        const scenario: Scenario = {...emptyScenario(), lines: [seeded]};
        expect(untouchedSeedLines(scenario)).toBe(1);
        const warnings = comparisonWarnings(compareFixture({warnings: ["valued in EUR"]}), scenario);
        expect(warnings[0]).toBe("Comparison: valued in EUR");
        expect(warnings[1]).toMatch(/^Comparison: 1 row is an average of the last 12 months, which include the periods being compared/);
        // Edited: no longer an untouched average.
        const edited = {...withQuantity(seeded, dec(60000, 2)), served: dec(50000, 2)};
        expect(untouchedSeedLines({...scenario, lines: [edited]})).toBe(0);
        expect(comparisonWarnings(compareFixture(), emptyScenario())).toEqual([]);
    });
});
