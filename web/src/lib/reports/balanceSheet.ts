// Balance sheet (WP-06). Pure TS: no Svelte/DOM imports.

import {accountTotals, atDepth, rollUp} from "../domain/aggregate";
import {maAdd, maNeg} from "../domain/money";
import type {ISODate, Transaction} from "../domain/types";
import {buildSection} from "./sections";
import type {SectionedReport} from "./types";

/**
 * Asset + liability balances as of `asOf` (INCLUSIVE: postings dated ≤ asOf).
 * hledger's `-e DATE` is exclusive, so `hledger bs -e D` ≙ `balanceSheet(txns, {asOf: dayBefore(D)})`.
 *
 * Presentation matches `hledger bs`: liabilities are sign-flipped (positive =
 * owed); `grandTotal` = assets − liabilities(displayed) = net.
 * Equity section: post-MVP flag (see plans/06-reports-engine.md).
 */
export function balanceSheet(txns: Transaction[], opts: {asOf: ISODate; depth: number}): SectionedReport {
    const direct = accountTotals(txns, {to: opts.asOf});
    const clamped = atDepth(rollUp(direct), opts.depth);
    const assets = buildSection("Assets", "asset", direct, clamped, false);
    const liabilities = buildSection("Liabilities", "liability", direct, clamped, true);
    return {asOf: opts.asOf, sections: [assets, liabilities], grandTotal: maAdd(assets.total, maNeg(liabilities.total))};
}
