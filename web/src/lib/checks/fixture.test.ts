// WP-08/WP-10 DoD: the fixture journal's deliberate problem records (WP-09,
// plus the WP-10 stock records) are all flagged with the correct severities.
// Input is the RAW v1.52 API snapshot through the normalizer — the same path
// production data takes.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {normalizePrices, normalizeTransactions} from "../api/normalize";
import {runChecks, type Problem} from "./engine";

const raw: unknown = JSON.parse(readFileSync(new URL("../../../../fixtures/api/v1.52/transactions.json", import.meta.url), "utf8"));
const rawPrices: unknown = JSON.parse(readFileSync(new URL("../../../../fixtures/api/v1.52/prices.json", import.meta.url), "utf8"));
const txns = normalizeTransactions(raw);
const problems = runChecks(txns, {prices: normalizePrices(rawPrices)});

const dateOf = (p: Problem): string => txns.find((t) => t.index === p.txnIndex)?.date ?? "?";
const byRule = (rule: string): Problem[] => problems.filter((p) => p.rule === rule);

describe("UNIT checks over fixture API snapshot", () => {
    it("flags the pending 2026-07-02 flight as a warning", () => {
        const pending = byRule("pending");
        expect(pending).toHaveLength(1);
        expect(pending[0].severity).toBe("warning");
        expect(dateOf(pending[0])).toBe("2026-07-02");
    });

    it("flags the 2026-06-20 expenses:unknown posting as a warning", () => {
        const uncategorized = byRule("uncategorized");
        expect(uncategorized).toHaveLength(1);
        expect(uncategorized[0].severity).toBe("warning");
        expect(uncategorized[0].message).toContain("expenses:unknown");
        expect(dateOf(uncategorized[0])).toBe("2026-06-20");
    });

    it("flags the empty-description 2026-06-28 transaction as info", () => {
        const missing = byRule("missing-description");
        expect(missing).toHaveLength(1);
        expect(missing[0].severity).toBe("info");
        expect(dateOf(missing[0])).toBe("2026-06-28");
    });

    it("reports no unbalanced transactions (hledger already validated the journal; costs balance at cost)", () => {
        expect(byRule("unbalanced")).toEqual([]);
    });

    it("flags the 2025-08-20 GLD gift lot as missing basis (warning)", () => {
        const missingBasis = byRule("stock-missing-basis");
        expect(missingBasis).toHaveLength(1);
        expect(missingBasis[0].severity).toBe("warning");
        expect(missingBasis[0].message).toContain("GLD");
        expect(dateOf(missingBasis[0])).toBe("2025-08-20");
    });

    it("flags GLD as unpriced (no P directive, no usable cost annotation)", () => {
        const unpriced = byRule("stock-unpriced");
        expect(unpriced).toHaveLength(1);
        expect(unpriced[0].severity).toBe("warning");
        expect(unpriced[0].message).toContain("GLD");
        expect(dateOf(unpriced[0])).toBe("2025-08-20");
    });

    it("flags the 2026-06-22 never-bought TSLA sell as negative shares (warning)", () => {
        const negative = byRule("stock-negative");
        expect(negative).toHaveLength(1);
        expect(negative[0].severity).toBe("warning");
        expect(negative[0].message).toContain("TSLA");
        expect(dateOf(negative[0])).toBe("2026-06-22");
    });

    it("flags exactly the transactions dated after today as future-dated (clock-independent)", () => {
        // Independent local-parts "today" (never `new Date("YYYY-MM-DD")` — see plans/00 §dates).
        const now = new Date();
        const localToday = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
        const expected = txns
            .filter((t) => t.date > localToday)
            .map((t) => t.index)
            .sort((a, b) => a - b);
        const flagged = byRule("future-date")
            .map((p) => p.txnIndex)
            .sort((a, b) => a - b);
        expect(flagged).toEqual(expected);
        expect(problems.filter((p) => p.severity === "error")).toEqual([]);
    });
});
