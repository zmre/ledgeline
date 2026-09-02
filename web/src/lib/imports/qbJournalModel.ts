// The QuickBooks Online Journal import panel's decisions (WP-17 Phase C), all
// of them, as pure functions — same discipline `importModel.ts`/`aliasModel.ts`
// state at their own heads: no Svelte, no DOM, no `fetch`, so every judgement
// call is tested by naming its inputs and reading its answer.
//
// The plan's Phase C contract is deliberately narrow: the ONLY client-side
// detection this screen may do is read `StagedFile.format` — no heuristics, no
// confidence UI, no confirmation step (`POST /api/import/stage` already
// decided, server-side, before the SPA sees anything). `isQuickbooksJournal*`
// below is that one branch and nothing more.
//
// Resolving an unmapped account reuses the account-alias screen's OWN
// validation (`aliasModel.validateRow`) rather than re-deriving a second copy
// of it: a QuickBooks account name becomes the alias PATTERN (fixed, not
// user-editable — it is the exact string the export carries) and the user
// types only the REPLACEMENT, but both still have to survive the same rules
// the engine enforces, and `validateRow` already knows them.

import {validateRow, type AliasDraft} from "./aliasModel";
import type {SaveAliasEdit} from "$lib/api/native";
import type {AliasFile, QbDateFormat, QbFileOrdering, QbIdMatches, QbOrdering, QbPreview, StagedFile} from "./importTypes";

// ---------------------------------------------------------------------------
// The one branch point: is this staged upload a QuickBooks Journal export?
// ---------------------------------------------------------------------------

/** The exact `format` string `POST /api/import/stage` sends for this pipeline. Nothing here reproduces `qb_journal::detect`. */
export const QB_JOURNAL_FORMAT = "quickbooks-journal";

export function isQuickbooksJournalFormat(format: string): boolean {
    return format === QB_JOURNAL_FORMAT;
}

/** Whether the currently staged file (if any) is a QuickBooks Journal export — the panel's own routing question. */
export function isQuickbooksJournalStage(staged: StagedFile | null): boolean {
    return staged !== null && isQuickbooksJournalFormat(staged.format);
}

// ---------------------------------------------------------------------------
// The date-format ambiguity affordance
//
// Phase A's own contract amendment flagged this as needing a Phase C UI
// element the original sketch never mentioned: `01/02/2026` is two different
// days depending on a QuickBooks account preference the export does not
// record, and `guess_date_format`'s `ambiguous` flag is the only evidence
// there is. Surfaced, not resolved — there is nothing to DO about it here
// beyond asking the user to look at the sample before committing.
// ---------------------------------------------------------------------------

/** The ambiguity notice, or null when the export contained enough evidence to be sure. */
export function dateFormatNotice(dateFormat: QbDateFormat): string | null {
    if (!dateFormat.ambiguous) return null;
    return (
        `Dates were read as ${dateFormat.format}, but this export doesn't contain a date with a day or month above 12 — the only evidence that ` +
        "settles which is which. Check a few dates in the sample below before committing; if they read wrong, re-export a wider date range."
    );
}

// ---------------------------------------------------------------------------
// Resolving unmapped accounts
// ---------------------------------------------------------------------------

/** One unmapped account's mapping row, in the alias editor's own draft shape — reused so it can reuse that editor's validation. */
export function mappingDraft(account: string, replacement: string): AliasDraft {
    return {index: null, pattern: account, replacement, regex: false, deleted: false, locked: false};
}

/** Every problem with one mapping row's typed replacement (or its fixed pattern), in the engine's own words. */
export function mappingProblems(account: string, replacement: string): string[] {
    return validateRow(mappingDraft(account, replacement));
}

/**
 * The `append` edits a "Map accounts" submit sends: one per unmapped account
 * with a non-blank, valid typed replacement.
 *
 * Silently skips a blank or invalid row rather than refusing the whole submit
 * — the user may only have finished typing some of them, and the ones that
 * land still shrink `unmappedAccounts` on the next preview; the ones left
 * blank simply stay listed for a later pass. {@link mappingProblems} is what
 * the form shows inline so a skip is never a silent one.
 */
export function mappingEdits(unmapped: readonly string[], drafts: Readonly<Record<string, string>>): SaveAliasEdit[] {
    return unmapped.reduce<SaveAliasEdit[]>((edits, account) => {
        const replacement = (drafts[account] ?? "").trim();
        if (replacement === "" || mappingProblems(account, replacement).length > 0) return edits;
        edits.push({kind: "append", pattern: account, replacement, regex: false});
        return edits;
    }, []);
}

/** Whether at least one typed row is ready to submit — the "Map accounts" button's own gate. */
export function hasMappingsToSave(unmapped: readonly string[], drafts: Readonly<Record<string, string>>): boolean {
    return mappingEdits(unmapped, drafts).length > 0;
}

/**
 * Which journal file a new alias should be appended to: the first WRITABLE
 * one, in listing order.
 *
 * `GET /api/aliases` lists files root-first (`Journal::source_files` order —
 * see `alias_api.rs`), the same order the account-alias editor defaults its
 * own selection from (`AliasPanel.svelte`'s `files.find(...) ?? files[0]`),
 * so this mirrors that rather than inventing a second "which file" rule. Null
 * when nothing here can be written to at all.
 */
export function defaultAliasTargetFile(files: readonly AliasFile[]): AliasFile | null {
    return files.find((file) => file.writable) ?? null;
}

// ---------------------------------------------------------------------------
// The commit gate
// ---------------------------------------------------------------------------

/** Whether the commit button may be pressed: the preview has answered, and every account it found resolves. */
export function canCommitQbJournal(preview: QbPreview | null): boolean {
    return preview !== null && preview.unmappedAccounts.length === 0;
}

// ---------------------------------------------------------------------------
// Id matching — the same sentence `importModel.idMatchesSummary` writes for
// the CSV path, minus the status-sync half this pipeline never produces (see
// `QbIdMatches`'s own doc comment).
// ---------------------------------------------------------------------------

/** The id-matching section's headline, or null when there is nothing worth a section for (see `importModel.idMatchesSummary`'s own reasoning). */
export function qbIdMatchesSummary(idMatches: QbIdMatches | null): string | null {
    if (idMatches === null || idMatches.conflictingTotal === 0) return null;
    const n = idMatches.conflictingTotal;
    return `${n} row${n === 1 ? "" : "s"} the journal already holds differently — left untouched, since a field disagreement more likely means you edited it on purpose than that the export's data changed.`;
}

// ---------------------------------------------------------------------------
// Post-commit ordering — per file, unlike the CSV path's single target
// ---------------------------------------------------------------------------

/** Which touched files a re-sort should be offered for. */
export function filesNeedingSort(ordering: QbOrdering): readonly QbFileOrdering[] {
    return ordering.files.filter((file) => !file.inOrder);
}

/** The re-sort offer's sentence for one out-of-order file, or null when it is already in order. */
export function qbReorderOffer(file: QbFileOrdering): string | null {
    if (file.inOrder) return null;
    const moves = file.moves.length;
    return `${file.journalId} is no longer in date order — ${moves} transaction${moves === 1 ? "" : "s"} would move.`;
}
