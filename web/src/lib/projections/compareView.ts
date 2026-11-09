// What the Projections page says while comparing a projection with actuals
// (plans/34-projection-backtest.md): the "as projected from" date, the
// runway's sentence, and the comparison's warnings. Pure, so each is tested
// under node.
//
// Wording (Patrick, 2026-11-09): a gap between actual and projected is a
// DIFFERENCE, "above" or "below" / "more" or "less". Never a miss or an
// error, and never good or bad: beating the projection is as interesting as
// falling short of it.

import {cmp, type Dec} from "#lib/domain/money.js";
import {MONTH_NAMES} from "#lib/reports/periods.js";
import {compareCells} from "#lib/reports/ui/compareColumns.js";
import {fmtPct} from "#lib/reports/ui/incomeStatementRows.js";
import {activeScenario} from "./scenarioModel";
import type {Comparison, Scenario} from "./types";
import type {RunwayStatement} from "./projectionView";

/** `2026-07-01` → `1 Jul 2026`, the day a comparison's projection is made on. */
export function dayLabel(iso: string): string {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
    if (match === null) return iso;
    return `${Number(match[3])} ${MONTH_NAMES[Number(match[2]) - 1] ?? match[2]} ${match[1]}`;
}

/** The runway's legend name for the projected line, and the band's middle heading. */
export function asProjectedLabel(compare: Comparison): string {
    return `As projected from ${dayLabel(compare.from)}`;
}

/**
 * The runway's sentence while comparing: "Cash at the end of Sep 2026 was
 * $31,850, $1,850 (5.5%) less than projected on 1 Jul 2026." Read off the last
 * actual period's closing cash and the projection's, in `commodity`.
 */
export function compareRunwayStatement(compare: Comparison, commodity: string, format: (n: number) => string): RunwayStatement {
    const n = compare.columns;
    const lastLabel = compare.balanceSheet.columns[n - 1]?.label ?? "the window";
    const cells = compareCells([...compare.cash.actual, ...compare.cash.projected], n, "balance", commodity);
    const amount = (dec: Dec | undefined): number => (dec === undefined ? 0 : Number(dec.m) / 10 ** dec.p);
    const actual = amount(cells.actual.get(commodity));
    const delta = cells.delta.get(commodity);
    const on = dayLabel(compare.from);
    const headline = `Cash at the end of ${lastLabel} was ${format(actual)},`;
    if (delta === undefined || delta.m === 0n) {
        return {tone: "ok", headline, detail: `the same as projected on ${on}.`};
    }
    const size = format(Math.abs(amount(delta)));
    const pct = cells.pct === null ? "" : ` (${fmtPct(Math.abs(cells.pct))})`;
    const way = cmp(delta, {m: 0n, p: 0}) > 0 ? "more" : "less";
    return {tone: "ok", headline, detail: `${size}${pct} ${way} than projected on ${on}.`};
}

/** How many active rules are untouched seed averages: the lookahead the backtest cannot see (D7). */
export function untouchedSeedLines(scenario: Scenario): number {
    const groups = new Set(
        activeScenario(scenario)
            .lines.filter((line) => line.source === "unbudgeted" && (line.served === undefined || cmp(line.served, line.amount.quantity) === 0))
            .map((line) => line.group)
    );
    return groups.size;
}

/**
 * The comparison's lines for the page's warnings alert, each prefixed
 * "Comparison: ": the engine's (the as-projected run's own, and a valuation
 * target that changed inside the window), then the seed lookahead line (D7),
 * which only the page can see. Only a warning: the scenario is never re-seeded.
 */
export function comparisonWarnings(compare: Comparison, scenario: Scenario): string[] {
    const seeded = untouchedSeedLines(scenario);
    const lookahead =
        seeded === 0
            ? []
            : [
                  `${seeded} ${seeded === 1 ? "row is an average" : "rows are averages"} of the last 12 months, which include the periods being compared, so ${
                      seeded === 1 ? "it" : "they"
                  } will look more accurate than a real forecast would have been.`,
              ];
    return [...compare.warnings, ...lookahead].map((warning) => `Comparison: ${warning}`);
}
