// Sign → colour, decided once.
//
// This mapping was hand-rolled at ten sites with FIVE different renderings of
// the neutral/zero case (`text-base-content/50`, `/60`, `/40`, `""`, and — at
// `HoldingsStats` — `text-success`, so a portfolio that has gained exactly
// nothing announced it in success green).
//
// Two different questions were also being answered by one name, which is why
// the copies could not be shared:
//
//   `signClass`      — what SIGN is this number? Up is green, down is red.
//                      Correct for a gain, a net, a balance change.
//   `sentimentClass` — is this change GOOD? Direction alone cannot say: revenue
//                      up is green, expenses up is red. Callers pass which.
//
// Deliberately NOT unified into these: the "negative money is red, everything
// else inherits" rule used by the journal amount cells, the totals footer and
// the report tables. That is a two-way rule about debits, not a three-way rule
// about sentiment, and widening it would paint every positive amount in the
// journal green. Likewise the budget bar's under/over/on-plan verdict, which is
// a comparison against a goal rather than a sign.

/** The neutral rendering — zero, or a figure that does not exist. Muted, never green. */
export const NEUTRAL_CLASS = "text-base-content/50";

/** daisyUI colour class for a signed figure: success (+), error (−), neutral (0 / absent). */
export function signClass(value: number | bigint | null): string {
    if (value === null) return NEUTRAL_CLASS;
    const positive = typeof value === "bigint" ? value > 0n : value > 0;
    const negative = typeof value === "bigint" ? value < 0n : value < 0;
    return positive ? "text-success" : negative ? "text-error" : NEUTRAL_CLASS;
}

/**
 * daisyUI colour class for a CHANGE, coloured by whether it is welcome.
 *
 * `goodWhenUp` is the caller's judgement about the metric, not about the
 * number: revenue rising is green, expenses rising is red, and both are the
 * same positive delta.
 */
export function sentimentClass(value: number | bigint | null, goodWhenUp: boolean): string {
    if (value === null) return NEUTRAL_CLASS;
    const positive = typeof value === "bigint" ? value > 0n : value > 0;
    const negative = typeof value === "bigint" ? value < 0n : value < 0;
    if (!positive && !negative) return NEUTRAL_CLASS;
    return positive === goodWhenUp ? "text-success" : "text-error";
}
