// Income statement / P&L (WP-06). Pure TS: no Svelte/DOM imports.

import {accountTotals, atDepth, rollUp} from "../domain/aggregate";
import {maAdd, maNeg} from "../domain/money";
import type {ISODate, Transaction} from "../domain/types";
import {buildSection} from "./sections";
import type {SectionedReport} from "./types";

/**
 * Revenues + expenses over [from, to] (both INCLUSIVE: hledger's
 * `is -b B -e E` ≙ `incomeStatement(txns, {from: B, to: dayBefore(E), ...})`).
 *
 * Presentation matches `hledger is`: revenues are sign-flipped (positive =
 * earned); `grandTotal` = revenues(displayed) − expenses = net income.
 */
export function incomeStatement(txns: Transaction[], opts: {from: ISODate; to: ISODate; depth: number}): SectionedReport {
    const direct = accountTotals(txns, {from: opts.from, to: opts.to});
    const clamped = atDepth(rollUp(direct), opts.depth);
    const revenues = buildSection("Revenues", "revenue", direct, clamped, true);
    const expenses = buildSection("Expenses", "expense", direct, clamped, false);
    return {from: opts.from, to: opts.to, sections: [revenues, expenses], grandTotal: maAdd(revenues.total, maNeg(expenses.total))};
}
