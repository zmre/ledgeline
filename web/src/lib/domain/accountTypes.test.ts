import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {
    ACCEPTED_TYPE_TAGS,
    cashPredicate,
    declaredTypes,
    inferAccountType,
    isAccountType,
    parseAccountTypeTag,
    resolveAccountType,
    type AccountDecl,
    type AccountType,
} from "./accountTypes";

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

// ===========================================================================
// The shared truth table (fixtures/account-types/classification-cases.json)
// ===========================================================================
//
// This module and `crates/ledgeline-core/src/reports/account_types.rs` are two
// implementations of ONE specification, and both are checked against the same
// file — see its `_comment`, and `crates/ledgeline-core/tests/account_types.rs`
// for the other half.
//
// The 2026-09 Projections sign bug is why this exists. This module's `type:`
// vocabulary was a SUBSET of the engine's, so a declaration it could not parse
// was dropped and the account fell through to English name inference:
// `income:contractors ; type: expenses` resolved as revenue here and as expense
// there. `signedQuantity` — the one place a scenario amount is negated — then
// negated a cost on its way to an engine that booked it as one, and a $100,000
// payroll paid money IN, forever (plan 22, amendment 42).
//
// A unit test for THIS module alone could not have caught that. Only a claim
// checked on both sides of the wire can.

interface TagRow {
    _why?: string;
    value: string;
    type: AccountType | null;
}
interface CaseRow {
    _why?: string;
    declared: [string, string][];
    account: string;
    type: AccountType | null;
    isRevenue: boolean;
    isExpense: boolean;
    isAsset: boolean;
    isEquity: boolean;
}

const shared = JSON.parse(readFileSync(new URL("../../../../fixtures/account-types/classification-cases.json", import.meta.url), "utf8")) as {
    tags: TagRow[];
    cases: CaseRow[];
};

describe("UNIT domain/accountTypes — the engine agrees with this file", () => {
    it("parses every `type:` spelling the shared table lists", () => {
        expect(shared.tags.length, "the shared table lost its tag rows").toBeGreaterThanOrEqual(37);
        for (const row of shared.tags) {
            expect(parseAccountTypeTag(row.value), `\`; type: ${row.value}\` must parse as the shared table says. ${row._why ?? ""}`).toBe(row.type);
        }
    });

    it("accepts EXACTLY the spellings the shared table lists, and no others", () => {
        // The test above pins every row of the fixture. This one pins the other
        // direction — that the fixture names every spelling the module takes —
        // and together they make the two sets EQUAL.
        //
        // Which is the check that would have caught the 2026-09 drift by
        // construction. Before it, `TYPE_BY_WORD` could gain a word with no
        // fixture row and nothing failed, so the engine was never told; the
        // table pinned examples where what it needed to pin was a VOCABULARY.
        // Adding a spelling now fails here until the shared file names it, and
        // the shared file is what the engine's own suite reads.
        const listed = shared.tags.filter((row) => row.type !== null).map((row) => row.value.trim().toLowerCase());
        expect([...ACCEPTED_TYPE_TAGS].sort()).toEqual([...new Set(listed)].sort());
    });

    it("resolves every (declarations, account) the shared table lists", () => {
        expect(shared.cases.length, "the shared table lost its resolution rows").toBeGreaterThanOrEqual(23);
        for (const row of shared.cases) {
            const decls: AccountDecl[] = row.declared.map(([name, tag]) => ({name, type: parseAccountTypeTag(tag)}));
            const declared = declaredTypes(decls);
            const context = `${row.account} against ${JSON.stringify(row.declared)}. ${row._why ?? ""}`;
            expect(resolveAccountType(row.account, declared), `resolving ${context}`).toBe(row.type);
            expect(isAccountType(row.account, declared, "revenue"), `isRevenue for ${context}`).toBe(row.isRevenue);
            expect(isAccountType(row.account, declared, "expense"), `isExpense for ${context}`).toBe(row.isExpense);
            expect(isAccountType(row.account, declared, "asset"), `isAsset for ${context}`).toBe(row.isAsset);
            expect(isAccountType(row.account, declared, "equity"), `isEquity for ${context}`).toBe(row.isEquity);
        }
    });
});
