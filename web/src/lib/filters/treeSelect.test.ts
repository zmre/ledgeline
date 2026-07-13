import {describe, expect, it} from "vitest";
import {buildAccountTree} from "$lib/domain/accounts";
import {filterTree, selectionState, toggleSubtreeRoot} from "./treeSelect";

const names = ["assets:bank:checking", "assets:bank:savings", "assets:broker", "expenses:food:groceries", "expenses:rent"];

describe("UNIT treeSelect", () => {
    describe("selectionState", () => {
        it("is checked for a selected name and everything beneath it", () => {
            const selected = new Set(["assets:bank"]);
            expect(selectionState(selected, "assets:bank")).toBe("checked");
            expect(selectionState(selected, "assets:bank:checking")).toBe("checked");
        });

        it("is indeterminate for ancestors of a selection only", () => {
            const selected = new Set(["assets:bank:checking"]);
            expect(selectionState(selected, "assets")).toBe("indeterminate");
            expect(selectionState(selected, "assets:bank")).toBe("indeterminate");
            expect(selectionState(selected, "assets:bank:savings")).toBe("unchecked");
            expect(selectionState(selected, "expenses")).toBe("unchecked");
        });

        it("does not match name prefixes as ancestors", () => {
            const selected = new Set(["assets:bank"]);
            expect(selectionState(selected, "assets:bankx")).toBe("unchecked");
        });

        it("is unchecked everywhere when nothing is selected", () => {
            expect(selectionState(new Set(), "assets")).toBe("unchecked");
        });
    });

    describe("toggleSubtreeRoot (shared by the filters and holdings stores)", () => {
        it("adds then removes a plain selection without mutating the input", () => {
            const empty = new Set<string>();
            const added = toggleSubtreeRoot(empty, "expenses:food");
            expect([...added]).toEqual(["expenses:food"]);
            expect(empty.size).toBe(0);
            expect(toggleSubtreeRoot(added, "expenses:food").size).toBe(0);
        });

        it("selecting a parent prunes selected descendants (stores only the subtree root)", () => {
            let selected: ReadonlySet<string> = new Set(["assets:bank:checking", "assets:bank:savings", "expenses:food"]);
            selected = toggleSubtreeRoot(selected, "assets:bank");
            expect([...selected].sort()).toEqual(["assets:bank", "expenses:food"]);
        });

        it("toggling a covered descendant deselects the covering ancestor", () => {
            expect(toggleSubtreeRoot(new Set(["assets"]), "assets:bank:checking").size).toBe(0);
        });

        it("does not treat name prefixes as ancestors (assets:bank vs assets:bankx)", () => {
            const selected = toggleSubtreeRoot(new Set(["assets:bank"]), "assets:bankx");
            expect([...selected].sort()).toEqual(["assets:bank", "assets:bankx"]);
        });
    });

    describe("filterTree", () => {
        const tree = buildAccountTree(names);

        it("returns the tree unchanged for an empty or blank query", () => {
            expect(filterTree(tree, "")).toBe(tree);
            expect(filterTree(tree, "   ")).toBe(tree);
        });

        it("finds deep accounts and keeps their ancestors visible", () => {
            const found = filterTree(tree, "groceries");
            expect(found.map((n) => n.fullName)).toEqual(["expenses"]);
            expect(found[0].children.map((n) => n.fullName)).toEqual(["expenses:food"]);
            expect(found[0].children[0].children.map((n) => n.fullName)).toEqual(["expenses:food:groceries"]);
        });

        it("keeps the whole subtree of a matching node", () => {
            const found = filterTree(tree, "bank");
            expect(found.map((n) => n.fullName)).toEqual(["assets"]);
            const bank = found[0].children[0];
            expect(bank.fullName).toBe("assets:bank");
            expect(bank.children.map((n) => n.fullName)).toEqual(["assets:bank:checking", "assets:bank:savings"]);
        });

        it("is case-insensitive and matches across segments of the full name", () => {
            const found = filterTree(tree, "ASSETS:BRO");
            expect(found.map((n) => n.fullName)).toEqual(["assets"]);
            expect(found[0].children.map((n) => n.fullName)).toEqual(["assets:broker"]);
        });

        it("returns an empty list when nothing matches", () => {
            expect(filterTree(tree, "zzz")).toEqual([]);
        });
    });
});
