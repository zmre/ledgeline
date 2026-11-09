// The Balances tab's balance line (plan 36 amendment A1), as pure functions:
// labels, the heading's window, the tooltip's lines and the failure markers.
// No Svelte runtime, so node-testable; `BalanceTrend.svelte` draws them.
//
// Every number here is the ENGINE's (`/api/journal/running/series`). What
// lives here is only how they are shown.

import {toNumber, type Dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {trendLabels} from "#lib/holdings/ui/trendView.js";
import {assertionDetail} from "#lib/journal/runningBalance.js";
import type {BalanceSeries, BalanceSeriesFailure} from "#lib/journalView/types.js";
import {MONTH_NAMES} from "#lib/reports/periods.js";

/**
 * The x labels: the holdings chart's rule, so the two balance-over-time lines
 * read alike — days and weeks by their date ("Mar 2"), months and coarser by
 * the engine's label ("Mar 2026").
 */
export function balanceLabels(series: BalanceSeries): string[] {
    return trendLabels(series.points.map((p) => ({bucket: p.bucket, label: p.label, date: p.end})));
}

function shortDate(date: string, withYear: boolean): string {
    const month = MONTH_NAMES[Number(date.slice(5, 7)) - 1] ?? date.slice(5, 7);
    const day = Number(date.slice(8, 10));
    return withYear ? `${month} ${day}, ${date.slice(0, 4)}` : `${month} ${day}`;
}

/** "Mar 1 – Nov 9, 2026", or "Dec 1, 2025 – Mar 1, 2026" across a year end. */
export function windowNote(from: string, to: string): string {
    if (from === to) return shortDate(to, true);
    const sameYear = from.slice(0, 4) === to.slice(0, 4);
    return `${shortDate(from, !sameYear)} – ${shortDate(to, true)}`;
}

/** "$ · Mar 1 – Nov 9, 2026": what sits beside the account in the heading. */
export function trendNote(series: BalanceSeries): string {
    return `${series.commodity} · ${windowNote(series.from, series.to)}`;
}

function failureLine(failure: BalanceSeriesFailure, commodity: string, style: AmountStyle): string {
    const amount = (qty: Dec) => ({commodity, qty, style});
    const detail = assertionDetail({
        account: failure.account,
        op: failure.op,
        asserted: amount(failure.asserted),
        calculated: amount(failure.calculated),
        ok: false,
    });
    return `✗ ${failure.date} ${detail}`;
}

/**
 * The tooltip's lines under the balance: the day it is taken at, then each
 * failing assertion in the period with what it asserted, what was found and
 * the difference (the Balance column's own wording).
 */
export function periodLines(series: BalanceSeries, index: number): string[] {
    const point = series.points[index];
    if (point === undefined) return [];
    return [`at end of ${point.end}`, ...point.failures.map((failure) => failureLine(failure, series.commodity, series.style))];
}

/** One failing period's marker: the line's point, and where the assertion says it should be. */
export interface FailureMarker {
    index: number;
    /** The balance at the period's end: where the line is. */
    balance: number;
    /** Where the line would be had the books matched the assertion. */
    expected: number;
    /** One line per failing assertion, for the marker's own `<title>`. */
    title: string;
}

/**
 * One marker per period holding a failing assertion. Several failures in one
 * period draw one marker (the line point is one point); its hairline runs to
 * the LARGEST gap, so the worst disagreement is the one you see, and its title
 * lists every one.
 */
export function failureMarkers(series: BalanceSeries): FailureMarker[] {
    return series.points.flatMap((point, index): FailureMarker[] => {
        if (point.failures.length === 0) return [];
        const balance = toNumber(point.balance);
        const expected = point.failures
            .map((failure) => toNumber(failure.expected))
            .reduce((worst, value) => (Math.abs(value - balance) > Math.abs(worst - balance) ? value : worst));
        const title = point.failures.map((failure) => failureLine(failure, series.commodity, series.style)).join("\n");
        return [{index, balance, expected, title}];
    });
}

/** The values the markers draw, which the chart's y-domain must also fit. */
export function markerExtent(markers: readonly FailureMarker[]): number[] {
    return markers.map((marker) => marker.expected);
}

/** The transaction a click on period `index` scrolls to, or null when nothing posted in it. */
export function periodTarget(series: BalanceSeries, index: number): number | null {
    return series.points[index]?.tindex ?? null;
}

/** The plotted values, one per period. */
export function balanceValues(series: BalanceSeries): number[] {
    return series.points.map((point) => toNumber(point.balance));
}
