// The Books panel, mounted: what the journal holds, as the server says it.
import {render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import BooksPanel from "./BooksPanel.svelte";
import {BOOKS_105} from "./fixtures.js";

describe("COMPONENT BooksPanel", () => {
    it("shows the booking's postings and the open receivable", () => {
        render(BooksPanel, {books: BOOKS_105});
        expect(screen.getByText("Accrual basis")).toBeDefined();
        expect(screen.getByTestId("books-outstanding").textContent).toBe("$8,350.00");
        const entries = screen.getAllByTestId("books-entry");
        expect(entries).toHaveLength(1);
        expect(entries[0].textContent).toContain("books.journal:212");
        expect(entries[0].textContent).toContain("liabilities:customer-deposits");
        expect(screen.getByTestId("books-mirror").textContent).toContain("No");
        expect(screen.queryByRole("status")).toBeNull();
        expect(screen.queryByTestId("books-empty")).toBeNull();
    });

    it("lists payments against the receivable", () => {
        render(BooksPanel, {
            books: {
                ...BOOKS_105,
                receivable: {
                    ...BOOKS_105.receivable!,
                    state: "paid",
                    outstanding: "$0.00",
                    payments: [
                        {date: "2026-09-20", amount: "$1,000.00"},
                        {date: "2026-10-05", amount: "$2,000.00"},
                    ],
                },
                paid: {state: "full", date: "2026-10-05"},
                books_paid_text: "2026-10-05",
            },
        });
        const receivable = screen.getByTestId("books-receivable");
        expect(receivable.textContent).toContain("paid");
        expect(receivable.textContent).toContain("2026-09-20");
        expect(receivable.textContent).toContain("$2,000.00");
        expect(screen.getByTestId("books-mirror").textContent).toContain("Paid 2026-10-05");
    });

    it("says when the file's mirror disagrees with the books", () => {
        render(BooksPanel, {books: {...BOOKS_105, in_step: false, books_paid_text: "partial 2026-11-05 $1,000"}});
        expect(screen.getByRole("status").textContent).toContain("partial 2026-11-05 $1,000");
    });

    it("says a draft is never booked", () => {
        render(BooksPanel, {books: {...BOOKS_105, status: "draft", bookings: [], receivable: null, paid: null, books_paid_text: null, in_step: null}});
        expect(screen.getByTestId("books-empty").textContent).toContain("drafts never touch the books");
        expect(screen.queryByTestId("books-receivable")).toBeNull();
    });

    it("explains cash books, where an invoice books at payment", () => {
        render(BooksPanel, {books: {...BOOKS_105, basis: "cash", bookings: [], receivable: null}});
        expect(screen.getByText("Cash basis")).toBeDefined();
        expect(screen.getByTestId("books-empty").textContent).toContain("when a payment is applied");
    });

    it("shows a void's reversal and the books' problems", () => {
        render(BooksPanel, {
            books: {
                ...BOOKS_105,
                status: "void",
                voids: [{date: "2026-10-10", description: "Void Invoice 103", file: "books.journal", line: 300, postings: []}],
                problems: [{severity: "warning", message: "the booking's total differs from the file's"}],
            },
        });
        expect(screen.getAllByTestId("books-entry")).toHaveLength(2);
        expect(screen.getByText("Void")).toBeDefined();
        expect(screen.getByTestId("books-problems").textContent).toContain("differs");
    });
});
