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

export interface Posting {
    account: string;
    amounts: Amount[];
    status: TxnStatus;
    comment: string;
    tags: [string, string][];
    date?: ISODate;
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
