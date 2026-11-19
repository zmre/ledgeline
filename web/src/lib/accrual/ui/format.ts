// Display helpers for the accrual UI's components (plan 45 Phase 3), kept in
// TS so tests and other modules can import them.

import type {AccrualStatus, SettleProposal} from "../types";

const ROLE_NOUN: Record<string, string> = {
    "deferred-revenue": "deferred revenue",
    prepaid: "prepaid expense",
    receivable: "receivable",
    payable: "payable",
};

/** Why the helpers are on (D-gate, Risks "basis inference false positives"). */
export function reasonText(status: AccrualStatus): string {
    const {reason} = status;
    const noun = reason.role === null ? "accrual account" : (ROLE_NOUN[reason.role] ?? reason.role);
    switch (reason.kind) {
        case "basis":
            return 'On because ledgeline.toml sets [books] basis = "accrual".';
        case "declared":
            return `On because ${reason.account ?? "an account"} is declared a ${noun}.`;
        case "inferred":
            return `On because ${reason.account ?? "an account"} looks like a ${noun}. Set [books] basis = "cash" in ledgeline.toml to turn this off.`;
        default:
            return "On for this ledger.";
    }
}

/** A lookback in days as the ▾ says it. */
export function lookbackLabel(days: number): string {
    if (days === 0) return "All time";
    if (days % 365 === 0) return days === 365 ? "1 year" : `${days / 365} years`;
    return `${days} days`;
}

/** A proposal's identity: its two postings. */
export function proposalKey(proposal: SettleProposal): string {
    return `${proposal.invoice.index}:${proposal.invoice.posting}-${proposal.payment.index}:${proposal.payment.posting}`;
}

/** Basis points as a percent with one decimal: 300 → "3.0". */
export function bpsToPercent(bps: number): string {
    return (bps / 100).toFixed(1);
}

/** A typed percent as basis points, or null when it isn't a number. */
export function percentToBps(text: string): number | null {
    const value = Number(text.trim().replace(/%$/, ""));
    if (text.trim() === "" || !Number.isFinite(value)) return null;
    return Math.round(value * 100);
}
