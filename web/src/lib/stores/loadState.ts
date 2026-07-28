// Which branch a data surface should render, decided in ONE place.
//
// Every async report store here exposes the same pair — a `status` and the last
// successfully decoded payload — and every surface consuming one had written
// the same `{#if}` chain by hand, with the same two defects (FE-5):
//
//   1. the "render data" branch was tested BEFORE the error branch, and
//   2. the error branch additionally required `payload === null`
//
// so once anything had loaded the error branch was unreachable. A refetch that
// 500s left the PREVIOUS answer on screen — December's balance sheet under a
// control that reads June — with nothing to say it had failed. Deciding here
// makes the ordering a tested property instead of four hand-written chains.

/** The status every data store in `lib/stores` reports. */
export type LoadStatus = "idle" | "loading" | "ready" | "error";

/** The branch a surface should render. */
export type DataView = "error" | "loading" | "data";

/**
 * Pick the branch for a surface backed by one async store.
 *
 * `hasPayload` is "a payload has been decoded at some point" — NOT "it is worth
 * showing". `matchesRequest` is what makes that distinction: pass false while
 * the held payload answers a question the user is no longer asking (a different
 * report tab — FE-1) and it is treated as not-yet-loaded rather than rendered
 * under the new question's label.
 *
 * Failure outranks stale data deliberately: a surface that cannot honour the
 * current request must say so, not keep quietly serving the previous one.
 */
export function dataView(status: LoadStatus, hasPayload: boolean, matchesRequest = true): DataView {
    if (status === "error") return "error";
    if (!hasPayload || !matchesRequest) return "loading";
    return "data";
}
