// WP-08 DoD: the fixture journal's deliberate problem records (WP-09) are all
// flagged with the correct severities. Input is the RAW v1.52 API snapshot
// through the normalizer — the same path production data takes.
//
// The WP-10 STOCK records of the same journal (the 2025-08-20 GLD gift, the
// 2026-06-22 TSLA sell) are no longer computed here: the Rust holdings engine
// reports them through /api/diagnostics, so they are asserted end to end in
// checks/stock-diagnostics.test.ts, which feeds this same journal's captured
// wire payload through the pipeline. See DRY-1.

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

    it("computes no stock findings locally — the engine owns those now", () => {
        // Asserted rather than assumed: a stray local reimplementation of any of
        // the three would double every stock finding in the drawer, since the
        // engine's copy arrives through CheckContext.diagnostics regardless.
        expect(problems.filter((p) => p.rule.startsWith("stock-"))).toEqual([]);
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
