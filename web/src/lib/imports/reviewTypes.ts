// Domain types for reviewing an import before it is written (plan 32 Phase 4,
// D23–D26, D30), decoded from `/api/import/dry-run`, `/api/import/rule-preview`
// and the QuickBooks preview by `api/importReviewDecode.ts`.
//
// A STAGED transaction is one the import would write but has not: it has no
// journal index, so it is addressed by where it sits in the import — its
// ordinal in the dry run's `entries` (CSV), or its QuickBooks Trans # (D30).
// Everything else about it is the review queue item the journal overlay
// already renders, so the overlay runs over staged cards unchanged.
//
// No Svelte/DOM imports.

import type {EditResult} from "#lib/api/editFailure.js";
import type {WireBatchEdit, WireEditOp} from "#lib/api/native.js";
import type {QueueItem} from "#lib/review/types.js";

/** Where a staged transaction sits in its import: the key its overrides are held by. */
export type StagedRef = {readonly kind: "csv"; readonly ordinal: number} | {readonly kind: "qb"; readonly id: string};

/** Whether a QuickBooks transaction is new to the journal, already held identically, or held differently. */
export type QbClassification = "new" | "unchanged" | "changed";

/**
 * One transaction the import would write, as a review card.
 *
 * The ROW (description, postings, tags, `unknown`, `reviewed`) is the
 * transaction as it currently reads — overrides included. `fingerprint` and
 * `key` belong to the GENERATED text, before any override: the fingerprint is
 * what an override is checked against, and the key is the original payee's
 * (D11), so similar transactions are found by what the bank sent.
 *
 * `txn.index` is `ordinal + 1` for a CSV import and the export position + 1
 * for QuickBooks: unique within the import, never a journal index.
 */
export interface StagedTxn extends QueueItem {
    readonly ref: StagedRef;
    /** An override was applied to produce this row. Always false for QuickBooks (its preview takes none). */
    readonly overridden: boolean;
    /** QuickBooks only; null for a CSV import. */
    readonly classification: QbClassification | null;
}

/**
 * The raw CSV record behind each staged transaction (D25), for building a
 * rule's matcher from the field that produced the description.
 *
 * `rows[i]` is `txns[i]`'s record when `aligned`. When hledger's transactions
 * and the file's records do not line up one to one (`if … skip`, `end`, a
 * trailer), `aligned` is false, `rows` is empty, and the rule builder matches
 * the whole record instead.
 */
export interface StagedRecords {
    readonly aligned: boolean;
    /** The converted CSV's header row, or null when it has none. */
    readonly header: readonly string[] | null;
    /** The rules file's `fields` names, or null when it declares none. */
    readonly fields: readonly string[] | null;
    readonly rows: readonly (readonly string[])[];
}

/** One matcher of a candidate rule. `field: null` matches the whole CSV record. */
export interface RuleMatcher {
    readonly field: string | null;
    readonly pattern: string;
}

/** A candidate `if` block for the live check (D25). */
export interface RuleSpec {
    readonly matchers: readonly RuleMatcher[];
    readonly assignments: readonly {readonly field: string; readonly value: string}[];
}

/** One transaction the candidate rule would change, before and after, as journal text. */
export interface RuleChange {
    readonly ordinal: number;
    readonly before: string;
    readonly after: string;
}

/**
 * `POST /api/import/rule-preview` → 200 ok: "Matches 7 of 112 rows (3 already
 * categorised)". Also the Journal view's live check (D27) when it ran against
 * the rules file's newest statement (`basis: "csv"`).
 */
export interface RulePreviewOk {
    readonly ok: true;
    /** Absent for an import's own preview; `"csv"` for the Journal view's check against the rules file's newest statement. */
    readonly basis?: "csv";
    /** `basis: "csv"`: the statement's file name. */
    readonly file?: string;
    /**
     * `basis: "csv"`: the pattern met no row of the statement. On a journal card
     * the pattern comes from the transaction's description, which a
     * description-only rule may have renamed from the bank's text (D27's
     * caveat), so this is the builder's cue to say so.
     */
    readonly noCsvMatch?: boolean;
    /** Rows of this import the rule's matchers select. */
    readonly matched: number;
    /** Rows in this import. */
    readonly total: number;
    /** Of `matched`, how many another rule had already categorised. */
    readonly alreadyCategorised: number;
    /** At most 50 of them; `changedTotal` is the real count. */
    readonly changed: readonly RuleChange[];
    readonly changedTotal: number;
}

/** hledger refused the rules file with the rule appended; `stderr` is shown verbatim. */
export interface RulePreviewFailed {
    readonly ok: false;
    readonly stderr: string;
}

/**
 * `POST /api/review/rule-preview` with no statement at hand (D27's fallback):
 * of the journal's transactions funded from the rules file's `account1`
 * (`total`), how many have a description the rule's patterns match.
 */
export interface RulePreviewJournal {
    readonly ok: true;
    readonly basis: "journal";
    readonly matched: number;
    readonly total: number;
}

export type RulePreview = RulePreviewOk | RulePreviewJournal | RulePreviewFailed;

/** What an import's own preview (`POST /api/import/rule-preview`) can answer. */
export type ImportRulePreview = RulePreviewOk | RulePreviewFailed;

/**
 * One review edit to a staged transaction, as the import store holds it.
 *
 * `ops` are cumulative (every save on the card merged, `mergeOps`) and apply to
 * the GENERATED text `fingerprint` names. `original` is the generated item it
 * was made on, which is what replay compares a regenerated transaction against
 * (D25). `flagged` means a replay found the generated text changed underneath
 * in a way the edit did not account for.
 */
