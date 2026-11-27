// The Accounts tree's folding, the lists other pages take from it, and what
// the account dialog writes.
import {describe, expect, it} from "vitest";
import {accountRow, accountsView} from "../testing.js";
import {formOf, inheritedText, ownValue, tagChanges} from "./edit.js";
import {ancestorsOf, customersOf, isArchived, parentOf, pickerAccounts, visibleRows, withChildren} from "./tree.js";

describe("UNIT time accounts tree", () => {
    const rows = accountsView().accounts;

    it("knows parents, ancestors and branches", () => {
        expect(parentOf("clients:acme:tax")).toBe("clients:acme");
        expect(parentOf("clients")).toBeNull();
        expect(ancestorsOf("a:b:c")).toEqual(["a:b", "a"]);
        expect([...withChildren(rows)].sort()).toEqual(["clients", "clients:acme", "internal"]);
    });

    it("hides what is under a folded branch, at any depth", () => {
        expect(visibleRows(rows, new Set()).length).toBe(rows.length);
        expect(visibleRows(rows, new Set(["clients:acme"])).map((r) => r.account)).toEqual(["clients", "clients:acme", "internal", "internal:old"]);
        expect(visibleRows(rows, new Set(["clients"])).map((r) => r.account)).toEqual(["clients", "internal", "internal:old"]);
    });

    it("the picker leaves archived accounts out and takes the undeclared", () => {
        expect(isArchived(rows[5])).toBe(true);
        expect(pickerAccounts(accountsView())).toEqual([
            "clients",
            "clients:acme",
            "clients:acme:advisory",
            "clients:acme:tax-return",
            "internal",
            "internal:training",
        ]);
        expect(customersOf(accountsView())).toEqual(["Acme Corp"]);
    });
});

describe("UNIT time account dialog writes", () => {
    const acme = accountRow();
    const advisory = accountRow({account: "clients:acme:advisory", customer: {value: "Acme Corp", from: "clients:acme", inherited: true}});

    it("an inherited value is not the account's own", () => {
        expect(ownValue(advisory.customer)).toBe("");
        expect(ownValue(acme.customer)).toBe("Acme Corp");
        expect(formOf(advisory)).toEqual({customer: "", billable: "", item: "ARCH", archived: false});
        expect(inheritedText(advisory.customer)).toBe("from clients:acme");
        expect(inheritedText(acme.customer)).toBe("");
    });

    it("writes only what changed; empty removes the tag", () => {
        expect(tagChanges(acme, formOf(acme))).toEqual([]);
        expect(tagChanges(acme, {customer: " Globex ", billable: "no", item: "", archived: true})).toEqual([
            {tag: "customer", value: "Globex"},
            {tag: "billable", value: "no"},
            {tag: "item", value: null},
            {tag: "archived", value: "yes"},
        ]);
        const archived = accountRow({archived: {value: "yes", from: "clients:acme", inherited: false}});
        expect(tagChanges(archived, {...formOf(archived), archived: false})).toEqual([{tag: "archived", value: null}]);
    });
});
