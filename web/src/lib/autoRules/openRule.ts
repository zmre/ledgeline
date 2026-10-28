// The imperative half of "this posting came from a rule": navigate to
// Settings › Automatic postings with that rule focused (plan 30 Decision 8).
// Split from `nav.ts` so the URL arithmetic stays testable in the node
// project without SvelteKit's router around it.

import {goto} from "$app/navigation";
import {resolve} from "$app/paths";
import {autoRuleSearch} from "./nav";

export async function openAutoRule(rule: number): Promise<void> {
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve("/settings") IS the route id; the query string is appended
    await goto(`${resolve("/settings")}${autoRuleSearch(rule)}`);
}
