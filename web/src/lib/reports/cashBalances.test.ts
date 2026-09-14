import {describe, expect, it} from "vitest";
import type {AccountDecl} from "../domain/accountTypes";
import {formatAmount} from "../domain/money";
import type {AmountStyle, Transaction} from "../domain/types";
import {accountBalances, balanceCommodities, cashViewClassifier, guessedLongTerm, summarize, visibleRows} from "./cashBalances";
import {amt, txn, usd} from "./test-helpers";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const show = (qty: {m: bigint; p: number}, commodity = "$"): string => formatAmount({commodity, qty, style: STYLE});

/** A chart of accounts with no English in it, so a name-matching regression reads zero instead of passing by luck. */
const SPANISH: AccountDecl[] = [
    {name: "activo:banco:corriente", type: "cash"},
    {name: "activo:cartera", type: "asset"},
    {name: "pasivo:tarjeta", type: "liability"},
    {name: "pasivo:hipoteca", type: "liability", bsterm: "noncurrent"},
    {name: "gastos:oficina", type: "expense"},
];

describe("UNIT reports/cashBalances", () => {
    describe("cashViewClassifier", () => {
        it("admits cash and liabilities by DECLARED type, never by name", () => {
            const classify = cashViewClassifier(SPANISH);
            expect(classify("activo:banco:corriente")).toBe("cash");
            expect(classify("pasivo:tarjeta")).toBe("liability");
            // Declared a plain asset: a brokerage or a house is not cash.
            expect(classify("activo:cartera")).toBeNull();
            expect(classify("gastos:oficina")).toBeNull();
        });

        it("inherits both tags to sub-accounts", () => {
            const classify = cashViewClassifier(SPANISH);
            expect(classify("activo:banco:corriente:eur")).toBe("cash");
            expect(classify("pasivo:hipoteca:principal")).toBeNull(); // non-current, inherited
        });

        it("drops non-current accounts and keeps everything untagged", () => {
            const classify = cashViewClassifier(SPANISH);
            expect(classify("pasivo:hipoteca")).toBeNull();
            expect(classify("pasivo:tarjeta")).toBe("liability"); // untagged ⇒ current
        });

        it("guesses long-term from the name only when no tag in the ancestry said otherwise", () => {
            const classify = cashViewClassifier([
                {name: "liabilities:mortgage", type: "liability"}, // untagged: the guess gets a vote
                {name: "liabilities:heloc", type: "liability", bsterm: "current"}, // tagged: it does not
                {name: "liabilities:cc", type: "liability"},
            ]);
            expect(classify("liabilities:mortgage")).toBe("guessed-long-term");
            expect(classify("liabilities:mortgage:escrow")).toBe("guessed-long-term");
            // A tag always beats a guess, in EITHER direction.
            expect(classify("liabilities:heloc")).toBe("liability");
            expect(classify("liabilities:cc")).toBe("liability");
        });

        it("distinguishes a guess from a declaration, because only one needs explaining", () => {
            const classify = cashViewClassifier([{name: "liabilities:car", type: "liability", bsterm: "noncurrent"}]);
            // Declared out: plain null, the owner wrote the tag and knows.
            expect(classify("liabilities:car")).toBeNull();
        });

        it("falls back to hledger's name heuristic only when NOTHING is declared", () => {
            const classify = cashViewClassifier([]);
            expect(classify("assets:bank:checking")).toBe("cash");
            expect(classify("liabilities:cc:visa")).toBe("liability");
            expect(classify("assets:broker:taxable:aapl")).toBeNull();
        });
    });

    describe("accountBalances", () => {
        const txns: Transaction[] = [
            txn("2026-01-05", [
                ["activo:banco:corriente", usd(500_000)],
                ["patrimonio:inicio", usd(-500_000)],
            ]),
            txn("2026-03-10", [
                ["gastos:oficina", usd(12_000)],
                ["pasivo:tarjeta", usd(-12_000)],
            ]),
            txn("2026-06-01", [
                ["gastos:oficina", usd(30_000)],
                ["activo:banco:corriente", usd(-30_000)],
            ]),
            // Non-current, so it must not appear at all.
            txn("2026-06-02", [
                ["pasivo:hipoteca", usd(-400_000_00)],
                ["activo:cartera", usd(400_000_00)],
            ]),
        ];

        it("sums the whole journal per account and keeps hledger's signs", () => {
            const balances = accountBalances(txns, SPANISH, "2026-12-31");
            const {cash, liabilities, totalCash, totalLiabilities, net} = summarize(balances, "$");
            expect(cash.map((row) => [row.account, show(row.qty)])).toEqual([["activo:banco:corriente", "$4,700.00"]]);
            // A liability you owe is already negative in hledger; nothing is flipped.
            expect(liabilities.map((row) => [row.account, show(row.qty)])).toEqual([["pasivo:tarjeta", "$-120.00"]]);
            expect(show(totalCash)).toBe("$4,700.00");
            expect(show(totalLiabilities)).toBe("$-120.00");
            expect(show(net)).toBe("$4,580.00");
        });

        it("excludes non-current accounts from both lists", () => {
            const balances = accountBalances(txns, SPANISH, "2026-12-31");
            expect(balances.map((balance) => balance.account)).not.toContain("pasivo:hipoteca");
        });

        it("stops at asOf, inclusive — later postings do not count", () => {
            const balances = accountBalances(txns, SPANISH, "2026-05-31");
            const {cash} = summarize(balances, "$");
            // The June withdrawal is out of range, so the opening deposit stands alone.
            expect(cash.map((row) => show(row.qty))).toEqual(["$5,000.00"]);
        });

        it("dates each account by its own most recent posting, not the journal's", () => {
            const balances = accountBalances(txns, SPANISH, "2026-12-31");
            const asOf = new Map(balances.map((balance) => [balance.account, balance.asOf]));
            expect(asOf.get("activo:banco:corriente")).toBe("2026-06-01");
            expect(asOf.get("pasivo:tarjeta")).toBe("2026-03-10");
        });

        it("prefers a posting's own date over its transaction's", () => {
            const dated = txn("2026-07-01", [
                ["activo:banco:corriente", usd(100)],
                ["patrimonio:inicio", usd(-100)],
            ]);
            dated.postings[0] = {...dated.postings[0], date: "2026-08-15"};
            const balances = accountBalances([dated], SPANISH, "2026-12-31");
            expect(balances[0].asOf).toBe("2026-08-15");
        });

        it("marks an account confirmed when its latest posting carried a balance assertion", () => {
            const checked = txn("2026-09-01", [
                ["activo:banco:corriente", usd(100)],
                ["patrimonio:inicio", usd(-100)],
            ]);
            checked.postings[0] = {...checked.postings[0], balanceAssertion: {amount: usd(4_800_00), inclusive: false, total: false}};
            const balances = accountBalances([...txns, checked], SPANISH, "2026-12-31");
            const bank = balances.find((balance) => balance.account === "activo:banco:corriente");
            expect(bank?.confirmed).toBe(true);
            expect(bank?.asOf).toBe("2026-09-01");
            // An older assertion does not make a newer, unchecked figure "confirmed".
            expect(accountBalances(txns, SPANISH, "2026-12-31").find((b) => b.account === "activo:banco:corriente")?.confirmed).toBe(false);
        });

        it("keeps an untagged mortgage out, and NAMES it so the guess is not silent", () => {
            // Nothing here declares a term, so the name heuristic is the only
            // thing standing between a 30-year mortgage and the "short-term
            // liabilities" figure.
            const untagged: AccountDecl[] = [
                {name: "activo:banco:corriente", type: "cash"},
                {name: "pasivo:tarjeta", type: "liability"},
                {name: "liabilities:mortgage", type: "liability"},
            ];
            const withMortgage = [
                ...txns,
                txn("2026-04-01", [
                    ["liabilities:mortgage", usd(-25_000_000)],
                    ["activo:banco:corriente", usd(25_000_000)],
                ]),
            ];
            const balances = accountBalances(withMortgage, untagged, "2026-12-31");
            expect(balances.map((b) => b.account)).not.toContain("liabilities:mortgage");
            expect(guessedLongTerm(withMortgage, untagged, "2026-12-31")).toEqual(["liabilities:mortgage"]);
        });

        it("names nothing when every exclusion was the owner's own tag", () => {
            // SPANISH tags the mortgage `bsterm: noncurrent`, so there is no
            // guess to disclose — and disclosing a tag the user wrote would be
            // noise.
            expect(guessedLongTerm(txns, SPANISH, "2026-12-31")).toEqual([]);
        });

        it("returns the identical array for identical arguments (the shared memo)", () => {
            // The Balances view and the `negative-cash` check rule must not each
            // pay for a whole-journal pass.
            expect(accountBalances(txns, SPANISH, "2026-12-31")).toBe(accountBalances(txns, SPANISH, "2026-12-31"));
            expect(accountBalances(txns, SPANISH, "2026-12-31")).not.toBe(accountBalances(txns, SPANISH, "2026-11-30"));
        });
    });

    describe("multi-commodity", () => {
        const decls: AccountDecl[] = [
            {name: "assets:bank:usd", type: "cash"},
            {name: "assets:bank:eur", type: "cash"},
        ];
        const txns: Transaction[] = [
            txn("2026-02-01", [
                ["assets:bank:usd", usd(100_000)],
                ["equity:opening", usd(-100_000)],
            ]),
            txn("2026-02-02", [
                ["assets:bank:eur", amt("EUR", 250_000, 2)],
                ["equity:opening", amt("EUR", -250_000, 2)],
            ]),
        ];

        it("never sums across commodities; each is its own view", () => {
            const balances = accountBalances(txns, decls, "2026-12-31");
            expect(balanceCommodities(balances)).toEqual(["$", "EUR"]);
            expect(summarize(balances, "$").cash.map((row) => row.account)).toEqual(["assets:bank:usd"]);
            expect(summarize(balances, "EUR").cash.map((row) => row.account)).toEqual(["assets:bank:eur"]);
        });

        it("omits an account that has never held the selected commodity", () => {
            const balances = accountBalances(txns, decls, "2026-12-31");
            expect(summarize(balances, "$").cash).toHaveLength(1);
        });
    });

    describe("ordering and zero rows", () => {
        const decls: AccountDecl[] = [
            {name: "assets:a", type: "cash"},
            {name: "assets:b", type: "cash"},
            {name: "assets:c", type: "cash"},
        ];
        const txns: Transaction[] = [
            txn("2026-01-01", [
                ["assets:a", usd(10_000)],
                ["assets:b", usd(50_000)],
                ["assets:c", usd(60_000)],
                ["equity:opening", usd(-120_000)],
            ]),
            // c is emptied back to exactly zero, and b is overdrawn past it.
            txn("2026-01-02", [
                ["assets:c", usd(-60_000)],
                ["assets:b", usd(-100_000)],
                ["equity:opening", usd(160_000)],
            ]),
        ];

        it("sorts by magnitude, so an overdrawn account is not buried", () => {
            const {cash} = summarize(accountBalances(txns, decls, "2026-12-31"), "$");
            expect(cash.map((row) => [row.account, show(row.qty)])).toEqual([
                ["assets:b", "$-500.00"],
                ["assets:a", "$100.00"],
                ["assets:c", "$0.00"],
            ]);
        });

        it("keeps an emptied account until the reader asks to hide it", () => {
            const {cash} = summarize(accountBalances(txns, decls, "2026-12-31"), "$");
            expect(visibleRows(cash, false).map((row) => row.account)).toEqual(["assets:b", "assets:a", "assets:c"]);
            expect(visibleRows(cash, true).map((row) => row.account)).toEqual(["assets:b", "assets:a"]);
        });
    });

    it("reads an empty journal as empty rather than as zeroes", () => {
        const balances = accountBalances([], SPANISH, "2026-12-31");
        expect(balances).toEqual([]);
        expect(balanceCommodities(balances)).toEqual([]);
        const {cash, liabilities, net} = summarize(balances, "$");
        expect(cash).toEqual([]);
        expect(liabilities).toEqual([]);
        expect(show(net)).toBe("$0.00");
    });
});
