// The three payment-matching settings the invoicer shares with Ledgeline
// (SETTINGS-PLAN D2): their names in the shared prefs, ranges and defaults
// (Ledgeline's, `ui_prefs/ledgeline.rs`), and the tolerance as a percentage
// on screen but basis points in the file.

/** The matching keys, as `ui-prefs.toml` spells them. */
export const MATCHING_KEYS = ["settlementLookbackDays", "settlementToleranceBps", "settlementLeadDays"] as const;
export type MatchingKey = (typeof MATCHING_KEYS)[number];

export type Matching = Record<MatchingKey, number>;

/** Each matching key's range and default (Ledgeline's, `ui_prefs/ledgeline.rs`). */
export const MATCHING_SPEC: Readonly<Record<MatchingKey, {min: number; max: number; fallback: number}>> = {
    settlementLookbackDays: {min: 0, max: 3650, fallback: 180},
    settlementToleranceBps: {min: 0, max: 10_000, fallback: 300},
    settlementLeadDays: {min: 0, max: 60, fallback: 4},
};

export const defaultMatching = (): Matching => ({
    settlementLookbackDays: MATCHING_SPEC.settlementLookbackDays.fallback,
    settlementToleranceBps: MATCHING_SPEC.settlementToleranceBps.fallback,
    settlementLeadDays: MATCHING_SPEC.settlementLeadDays.fallback,
});

/** `value` as a whole number inside `key`'s range; not a number: the default. */
export function clampMatching(key: MatchingKey, value: unknown): number {
    const {min, max, fallback} = MATCHING_SPEC[key];
    if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
    return Math.min(max, Math.max(min, Math.round(value)));
}

/** Basis points as a percentage, one decimal at most: 300 → 3, 250 → 2.5. */
export const bpsToPercent = (bps: number): number => Math.round(bps / 10) / 10;

/** A percentage as whole basis points: 2.5 → 250 (null: not a number). */
export function percentToBps(percent: number): number | null {
    return Number.isFinite(percent) ? Math.round(percent * 100) : null;
}
