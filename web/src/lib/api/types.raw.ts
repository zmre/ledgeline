// Permissive mirrors of hledger-web wire JSON (WP-02).
// INTERNAL TO lib/api — nothing outside lib/api may import this module.
// hledger's JSON is a dump of internal Haskell types and drifts between
// releases, so every drift-prone field is optional and normalize.ts tolerates
// both the old (aprice/asdecimalpoint/UnitPrice) and new
// (acost/asdecimalmark/UnitCost) spellings. Verified against a live
// hledger 1.52: it already emits acost/asdecimalmark/UnitCost|TotalCost.

export interface RawQuantity {
    floatingPoint?: number;
    decimalPlaces?: number;
    decimalMantissa?: number;
}

/** [separator char, group sizes right-to-left (last repeats)] */
export type RawDigitGroups = [string, number[]];

export interface RawAmountStyle {
    ascommodityside?: string; // "L" | "R"
    ascommodityspaced?: boolean;
    /** number, or "NaturalPrecision", or a tagged object in some releases */
    asprecision?: number | string | {tag?: string; contents?: number} | null;
    asdecimalpoint?: string | null; // older releases
    asdecimalmark?: string | null; // 1.5x+
    asdigitgroups?: RawDigitGroups | null;
    asrounding?: string;
}

/** tag: "UnitCost" | "TotalCost" (new) or "UnitPrice" | "TotalPrice" (old) */
export interface RawCost {
    tag?: string;
    contents?: RawAmount;
}

export interface RawAmount {
    acommodity?: string;
    aquantity?: RawQuantity;
    astyle?: RawAmountStyle;
    aprice?: RawCost | null; // older releases
    acost?: RawCost | null; // 1.5x / 2.0-preview
    aismultiplier?: boolean; // older releases
    acostbasis?: unknown; // 2.0-preview
}

export interface RawPosting {
    paccount?: string;
    pamount?: RawAmount[];
    pstatus?: string; // "Unmarked" | "Pending" | "Cleared"
    pcomment?: string;
    ptags?: unknown[];
    pdate?: string | null;
    pdate2?: string | null;
    pbalanceassertion?: unknown;
    ptype?: string;
    poriginal?: unknown;
    ptransaction_?: string;
}

export interface RawTransaction {
    tindex?: number;
    tdate?: string;
    tdate2?: string | null;
    tstatus?: string;
    tdescription?: string;
    tcode?: string;
    tcomment?: string;
    ttags?: unknown[];
    tpostings?: RawPosting[];
    tprecedingcomment?: string;
    tsourcepos?: unknown;
}

/** /prices in 1.52 returns MarketPrice records (no amount style). */
export interface RawMarketPrice {
    mpdate?: string;
    mpfrom?: string;
    mpto?: string;
    mprate?: RawQuantity;
}

/** Some releases return full price directives with a styled amount. */
export interface RawPriceDirective {
    pddate?: string;
    pdcommodity?: string;
    pdamount?: RawAmount;
}
