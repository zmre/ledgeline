import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {accountTermOf, accountTypeOf, isAccountType, NO_ACCOUNT_CLASSES, type AccountClass, type AccountClasses} from "./accountClass";

const cls = (type: AccountClass["type"], typeDeclared = false, term: AccountClass["term"] = null, termGuessed = false): AccountClass => ({
    type,
    typeDeclared,
    term,
    termGuessed,
});

describe("UNIT domain/accountClass", () => {
    describe("accountTypeOf", () => {
        it("answers from the engine's table for every name it classified, even against the name", () => {
            const classes: AccountClasses = new Map([
                ["income:contractors", cls("expense", true)],
                ["activo", cls("asset")],
            ]);
            expect(accountTypeOf("income:contractors", classes)).toBe("expense");
            expect(accountTypeOf("activo", classes)).toBe("asset");
        });

        it("inherits a DECLARED ancestor's type for a name the engine never saw", () => {
            const classes: AccountClasses = new Map([
                ["ingresos", cls("revenue", true)],
                ["ingresos:nomina", cls("revenue", true)],
            ]);
            expect(accountTypeOf("ingresos:nomina:bonus", classes)).toBe("revenue");
            expect(accountTypeOf("ingresos:new", classes)).toBe("revenue");
        });

        it("falls back to name inference when the nearest known ancestor's type was not declared", () => {
            // `activo` resolved to asset only through its declared CHILDREN; a
            // new child has none, so the engine would answer by name — nothing.
            const classes: AccountClasses = new Map([
                ["activo", cls("asset")],
                ["assets", cls("asset")],
            ]);
            expect(accountTypeOf("activo:nuevo", classes)).toBeNull();
            // …and a cash-like segment in the new part still reads as cash.
            expect(accountTypeOf("assets:newbank:checking", classes)).toBe("cash");
        });

        it("infers from the name with no table at all, and survives degenerate names", () => {
            expect(accountTypeOf("income:salary", NO_ACCOUNT_CLASSES)).toBe("revenue");
            expect(accountTypeOf("", NO_ACCOUNT_CLASSES)).toBeNull();
            expect(accountTypeOf(":assets", NO_ACCOUNT_CLASSES)).toBeNull();
            expect(accountTypeOf("::", NO_ACCOUNT_CLASSES)).toBeNull();
        });
    });

    it("isAccountType folds the subtypes into their roots", () => {
        const classes: AccountClasses = new Map([
            ["a", cls("cash")],
            ["g", cls("gain")],
            ["v", cls("conversion")],
        ]);
        expect(isAccountType("a", classes, "asset")).toBe(true);
        expect(isAccountType("a", classes, "cash")).toBe(true);
        expect(isAccountType("g", classes, "revenue")).toBe(true);
        expect(isAccountType("v", classes, "equity")).toBe(true);
        expect(isAccountType("v", classes, "asset")).toBe(false);
        expect(isAccountType("nothing", classes, "asset")).toBe(false);
    });

    it("accountTermOf reads the term and whether it was guessed", () => {
        const classes: AccountClasses = new Map([["liabilities:mortgage", cls("liability", false, "noncurrent", true)]]);
        expect(accountTermOf("liabilities:mortgage", classes)).toEqual({term: "noncurrent", guessed: true});
        expect(accountTermOf("liabilities:new", classes)).toEqual({term: null, guessed: false});
    });
});

/**
 * The inference half of the shared truth table. Declaration-driven resolution
 * now happens only in the engine (crates/ledgeline-core/tests/account_types.rs
 * runs every row), but a name the engine never saw is still inferred HERE, so
 * the rows with no declarations must agree on this side too.
 */
describe("UNIT domain/accountClass — inference agrees with the shared table", () => {
    const table = JSON.parse(readFileSync(new URL("../../../../fixtures/account-types/classification-cases.json", import.meta.url), "utf8")) as {
        cases: {declared: unknown[]; account: string; type: string | null}[];
    };
    const inferenceRows = table.cases.filter((row) => row.declared.length === 0);

    it("has inference rows to check", () => {
        expect(inferenceRows.length).toBeGreaterThanOrEqual(5);
    });

    it.each(inferenceRows.map((row) => [row.account, row.type] as const))("infers %s as %s", (account, type) => {
        expect(accountTypeOf(account, NO_ACCOUNT_CLASSES)).toBe(type);
    });
});
