import {describe, expect, it} from "vitest";
import {declaredTerms, inferBsTerm, parseBsTermTag, resolveBsTerm, type TermDecl} from "./accountTerms";

describe("UNIT domain/accountTerms", () => {
    describe("parseBsTermTag", () => {
        it("accepts the two canonical spellings, trimmed and case-insensitively", () => {
            expect(parseBsTermTag("current")).toBe("current");
            expect(parseBsTermTag(" Noncurrent ")).toBe("noncurrent");
            expect(parseBsTermTag("CURRENT")).toBe("current");
        });

        it("accepts every synonym the Rust parser does", () => {
            for (const spelling of ["current", "short", "shortterm", "short-term"]) {
                expect(parseBsTermTag(spelling)).toBe("current");
            }
            for (const spelling of ["noncurrent", "non-current", "long", "longterm", "long-term"]) {
                expect(parseBsTermTag(spelling)).toBe("noncurrent");
            }
        });

        it("refuses anything else rather than guessing", () => {
            // A term that quietly fell back would file a balance under the wrong
            // subtotal and leave the statement looking correct.
            for (const outside of ["", "curr", "currentt", "non current", "long-ish"]) {
                expect(parseBsTermTag(outside)).toBeNull();
            }
        });
    });

    describe("resolveBsTerm", () => {
        const decls: TermDecl[] = [
            {name: "liabilities:mortgage", bsterm: "noncurrent"},
            {name: "liabilities:card", bsterm: "current"},
            {name: "assets:bank", bsterm: null}, // declared, but with no term
        ];
        const declared = declaredTerms(decls);

        it("keeps only the accounts that actually declared a term", () => {
            expect([...declared.entries()]).toEqual([
                ["liabilities:mortgage", "noncurrent"],
                ["liabilities:card", "current"],
            ]);
        });

        it("prefers an account's own declaration", () => {
            expect(resolveBsTerm("liabilities:mortgage", declared)).toBe("noncurrent");
            expect(resolveBsTerm("liabilities:card", declared)).toBe("current");
        });

        it("inherits from the nearest declared ancestor", () => {
            expect(resolveBsTerm("liabilities:mortgage:escrow", declared)).toBe("noncurrent");
            expect(resolveBsTerm("liabilities:card:visa:business", declared)).toBe("current");
        });

        it("returns null for an undeclared account, leaving the default to the caller", () => {
            expect(resolveBsTerm("assets:bank:checking", declared)).toBeNull();
            expect(resolveBsTerm("liabilities", declared)).toBeNull();
            expect(resolveBsTerm("equity:opening", declared)).toBeNull();
        });

        it("matches on whole segments, not string prefixes", () => {
            // `liabilities:cardiology` is not a sub-account of `liabilities:card`.
            expect(resolveBsTerm("liabilities:cardiology", declared)).toBeNull();
        });
    });

    describe("inferBsTerm (the name guess of last resort)", () => {
        it("reads the tag's own vocabulary written into the name, for either kind", () => {
            for (const account of [
                "liabilities:long-term-debt",
                "liabilities:longterm:notes",
                "liabilities:non-current:provisions",
                "assets:bank:noncurrent-reserve",
            ]) {
                expect(inferBsTerm(account, "liability")).toBe("noncurrent");
            }
            expect(inferBsTerm("assets:bank:long-term-cd", "cash")).toBe("noncurrent");
        });

        it("REGRESSION: the current PORTION of a long-term debt is current", () => {
            // "Long-Term Debt, Current Maturities" is a real FASB line item and a
            // CURRENT liability. A plain search for "long-term" gets it exactly
            // backwards, which is the one way this heuristic could hide a debt
            // that is due this year.
            for (const account of [
                "liabilities:long-term-debt:current-maturities",
                "liabilities:longterm:current-portion",
                "liabilities:mortgage:due-within-one-year",
            ]) {
                expect(inferBsTerm(account, "liability")).toBeNull();
            }
        });

        it("catches the long-dated liabilities people actually write", () => {
            for (const account of [
                "liabilities:mortgage",
                "liabilities:mortgage:wells-fargo",
                "Liabilities:Mortgage", // Beancount capitalizes
                "liabilities:mortage:chase", // the observed misspelling
                "liabilities:heloc",
                "liabilities:loans:student",
                "Liabilities:Loans:nz_student_loan",
                "liabilities:pension",
            ]) {
                expect(inferBsTerm(account, "liability")).toBe("noncurrent");
            }
        });

        it("has no opinion on the words that only USUALLY mean long-term", () => {
            // Every one of these was considered and rejected: a false positive
            // silently drops a real debt out of the reader's short-term picture,
            // so `loan` (which may be due this month), a car loan, a lease and a
            // note payable are all left to the tag. A miss is one tag away; a
            // balance the reader never saw is not.
            for (const account of [
                "liabilities:loans:personal",
                "liabilities:car",
                "liabilities:auto-loan",
                "liabilities:lease:office",
                "liabilities:notes-payable",
                "liabilities:bonds",
                "liabilities:creditcard",
                "liabilities:cc:visa",
            ]) {
                expect(inferBsTerm(account, "liability")).toBeNull();
            }
        });

        it("never overrules a `type: C` declaration on the strength of a word", () => {
            // The owner declared these cash. Only the explicit vocabulary above
            // may contradict that; the liability word list never gets a vote.
            expect(inferBsTerm("assets:pension:cash-account", "cash")).toBeNull();
            expect(inferBsTerm("assets:student:allowance", "cash")).toBeNull();
            expect(inferBsTerm("assets:mortgage-escrow", "cash")).toBeNull();
        });

        it("leaves the corpus's known traps alone", () => {
            // Each of these is a real path from the survey that a looser list
            // would have misfiled.
            expect(inferBsTerm("assets:bank:savings", "cash")).toBeNull();
            expect(inferBsTerm("assets:current", "cash")).toBeNull(); // British for "checking"
            expect(inferBsTerm("assets:cash:lira", "cash")).toBeNull(); // not `ira`
            expect(inferBsTerm("liabilities:wilson-ltd", "liability")).toBeNull(); // a company suffix
            expect(inferBsTerm("liabilities:sep:settlement", "liability")).toBeNull(); // September
        });

        it("matches whole words at any depth, not substrings", () => {
            expect(inferBsTerm("liabilities:a:b:c:mortgage", "liability")).toBe("noncurrent");
            // `mortgageable` and `studentship` are not the words.
            expect(inferBsTerm("liabilities:mortgageable", "liability")).toBeNull();
            expect(inferBsTerm("liabilities:studentship-fund", "liability")).toBeNull();
        });

        it("is case-insensitive and indifferent to the separator", () => {
            // The adjacent-pair join is what makes one entry cover every way a
            // two-word concept gets written — which is the whole reason
            // `non-current`, `non_current` and `noncurrent` need only one.
            for (const account of ["Liabilities:HELOC", "liabilities:heloc", "liabilities:Long_Term:debt", "liabilities:LONG TERM:debt"]) {
                expect(inferBsTerm(account, "liability")).toBe("noncurrent");
            }
        });
    });
});
