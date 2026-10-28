// The main journal's `=` auto-posting rules (plan 30 Phase 5), as
// `GET /api/auto-rules` describes them, and the one change
// `PUT /api/auto-rules` makes to them (amendment 22).

import type {AutoRuleForm, WireAutoRuleFormIn} from "./ruleForm";

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
    /** Its file's handle, or null when that file cannot be written. */
    journalId: string | null;
    /** Its position among its own file's rules. */
    block: number;
    /** Its file's revision, or null with `journalId`. */
    revision: string | null;
    /** The rule as the editor's form, or null when a form cannot state it (the text is edited instead). */
    form: AutoRuleForm | null;
}

/** A writable file and the revision a write to it must match. */
export interface FileRevision {
    journalId: string;
    revision: string;
}

export interface AutoRulesListing {
    /** Whether `--auto` is in the general section of the hledger.conf in force. */
    inForce: boolean;
    /** That file's relative handle (`hledger.conf`, `../hledger.conf`), or null for none. */
    conf: string | null;
    /** The file replaces hledger's command word, so none of its options apply. */
    confHijacked: boolean;
    /** Whether this server writes the journal at all. */
    editable: boolean;
    /** Where a new rule is appended (the main journal), or null when it cannot be written. */
    newRule: FileRevision | null;
    rules: AutoRule[];
}

/** A rule as `PUT` takes it: its text, or its form. */
export type AutoRuleIn = {raw: string} | {form: WireAutoRuleFormIn};

/** `PUT /api/auto-rules`. `was` is the block as last listed: a rule moved since is a 409. */
export interface SaveAutoRuleBody {
    journalId: string;
    revision: string;
    change: {kind: "add"; rule: AutoRuleIn} | {kind: "replace"; block: number; was: string; rule: AutoRuleIn} | {kind: "remove"; block: number; was: string};
}
