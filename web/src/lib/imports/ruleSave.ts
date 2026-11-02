// "Remember as a rule" writes (plan 32 D25, D27): one `if` block appended at
// the END of a rules file (hledger's last matching block wins), through the
// rules editor's own PUT and pinned to the revision just read — so a rule can
// never land on top of an edit made elsewhere. The import's builder and the
// Journal view's share this; neither writes rules any other way.

import {classify, type EditResult} from "#lib/api/editFailure.js";
import {LedgelineApi} from "#lib/api/native.js";
import {decodeRulesDoc} from "#lib/api/nativeDecode.js";
import {appendRule, blankRule, toForm, toSaveRequest, type IfBlockItem, type RulesForm} from "./model";
import type {RuleSpec} from "./reviewTypes";
import {rulesStore} from "./rulesStore.svelte";

/** Append `rule` to the rules file `rulesId` on the engine at `serverUrl`. Writes only that file. */
export async function appendRuleTo(serverUrl: string, rulesId: string, rule: RuleSpec): Promise<EditResult> {
    let baseline: RulesForm;
    try {
        baseline = toForm(decodeRulesDoc(await new LedgelineApi(serverUrl).getRules(rulesId)));
    } catch (error) {
        return {ok: false, failure: classify(error)};
    }
    const block: IfBlockItem = {
        ...blankRule(),
        // One group per matcher: OR-ed, hledger's stacked `if` lines.
        groups: rule.matchers.map((matcher) => ({matchers: [{field: matcher.field ?? "", pattern: matcher.pattern}]})),
        assignments: rule.assignments.map((assignment) => ({field: assignment.field, value: assignment.value})),
    };
    const outcome = await rulesStore.save(rulesId, toSaveRequest(baseline, {...baseline, items: appendRule(baseline.items, block)}));
    return outcome.ok ? {ok: true} : {ok: false, failure: outcome.failure};
}
