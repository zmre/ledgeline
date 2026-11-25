import {describe, expect, it} from "vitest";
import {BOOKS_105} from "./components/fixtures.js";
import type {InvoiceBooks} from "./invoices.js";
import {candidateHints, canApplyPayment, paymentOutcome, type PaymentCandidate, type PaymentResult} from "./payments.js";

/** 207 on the cash fixture: sent, unpaid. */
const CASH_207: InvoiceBooks = {
    ...BOOKS_105,
    basis: "cash",
    number: "207",
    bookings: [],
    receivable: null,
    settlement: {state: "unpaid", total: "$7,000.00", paid: "$0.00", outstanding: "$7,000.00", paid_on: null, due_date: "2026-12-16"},
};

const wire: PaymentCandidate = {
    reference: "0123456789abcdef",
    date: "2026-11-17",
    description: "WIRE CUSTOMER X INV 207",
    received: "$3,500.00",
    exact: false,
    by_number: true,
    by_customer: true,
    gap_days: 1,
};

describe("UNIT Apply payment", () => {
    it("is offered on a sent, unpaid or part-paid invoice in cash books only", () => {
        expect(canApplyPayment(CASH_207)).toBe(true);
        expect(canApplyPayment({...CASH_207, settlement: {...CASH_207.settlement!, state: "partial"}})).toBe(true);
        expect(canApplyPayment({...CASH_207, settlement: null})).toBe(true);
        expect(canApplyPayment({...CASH_207, settlement: {...CASH_207.settlement!, state: "paid"}})).toBe(false);
        expect(canApplyPayment({...CASH_207, status: "draft", settlement: null})).toBe(false);
        expect(canApplyPayment({...CASH_207, status: "void"})).toBe(false);
        // Accrual books link the payment in Ledgeline.
        expect(canApplyPayment(BOOKS_105)).toBe(false);
        expect(canApplyPayment(null)).toBe(false);
    });

    it("says why a deposit is offered, in words", () => {
        expect(candidateHints(wire)).toEqual(["Names this invoice", "From this customer", "Pays part", "1 day after the invoice"]);
        expect(candidateHints({...wire, by_number: false, by_customer: false, exact: true, gap_days: 0})).toEqual(["Pays the rest", "Same day as the invoice"]);
        expect(candidateHints({...wire, gap_days: 12})).toContain("12 days after the invoice");
    });

    it("says what an apply does, and did", () => {
        const preview: PaymentResult = {
            written: false,
            number: "207",
            date: "2026-11-17",
            description: "WIRE CUSTOMER X INV 207",
            received: "$3,500.00",
            applied: [],
            partial: true,
            remaining: "$3,500.00",
            warnings: [],
        };
        expect(paymentOutcome(preview)).toBe("Pays $3,500.00; $3,500.00 still due.");
        expect(paymentOutcome({...preview, written: true})).toBe("Applied $3,500.00; $3,500.00 still due.");
        expect(paymentOutcome({...preview, written: true, partial: false, remaining: "$0.00"})).toBe("Applied $3,500.00: paid in full.");
    });
});
