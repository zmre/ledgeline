// A customer page's time accounts: which rows link the customer (own or
// inherited, by ledger customer or title), archived ones left out.
import {describe, expect, it} from "vitest";
import {accountRow, accountsView, qty} from "../testing.js";
import {customerQuery, linkedAccounts, monthOf, monthQuery, unbilledQuery} from "./customerTime.js";

describe("UNIT linkedAccounts", () => {
    it("lists the customer's account and the accounts that inherit it, as a tree", () => {
        const rows = linkedAccounts(accountsView(), ["Acme Corp", "Acme Corporation"]);
        expect(rows.map((r) => [r.account, r.indent, r.inheritedFrom])).toEqual([
            ["clients:acme", 0, null],
            ["clients:acme:advisory", 1, "clients:acme"],
            ["clients:acme:tax-return", 1, "clients:acme"],
        ]);
        expect(rows[0]).toMatchObject({month: 244 * 60, unbilled: 244 * 60, customer: "Acme Corp"});
    });

    it("matches the title too, and finds nothing for another customer", () => {
        expect(linkedAccounts(accountsView(), ["acme-ledger", "Acme Corp"])).toHaveLength(3);
        expect(linkedAccounts(accountsView(), ["Globex", ""])).toEqual([]);
    });

    it("leaves archived accounts out", () => {
        const view = accountsView({
            accounts: [
                accountRow({month: qty(10)}),
                accountRow({account: "clients:acme:old", depth: 2, archived: {value: "yes", from: "clients:acme:old", inherited: false}}),
            ],
        });
        expect(linkedAccounts(view, ["Acme Corp"]).map((r) => r.account)).toEqual(["clients:acme"]);
    });
});

describe("UNIT Entries queries", () => {
    it("names the account, the period and the billing state", () => {
        expect(monthQuery("clients:acme")).toBe("period=this-month&account=clients%3Aacme");
        expect(unbilledQuery("clients:acme")).toBe("period=all&billed=unbilled&account=clients%3Aacme");
        expect(customerQuery("Acme Corp")).toBe("period=all&customer=Acme+Corp");
        expect(monthOf("2026-11-26")).toBe("2026-11");
    });
});
