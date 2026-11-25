// Apply payment (cash books, invoicer I17): pick a deposit Ledgeline imported
// or synced, and its income side is re-split onto the invoice's accounts.
// The invoicer never creates a payment. Shapes are the server's
// (`/api/invoices/{num}/payments`, src/app/editor.rs); amounts arrive in the
// books' style. Accrual books link payments in Ledgeline instead, so they
// never offer it.
import type {InvoiceBooks} from "./invoices.js";

/** A deposit that might pay the invoice, best first. */
export interface PaymentCandidate {
    /** Names the deposit to the server; never shown. */
    reference: string;
    date: string;
    description: string;
    received: string;
    /** It pays the rest exactly (else part of it). */
    exact: boolean;
    /** Its description names the invoice number. */
    by_number: boolean;
    /** Its customer is the invoice's. */
    by_customer: boolean;
    /** Days from the invoice to the deposit. */
    gap_days: number;
}

/** `GET /api/invoices/{num}/payments`: what is due and the deposits that might pay it. */
export interface PaymentOptions {
    number: string;
    commodity: string;
    total: string;
    paid: string;
    remaining: string;
    candidates: PaymentCandidate[];
}

/** `POST /api/invoices/{num}/payments`: what an apply wrote, or (dry) would write. */
export interface PaymentResult {
    written: boolean;
    number: string;
    date: string;
    description: string;
    received: string;
    applied: {account: string; amount: string}[];
    partial: boolean;
    remaining: string;
    warnings: string[];
}

/**
 * Whether the invoice page offers Apply payment: cash books, a sent invoice,
 * not paid in full. Accrual books never do (the payment settles the
 * receivable, linked in Ledgeline).
 */
export function canApplyPayment(books: InvoiceBooks | null): boolean {
    if (books === null || books.basis !== "cash" || books.status !== "sent") return false;
    const state = books.settlement?.state;
    return state !== "paid" && state !== "void";
}

/** Why a deposit is offered, in words: "Names this invoice · Pays the rest · 12 days after the invoice". */
export function candidateHints(candidate: PaymentCandidate): string[] {
    const days = candidate.gap_days;
    return [
        ...(candidate.by_number ? ["Names this invoice"] : []),
        ...(candidate.by_customer ? ["From this customer"] : []),
        candidate.exact ? "Pays the rest" : "Pays part",
        days === 0 ? "Same day as the invoice" : `${days} ${days === 1 ? "day" : "days"} after the invoice`,
    ];
}

/** One line on what applying it does (or did): "Pays $3,500.00; $3,500.00 still due." */
export function paymentOutcome(result: PaymentResult): string {
    const verb = result.written ? "Applied" : "Pays";
    return result.partial ? `${verb} ${result.received}; ${result.remaining} still due.` : `${verb} ${result.received}: paid in full.`;
}
