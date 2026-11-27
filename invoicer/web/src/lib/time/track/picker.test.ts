// The account picker's order: current, recents, then A–Z with nothing typed;
// fuzzy (recent first among equals) when typed; Create account… last.
import {describe, expect, it} from "vitest";
import {pickerOptions} from "./picker.js";
import {baseTitle, prefixedTitle} from "./titlePrefix.js";

const names = ["internal:sales", "clients:acme:advisory", "clients:acme:tax-return", "internal:admin", "clients:globex:audit"];

describe("UNIT the account picker's options", () => {
    it("puts the current account, then recents, then the rest A–Z", () => {
        const options = pickerOptions("", names, ["internal:admin", "clients:acme:advisory"], "internal:sales", true).map((o) => o.account);
        expect(options).toEqual(["internal:sales", "internal:admin", "clients:acme:advisory", "clients:acme:tax-return", "clients:globex:audit"]);
    });

    it("matches by segments, recent accounts first among equally good ones", () => {
        const options = pickerOptions("ac", names, ["clients:acme:tax-return"], null, true);
        expect(options.slice(0, 2).map((o) => o.account)).toEqual(["clients:acme:tax-return", "clients:acme:advisory"]);
        expect(options.at(-1)).toEqual({kind: "create", account: "ac"});
    });

    it("offers no Create for an account that exists, or when creating is off", () => {
        expect(pickerOptions("internal:admin", names, [], null, true).some((o) => o.kind === "create")).toBe(false);
        expect(pickerOptions("new:one", names, [], null, false).some((o) => o.kind === "create")).toBe(false);
        expect(pickerOptions("new:one", names, [], null, true)).toEqual([{kind: "create", account: "new:one"}]);
    });
});

describe("UNIT the window title's timer", () => {
    it("puts the elapsed time in front, and takes it off again", () => {
        expect(prefixedTitle("Invoices · Ledgeline Invoicer", "1h 22m")).toBe("▶ 1h 22m · Invoices · Ledgeline Invoicer");
        expect(prefixedTitle("▶ 1h 22m · Invoices · Ledgeline Invoicer", "1h 23m")).toBe("▶ 1h 23m · Invoices · Ledgeline Invoicer");
        expect(prefixedTitle("▶ 1h 22m · Invoices · Ledgeline Invoicer", null)).toBe("Invoices · Ledgeline Invoicer");
        expect(baseTitle("Track · Ledgeline Invoicer")).toBe("Track · Ledgeline Invoicer");
    });
});
