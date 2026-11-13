// The Budget tab's bars as the screen reads them — words and geometry, from the
// engine's verdicts (plan 26 B6), for the Banktivity-style table that draws
// them (`BudgetTable.svelte`, plan 41) at every width.
//
// Nothing here decides a verdict: the engine already has. This picks the
// sentence for the engine's label kind, formats amounts with the journal's
// styles, and sizes bars (`barGeometry`).

import type {BudgetSortKey, SortDir} from "#lib/budget/sort.js";
import {sortBudgetLines} from "#lib/budget/sort.js";
import type {MixedAmount} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {formatTotals} from "#lib/journal/rowModel.js";
import {barGeometry, primaryValue, type BarGeometry} from "#lib/reports/budgetSummary.js";
import type {BudgetBar as WireBar, BudgetHealth, BudgetLabelKind, BudgetReport, BudgetSummarySection} from "#lib/reports/types.js";

/** Which way round a section reads: earning toward a target, or spending against a budget. */
export type BudgetSectionKind = BudgetSummarySection["kind"];

/** One bar, ready to render. */
export interface BudgetBarView {
    account: string;
    spentText: string;
    budgetText: string;
    /** The engine's sentence, amount included ("$170.00 left"). */
    remainderText: string;
    /** The engine's label kind, for a layout that styles "over" apart from "behind pace". */
    labelKind: BudgetLabelKind;
    /** null for a multi-commodity line, which has no single comparison to make. */
    health: BudgetHealth | null;
    pct: number | null;
    geom: BarGeometry | null;
    /** The goal in magnitude, so the sort control can order bars by it. */
    goal: MixedAmount;
}

/** One section's header bar and its rows. */
export interface BudgetSectionView {
    kind: BudgetSectionKind;
    title: string;
    verbSpent: string; // "Earned" / "Spent"
    verbOf: string; // "target" / "budgeted"
    overall: BudgetBarView;
    bars: BudgetBarView[];
}

const WORDS: Record<BudgetSectionKind, {title: string; verbSpent: string; verbOf: string}> = {
    revenue: {title: "Income", verbSpent: "Earned", verbOf: "target"},
    expense: {title: "Expenses", verbSpent: "Spent", verbOf: "budgeted"},
};

/** One MixedAmount → a compact string ("$352", or "$1, €2" multi-commodity; "0" when empty). */
export function formatBudgetAmount(ma: MixedAmount, styles: ReadonlyMap<string, AmountStyle>): string {
    const parts = formatTotals(ma, styles).map((l) => l.text);
    return parts.length > 0 ? parts.join(", ") : "0";
}

/** The words for a bar's state, in the vocabulary plans/20 fixes. The engine picks the sentence and its amount. */
export function labelText(label: WireBar["label"], styles: ReadonlyMap<string, AmountStyle>): string {
    const amount = label.amount === null ? "" : formatBudgetAmount(label.amount, styles);
    switch (label.kind) {
        case "onPlan":
            return "on plan";
        case "behindPace":
            return `${amount} behind pace`;
        case "aboveTarget":
            return `${amount} above target`;
        case "toGo":
            return `${amount} to go`;
        case "over":
            return `${amount} over`;
        case "aheadOfPace":
            return `${amount} ahead of pace`;
        case "left":
            return `${amount} left`;
    }
}

/** An engine bar → what a row renders. Geometry is the only arithmetic, and it is pixels. */
export function toBarView(bar: WireBar, styles: ReadonlyMap<string, AmountStyle>): BudgetBarView {
    // A bar with no health has no single magnitude to draw (multi-commodity):
    // the engine withheld the verdict, so no geometry either.
    const spent = primaryValue(bar.actual);
    const budget = primaryValue(bar.goal);
    const pace = primaryValue(bar.pace);
    const geom = bar.health !== null && spent !== null && budget !== null && pace !== null ? barGeometry(spent, budget, pace) : null;
    return {
        account: bar.account,
        spentText: formatBudgetAmount(bar.actual, styles),
        budgetText: formatBudgetAmount(bar.goal, styles),
        remainderText: labelText(bar.label, styles),
        labelKind: bar.label.kind,
        health: bar.health,
        pct: geom !== null && geom.ratio !== null ? Math.round(geom.ratio * 100) : null,
        geom,
        goal: bar.goal,
    };
}

/**
 * The report's sections, each sorted by the page's one order control.
 *
 * A bar's goal is ALREADY the span total, so it is comparable as it stands —
 * the editor sorts by the engine's annualised figures instead.
 */
export function budgetSections(report: BudgetReport, styles: ReadonlyMap<string, AmountStyle>, sort: BudgetSortKey, dir: SortDir): BudgetSectionView[] {
    return report.summary.sections.map((section) => ({
        kind: section.kind,
        ...WORDS[section.kind],
        overall: toBarView(section.overall, styles),
        bars: sortBudgetLines(
            section.bars.map((bar) => toBarView(bar, styles)),
            sort,
            dir,
            (bar) => ({account: bar.account, amount: bar.goal})
        ),
    }));
}

/**
 * The exact dates the bars cover, not just the month names: the bars cover
 * whole months, so when the controls asked for a partial one this is the only
 * thing telling the user the bar (and the journal link) is wider than what
 * they typed.
 */
export function budgetPeriodLabel(report: BudgetReport, from: string, to: string): string {
    if (report.buckets.length === 0) return "";
    const first = report.periods[0].label;
    if (report.buckets.length === 1) return `${first} (${from} – ${to})`;
    return `${first} – ${report.periods[report.periods.length - 1].label} (${from} – ${to})`;
}
