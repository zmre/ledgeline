// "Remember as a rule" on a JOURNAL card (plan 32 D27): the rules files the
// transaction could have come from, and the rule builder's source for the one
// picked. The engine infers the candidates (`GET /api/review/rules-for`); the
// live check is `POST /api/review/rule-preview`; saving is the same rules-PUT
// append the import's builder uses (`imports/ruleSave.ts`).
//
// The import's source is one fixed file. A journal card's is a pick from
// candidates, so the overlay gets a `RuleChoice` and hands the builder
// `choice.source(picked)` — the builder's own contract stays one file.
//
// No Svelte/DOM imports; `ruleChoice` is pure over its I/O.

import type {EditResult} from "#lib/api/editFailure.js";
import {decodeJournalRulePreview} from "#lib/api/importReviewDecode.js";
import {LedgelineApi} from "#lib/api/native.js";
import {decodeRulesFor} from "#lib/api/reviewDecode.js";
import type {RuleChoice, RulePreview, RuleSource, RuleSpec, RulesCandidate, RulesFor} from "#lib/imports/reviewTypes.js";
import {descriptionPrefill, rulesFileLabel} from "#lib/imports/ruleBuilder.js";
import {appendRuleTo} from "#lib/imports/ruleSave.js";
import {settings} from "#lib/stores/settings.svelte.js";

/** What a journal card's rule source does over the network: the live check and the save. */
export interface JournalRuleIo {
    preview(index: number, rulesId: string, rule: RuleSpec, signal?: AbortSignal): Promise<RulePreview>;
    save(rulesId: string, rule: RuleSpec): Promise<EditResult>;
}

/** The rule as the wire carries it: just the matchers and the assignments. */
function wireRule(rule: RuleSpec): {matchers: {field: string | null; pattern: string}[]; assignments: {field: string; value: string}[]} {
    return {
        matchers: rule.matchers.map((matcher) => ({field: matcher.field, pattern: matcher.pattern})),
        assignments: rule.assignments.map((assignment) => ({field: assignment.field, value: assignment.value})),
    };
}

/** The engine at `serverUrl`'s preview and the shared rules-PUT append. */
export function engineRuleIo(serverUrl: string): JournalRuleIo {
    return {
        preview: async (index, rulesId, rule, signal) =>
            decodeJournalRulePreview(await new LedgelineApi(serverUrl, {signal}).reviewRulePreview({index, rulesId, rule: wireRule(rule)})),
        save: (rulesId, rule) => appendRuleTo(serverUrl, rulesId, rule),
    };
}

/**
 * The choice `found` offers, or null when it has no candidates (the toggle is
 * hidden). `recommended` is the one candidate the engine recommends, or null
 * when `needsChoice` — then nothing is preselected and no rule may be saved
 * until the user picks.
 */
export function ruleChoice(found: RulesFor, io: JournalRuleIo): RuleChoice | null {
    if (found.candidates.length === 0) return null;
    const recommended = found.needsChoice ? null : (found.candidates.find((candidate) => candidate.recommended)?.rulesId ?? null);
    return Object.freeze({
        candidates: found.candidates,
        recommended,
        needsChoice: found.needsChoice,
        source(rulesId: string): RuleSource {
            const candidate = found.candidates.find((entry) => entry.rulesId === rulesId);
            if (candidate === undefined) throw new Error(`${rulesId} is not a rules file this transaction could have come from`);
            return Object.freeze({
                prefill: descriptionPrefill(candidate),
                fileLabel: candidate.label,
                rulesLabel: rulesFileLabel(
                    candidate.rulesId,
                    found.candidates.map((entry) => entry.rulesId)
                ),
                account: candidate.account1,
                preview: (rule: RuleSpec, signal?: AbortSignal) => io.preview(found.index, rulesId, rule, signal),
                save: (rule: RuleSpec) => io.save(rulesId, rule),
                // A new rule changes no transaction already in the journal; the
                // card's own save (which the overlay makes after the rule's) does.
                refresh: () => Promise.resolve(null),
            });
        },
    });
}

/**
 * The short reason beside "Saving to citi-2026.csv.rules" (D27): "newest of 2
 * for citi" when `candidate` is the newest of a family of several (the
 * highest year among the candidates of its family, or a family with no years,
 * whose pick is already its newest). An older member of a family gets its
 * LAST reason, the engine's word on its place in the family ("Named for 2025,
 * this transaction's year"); the first is always the account, which every
 * candidate shares. A file alone in its family gets that first reason. Empty
 * when the engine gave none.
 */
export function fileReason(candidates: readonly RulesCandidate[], candidate: RulesCandidate): string {
    if (candidate.familySize > 1) {
        const own = candidate.year;
        const years = candidates.filter((other) => other.family === candidate.family).map((other) => other.year);
        const newest = years.every((year) => year === null) || (own !== null && years.every((year) => year === null || year <= own));
        if (newest) return `newest of ${candidate.familySize} for ${candidate.family}`;
        return candidate.reasons[candidate.reasons.length - 1] ?? "";
    }
    return candidate.reasons[0] ?? "";
}

/**
 * Load the choice for journal transaction `index` from the connected engine:
 * null with no engine or no candidates. Rejects with the request's error (the
 * overlay shows the toggle only on success).
 */
export async function loadRuleChoice(index: number, signal?: AbortSignal): Promise<RuleChoice | null> {
    const url = settings.serverUrl;
    if (url === null) return null;
    return ruleChoice(decodeRulesFor(await new LedgelineApi(url, {signal}).reviewRulesFor(index)), engineRuleIo(url));
}
