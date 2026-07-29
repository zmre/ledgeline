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

/** A `=`/`==`/`=*`/`==*` balance assertion; `baposition` is source info we ignore. */
export interface RawBalanceAssertion {
    baamount?: RawAmount;
    bainclusive?: boolean;
    batotal?: boolean;
    baposition?: unknown;
}

export interface RawPosting {
    paccount?: string;
    pamount?: RawAmount[];
    pstatus?: string; // "Unmarked" | "Pending" | "Cleared"
    pcomment?: string;
    ptags?: unknown[];
    pdate?: string | null;
    pdate2?: string | null;
    pbalanceassertion?: RawBalanceAssertion | null;
    ptype?: string; // "RegularPosting" | "VirtualPosting" | "BalancedVirtualPosting"
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

/**
 * One engine-computed journal diagnostic (unbalanced transaction / failed
 * balance assertion). These are ADVISORY: the engine reports them instead of
 * refusing to open the journal, so the decoder skips malformed entries rather
 * than throwing — see normalizeDiagnostics.
 */
export interface RawDiagnostic {
    /** 0-based position in the served transactions array (NOT hledger's 1-based tindex). */
    txnIndex?: number;
    rule?: string; // "unbalanced" | "assertion"
    severity?: string; // "error"
    message?: string; // hledger-style, may be multi-line
}

/**
 * The journal payload envelope. A plain hledger-web (and any engine build from
 * before diagnostics existed) answers /transactions with a BARE ARRAY, so both
 * shapes have to decode.
 */
export interface RawJournalPayload {
    transactions?: RawTransaction[];
    diagnostics?: unknown;
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

/** /accounts element. We only read the name + declaration tags (the `type:` tag); balances (`adata`) and the tree links are ignored. */
export interface RawAccountDeclarationInfo {
    aditags?: unknown[]; // array of [key, value] pairs, same shape as ttags/ptags
}
export interface RawAccount {
    aname?: string;
    adeclarationinfo?: RawAccountDeclarationInfo | null;
}
