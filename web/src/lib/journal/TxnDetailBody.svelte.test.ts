// The read-only transaction body (plan 45 Phase 3b), shared by the journal's
// detail sheet and a settlement card's Details: every posting with its account,
// amount and comment; the transaction comment and Tags: line; the matched
// posting marked.

import {render, screen, within} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import type {Transaction} from "#lib/domain/types.js";
import TxnDetailBody from "./TxnDetailBody.svelte";

const STYLE = {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]] as [string, number[]]};

/** A synthetic payment, $30 short of its invoice, with a fee leg. */
const TXN: Transaction = {
    index: 41,
    date: "2026-07-20",
    status: "cleared",
    description: "Tailspin Payment",
    code: "",
    comment: "wire received, invoice: 2001",
    tags: [["invoice", "2001"]],
    postings: [
        {account: "assets:bank:checking", amounts: [{commodity: "$", qty: {m: 297000n, p: 2}, style: STYLE}], status: "unmarked", comment: "", tags: []},
        {account: "expenses:payment fees", amounts: [{commodity: "$", qty: {m: 3000n, p: 2}, style: STYLE}], status: "unmarked", comment: "card fee", tags: []},
        {
            account: "assets:accounts receivable",
            amounts: [{commodity: "$", qty: {m: -300000n, p: 2}, style: STYLE}],
            status: "unmarked",
            comment: "customer: Tailspin",
            tags: [["customer", "Tailspin"]],
        },
    ],
};

describe("COMPONENT TxnDetailBody", () => {
    it("shows every posting with its amount and comment, the comment and the tags", () => {
        render(TxnDetailBody, {txn: TXN});
        expect(screen.getByTestId("detail-date").textContent).toBe("2026-07-20");
        expect(screen.getByTestId("detail-description").textContent).toBe("Tailspin Payment");
        expect(screen.getByTestId("detail-comment").textContent).toBe("wire received");
        expect(screen.getByTestId("detail-tags").textContent).toBe("Tags: invoice:2001");
        const rows = within(screen.getByTestId("detail-postings")).getAllByRole("listitem");
        expect(rows.map((row) => row.textContent?.replace(/\s+/g, " ").trim())).toEqual([
            "assets:bank:checking $2,970.00",
            "expenses:payment fees $30.00 card fee",
            "assets:accounts receivable $-3,000.00",
        ]);
        expect(screen.queryAllByTestId("detail-posting-comment").map((node) => node.textContent)).toEqual(["card fee"]);
        // The sheet's heading by default; a line of text inside a card.
        expect(screen.getByTestId("detail-description").tagName).toBe("H2");
    });

    it("marks the highlighted posting, and only it", () => {
        render(TxnDetailBody, {txn: TXN, highlight: 2, compact: true});
        const rows = within(screen.getByTestId("detail-postings")).getAllByRole("listitem");
        expect(rows.map((row) => row.hasAttribute("data-highlighted"))).toEqual([false, false, true]);
        expect(rows[2]?.getAttribute("aria-current")).toBe("true");
        expect(screen.getByTestId("detail-description").tagName).toBe("P");
    });
});
