// The decisions behind "Remember as a rule" (plan 32 D25): which CSV field a
// rule built from a review card should match, the pattern it is prefilled
// with, what it sets, and how its live check's changed rows read.
//
// Pure: no Svelte, no fetch. `review/ui/RuleBuilder.svelte` renders these and
// `ruleBuilder.test.ts` pins them.

import {txnToForm, type CardChange} from "#lib/api/editMapping.js";
import type {Transaction} from "#lib/domain/types.js";
import type {RuleSpec, StagedRecords} from "./reviewTypes";

/** How long a candidate rule must sit still before its live check runs (D25: debounced). */
export const RULE_PREVIEW_DEBOUNCE_MS = 250;

/** The rules-file field a description comes from. */
const DESCRIPTION_FIELD = "description";

/**
 * The words of `text` the engine's `similarity_key` keeps (D10): the payee
 * part, NFKC-folded and lowercased, without any token that holds a digit, split
 * on everything that is not a letter or a digit. Mirrors
 * `ledgeline_core::review::similarity_key`.
 */
export function keyTokens(text: string): string[] {
    const payee = text.split("|", 1)[0] ?? "";
    return payee
        .normalize("NFKC")
        .toLowerCase()
        .split(/\s+/u)
        .filter((token) => token !== "" && !/\p{N}/u.test(token))
        .flatMap((token) => token.split(/[^\p{Alphabetic}\p{N}]+/u))
        .filter((word) => word !== "");
}

/** `text` with every regex metacharacter escaped, so it matches itself. */
export function escapeRegex(text: string): string {
    return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Whether `tokens` appear in `text` in order (as substrings, case already folded). */
function inOrder(tokens: readonly string[], text: string): boolean {
    let from = 0;
    for (const token of tokens) {
        const at = text.indexOf(token, from);
        if (at < 0) return false;
        from = at + token.length;
    }
    return true;
}

/**
 * The pattern a new rule is prefilled with (D25): the similarity key's
 * tokens, regex-escaped, joined with `.*`. hledger's matchers are already
 * case-insensitive, so the lowercased key matches the bank's capitals.
 *
 * `source` is the raw CSV text the rule will be tested against (the
 * description cell, when the dry run lined records up with transactions). The
 * key belongs to the GENERATED description, which a description-only rule may
 * have rewritten; when its tokens are not in the raw text, the raw text's own
 * tokens are used instead, so the rule matches what the bank actually sends.
 */
export function rulePattern(key: string | null, source: string | null): string {
    const fromKey = key === null ? [] : key.split(" ").filter((token) => token !== "");
    let tokens = fromKey;
    if (source !== null) {
        const folded = source.normalize("NFKC").toLowerCase();
        if (fromKey.length === 0 || !inOrder(fromKey, folded)) {
            const own = keyTokens(source);
            if (own.length > 0) tokens = own;
        }
    }
    return tokens.map(escapeRegex).join(".*");
}

/** The field a new rule matches by default: `description` when records line up and the rules file names it, else the whole record (null). */
export function defaultMatchField(records: StagedRecords): string | null {
    return records.aligned && records.fields !== null && records.fields.includes(DESCRIPTION_FIELD) ? DESCRIPTION_FIELD : null;
}

/** The fields a matcher may be scoped to: the rules file's own `fields` names, once each, in order. */
export function matchFields(records: StagedRecords): string[] {
    return [...new Set((records.fields ?? []).map((name) => name.trim()).filter((name) => name !== ""))];
}

/** The raw CSV cell behind staged transaction `ordinal` for the rules file's field `field`, or null when it cannot be known. */
export function sourceText(records: StagedRecords, ordinal: number, field: string): string | null {
    if (!records.aligned) return null;
    const at = records.fields?.indexOf(field) ?? -1;
    return at < 0 ? null : (records.rows[ordinal]?.[at] ?? null);
}

/** One thing a rule built from a card can set. */
export interface RuleTarget {
    /** The rules-file field: `account2`, `account1`, `description`. */
    readonly field: string;
    readonly value: string;
    /** Checked when the builder opens. */
    readonly defaultOn: boolean;
}

/**
 * What a rule built from `change` can set (D25's "Then set"): each account the
 * card moved, as the `accountN` slot of the posting that held it (hledger
 * writes posting N from `accountN`), and the new description.
 *
 * The accounts are checked to begin with. The description is checked only
 * when it is the whole change: a rename alongside a recategorisation is as
 * often this one transaction's tidy-up as it is the payee's name, and a rule
 * that renames every match is the bigger commitment.
 */
export function ruleTargets(original: Transaction, change: CardChange): RuleTarget[] {
    const targets: RuleTarget[] = [];
    txnToForm(original).postings.forEach((posting, at) => {
        const to = change.accounts.get(posting.account);
        const field = `account${at + 1}`;
        if (to !== undefined && !targets.some((target) => target.field === field)) targets.push({field, value: to, defaultOn: true});
    });
    if (change.description !== null) targets.push({field: DESCRIPTION_FIELD, value: change.description, defaultOn: targets.length === 0});
    return targets;
}

/** The candidate rule, or null while it has no pattern or nothing to set. */
export function ruleSpec(field: string | null, pattern: string, targets: readonly RuleTarget[], on: ReadonlySet<string>): RuleSpec | null {
    const trimmed = pattern.trim();
    const assignments = targets.filter((target) => on.has(target.field)).map((target) => ({field: target.field, value: target.value}));
    if (trimmed === "" || assignments.length === 0) return null;
    return {matchers: [{field, pattern: trimmed}], assignments};
}

/**
 * The `%name` / `%N` field references hledger would find in an assignment
 * value. hledger reads `%` followed by a letter, digit, `_` or `-` as a CSV
 * field reference and substitutes that field, or nothing when there is no
 * such field. A `%` before a space, a dot or a bracket stays literal.
 *
 * There is no escape. Checked against hledger 1.52: `%%desc` gives `%` and
 * then the field, and `\%desc` keeps the backslash and still substitutes. So a
 * rule that sets "50%off" writes "50" and the builder can only warn
 * (`importReviewLive.integration.test.ts` pins the substitution).
 */
export function fieldReferences(value: string): string[] {
    return [...value.matchAll(/%[\p{L}\p{N}_-]+/gu)].map((match) => match[0]);
}

/** A changed row of the live check, as a header line and the lines the rule takes away and adds. */
export interface ChangedLines {
    readonly title: string;
    readonly removed: readonly string[];
    readonly added: readonly string[];
}

/**
 * Read one live-check change (`before`/`after` journal text) as what moved:
 * the transaction's first line, then the lines only `before` has and the
 * lines only `after` has. Runs of spaces are collapsed first: hledger
 * re-aligns the amounts of a transaction whose account got longer, and a
 * line that only moved its amount has not changed.
 */
export function changedLines(before: string, after: string): ChangedLines {
    const lines = (text: string): string[] =>
        text
            .split("\n")
            .map((line) => line.trim().replace(/\s+/g, " "))
            .filter((line) => line !== "");
    const was = lines(before);
    const now = lines(after);
    const wasSet = new Set(was);
    const nowSet = new Set(now);
    return {
        title: now[0] ?? was[0] ?? "",
        removed: was.filter((line) => !nowSet.has(line)),
        added: now.filter((line) => !wasSet.has(line)),
    };
}

/** The file name a path ends in, for "rows in checking.csv". */
export function baseName(path: string): string {
    const parts = path.split(/[/\\]/);
    return parts[parts.length - 1] || path;
}
