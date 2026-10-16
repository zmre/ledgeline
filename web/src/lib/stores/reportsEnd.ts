// The end date shared across the report tabs, for this session only.
//
// The reports page records here the end date the user last set on any tab (or
// that a deep link opened on), and moves the next tab it shows to it; see
// `alignToEnd` in reports/ui/params.ts for what happens to that tab's start.
// Module state on purpose: it outlives the page, so leaving /reports and coming
// back keeps it, and it is never written to storage, so a reload or a new run
// starts from the defaults again.

import type {ISODate} from "#lib/domain/types.js";

let remembered: ISODate | null = null;

export const reportsEnd = {
    /** The most recently set end date, or `null` when none has been set this session. */
    get(): ISODate | null {
        return remembered;
    },
    /** Record `end` as the most recently set end date. */
    set(end: ISODate): void {
        remembered = end;
    },
    /** Forget it (tests). */
    reset(): void {
        remembered = null;
    },
};
