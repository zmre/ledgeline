// The columns of a period report's table (cash flow, net worth) — shared by
// `ReportTable.svelte` and the xlsx export's `addPeriod`, so the workbook cannot
// grow a Total column the screen does not show, or drop one it does.
//
// A Total column exists only when the report CARRIES totals (`grandTotal`,
// cash flow only, plans/24). Net worth's columns are point-in-time snapshots,
// and summing them would mean nothing, so its table is exactly what it was.

import type {MixedAmount} from "#lib/domain/money.js";
import {bucketLabel} from "#lib/reports/periods.js";
import type {PeriodReport, PeriodRow} from "#lib/reports/types.js";

export interface PeriodTable {
    /** One heading per figure column, in order. */
    headers: string[];
    /** A row's figures, one per heading. */
    cells: (row: PeriodRow) => MixedAmount[];
    /** The Net row's figures, one per heading. */
    net: MixedAmount[];
}

/**
 * One column per bucket, then a Total when the report carries one.
 *
 * With exactly ONE bucket (cash flow's Interval = Total) a Total column would
 * repeat it beside itself, so the table prints the one column, headed "Total".
 */
export function periodTable(report: PeriodReport): PeriodTable {
    const grand = report.grandTotal;
    const total = (row: PeriodRow): MixedAmount => row.total ?? new Map();
    if (grand === undefined) {
        return {headers: report.buckets.map(bucketLabel), cells: (row) => row.values, net: report.totals};
    }
    if (report.buckets.length === 1) {
        return {headers: ["Total"], cells: (row) => [total(row)], net: [grand]};
    }
    return {
        headers: [...report.buckets.map(bucketLabel), "Total"],
        cells: (row) => [...row.values, total(row)],
        net: [...report.totals, grand],
    };
}
