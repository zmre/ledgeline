// Problem types and the two groupings every consumer shares (WP-08). Pure TS —
// no Svelte/DOM imports.
//
// The checks themselves run in the engine now (plan 26 B4): `/api/diagnostics`
// serves the engine diagnostics AND the five rules that used to live in
// `./rules.ts` (pending, uncategorized, negative-cash, missing-description,
// future-date), as of the browser's local date. See
// crates/ledgeline-core/src/view/checks.rs.

export type Severity = "error" | "warning" | "info";

/**
 * One finding, anchored to EITHER a transaction or an account — never both, and
 * never neither.
 *
 * `txnIndex` is hledger's 1-based `Transaction.index` (the engine's `tindex`).
 * It is null for a finding about an ACCOUNT — an `account` directive
 * (`account-tag`) or a running balance (`negative-cash`) — which has no one
 * transaction to point at. Consumers that jump to or flag a row must treat null
 * as "nothing to jump to" rather than as index 0.
 */
export interface Problem {
    txnIndex: number | null;
    /** The account, for an account-anchored finding. */
    account?: string;
    /** The anchored transaction's date and description, so a list needs no journal to draw it. */
    date?: string;
    description?: string;
    rule: string;
    severity: Severity;
    message: string;
}

const SEVERITY_RANK: Record<Severity, number> = {info: 0, warning: 1, error: 2};

/** The most severe level present, or null when there are no problems. */
export function maxSeverity(problems: readonly Problem[]): Severity | null {
    let worst: Severity | null = null;
    for (const problem of problems) {
        if (worst === null || SEVERITY_RANK[problem.severity] > SEVERITY_RANK[worst]) worst = problem.severity;
        if (worst === "error") break;
    }
    return worst;
}

/**
 * Group problems by transaction index for O(1) row lookup.
 *
 * Unanchored problems are SKIPPED rather than collected under a placeholder key:
 * this map exists to decide which rows get a flag dot, and a finding about an
 * account belongs to no row. It is still in the full list, so the drawer and
 * the badge count it.
 */
export function groupByTxn(problems: readonly Problem[]): Map<number, Problem[]> {
    const byTxn = new Map<number, Problem[]>();
    for (const problem of problems) {
        if (problem.txnIndex === null) continue;
        const list = byTxn.get(problem.txnIndex);
        if (list === undefined) byTxn.set(problem.txnIndex, [problem]);
        else list.push(problem);
    }
    return byTxn;
}
