// The payment-matching settings the invoicer shares with Ledgeline
// (SETTINGS-PLAN D2): their names in the shared prefs, ranges and defaults
// (Ledgeline's, `ui_prefs/ledgeline.rs`).
//
// Ledgeline's third one, `settlementToleranceBps`, isn't here: the invoicer's
// matcher (Ledgeline's `apply::candidates`) offers a short deposit as a part
// payment whatever the tolerance, so the setting would do nothing in this
// app. It is never shown or saved here, and ignored when the prefs carry it.

/** The matching keys, as `ui-prefs.toml` spells them. */
export const MATCHING_KEYS = ["settlementLookbackDays", "settlementLeadDays"] as const;
export type MatchingKey = (typeof MATCHING_KEYS)[number];

export type Matching = Record<MatchingKey, number>;

/** Each matching key's range and default (Ledgeline's, `ui_prefs/ledgeline.rs`). */
export const MATCHING_SPEC: Readonly<Record<MatchingKey, {min: number; max: number; fallback: number}>> = {
    settlementLookbackDays: {min: 0, max: 3650, fallback: 180},
    settlementLeadDays: {min: 0, max: 60, fallback: 4},
};

export const defaultMatching = (): Matching => ({
    settlementLookbackDays: MATCHING_SPEC.settlementLookbackDays.fallback,
    settlementLeadDays: MATCHING_SPEC.settlementLeadDays.fallback,
});

/** `value` as a whole number inside `key`'s range; not a number: the default. */
export function clampMatching(key: MatchingKey, value: unknown): number {
    const {min, max, fallback} = MATCHING_SPEC[key];
    if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
    return Math.min(max, Math.max(min, Math.round(value)));
}
