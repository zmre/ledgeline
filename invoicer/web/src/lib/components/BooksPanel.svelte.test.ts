// The Books panel, mounted: where the invoice stands in one line, where it was
// booked (account by account), its payments and credits, and the file's
// `paid:` line only when it disagrees with the books.
import {cleanup, fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import BooksPanel from "./BooksPanel.svelte";
import {BOOKS_105} from "./fixtures.js";

describe("COMPONENT BooksPanel", () => {
    it("says where the booking went, account by account, and that it is unpaid", () => {
        render(BooksPanel, {books: BOOKS_105});
        expect(screen.getByText("Accrual basis")).toBeDefined();
        expect(screen.getByTestId("books-status").textContent).toBe("Unpaid: $8,350.00 due 2026-12-10");
        const booking = screen.getByTestId("books-booking");
        expect(booking.textContent).toContain("books.journal:212");
        const rows = within(booking).getAllByTestId("books-posting");
        const cells = (row: HTMLElement): string =>
            Array.from(row.querySelectorAll("td"), (td) => (td.textContent ?? "").replace(/\s+/g, " ").trim()).join(" | ");
        expect(rows.map(cells)).toEqual([
            "assets:accounts receivable receivable | $8,350.00",
            "revenue:consulting | -$7,200.00",
            "revenue:consulting:delivery | -$9,500.00",
            "liabilities:customer-deposits | $8,350.00",
        ]);
        // The mirror is said only when it disagrees.
        expect(screen.queryByTestId("books-mirror-problem")).toBeNull();
        expect(screen.queryByText(/(^|\s)paid:/)).toBeNull();
        expect(screen.queryByTestId("books-empty")).toBeNull();
    });

    it("lists payments and credits: date, account, description, amount", () => {
        render(BooksPanel, {
            books: {
                ...BOOKS_105,
                receivable: {
                    ...BOOKS_105.receivable!,
                    state: "paid",
                    outstanding: "$0.00",
                    payments: [
                        {
                            date: "2026-09-20",
                            amount: "$1,000.00",
                            kind: "payment",
                            description: "CONTOSO ACH",
                            from: ["assets:bank:checking"],
                            file: "books.journal",
                            line: 387,
                        },
                        {
                            date: "2026-10-05",
                            amount: "$2,000.00",
                            kind: "credit",
                            description: "Credit note 109",
                            from: ["revenue:discounts"],
                            file: "books.journal",
                            line: 399,
                        },
                    ],
                },
                settlement: {state: "paid", total: "$3,000.00", paid: "$3,000.00", outstanding: "$0.00", paid_on: "2026-10-05", due_date: "2026-10-01"},
                paid: {state: "full", date: "2026-10-05"},
                books_paid_text: "2026-10-05",
            },
        });
        expect(screen.getByTestId("books-status").textContent).toBe("Paid in full on 2026-10-05");
        const rows = screen.getAllByTestId("books-payment");
        expect(rows).toHaveLength(2);
        expect(rows[0].textContent).toContain("2026-09-20");
        expect(rows[0].textContent).toContain("assets:bank:checking");
        expect(rows[0].textContent).toContain("CONTOSO ACH");
        expect(rows[0].textContent).toContain("$1,000.00");
        expect(rows[1].textContent).toContain("credit");
        expect(rows[1].textContent).toContain("revenue:discounts");
    });

    it("says, in plain words, when the file's paid: line disagrees with the books", () => {
        const problem = "The invoice file says it is unpaid (`paid: no`), but the books say it was paid in full on 2026-10-05.";
        render(BooksPanel, {books: {...BOOKS_105, in_step: false, mirror_problem: problem}});
        expect(screen.getByTestId("books-mirror-problem").textContent).toBe(problem);
    });

    it("says a draft is never booked", () => {
        render(BooksPanel, {
            books: {...BOOKS_105, status: "draft", bookings: [], receivable: null, paid: null, books_paid_text: null, in_step: null, settlement: null},
        });
        expect(screen.getByTestId("books-empty").textContent).toContain("drafts never touch the books");
        expect(screen.queryByTestId("books-status")).toBeNull();
        expect(screen.queryByTestId("books-booking")).toBeNull();
    });

    it("shows cash books' payments, where they book an invoice", () => {
        render(BooksPanel, {
            books: {
                ...BOOKS_105,
                basis: "cash",
                bookings: [],
                receivable: null,
                payments: [
                    {
                        date: "2026-11-12",
                        description: "WIRE FABRIKAM INV 202",
                        file: "books.journal",
                        line: 26,
                        amount: "$500.00",
                        postings: [
                            {account: "assets:bank:checking", amounts: ["$500.00"]},
                            {account: "revenue:consulting", amounts: ["-$500.00"]},
                        ],
                    },
                ],
                settlement: {state: "partial", total: "$2,000.00", paid: "$500.00", outstanding: "$1,500.00", paid_on: null, due_date: "2026-12-01"},
            },
        });
        expect(screen.getByText("Cash basis")).toBeDefined();
        expect(screen.getByTestId("books-status").textContent).toBe("Partly paid: $500.00 of $2,000.00, $1,500.00 due 2026-12-01");
        const payment = screen.getByTestId("books-cash-payment");
        expect(payment.textContent).toContain("Paid $500.00");
        expect(within(payment).getAllByTestId("books-posting")).toHaveLength(2);
    });

    it("explains cash books with nothing booked yet", () => {
        render(BooksPanel, {books: {...BOOKS_105, basis: "cash", bookings: [], receivable: null}});
        expect(screen.getByTestId("books-empty").textContent).toContain("when a payment is applied");
    });

    it("shows a void's reversal and the books' problems", () => {
        render(BooksPanel, {
            books: {
                ...BOOKS_105,
                status: "void",
                voids: [{date: "2026-10-10", description: "Void Invoice 103", file: "books.journal", line: 300, postings: []}],
                settlement: {...BOOKS_105.settlement!, state: "void"},
                problems: [{severity: "warning", message: "the booking's total differs from the file's"}],
            },
        });
        expect(screen.getByTestId("books-void").textContent).toContain("Void Invoice 103");
        expect(screen.getByTestId("books-status").textContent).toContain("Void");
        expect(screen.getByTestId("books-problems").textContent).toContain("differs");
    });
});

describe("COMPONENT BooksPanel: Apply payment", () => {
    const cash = {
        ...BOOKS_105,
        basis: "cash" as const,
        number: "207",
        bookings: [],
        receivable: null,
        settlement: {state: "unpaid" as const, total: "$7,000.00", paid: "$0.00", outstanding: "$7,000.00", paid_on: null, due_date: "2026-12-16"},
    };

    it("offers Apply payment… on a sent, unpaid invoice in cash books", async () => {
        const onApplyPayment = vi.fn();
        render(BooksPanel, {books: cash, onApplyPayment});
        await fireEvent.click(screen.getByTestId("books-apply-payment"));
        expect(onApplyPayment).toHaveBeenCalledOnce();
    });

    it("never in accrual books, nor once paid", () => {
        render(BooksPanel, {books: BOOKS_105, onApplyPayment: vi.fn()});
        expect(screen.queryByTestId("books-apply-payment")).toBeNull();
        cleanup();
        render(BooksPanel, {books: {...cash, settlement: {...cash.settlement, state: "paid" as const}}, onApplyPayment: vi.fn()});
        expect(screen.queryByTestId("books-apply-payment")).toBeNull();
    });
});
