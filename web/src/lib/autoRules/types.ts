// The main journal's `=` auto-posting rules (plan 30 Phase 5), as
// `GET /api/auto-rules` describes them.

/** Something wrong with one rule (plan 30 Decisions 9 and 11). */
export interface AutoRuleProblem {
    /** hledger would refuse the journal (`refused`, `unbalanced`), or the rule names an account nothing else does. */
    kind: "refused" | "unbalanced" | "unknownAccount";
    message: string;
    /** The transaction's 0-based position, for the first two. */
    txnIndex: number | null;
    account: string | null;
}

/** One `= QUERY` block of the main journal (or a file it includes). */
export interface AutoRule {
    /** Position in the journal's rule list — what a generated posting's `generated.rule` names. */
    index: number;
    /** Relative to the journal's directory. */
    file: string;
    /** The `=` line, 1-based. */
    line: number;
    query: string;
    /** The block exactly as written. */
    raw: string;
    /** Why it never fires, or null. */
    unsupported: string | null;
    /** How many postings it generated in the journal as served (0 while `--auto` is off). */
    generated: number;
    problems: AutoRuleProblem[];
}

export interface AutoRulesListing {
    /** Whether `--auto` is in the general section of the hledger.conf in force. */
    inForce: boolean;
    /** That file's relative handle (`hledger.conf`, `../hledger.conf`), or null for none. */
    conf: string | null;
    /** The file replaces hledger's command word, so none of its options apply. */
    confHijacked: boolean;
    rules: AutoRule[];
}
