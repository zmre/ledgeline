// Every decision the account-alias screens make, as pure functions.
//
// No Svelte, no DOM, no fetch — same rule, and same reason, as `importModel.ts`:
// the vitest project here is node-only and excludes `*.svelte.test.ts`, so a
// decision that lives inside a component is untested by construction. The two
// callers are `ui/AliasPanel.svelte` (the editor) and `ui/DryRunPanel.svelte`
// (the "these accounts will be rewritten" notice), and neither holds a rule of
// its own.
//
// The English here is the English the user reads. It is in this file rather than
// in a template for the same reason the logic is.

import type {AliasEffect, AliasEntry, AliasFile, AliasRename} from "$lib/imports/importTypes";
import type {SaveAliasEdit, SaveAliasesBody} from "$lib/api/native";

// ---------------------------------------------------------------------------
// Which aliases are relevant to the staged data
// ---------------------------------------------------------------------------

/**
 * One alias, and the renames it is responsible for in this import.
 *
 * `renames` empty with `attributable: false` means "this is a regex alias and
 * the engine's measurement does not say which regex did what" — see
 * {@link relevantAliases} for why that is stated rather than guessed at.
 */
export interface AliasRelevance {
    readonly alias: AliasEntry;
    readonly renames: readonly AliasRename[];
    /** True when this alias PROVABLY explains every rename attributed to it. */
    readonly attributable: boolean;
}

/**
 * Does the plain (non-regex) alias `pattern` match the account `name`?
 *
 * hledger's rule, verified against 1.52: an exact match, or a prefix ending at a
 * colon boundary. `alias a = b` rewrites `a` and `a:sub`, and does NOT touch
 * `abc` — which is the case a naive `startsWith` gets wrong, and getting it
 * wrong here would attribute a rename to the wrong line of the user's journal.
 */
export function plainAliasMatches(pattern: string, name: string): boolean {
    return name === pattern || name.startsWith(`${pattern}:`);
}

/**
 * Which of the journal's aliases are worth showing beside this import, and what
 * each one did.
 *
 * The renames are the ENGINE's measurement — it runs the same import with no
 * `--alias` and diffs the two proposals — so this function never has to decide
 * *whether* an account was rewritten, only *which alias to put the rename next
 * to*. That split is deliberate. Attribution is a display nicety; deciding what
 * hledger's regexes match is not something a TypeScript function should be doing
 * at all, since it would be a second, differently-wrong copy of a regex engine
 * the Rust side explicitly declines to reimplement.
 *
 * So: a plain alias is attributed when it provably explains the rename, and a
 * regex alias is attributed by elimination — it is offered as an explanation for
 * the renames no plain alias accounts for, marked `attributable: false` so the
 * UI can word it as a possibility rather than a fact. An alias that explains
 * nothing is left out entirely, which is what keeps the section quiet when the
 * journal's aliases have nothing to do with the statement in hand.
 */
export function relevantAliases(aliases: readonly AliasEntry[], effect: AliasEffect | null): AliasRelevance[] {
    if (effect === null || effect.renames.length === 0) return [];
    const forwarded = aliases.filter((alias) => alias.forwarded);
    const claimed = new Set<string>();

    const plain: AliasRelevance[] = forwarded
        .filter((alias) => !alias.regex)
        .map((alias) => {
            const renames = effect.renames.filter((rename) => plainAliasMatches(alias.pattern, rename.from) && rename.to.startsWith(alias.replacement));
            for (const rename of renames) claimed.add(rename.from);
            return {alias, renames, attributable: true};
        })
        .filter((entry) => entry.renames.length > 0);

    // Whatever no plain alias explains is the regex aliases' doing. Offered to
    // each of them rather than assigned to one, because proving which would mean
    // running hledger's regex dialect here.
    const unexplained = effect.renames.filter((rename) => !claimed.has(rename.from));
    const regex: AliasRelevance[] =
        unexplained.length === 0 ? [] : forwarded.filter((alias) => alias.regex).map((alias) => ({alias, renames: unexplained, attributable: false}));

    return [...plain, ...regex].sort((a, b) => a.alias.line - b.alias.line);
}

/**
 * The headline for the rewrite notice, or null when there is nothing to say.
 *
 * Null in two distinct cases that look the same on screen and must: no alias is
 * in force at all (`effect === null`), and aliases are in force but matched
 * nothing in this statement (`renames` empty). Either way the section is hidden
 * — "keep it quiet when no alias matches anything in the staged data".
 */
