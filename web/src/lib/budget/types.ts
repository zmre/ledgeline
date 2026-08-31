// The budget EDITOR's domain types — the `~` periodic rules the budget report
// measures against, as the Budget tab lists and rewrites them.
//
// Distinct from `$lib/reports/types.ts`'s `BudgetReport`, which is the computed
// actual-vs-goal report. That one answers "how am I doing"; these answer "what
// did I say I would do, and where is it written down". The tab shows both.
//
// Every field mirrors `ledgeline-server/src/budget_api.rs`'s wire types. The
// pairing that matters most is `amount` and `entry`:
//
//   - `amount` is what the FILE says, signed the way hledger signs it (income
//     negative).
//   - `entry` is what the USER types, always a magnitude.
//
// `inverted` says whether those two differ. The engine decides it from the
// account's declared type and does the flip in both directions, so nothing here
// — and nothing in a component — ever negates a number itself. See the "Signs"
// section of `budget_api.rs` for why that rule has exactly one home.

import type {Dec, MixedAmount} from "$lib/domain/money";
import type {ISODate} from "$lib/domain/types";

/** The five recurrence intervals hledger's `~` rules (and Ledgeline) model. */
export type BudgetPeriod = "daily" | "weekly" | "monthly" | "quarterly" | "yearly";

/** The periods the editor actually offers, in the order it offers them. */
export const BUDGET_PERIODS: {id: BudgetPeriod; label: string; plural: string}[] = [
    {id: "weekly", label: "Weekly", plural: "weeks"},
    {id: "monthly", label: "Monthly", plural: "months"},
    {id: "quarterly", label: "Quarterly", plural: "quarters"},
    {id: "yearly", label: "Annual", plural: "years"},
];

/** Whether `value` is one of the five modelled periods. */
export function isBudgetPeriod(value: string): value is BudgetPeriod {
    return value === "daily" || value === "weekly" || value === "monthly" || value === "quarterly" || value === "yearly";
}

/** A single-commodity amount as the editor's number box holds it. */
export interface BudgetEntry {
    commodity: string;
    /** The magnitude the user sees and types — never negative for an income goal. */
    value: Dec;
}

/** One goal: an account and an amount, inside a rule. */
export interface BudgetGoal {
    /** Handle for `set`/`remove`, scoped to this FILE. A scan ordinal, not a durable id. */
    index: number;
    /** 1-based file line, for display. */
    line: number;
    account: string;
    /** Written `(account)` — the unbalanced-virtual form every budget example uses. */
    unbalanced: boolean;
    /** The amount exactly as the file writes it, or null when the line has none (the inferred leg). */
    amount: MixedAmount | null;
    /** The magnitude to show and to send back, or null for the same lines `amount` is. */
    entry: BudgetEntry | null;
    /** Whether `entry` is the negation of `amount`. */
    inverted: boolean;
    /** Why this goal is read-only, or null when it can be edited. */
    locked: string | null;
}

/** One `~ PERIOD  description` rule. */
export interface BudgetRule {
    /** Handle for `add`, scoped to this file. */
    block: number;
    line: number;
    /** One of {@link BudgetPeriod}, or the raw text when the engine could not model it (then `locked` is set). */
    period: string;
    /** `--budget=DESCPAT` matches a substring of this. */
    description: string;
    /** Why this whole rule is read-only, or null. */
    locked: string | null;
    goals: BudgetGoal[];
}

/** One journal file's rules, and the revision a save must quote. */
export interface BudgetFile {
    journalId: string;
    label: string;
    /** Echo this back in a save to prove the edit is against these bytes. */
    revision: string;
    writable: boolean;
    rules: BudgetRule[];
}

/** `GET /api/budget/lines`. */
export interface BudgetListing {
    /** False when no journal is bound to an editor: the screen is read-only and says why. */
    editable: boolean;
    /** Where a new goal goes by default, or null when there is nowhere. */
    defaultTarget: string | null;
    /** Whether the "create a budget file" button would succeed. */
    canCreateFile: boolean;
    /** The name that button would create, so the UI can say it out loud. */
    createFileName: string;
    files: BudgetFile[];
}

/** `POST /api/budget/file` — what was created. */
export interface CreatedBudgetFile {
    journalId: string;
    label: string;
    /** The `include` line appended to the main journal, verbatim. */
    includedAs: string;
    mainJournalId: string;
}

/** One period of an account's recent activity. */
export interface ReferencePeriod {
    key: string;
    /** The key rendered for a person, e.g. `Aug 2026`. */
    label: string;
    start: ISODate;
    /** Inclusive end, clamped to the as-of date. */
    end: ISODate;
    /** False for a period that is still running — show it as "so far". */
    complete: boolean;
    /** Subaccount-inclusive, oriented the same way a goal on this account is. */
    total: MixedAmount;
}

/** `GET /api/budget/reference` — what one account actually did, period by period. */
export interface AccountReference {
    account: string;
    interval: string;
    /** Whether these figures are negated relative to the journal (an income account). */
    inverted: boolean;
    periods: ReferencePeriod[];
    /**
     * The mean over the COMPLETE periods, oriented like {@link periods}.
     *
     * The running period is deliberately left out: a month that is four days old
     * would drag the mean down by however far through the month you happen to be,
     * which changes daily for reasons that have nothing to do with spending.
     */
    average: MixedAmount;
    /**
     * How many periods {@link average} covers.
     *
     * **Zero means there is no average** — a different fact from an average of
     * zero, and the one a caller must not print a number for.
     */
    averagedPeriods: number;
}

// ---------------------------------------------------------------------------
// The goal form
// ---------------------------------------------------------------------------

/**
 * What the add/edit modal was opened to do.
 *
 * Here rather than in the component because two files need it — the modal that
 * renders it and the page that builds it — and a Svelte instance script is not a
 * place to export a type from.
 */
export interface GoalDraft {
    /** The goal being edited, or null when adding a new one. */
    goal: BudgetGoal | null;
    /** The rule it belongs to (edit), or the rule to add into (add). Null means a new rule. */
    rule: BudgetRule | null;
    /** The file this goal lives in, or will. */
    journalId: string;
    period: BudgetPeriod;
    account: string;
    /** The amount as typed — always a magnitude, never a sign. */
    amount: string;
}

/** What a confirmed modal hands back. */
export interface GoalSubmission {
    period: BudgetPeriod;
    account: string;
    value: Dec;
}
