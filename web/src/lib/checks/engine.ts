// Background-check engine (WP-08): LSP-style attention flags over the
// normalized journal. Pure TS — no Svelte/DOM imports; rules live in
// ./rules.ts and adding one is a single entry in ALL_RULES.

import type {PriceDirective, Transaction} from "../domain/types";
import {ALL_RULES} from "./rules";

export type Severity = "error" | "warning" | "info";

export interface Problem {
    txnIndex: number;
    rule: string;
    severity: Severity;
    message: string;
}

/** Journal-wide inputs beyond the transactions themselves (WP-10 contract change: stock rules need P directives). */
export interface CheckContext {
    prices: PriceDirective[];
}

export interface CheckRule {
    id: string;
    run(txns: Transaction[], ctx: CheckContext): Problem[];
}

export {ALL_RULES} from "./rules";

/** Run `rules` (default: ALL_RULES) over the journal, concatenating their findings in rule order. */
export function runChecks(txns: Transaction[], ctx: CheckContext, rules: CheckRule[] = ALL_RULES): Problem[] {
    return rules.flatMap((rule) => rule.run(txns, ctx));
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

/** Group problems by transaction index for O(1) row lookup. */
export function groupByTxn(problems: readonly Problem[]): Map<number, Problem[]> {
    const byTxn = new Map<number, Problem[]>();
    for (const problem of problems) {
        const list = byTxn.get(problem.txnIndex);
        if (list === undefined) byTxn.set(problem.txnIndex, [problem]);
        else list.push(problem);
    }
    return byTxn;
}
