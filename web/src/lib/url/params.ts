// The three shapes every params ⇄ URL-query codec validates, written once.
//
// A codec reads untrusted text — a pasted link, a stale bookmark — and every
// one of them needs the same three answers: is this value in a closed
// vocabulary, is this an integer inside a range, is this a date. The copies had
// already drifted into three spellings of the second one (`clampedInt`,
// `parseInt1`, and an inline `Math.min(Math.max(…))`) while `oneOf` was
// character-for-character identical in two files and the ISO-date regex was
// declared in four.
//
// Here rather than in `reports/ui/params.ts`, where the first copies lived: a
// budget codec importing its vocabulary check from the reports tab would
// preserve an ordering accident as a dependency. `oneOf` and `clampedInt` take
// `string | null` because that is exactly what `URLSearchParams.get` returns.
//
// `isIsoDate` is the one export whose callers are not all query strings — a
// transaction form and a journal's period expression validate the same shape.
// It lives here anyway, because the alternative is a fifth place to look for
// one regex.
//
// Pure module — no Svelte, no DOM, no clock.

/** A value validated against a closed vocabulary, falling back when it is not a member. */
export function oneOf<T extends string>(allowed: readonly T[], value: string | null, dflt: T): T {
    return allowed.find((member) => member === value) ?? dflt;
}

/** An integer param clamped to `[lo, hi]`; absent or malformed falls back. */
export function clampedInt(value: string | null, lo: number, hi: number, dflt: number): number {
    if (value === null || !/^\d+$/.test(value)) return dflt;
    return Math.min(Math.max(Number(value), lo), hi);
}

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/**
 * Whether a string is a full `YYYY-MM-DD` date — the only spelling a
 * `type=date` input, a query param and the engine all accept.
 *
 * SHAPE ONLY: `2026-13-45` passes here and is refused by whatever actually
 * parses it. That is what all four copies of this regex already did, and
 * tightening it would reject dates the engine accepts.
 *
 * Exact, with no trimming of its own. `scenarioModel.isIsoDate` trims before
 * calling because a journal's period `raw` can carry surrounding space, while a
 * query param must not: `?from=%202026-01-01` is a different link from
 * `?from=2026-01-01` and was never accepted as one.
 */
export function isIsoDate(value: string): boolean {
    return ISO_DATE.test(value);
}
