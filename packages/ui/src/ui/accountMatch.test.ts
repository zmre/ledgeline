// The matcher carries most of this feature's test weight, because it is the
// part that can be wrong in a hundred quiet ways and the part jsdom can verify
// completely. The combobox around it is mostly wiring.

import {describe, expect, it} from "vitest";
import {longestCommonPrefix, matchAccounts, tabCompletion} from "./accountMatch";

const ACCOUNTS = [
    "expenses:groceries:costco",
    "expenses:groceries:whole-foods",
    "expenses:gas",
    "expenses:utilities:electric",
    "assets:broker:taxable:cash",
    "assets:bank:checking",
    "income:salary",
];

const names = (query: string, from: readonly string[] = ACCOUNTS): string[] => matchAccounts(query, from).map((m) => m.name);

describe("UNIT matchAccounts", () => {
    it("matches segment prefixes from the root", () => {
        expect(names("ex:gr")).toEqual(["expenses:groceries:costco", "expenses:groceries:whole-foods"]);
    });

    it("matches a run-together query as a subsequence", () => {
        // `exgro` — no colons, no contiguous substring. This is the case a
        // <datalist> could never do.
        expect(names("exgro")).toEqual(["expenses:groceries:costco", "expenses:groceries:whole-foods"]);
    });

    it("finds an account by its leaf alone", () => {
        expect(names("costco")).toEqual(["expenses:groceries:costco"]);
    });

    it("matches segments in order without starting at the root", () => {
        // Ranked first, not exclusively: `whole-foods` is still reachable from
        // `gr:co` by the last-resort subsequence tier, which is the point of
        // having tiers. Asserting exclusivity here would be asserting that the
        // fuzzy matcher is not fuzzy.
        expect(names("gr:co")[0]).toBe("expenses:groceries:costco");
    });

    it("ranks a shared segment prefix above a looser match", () => {
        // `e:g` prefixes expenses:groceries and expenses:gas from the root, so
        // those beat anything reachable only by subsequence.
        const ranked = names("e:g");

        expect(ranked.slice(0, 3)).toEqual(["expenses:gas", "expenses:groceries:costco", "expenses:groceries:whole-foods"]);
    });

    it("puts an exact match first even when longer accounts also match", () => {
        expect(names("expenses:gas")[0]).toBe("expenses:gas");
    });

    it("is case-insensitive but returns the journal's own spelling", () => {
        expect(names("EXPENSES:GAS")).toEqual(["expenses:gas"]);
    });

    it("returns everything alphabetically for an empty query", () => {
        // So the popup is useful before the first keystroke.
        expect(names("")).toEqual([...ACCOUNTS].sort((a, b) => a.localeCompare(b)));
    });

    it("treats a trailing colon as 'show me the children'", () => {
        // `expenses:` splits to ["expenses", ""], and an empty segment prefixes
        // anything — the behaviour falls out rather than being special-cased.
        // Shortest first within the tier, hence electric before whole-foods.
        expect(names("expenses:")).toEqual(["expenses:gas", "expenses:groceries:costco", "expenses:utilities:electric", "expenses:groceries:whole-foods"]);
    });

    it("returns nothing for a query longer than any account", () => {
        expect(names("expenses:groceries:costco:and:then:some")).toEqual([]);
    });

    it("returns nothing for a query that simply does not occur", () => {
        expect(names("zzzz")).toEqual([]);
    });

    it("breaks ties on length, then alphabetically, so results do not depend on journal order", () => {
        const shuffled = [...ACCOUNTS].reverse();

        expect(names("ex:gr", shuffled)).toEqual(names("ex:gr"));
    });

    it("handles non-ASCII account names", () => {
        const accounts = ["expenses:café:espresso", "expenses:caña"];

        expect(matchAccounts("caf", accounts).map((m) => m.name)).toEqual(["expenses:café:espresso"]);
        expect(matchAccounts("expenses:ca", accounts).map((m) => m.name)).toEqual(["expenses:caña", "expenses:café:espresso"]);
    });

    it("caps its results", () => {
        const many = Array.from({length: 200}, (_, at) => `expenses:item${at}`);

        expect(matchAccounts("ex", many, 10)).toHaveLength(10);
    });
});

describe("UNIT longestCommonPrefix", () => {
    it("is empty for no names", () => {
        expect(longestCommonPrefix([])).toBe("");
    });

    it("is the whole name for one name", () => {
        expect(longestCommonPrefix(["expenses:gas"])).toBe("expenses:gas");
    });

    it("stops mid-segment rather than at a colon", () => {
        // Character-wise, not segment-wise: stopping at `expenses:` would throw
        // away a character the user has already earned.
        expect(longestCommonPrefix(["expenses:groceries:costco", "expenses:gas"])).toBe("expenses:g");
    });

    it("is empty when nothing is shared", () => {
        expect(longestCommonPrefix(["assets:bank", "income:salary"])).toBe("");
    });
});

describe("UNIT tabCompletion", () => {
    it("extends to the shared prefix of the top tier", () => {
        expect(tabCompletion("ex:g", matchAccounts("ex:g", ACCOUNTS))).toBe("expenses:g");
    });

    it("completes fully when only one account matches", () => {
        expect(tabCompletion("costco", matchAccounts("costco", ACCOUNTS))).toBe("expenses:groceries:costco");
    });

    it("returns null with no matches, so Tab falls through to normal focus traversal", () => {
        // The anti-trap rule: if Tab can never be a no-op, the form is a keyboard
        // trap and there is no way out of the field.
        expect(tabCompletion("zzzz", matchAccounts("zzzz", ACCOUNTS))).toBeNull();
    });

    it("returns null when there is nothing left to add", () => {
        expect(tabCompletion("expenses:gas", matchAccounts("expenses:gas", ACCOUNTS))).toBeNull();
    });

    it("normalizes case even when it adds no characters", () => {
        expect(tabCompletion("EXPENSES:GAS", matchAccounts("EXPENSES:GAS", ACCOUNTS))).toBe("expenses:gas");
    });

    it("ignores lower tiers when computing the shared prefix", () => {
        // A loose subsequence match would drag the LCP back to nothing, making
        // Tab do nothing exactly when the ranking is most confident.
        const matches = matchAccounts("ex:gr", ACCOUNTS);

        expect(tabCompletion("ex:gr", matches)).toBe("expenses:groceries:");
    });
});
