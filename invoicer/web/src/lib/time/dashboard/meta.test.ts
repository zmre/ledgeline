import {describe, expect, it} from "vitest";
import {metaOf} from "./meta.js";

const acc = (account: string, seconds: number, customer: {value: string; from: string} | null = null, billable: string | null = null) => ({
    account,
    depth: account.split(":").length - 1,
    customer,
    billable: billable === null ? null : {value: billable},
    total: {seconds},
});

const STATE = {configured: true, person: "pat", weekly_target: 30};

describe("UNIT time dashboard meta", () => {
    const acme = {value: "Acme Corp", from: "clients:acme"};
    const globex = {value: "Globex", from: "clients:globex"};
    const accounts = {
        accounts: [
            acc("clients", 5000, null, "yes"),
            acc("clients:acme", 1000, acme, "yes"),
            acc("clients:acme:tax", 800, acme, "yes"),
            acc("clients:globex", 4000, globex, "yes"),
            acc("internal", 9000, null, "no"),
        ],
        undeclared: ["internal:training:course"],
    };
    const people = [
        {key: ["sam"], quantity: {seconds: 100}},
        {key: ["pat"], quantity: {seconds: 900}},
        {key: [""], quantity: {seconds: 50}},
    ];

    it("orders branches, customers and people by all-time hours, customers counted once per subtree", () => {
        const meta = metaOf(STATE, accounts, people);
        expect(meta.branchOrder).toEqual(["internal", "clients"]);
        expect(meta.customerOrder).toEqual(["Globex", "Acme Corp"]);
        expect(meta.customers).toEqual(["Acme Corp", "Globex"]);
        expect(meta.people).toEqual(["pat", "sam", ""]);
        expect(meta.maxDepth).toBe(3);
        expect(metaOf(STATE, {accounts: [acc("internal:x", 0)], undeclared: ["internal:x"]}, []).accounts).toEqual(["internal:x"]);
        expect(meta.billableCustomers).toBe(true);
        expect(meta.anyTime).toBe(true);
        expect(meta.weeklyTarget).toBe(30);
        expect(meta.firstDay).toBeNull();
        expect(metaOf(STATE, accounts, people, ["2025-03", "2026-11"]).firstDay).toBe("2025-03-01");
    });

    it("has no Unbilled tile without a billable account that names a customer", () => {
        const plain = {accounts: [acc("clients", 0, null, "yes"), acc("clients:acme", 0, acme, "no")], undeclared: []};
        expect(metaOf(STATE, plain, []).billableCustomers).toBe(false);
        expect(metaOf(STATE, plain, []).anyTime).toBe(false);
    });
});
