// Frozen domain types — Ledgeline's stable data model (WP-02).
// Only lib/api/normalize.ts may construct these from hledger wire JSON.
// Pure TS: no Svelte/DOM imports (ports to Rust later).

import type {Dec} from "./money";

/** "YYYY-MM-DD" — always compared lexically, never via `new Date(...)`. */
export type ISODate = string;

export type TxnStatus = "unmarked" | "pending" | "cleared";

export interface AmountStyle {
    side: "L" | "R";
    spaced: boolean;
    precision: number;
    decimalPoint: string;
    /** [separator, group sizes right-to-left; last size repeats] */
    digitGroups: [string, number[]] | null;
}

export interface Amount {
    commodity: string;
    qty: Dec;
    style: AmountStyle;
    /**
     * Cost/price annotation (`@` per-unit when `per`, `@@` total otherwise).
     * `qty` is ALWAYS the unsigned magnitude: the normalizer canonicalizes
     * hledger 1.52's signed `@@`/inferred totals to their absolute value, so
     * consumers apply the posting amount's sign themselves.
     */
    cost?: {commodity: string; qty: Dec; per: boolean};
}

/**
 * Real, unbalanced-virtual `(a)`, or balanced-virtual `[a]` (hledger's `ptype`).
 * Virtual postings are excluded from a transaction's real balance, and only
 * real postings belong on a balance sheet — so this is NOT cosmetic: turning a
 * `[budget:env]` envelope leg into a real posting moves money that was never
 * there onto every report.
 */
export type PostingType = "regular" | "virtual" | "balancedVirtual";

/**
 * A `=` / `==` / `=*` / `==*` balance assertion: the reconciliation anchor that
 * pins an account's running balance at that point in the file. `total` is `==`
 * (this commodity ONLY); `inclusive` is `=*` (include subaccounts).
 */
export interface BalanceAssertion {
    amount: Amount;
    inclusive: boolean;
    total: boolean;
}

export interface Posting {
    account: string;
    amounts: Amount[];
    status: TxnStatus;
    comment: string;
    tags: [string, string][];
    date?: ISODate;
    /**
     * Absent means `"regular"` — the overwhelmingly common case, left off so the
     * domain object stays small (same convention as `date`).
     */
    type?: PostingType;
    /** Absent when the posting asserts nothing. */
    balanceAssertion?: BalanceAssertion;
}

export interface Transaction {
    /** hledger tindex — stable id within a fetch. */
    index: number;
    date: ISODate;
    date2?: ISODate;
    status: TxnStatus;
    description: string;
    code: string;
    comment: string;
    tags: [string, string][];
    postings: Posting[];
    /** Precomputed lowercase search text (desc+comments+accounts+amounts+commodities). */
    haystack: string;
}

export interface PriceDirective {
    date: ISODate;
    commodity: string;
    price: Amount;
}
