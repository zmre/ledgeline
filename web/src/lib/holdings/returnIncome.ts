// Plan 48 C6: which income accounts' credits are RETURN on the holdings they
// buy. The engine decides (`ReturnIncome` in holdings/types.rs); this mirrors
// its rule so Settings can show which accounts a filter list catches.
//
// Relative imports only: lib/holdings/ is held to the purity test.

import type {AccountClasses} from "../domain/accountClass";

/** Whether `account`'s name contains any of `filters`, ignoring case (the engine's `ReturnIncome::matches`). */
export function matchesReturnIncome(account: string, filters: readonly string[]): boolean {
    const name = account.toLowerCase();
    return filters.some((filter) => {
        const needle = filter.trim().toLowerCase();
        return needle !== "" && name.includes(needle);
    });
}

/** The journal's income accounts, split by whether `filters` make their credits return. Sorted by name. */
export function splitIncomeAccounts(classes: AccountClasses, filters: readonly string[]): {asReturn: string[]; asNewMoney: string[]} {
    const income = [...classes].filter(([, cls]) => cls.type === "revenue").map(([name]) => name);
    income.sort();
    return {
        asReturn: income.filter((name) => matchesReturnIncome(name, filters)),
        asNewMoney: income.filter((name) => !matchesReturnIncome(name, filters)),
    };
}
