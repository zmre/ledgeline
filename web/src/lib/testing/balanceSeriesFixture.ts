// One `/api/journal/running/series` body, as the engine sends it (plan 36
// amendment A1), shared by the decoder, BalanceTrend, BalancesView and
// InsightsPanel tests so they all read the same bytes.
//
// `assets:bank`, weekly over Jan 2026: an opening of $6,000.00, a dip, and a
// failing `= $300.00` on checking in the week of Jan 31 that found $220.25 —
// so the line (at $5,402.25) "should" have been $79.75 higher, at $5,482.00.
// The last week has no posting, so no `tindex` to scroll to.

import {decodeBalanceSeries} from "#lib/api/journalViewDecode.js";
import type {BalanceSeries} from "#lib/journalView/types.js";

const USD_WIRE = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const cents = (m: number) => ({m: String(m), p: 2});

export function balanceSeriesWire(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        generation: "fake-1",
        account: "assets:bank",
        commodity: "$",
        style: USD_WIRE,
        interval: "weekly",
        from: "2026-01-01",
        to: "2026-02-03",
        points: [
            {bucket: "2026-W01", label: "W01 2026", end: "2026-01-04", balance: cents(600_000), tindex: 1},
            {bucket: "2026-W02", label: "W02 2026", end: "2026-01-11", balance: cents(598_425), tindex: 16},
            {bucket: "2026-W03", label: "W03 2026", end: "2026-01-18", balance: cents(592_225), tindex: 9},
            {bucket: "2026-W04", label: "W04 2026", end: "2026-01-25", balance: cents(540_225), tindex: 6},
            {
                bucket: "2026-W05",
                label: "W05 2026",
                end: "2026-02-01",
                balance: cents(540_225),
                tindex: 12,
                failures: [
                    {
                        tindex: 11,
                        date: "2026-01-31",
                        account: "assets:bank:checking",
                        op: "=",
                        asserted: cents(30_000),
                        calculated: cents(22_025),
                        expected: cents(548_200),
                    },
                ],
            },
            {bucket: "2026-W06", label: "W06 2026", end: "2026-02-03", balance: cents(540_225)},
        ],
        ...overrides,
    };
}

/** The same body, decoded. */
export function balanceSeries(overrides: Record<string, unknown> = {}): BalanceSeries {
    return decodeBalanceSeries(balanceSeriesWire(overrides));
}
