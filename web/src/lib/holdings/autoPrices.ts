// The quiet line beside the Holdings tab's "Update prices" button that says
// what the engine's background price updater (plan 41, `auto_prices.rs`) last
// did: "Prices auto-updated 2 h ago".
//
// Pure — the clock comes in as an argument — so every wording is unit-tested
// without fake timers. Relative imports only (`purity.test.ts`).

import type {AutoPricesStatus} from "./pricesTypes";

/** `seconds` ago, roughly, as a person would say it. */
export function ago(seconds: number): string {
    if (seconds < 60) return "just now";
    if (seconds < 3_600) return `${Math.floor(seconds / 60)} min ago`;
    if (seconds < 86_400) return `${Math.floor(seconds / 3_600)} h ago`;
    const days = Math.floor(seconds / 86_400);
    return `${days} day${days === 1 ? "" : "s"} ago`;
}

/** `seconds` from now, roughly. */
function fromNow(seconds: number): string {
    if (seconds < 60) return "in a minute";
    if (seconds < 3_600) return `in ${Math.ceil(seconds / 60)} min`;
    return `in ${Math.ceil(seconds / 3_600)} h`;
}

/**
 * What to say about the updater at `now` (Unix seconds), or null for nothing.
 *
 * Nothing at all while it is off — the setting lives on the Settings page, and
 * a note about a feature nobody turned on is clutter. Nothing for a run that
 * lost a race with an edit either: that is retried within the hour and was
 * never the user's problem.
 */
export function autoPricesNote(auto: AutoPricesStatus, now: number): string | null {
    if (!auto.enabled) return null;
    if (auto.backoffUntil !== null && auto.backoffUntil > now) {
        return `Yahoo Finance asked for a pause; automatic updates resume ${fromNow(auto.backoffUntil - now)}`;
    }
    if (auto.lastOutcome === "no-prices-file") return "Click Update prices once to start automatic updates";
    if (auto.lastRun === null) return null;
    const when = ago(Math.max(0, now - auto.lastRun));
    switch (auto.lastOutcome) {
        case "updated":
            return `Prices auto-updated ${when}`;
        case "up-to-date":
        case "nothing-new":
        case "rate-limited":
            return `Prices checked automatically ${when}`;
        case "failed":
            return `Automatic price update failed ${when}; it will retry`;
        default:
            return null;
    }
}