export interface StagedOverride {
    readonly ref: StagedRef;
    readonly fingerprint: string;
    readonly ops: readonly WireEditOp[];
    readonly original: StagedTxn;
    readonly flagged: boolean;
}

/** Why replay let an override go. */
export type DropReason = "covered" | "gone";

/** What replaying the held overrides over a fresh dry run did with each. */
export interface ReplayReport {
    /** Every override still held, re-targeted where needed (`flagged` ones included). */
    readonly kept: readonly StagedOverride[];
    readonly dropped: readonly {readonly override: StagedOverride; readonly reason: DropReason}[];
    /** The kept ones whose transaction changed underneath them. */
    readonly flagged: readonly StagedOverride[];
}

/** What one review save hands the import (the review store's staged sink, D24). */
export interface StagedSave {
    /** The card first, then any staged siblings the change was applied to. */
    readonly overrides: readonly {readonly item: StagedTxn; readonly ops: readonly WireEditOp[]}[];
    /** Similar journal transactions, each with the fingerprint it was shown with. */
    readonly journalEdits: readonly WireBatchEdit[];
}

/**
 * Where a staged review run's saves go instead of the network (D23): the
 * import store holding the overrides. Synchronous — nothing is written until
 * the import is.
 */
export interface StagedSink {
    stage(save: StagedSave): EditResult;
}

/**
 * How the rule builder starts a new rule's MATCH: the fields it may scope to,
 * the one it starts on, and the pattern for a card. An import builds the
 * pattern from the raw CSV cell (`importPrefill`); a journal card from its own
 * description (`descriptionPrefill`, D27), since it has no CSV record.
 */
export interface RulePrefill {
    /** The rules file's `fields` names, once each: the select's options besides "whole record". */
    readonly fields: readonly string[];
    /** The scope a new rule starts with: a field name, or null for the whole record. */
    readonly field: string | null;
    /** The pattern a new rule for `item` starts with, scoped to `field` (null: the whole record). */
    pattern(item: QueueItem, field: string | null): string;
}

/**
 * What the review overlay's rule builder (D25) needs: how to start the rule,
 * the live check, and the rules-file write — always for ONE rules file. An
 * import has one (`importStore.ruleSource()`); a QuickBooks run has none, so
 * "Remember as a rule" is not offered there (D30). A journal card picks one
 * from its `RuleChoice` (D27).
 */
export interface RuleSource {
    readonly prefill: RulePrefill;
    /** The statement's file name, for "Matches 7 of 112 rows in checking.csv". */
    readonly fileLabel: string;
    /** The rules file a saved rule is appended to, as the import names it. */
    readonly rulesLabel: string;
    /**
     * A journal card's (D27): the account the rules file books to, for the
     * live check's "of M transactions from <account>" when there is no
     * statement to test against. Absent for an import.
     */
    readonly account?: string;
    /** The live check. Null when the import names no rules file or journal. Rejects with the abort when `signal` fires. */
    preview(rule: RuleSpec, signal?: AbortSignal): Promise<RulePreview | null>;
    /** Append `rule` to the rules file (the existing rules PUT). Writes nothing else. */
    save(rule: RuleSpec): Promise<EditResult>;
    /**
     * Regenerate the import after the rule and the card's override are in,
     * replaying the held overrides. Resolves to why the regenerated import
     * failed (the engine's words, or hledger's stderr), or null when it is
     * fine — so the overlay can say so on top, not leave it to the panel
     * underneath. A journal card's resolves null: a new rule changes no
     * transaction already in the journal (the card's own save still does).
     */
    refresh(): Promise<string | null>;
}

/** One rules file that could have produced a journal transaction (D27), from `GET /api/review/rules-for/{index}`. */
export interface RulesCandidate {
    readonly rulesId: string;
    readonly label: string;
    /** The family's name (`citi` for `citi-2026.csv.rules`). */
    readonly family: string;
    /** How many candidates share the family. */
    readonly familySize: number;
    /** The year the file is named for, or null. */
    readonly year: number | null;
    /** The account that made it a candidate. */
    readonly account1: string;
    /** Which signals matched, in plain words. */
    readonly reasons: readonly string[];
    /** The file to save to unless the user picks another. At most one per list. */
    readonly recommended: boolean;
    /** `description` when the file's `fields` names it, else null (match the whole record). */
    readonly matchField: string | null;
    /** The file's `fields` names, or null. */
    readonly fields: readonly string[] | null;
}

/** `GET /api/review/rules-for/{index}`. */
export interface RulesFor {
    readonly index: number;
    /** Candidates exist and none is recommended: the user must pick before a rule can be saved. */
    readonly needsChoice: boolean;
    /** Best first. Empty: no rules file books to the transaction's account, so the toggle is hidden. */
    readonly candidates: readonly RulesCandidate[];
}

/**
 * "Remember as a rule" on a JOURNAL card (D27): the rules files it may go to,
 * and the builder's source for whichever is picked. Built by
 * `review/journalRules.ts`; null there when there are no candidates.
 */
export interface RuleChoice {
    readonly candidates: readonly RulesCandidate[];
    /** The candidate to preselect, or null when `needsChoice`. */
    readonly recommended: string | null;
    /** No preselection: show the picker empty, and save no rule until one is picked. */
    readonly needsChoice: boolean;
    /** The builder's source for saving to `rulesId`. Throws for an id not among `candidates`. */
    source(rulesId: string): RuleSource;
}
