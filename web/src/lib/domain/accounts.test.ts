import {describe, expect, it} from "vitest";
import {accountMatches, buildAccountTree, categorize, clampAccount} from "./accounts";

describe("UNIT accounts", () => {
    describe("buildAccountTree", () => {
        it("builds a nested, sorted tree from flat names", () => {
            const tree = buildAccountTree([
                "expenses:food",
                "assets:bank:checking",
                "assets:bank:savings",
                "assets:broker",
                "expenses",
                "assets:bank",
                "assets",
            ]);
            expect(tree.map((node) => node.fullName)).toEqual(["assets", "expenses"]);
            const assets = tree[0];
            expect(assets.name).toBe("assets");
            expect(assets.children.map((node) => node.fullName)).toEqual(["assets:bank", "assets:broker"]);
            expect(assets.children[0].children.map((node) => node.name)).toEqual(["checking", "savings"]);
        });

        it("creates missing intermediate ancestors", () => {
            const tree = buildAccountTree(["assets:bank:checking"]);
            expect(tree).toHaveLength(1);
            expect(tree[0].fullName).toBe("assets");
            expect(tree[0].children[0].fullName).toBe("assets:bank");
            expect(tree[0].children[0].children[0].fullName).toBe("assets:bank:checking");
        });

        it("handles empty input", () => {
            expect(buildAccountTree([])).toEqual([]);
        });
    });

    describe("clampAccount", () => {
        it("clamps to the requested depth", () => {
            expect(clampAccount("assets:morganstanley:checking", 1)).toBe("assets");
            expect(clampAccount("assets:morganstanley:checking", 2)).toBe("assets:morganstanley");
            expect(clampAccount("assets:morganstanley:checking", 3)).toBe("assets:morganstanley:checking");
            expect(clampAccount("assets", 4)).toBe("assets");
        });
    });

    describe("accountMatches", () => {
        it("matches exact names and sub-accounts only", () => {
            expect(accountMatches("assets:bank", "assets:bank")).toBe(true);
            expect(accountMatches("assets:bank", "assets:bank:checking")).toBe(true);
            expect(accountMatches("assets:bank", "assets:bankx")).toBe(false);
            expect(accountMatches("assets:bank:checking", "assets:bank")).toBe(false);
        });
    });

    describe("categorize", () => {
        it("maps hledger-convention roots, singular or plural, any case", () => {
            expect(categorize("assets:bank:checking")).toBe("asset");
            expect(categorize("Asset:cash")).toBe("asset");
            expect(categorize("liabilities:card")).toBe("liability");
            expect(categorize("equity:opening")).toBe("equity");
            expect(categorize("revenues:salary")).toBe("revenue");
            expect(categorize("income:consulting")).toBe("revenue");
            expect(categorize("expenses:food")).toBe("expense");
            expect(categorize("virtual:budget")).toBe("other");
        });
    });
});
