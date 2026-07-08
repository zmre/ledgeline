// Internal helper shared by the sectioned reports (balanceSheet/incomeStatement).
// Not part of the WP-06 public contract. Pure TS: no Svelte/DOM imports.

import {categorize, type RootCategory} from "../domain/accounts";
import {maAdd, maNeg, type MixedAmount} from "../domain/money";
import type {ReportRow, Section} from "./types";

/**
 * Build one report section from aggregated totals.
 *
 * @param direct  full-account-name direct totals (accountTotals output)
 * @param clamped rolled-up totals already clamped to the report depth (rollUp → atDepth)
 * @param flip    present the section sign-flipped (liabilities on bs, revenues on is —
 *                internally negative, displayed positive, hledger-style)
 */
export function buildSection(
    title: string,
    category: RootCategory,
    direct: Map<string, MixedAmount>,
    clamped: Map<string, MixedAmount>,
    flip: boolean
): Section {
    const rows: ReportRow[] = [];
    let total: MixedAmount = new Map();
    const accounts = [...clamped.keys()].filter((account) => categorize(account) === category).sort();
    for (const account of accounts) {
        const depth = account.split(":").length;
        const inclusive: MixedAmount = clamped.get(account) ?? new Map();
        const own: MixedAmount = direct.get(account) ?? new Map();
        rows.push({account, depth, own: flip ? maNeg(own) : own, inclusive: flip ? maNeg(inclusive) : inclusive});
        if (depth === 1) total = maAdd(total, inclusive); // roots carry the whole subtree
    }
    return {title, rows, total: flip ? maNeg(total) : total};
}