export function aliasNotice(effect: AliasEffect | null): string | null {
    if (effect === null || effect.renames.length === 0) return null;
    const count = effect.renames.length;
    const names = count === 1 ? "account name" : "account names";
    return `Your journal's aliases rewrite ${count} ${names} in this import.`;
}

/** One rename, as a sentence. */
export function renameText(rename: AliasRename): string {
    return `${rename.from} → ${rename.to}`;
}

/** How an alias reads on one line: `PW Roth IRA - 3077 → assets:…`. */
export function aliasText(alias: AliasEntry): string {
    return `${aliasPatternText(alias)} → ${alias.replacement}`;
}

/** The pattern as it is WRITTEN in the journal — a regex keeps its slashes. */
export function aliasPatternText(alias: AliasEntry): string {
    return alias.regex ? `/${alias.pattern}/` : alias.pattern;
}

/**
 * The badge beside an alias in the editor: what it is, and what it is not.
 *
 * Deliberately says nothing when the alias is both forwarded and editable —
 * a row that is working needs no decoration, and a screen where everything wears
 * a badge is a screen where no badge is read.
 */
export function aliasBadges(alias: AliasEntry): {text: string; tone: "warning" | "info"}[] {
    const badges: {text: string; tone: "warning" | "info"}[] = [];
    if (!alias.forwarded) badges.push({text: "not used for imports", tone: "warning"});
    if (!alias.editable) badges.push({text: "read-only", tone: "info"});
    if (alias.regex) badges.push({text: "regular expression", tone: "info"});
    return badges;
}

// ---------------------------------------------------------------------------
// The editor's form
// ---------------------------------------------------------------------------

/** One editable row of the alias editor. */
export interface AliasDraft {
    /** The engine's handle, or null for a row the user has just added. */
    readonly index: number | null;
    readonly pattern: string;
    readonly replacement: string;
    readonly regex: boolean;
    /** A row the user asked to remove. Kept in the list so the diff can see it. */
    readonly deleted: boolean;
    /** The engine will not rewrite this line, so the form must not offer to. */
    readonly locked: boolean;
}

/** The editor's whole state for one journal file. */
export interface AliasForm {
    readonly journalId: string;
    readonly label: string;
    readonly revision: string;
    readonly writable: boolean;
    readonly rows: readonly AliasDraft[];
}

/** A file listing as an editable form. */
export function toForm(file: AliasFile): AliasForm {
    return {
        journalId: file.journalId,
        label: file.label,
        revision: file.revision,
        writable: file.writable,
        rows: file.aliases.map((alias) => ({
            index: alias.index,
            pattern: alias.pattern,
            replacement: alias.replacement,
            regex: alias.regex,
            deleted: false,
            locked: !alias.editable,
        })),
    };
}

/** A blank row, for the "Add an alias" button. */
export function blankRow(): AliasDraft {
    return {index: null, pattern: "", replacement: "", regex: false, deleted: false, locked: false};
}

/** Has anything in this form actually changed? Drives the Save button. */
export function isDirty(baseline: AliasForm | null, draft: AliasForm | null): boolean {
    if (baseline === null || draft === null) return false;
    return toEdits(baseline, draft).length > 0;
}

/**
 * The changes between two forms, as the engine's edit list.
 *
 * Three rules, each one a decision not to write something:
 *
 * - a row whose pattern, replacement and form are all unchanged produces NO
 *   edit, so a save touches only what the user touched;
 * - a locked row never produces a `replace`, because the engine would refuse it
 *   and a UI that can construct a refused request is a UI that will;
 * - a row that was added and then deleted produces nothing at all, rather than a
 *   delete of a line that does not exist.
 */
export function toEdits(baseline: AliasForm, draft: AliasForm): SaveAliasEdit[] {
    const before = new Map(baseline.rows.filter((row) => row.index !== null).map((row) => [row.index as number, row]));
    const edits: SaveAliasEdit[] = [];
    for (const row of draft.rows) {
        if (row.index === null) {
            if (!row.deleted && (row.pattern !== "" || row.replacement !== "")) {
                edits.push({kind: "append", pattern: row.pattern, replacement: row.replacement, regex: row.regex});
            }
            continue;
        }
        if (row.deleted) {
            edits.push({kind: "delete", index: row.index});
            continue;
        }
        const original = before.get(row.index);
        if (original === undefined || row.locked) continue;
        if (original.pattern !== row.pattern || original.replacement !== row.replacement || original.regex !== row.regex) {
            edits.push({kind: "replace", index: row.index, pattern: row.pattern, replacement: row.replacement, regex: row.regex});
        }
    }
    return edits;
}

