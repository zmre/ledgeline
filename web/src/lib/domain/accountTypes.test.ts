import {describe, expect, it} from "vitest";
import {cashPredicate, declaredTypes, inferAccountType, parseAccountTypeTag, resolveAccountType, type AccountDecl} from "./accountTypes";

describe("UNIT domain/accountTypes", () => {
    describe("parseAccountTypeTag", () => {
        it("maps single-letter codes (any case, trimmed)", () => {
            expect(parseAccountTypeTag("C")).toBe("cash");
            expect(parseAccountTypeTag(" a ")).toBe("asset");
            expect(parseAccountTypeTag("l")).toBe("liability");
            expect(parseAccountTypeTag("E")).toBe("equity");
            expect(parseAccountTypeTag("R")).toBe("revenue");
            expect(parseAccountTypeTag("X")).toBe("expense");
            expect(parseAccountTypeTag("V")).toBe("conversion");
        });
        it("maps full words including the income alias", () => {
            expect(parseAccountTypeTag("Cash")).toBe("cash");
            expect(parseAccountTypeTag("asset")).toBe("asset");
            expect(parseAccountTypeTag("Income")).toBe("revenue");
            expect(parseAccountTypeTag("Revenue")).toBe("revenue");
        });
        it("returns null for anything unrecognized", () => {
            expect(parseAccountTypeTag("")).toBeNull();
            expect(parseAccountTypeTag("Z")).toBeNull();
            expect(parseAccountTypeTag("cashflow")).toBeNull();
        });
    });

    describe("inferAccountType (name fallback = hledger's default regexes)", () => {
        it("classifies cash-like asset names as cash, other assets as asset", () => {
            expect(inferAccountType("assets:bank:checking")).toBe("cash");
            expect(inferAccountType("assets:bank:wise:eur")).toBe("cash"); // descendant of a cash-like segment
            expect(inferAccountType("asset:savings")).toBe("cash"); // singular root
            expect(inferAccountType("assets:broker:taxable:aapl")).toBe("asset");
            expect(inferAccountType("assets")).toBe("asset");
        });
        it("classifies the other roots, and untyped names as null", () => {
            expect(inferAccountType("liabilities:cc:visa")).toBe("liability");
            expect(inferAccountType("equity:opening")).toBe("equity");
            expect(inferAccountType("income:salary")).toBe("revenue");
            expect(inferAccountType("expenses:bank")).toBe("expense"); // "bank" under expenses is NOT cash
            expect(inferAccountType("virtual:whatever")).toBeNull();
        });
    });

    describe("resolveAccountType (own → nearest ancestor → name)", () => {
        it("prefers a declared type over the name heuristic", () => {
            // A cash-NAMED account explicitly declared Asset is an Asset, not Cash.
            const declared = declaredTypes([{name: "assets:bank:checking", type: "asset"}]);
            expect(resolveAccountType("assets:bank:checking", declared)).toBe("asset");
        });
        it("inherits from the nearest declared ancestor, blocking name inference for descendants", () => {
            // `assets` declared Asset ⇒ an undeclared, cash-NAMED descendant inherits Asset (hledger semantics).
            const declared = declaredTypes([{name: "assets", type: "asset"}]);
            expect(resolveAccountType("assets:bankofamerica", declared)).toBe("asset");
            expect(resolveAccountType("assets:bankofamerica:sub", declared)).toBe("asset");
        });
        it("lets a nearer declaration override a farther one", () => {
            const declared = declaredTypes([
                {name: "assets", type: "asset"},
                {name: "assets:wallet", type: "cash"},
            ]);
            expect(resolveAccountType("assets:wallet:coins", declared)).toBe("cash");
            expect(resolveAccountType("assets:brokerage", declared)).toBe("asset");
        });
        it("falls back to the name when no ancestor is declared", () => {
            expect(resolveAccountType("assets:bank:checking", new Map())).toBe("cash");
        });
    });

    describe("cashPredicate", () => {
        it("with no declarations reduces to the name heuristic", () => {
            const isCash = cashPredicate([]);
            expect(isCash("assets:bank:checking")).toBe(true);
            expect(isCash("assets:broker:taxable:aapl")).toBe(false);
            expect(isCash("liabilities:cc:visa")).toBe(false);
        });
        it("honors declarations that diverge from names in BOTH directions", () => {
            const decls: AccountDecl[] = [
                {name: "assets", type: "asset"}, // blocks name-cash inference for undeclared asset descendants
                {name: "assets:wallet", type: "cash"}, // a non-cash NAME declared Cash
                {name: "assets:bank:checking", type: "asset"}, // a cash NAME declared Asset
                {name: "assets:bank:savings", type: null}, // present but untyped → inherits assets = Asset
            ];
            const isCash = cashPredicate(decls);
            expect(isCash("assets:wallet")).toBe(true);
            expect(isCash("assets:wallet:usdc")).toBe(true); // inherits Cash
            expect(isCash("assets:bank:checking")).toBe(false); // declared Asset beats the "checking" name
            expect(isCash("assets:bank:savings")).toBe(false); // untyped ⇒ inherits assets = Asset
            expect(isCash("assets:bankofamerica")).toBe(false); // undeclared, inherits assets = Asset (NOT name-cash)
        });
    });
});
