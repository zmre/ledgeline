// Keeps the report tabs on one end date (the reports page's side of
// `stores/reportsEnd.ts`). The page hands it its params at startup and after
// every change; it answers with the params to apply, and records the end date
// whenever the user moves the active tab's end.
//
// Kept out of the page so the rules are testable without mounting it:
// - A URL that names the opening tab's end wins over the remembered one, and
//   becomes the remembered one (a deep link is a date the user chose).
// - Otherwise the opening tab, and every tab switched to later, is moved to the
//   remembered end (`alignToEnd` decides its start).
// - Only a CHANGE to the active tab's end is remembered. A tab's default end
//   (today, or Insights' last complete month) is never mistaken for a choice.

import type {ISODate} from "#lib/domain/types.js";
import {alignToEnd, searchNamesEnd, tabEnd, type ReportParams, type ReportTab} from "./params";

/** Where the shared end lives: `reportsEnd` in the app, a fake in tests. */
export interface EndMemory {
    get(): ISODate | null;
    set(end: ISODate): void;
}

export interface EndSync {
    /** At startup, after the URL is applied: the params that put the opening tab on the shared end. */
    restore(params: ReportParams, search: string): Partial<ReportParams>;
    /** After any change: the params that put a newly opened tab on the shared end (`{}` otherwise). */
    observe(params: ReportParams): Partial<ReportParams>;
}

export function createEndSync(memory: EndMemory): EndSync {
    let tab: ReportTab | null = null;
    // The active tab's end as last settled, so an edit is told apart from a re-run.
    let seen: ISODate | null = null;

    function align(params: ReportParams): Partial<ReportParams> {
        const shared = memory.get();
        const patch = shared === null ? {} : alignToEnd(params, params.tab, shared);
        tab = params.tab;
        seen = tabEnd({...params, ...patch}, params.tab);
        return patch;
    }

    return {
        restore(params, search) {
            const end = tabEnd(params, params.tab);
            if (end !== null && searchNamesEnd(search)) {
                memory.set(end);
                tab = params.tab;
                seen = end;
                return {};
            }
            return align(params);
        },
        observe(params) {
            if (params.tab !== tab) return align(params);
            const end = tabEnd(params, params.tab);
            if (end !== seen) {
                seen = end;
                if (end !== null) memory.set(end);
            }
            return {};
        },
    };
}
