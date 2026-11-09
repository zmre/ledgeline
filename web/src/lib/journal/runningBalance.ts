// Running balances in the journal (plan 36): the decisions the pin chip, the
// Balance column and "narrow to gap" make, kept pure so they are node tests.
//
// The numbers themselves are the ENGINE's (`/api/journal/rows?balance=` and
// `/api/journal/running`). What lives here is only which of them to show and
// what a click on them does to the date range.

import {formatAmount, sub} from "#lib/domain/money.js";
import type {ISODate} from "#lib/domain/types.js";
import type {RunningAssertion, RunningAssertionPoint, RunningInfo} from "#lib/journalView/types.js";
import {addDays} from "#lib/reports/periods.js";
import type {RunningPin} from "#lib/stores/filters.svelte.js";

/**
 * The commodity the pinned columns are in.
 *
 * The pin's own choice when the subtree still holds it, otherwise the engine's
 * default (the first: most postings) — a commodity that vanished after an edit,
 * or a stale `balc=` in a shared link, falls back exactly as the engine does.
 * Before `/api/journal/running` has answered, the pin's choice stands (or null:
 * "the default, whatever it turns out to be"). `info` must be about the pinned
 * account (`runningFor` sees to that).
 */
export function pinnedCommodity(info: RunningInfo | null, pin: RunningPin | null): string | null {
    if (pin === null) return null;
    if (info === null) return pin.commodity;
    if (pin.commodity !== null && info.commodities.some((c) => c.commodity === pin.commodity)) return pin.commodity;
    return info.commodities[0]?.commodity ?? null;
}

/** The earliest failing assertion in `commodity`, in evaluation order, or null when every one holds. */
export function firstFailure(info: RunningInfo | null, commodity: string | null): RunningAssertionPoint | null {
    if (info === null || commodity === null) return null;
    return info.assertions[commodity]?.find((point) => !point.ok) ?? null;
}

/**
 * The date range "narrow to gap" sets for a failing assertion.
 *
 * From the day after the previous PASSING assertion in the same list (open when
 * there is none) to the failing one's date: everything that put the balance
 * wrong was counted in that window. `rowDate` is the failing transaction's own
 * date, which differs from the assertion's when a `date:` tag moved the leg —
 * the window must still contain the row, or narrowing would hide the very
 * transaction that was clicked.
 *
 * Returns null when the failure is not in `points` (a list from an older
 * generation, say): better no narrowing than the wrong one.
 */
export function gapRange(points: readonly RunningAssertionPoint[], tindex: number, rowDate?: ISODate): {from: ISODate | null; to: ISODate} | null {
    const at = points.findIndex((point) => point.tindex === tindex && !point.ok);
    if (at === -1) return null;
    const failing = points[at];
    const to = rowDate !== undefined && rowDate > failing.date ? rowDate : failing.date;
    let from: ISODate | null = null;
    for (let i = at - 1; i >= 0; i -= 1) {
        if (points[i].ok) {
            from = addDays(points[i].date, 1);
            break;
        }
    }
    // A passing check earlier the SAME day leaves a one-day window, not an inverted one.
    if (from !== null && from > to) from = to;
    if (from !== null && rowDate !== undefined && rowDate < from) from = rowDate;
    return {from, to};
}

/**
 * Widen `[from, to]` just enough to include `date`; an open end stays open.
 * What the Problems drawer does before scrolling to a row, so a jump never
 * lands on a transaction the date filter hides.
 */
export function widenRange(from: ISODate | null, to: ISODate | null, date: ISODate): {from: ISODate | null; to: ISODate | null} {
    return {
        from: from !== null && date < from ? date : from,
        to: to !== null && date > to ? date : to,
    };
}

/** "✓ checking:joint" — the mark names the asserted account only when it is not the pinned root. */
export function assertionLabel(assertion: RunningAssertion, pinned: string): string {
    const mark = assertion.ok ? "✓" : "✗";
    if (assertion.account === pinned) return mark;
    const under = assertion.account.startsWith(`${pinned}:`) ? assertion.account.slice(pinned.length + 1) : assertion.account;
    return `${mark} ${under}`;
}

/**
 * The tooltip: `assets:checking = $12,400.00 ✓`, or for a failure
 * `assets:checking = expected $12,400.00, found $12,150.00, off by -$250.00`.
 * "Off by" is found minus expected, so its sign says which way the books lean.
 */
export function assertionDetail(assertion: RunningAssertion): string {
    const expected = formatAmount(assertion.asserted);
    const head = `${assertion.account} ${assertion.op}`;
    if (assertion.ok || assertion.calculated === null) return `${head} ${expected} holds`;
    const found = assertion.calculated;
    const base = `${head} expected ${expected}, found ${formatAmount(found)}`;
    // Only a like-for-like difference means anything.
    if (found.commodity !== assertion.asserted.commodity) return base;
    return `${base}, off by ${formatAmount({...found, qty: sub(found.qty, assertion.asserted.qty)})}`;
}