/** The request body a save sends. */
export function toSaveRequest(baseline: AliasForm, draft: AliasForm): SaveAliasesBody {
    return {revision: baseline.revision, edits: toEdits(baseline, draft)};
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/** The engine's own caps, mirrored so the form can say so before the round trip. */
export const MAX_PATTERN_BYTES = 256;
/** See {@link MAX_PATTERN_BYTES}. */
export const MAX_REPLACEMENT_BYTES = 512;

const encoder = new TextEncoder();

/**
 * Every reason this row would be refused, in the engine's own words.
 *
 * A deliberate duplicate of `ledgeline_core::aliases`'s checks, and the
 * duplication is the point: the engine is the authority and refuses regardless,
 * but a user typing `a:b ; note` deserves to be told what is wrong while they
 * are typing it rather than after a round trip. The rules are stated once here
 * and once there because there is no third place they could live — and the
 * server tests pin the engine's half, so a drift shows up as a 400 the form
 * failed to predict, never as a value that got written.
 */
export function validateRow(row: AliasDraft): string[] {
    if (row.deleted) return [];
    const problems: string[] = [];
    const check = (what: string, value: string, cap: number) => {
        if (value === "") {
            problems.push(`The ${what} cannot be empty.`);
            return;
        }
        if (encoder.encode(value).length > cap) problems.push(`The ${what} is longer than ${cap} bytes.`);
        // The ASCII control range, spelled in escapes: a literal one in this
        // source would be invisible to every reviewer of this file.
        // eslint-disable-next-line no-control-regex
        if (/[\u0000-\u001f\u007f]/.test(value)) problems.push(`The ${what} cannot contain a control character.`);
        if (value.trim() !== value) problems.push(`The ${what} cannot begin or end with a space: hledger trims it, so it would not be saved as typed.`);
        if (value.includes(";") || value.includes("#"))
            problems.push(`The ${what} cannot contain ";" or "#": hledger reads those as part of the account name, not as a comment.`);
    };
    check("pattern", row.pattern, MAX_PATTERN_BYTES);
    check("replacement", row.replacement, MAX_REPLACEMENT_BYTES);
    if (row.regex) {
        if (unescapedSlash(row.pattern)) problems.push('A regular expression cannot contain an unescaped "/", which would end the pattern early.');
        if (row.pattern.includes("\\") && !row.pattern.includes("\\1")) {
            // A backslash in the PATTERN is an escape Ledgeline will not
            // re-derive; one in the replacement (`\1`) is a backreference and is
            // fine, which is why this only looks at the pattern.
            problems.push("A regular expression pattern cannot contain a backslash here: Ledgeline will not re-derive an escape it did not write.");
        }
    } else {
        if (row.pattern.startsWith("/")) problems.push('A pattern beginning with "/" is a regular expression — tick the box instead.');
        if (row.pattern.includes("=")) problems.push('A pattern cannot contain "=": hledger splits the line at the first one.');
    }
    return problems;
}

/** The first `/` in `text` that is not preceded by a backslash. */
function unescapedSlash(text: string): boolean {
    for (let i = 0; i < text.length; i += 1) {
        if (text[i] === "\\") {
            i += 1;
            continue;
        }
        if (text[i] === "/") return true;
    }
    return false;
}

/** Every problem in the form, row by row, prefixed so the user knows which row. */
export function validateForm(form: AliasForm): string[] {
    return form.rows.flatMap((row, at) => validateRow(row).map((problem) => `Alias ${at + 1}: ${problem}`));
}

/**
 * The sentence explaining what this screen does — and, just as importantly, what
 * it does not.
 *
 * Ledgeline reads `alias` directives but does NOT apply them to the journal it
 * shows you; hledger does that when it reads your journal. Saying so here is the
 * whole mitigation for that divergence, so it is a fixed string with a test
 * rather than a comment in a template somebody will trim.
 */
export const ALIAS_EXPLAINER =
    "An alias maps a name your bank uses onto one of your accounts. Ledgeline hands these to hledger when it imports a statement, " +
    "which is the only way an alias can reach a CSV. It does not rewrite the account names shown elsewhere in Ledgeline — " +
    "hledger applies these itself when it reads your journal.";
