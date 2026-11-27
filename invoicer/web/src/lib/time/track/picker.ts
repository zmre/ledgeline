// The account picker's options (plan 46 T-slots: Other… and the pin
// editor): with nothing typed, the current choice, then recent accounts,
// then the rest A–Z; typed, Ledgeline's segment-aware fuzzy match, recent
// accounts first among equally good matches. Create account… comes last
// when what's typed isn't an account yet.
import {matchAccounts} from "#ui/ui/accountMatch.js";

/** What the list offers: an account, or to create the typed one. */
export type PickerOption = {kind: "account"; account: string} | {kind: "create"; account: string};

/** The picker's options for `query` over `names` (recents and `current` first when nothing is typed). */
export function pickerOptions(
    query: string,
    names: readonly string[],
    recents: readonly string[],
    current: string | null,
    allowCreate: boolean
): PickerOption[] {
    const typed = query.trim();
    let accounts: string[];
    if (typed === "") {
        const first = [...(current === null ? [] : [current]), ...recents];
        const seen = new Set<string>();
        accounts = [...first, ...[...names].sort((a, b) => a.localeCompare(b))].filter((n) => (seen.has(n) ? false : (seen.add(n), true)));
    } else {
        const recency = new Map(recents.map((r, i) => [r, i]));
        accounts = matchAccounts(typed, names)
            .map((m, i) => ({...m, i}))
            .sort((a, b) => a.tier - b.tier || (recency.get(a.name) ?? Infinity) - (recency.get(b.name) ?? Infinity) || a.i - b.i)
            .map((m) => m.name);
    }
    const options: PickerOption[] = accounts.map((account) => ({kind: "account", account}));
    if (allowCreate && typed !== "" && !names.includes(typed)) options.push({kind: "create", account: typed});
    return options;
}
