// A percentage change between two exact money figures, shared by the income
// statement's Total-mode "% change" column and the projection comparison's Δ%
// (plan 34). Moved here from `incomeStatementRows.ts`, which re-exports it.

import {roundTo, type Dec, type MixedAmount} from "#lib/domain/money.js";
import {ZERO} from "#lib/format/amounts.js";

/**
 * Integer division rounded HALF AWAY FROM ZERO — the same convention
 * `roundTo`/`formatDec` apply to every money figure in this app, applied here to
 * a ratio so the percentage column cannot round the other way from the amounts
 * beside it.
 */
function divRoundHalfAway(numerator: bigint, denominator: bigint): bigint {
    const negative = numerator < 0n !== denominator < 0n;
    const a = numerator < 0n ? -numerator : numerator;
    const b = denominator < 0n ? -denominator : denominator;
    // floor(a/b + 1/2), done in integers.
    const quotient = (2n * a + b) / (2n * b);
    return negative ? -quotient : quotient;
}

/**
 * `(current − prior) ÷ |prior|`, as a percentage to ONE decimal — the % change
 * column — or null when there is no change to state.
 *
 * Computed from the exact `Dec` values and never from a formatted string
 * ([[journal-refresh-fingerprint]]): the displayed figures are already rounded
 * to cents, and a percentage derived from them would disagree with one derived
 * from the numbers the engine actually sent — visibly so on a large denominator.
 * The arithmetic is BigInt throughout, so nothing here goes through a float
 * until the final tenths-of-a-percent integer, which is exactly representable.
 *
 * The denominator is the prior figure's MAGNITUDE, so the sign always says
 * which way the line moved: an `other` box going from −$100 to −$50 improved,
 * and reads +50.0%, not −50.0%.
 *
 * `Option`-shaped on purpose. A line that was zero (or absent) last period has
 * no percentage change — not 0.0%, not ∞ — and `—` is the honest cell. So does
 * a report with no prior period at all (`prior` undefined: compare off, or
 * interval mode), and `base === null` (a journal with no base commodity), which
 * has no single figure to form a ratio from rather than one picked at random.
 *
 * `precision`, when given, also refuses a `prior` that rounds to zero at that
 * many places (plan 34 D5). Without it, only an exact zero is refused.
 */
export function pctChange(current: MixedAmount, prior: MixedAmount | undefined, base: string | null, precision?: number): number | null {
    if (base === null || prior === undefined) return null;
    const was: Dec | undefined = prior.get(base);
    if (was === undefined || was.m === 0n) return null;
    // Opt-in (plan 34 D5): a denominator the reader sees as zero at the base
    // commodity's display precision ($0.004 reads $0.00) gives a ratio that is
    // noise. The income statement's % change column never passes it.
    if (precision !== undefined && roundTo(was, precision).m === 0n) return null;
    const now: Dec = current.get(base) ?? ZERO;
    // Both on one scale, then (now − was) / |was| × 1000 = tenths of a percent.
    const scale = Math.max(now.p, was.p);
    const nowM = now.m * 10n ** BigInt(scale - now.p);
    const wasM = was.m * 10n ** BigInt(scale - was.p);
    return Number(divRoundHalfAway((nowM - wasM) * 1000n, wasM < 0n ? -wasM : wasM)) / 10;
}
