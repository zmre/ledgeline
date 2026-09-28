// Holding classifications for the pie's "by category" views: what
// `GET /api/holdings/profiles` answers. Every field mirrors
// `ledgeline-server/src/profiles_api.rs` / `commodity_profile.rs`.
//
// Pure types, relative imports only (see `purity.test.ts`).

/** The dimensions a holding can be classified along. Order is the dropdown's order. */
export const CATEGORY_DIMENSIONS = ["assetClass", "sector", "industry", "securityType", "category", "risk"] as const;

export type CategoryDimension = (typeof CATEGORY_DIMENSIONS)[number];

/** Everything the pie can be divided by: one slice per holding, or one per category. */
export type PieDimension = "holding" | CategoryDimension;

export const PIE_DIMENSIONS: readonly PieDimension[] = ["holding", ...CATEGORY_DIMENSIONS];

/** One label's share of a holding, 0 < weight ≤ 1. */
export interface CategoryWeight {
    label: string;
    weight: number;
}

/**
 * A holding's split along every dimension. Each list's weights sum to AT MOST
 * one; the shortfall is unclassified — never spread over the known labels.
 */
export type Breakdown = Readonly<Record<CategoryDimension, readonly CategoryWeight[]>>;

export interface SymbolProfile {
    symbol: string;
    breakdown: Breakdown;
}

/**
 * How the Yahoo half of an answer went. `ok` also covers "nothing needed
 * Yahoo" and "served from cache"; the other two mean some holdings are shown
 * from their commodity tags alone.
 */
export type YahooStatus = "ok" | "partial" | "unavailable";

export interface HoldingsProfiles {
    yahoo: YahooStatus;
    /** By commodity symbol. A symbol the journal does not know is absent. */
    profiles: ReadonlyMap<string, SymbolProfile>;
}
