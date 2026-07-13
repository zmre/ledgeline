import {beforeEach, describe, expect, it} from "vitest";
import {localToday} from "./filters.svelte";
import {defaultScope, holdingsScope} from "./holdings.svelte";

describe("UNIT holdings scope store (module state, reset between tests)", () => {
    beforeEach(() => {
        holdingsScope.replace(defaultScope());
    });

    it("defaults to include-everything as of today (never a remembered date)", () => {
        expect(holdingsScope.value.asOf).toBe(localToday());
        expect(holdingsScope.value.mode).toBe("include");
        expect(holdingsScope.value.accounts.size).toBe(0);
    });

    it("toggleAccount keeps the subtree-root invariant (same rules as the journal filters)", () => {
        holdingsScope.toggleAccount("assets:broker:taxable:vti");
        holdingsScope.toggleAccount("assets:broker:taxable:aapl");
        holdingsScope.toggleAccount("assets:broker");
        expect([...holdingsScope.value.accounts]).toEqual(["assets:broker"]);
        holdingsScope.toggleAccount("assets:broker:taxable:vti");
        expect(holdingsScope.value.accounts.size).toBe(0);
    });

    it("setMode switches include/exclude and keeps the selection", () => {
        holdingsScope.toggleAccount("assets:broker");
        holdingsScope.setMode("exclude");
        expect(holdingsScope.value.mode).toBe("exclude");
        expect([...holdingsScope.value.accounts]).toEqual(["assets:broker"]);
    });

    it("setAsOf changes only the date", () => {
        holdingsScope.toggleAccount("assets:broker");
        holdingsScope.setAsOf("2025-01-01");
        expect(holdingsScope.value.asOf).toBe("2025-01-01");
        expect([...holdingsScope.value.accounts]).toEqual(["assets:broker"]);
    });

    it("clear drops the selection but keeps mode and asOf", () => {
        holdingsScope.toggleAccount("assets:broker");
        holdingsScope.setMode("exclude");
        holdingsScope.setAsOf("2025-01-01");
        holdingsScope.clear();
        expect(holdingsScope.value.accounts.size).toBe(0);
        expect(holdingsScope.value.mode).toBe("exclude");
        expect(holdingsScope.value.asOf).toBe("2025-01-01");
    });

    it("replace swaps the whole scope and copies the account set", () => {
        const accounts = new Set(["assets:broker"]);
        holdingsScope.replace({accounts, mode: "exclude", asOf: "2024-12-31"});
        accounts.add("expenses"); // caller mutation must not leak in
        expect([...holdingsScope.value.accounts]).toEqual(["assets:broker"]);
        expect(holdingsScope.value.mode).toBe("exclude");
        expect(holdingsScope.value.asOf).toBe("2024-12-31");
    });
});
