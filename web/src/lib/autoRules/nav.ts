// Where a generated posting sends you: Settings › Automatic postings with its
// rule focused (plan 30 Decision 8). One builder, so the journal chips and the
// transaction modal can never disagree about the URL.

import {paramsToSearch} from "#lib/settings/params.js";

/** `/settings?tab=auto&rule=N`, relative to the app's base path (`resolve` it). */
export function autoRuleSearch(rule: number): string {
    return `?${paramsToSearch({tab: "auto", rule})}`;
}
