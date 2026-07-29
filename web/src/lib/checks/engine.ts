// Background-check engine (WP-08): LSP-style attention flags over the
// normalized journal. Pure TS — no Svelte/DOM imports; rules live in
// ./rules.ts and adding one is a single entry in ALL_RULES.

import type {AccountDecl} from "../domain/accountTypes";
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
    /** Declared account types, so rules classify by type rather than by name. */
    decls?: AccountDecl[];
    /**
     * Engine-computed findings (unbalanced transactions, failed balance
     * assertions) already decoded off the wire by normalizeDiagnostics.
     *
     * They live here rather than behind a CheckRule because they are
     * PRECOMPUTED: there is nothing to run, and the engine — which owns the
     * parsed journal — is the only thing that can compute them. Entering the
     * pipeline through the context means every existing consumer
     * (maxSeverity/groupByTxn, the badge, the drawer, row flags) picks them up
     * with no further plumbing.
     */
    diagnostics?: readonly Problem[];
    /**
     * True when the engine answered `/api/diagnostics` — i.e. it ran its own
     * balance check, and an EMPTY `diagnostics` means "checked, all clean"
     * rather than "nobody looked".
     *
     * The local `unbalanced` rule defers when this is set. It is a naive
     * every-commodity-must-sum-to-zero reimplementation, and hledger accepts two
     * shapes it would wrongly flag: a cost-derived residual within hledger's
     * `amountLooksZero` tolerance, and a two-commodity transaction it balances by
     * inferring a conversion cost. The engine reproduces both (verified against
     * hledger 1.52), so where both can answer, the engine wins. The local rule
     * still runs against a plain hledger-web, which has no such route.
     */
    engineChecked?: boolean;
}

export interface CheckRule {
    id: string;
    run(txns: Transaction[], ctx: CheckContext): Problem[];
}

export {ALL_RULES} from "./rules";

/**
 * Run `rules` (default: ALL_RULES) over the journal, concatenating their
 * findings in rule order.
 *
 * `ctx.diagnostics` (engine-computed, precomputed — see CheckContext) lead the
 * list, ahead of every rule finding. Two reasons: they are authoritative errors
 * from the component that parsed the journal, and the drawer groups by rule in
 * FIRST-APPEARANCE order, so leading with them puts the engine's hard errors at
 * the top while still letting the local `unbalanced` rule's findings fall into
 * the same "unbalanced" group. Order is otherwise unchanged, so it stays stable
 * across refreshes.
 */
export function runChecks(txns: Transaction[], ctx: CheckContext, rules: CheckRule[] = ALL_RULES): Problem[] {
    return [...(ctx.diagnostics ?? []), ...rules.flatMap((rule) => rule.run(txns, ctx))];
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
