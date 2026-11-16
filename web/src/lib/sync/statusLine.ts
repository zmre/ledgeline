// The sentences the bank sync screens say about themselves (plan 43 §4.2,
// §4.5): the Accounts tab's status line, each linked account's one-line
// summary, the Sync now toast, and the quota warning under the button.
//
// Pure — the clock and the locale come in as arguments. Relative imports only.

import {formatMoney, plural, shortDate, when} from "./format";
import type {WireConnection, WireLinkedAccount, WireSyncRun, WireSyncStatus} from "./types";

/** The newest successful sync across every connection, or null when none has ever succeeded. */
export function lastSynced(status: WireSyncStatus): number | null {
    const times = status.connections.map((c) => c.lastSuccess).filter((t): t is number => t !== null);
    return times.length === 0 ? null : Math.max(...times);
}

/**
 * `"Auto-sync on · last synced 7:12 AM · next around 1:15 PM"`.
 *
 * Null with nothing linked: the Accounts tab says how to start instead, and a
 * status line about nothing is clutter. "Syncing…" while a run is going,
 * because that is the one thing worth knowing then.
 */
export function statusLine(status: WireSyncStatus, now: number, locale?: string): string | null {
    if (!status.available) return null;
    if (status.running) return "Syncing…";
    if (status.accounts.length === 0) return null;
    if (status.connections.length > 0 && status.connections.every((c) => c.state === "elsewhere")) return "Bank sync runs on another computer";
    const parts = [status.autoSync ? "Auto-sync on" : "Auto-sync off"];
    const last = lastSynced(status);
    parts.push(last === null ? "not synced yet" : `last synced ${when(last, now, locale)}`);
    if (status.autoSync && status.nextAuto !== null) {
        parts.push(status.nextAuto <= now ? "next any minute" : `next around ${when(status.nextAuto, now, locale)}`);
    }
    return parts.join(" · ");
}

/**
 * One linked account, under its name: `"Synced 7:12 AM · 4 new · 2 to review ·
 * balance matches ✓"`.
 *
 * "to review" counts the held rows — the ones that will not be written until
 * someone says which they are. The balance clause appears only when both
 * sides are known; a difference is said with the bank's currency.
 */
export function accountLine(account: WireLinkedAccount, now: number, currency = "USD", locale?: string): string {
    if (!account.enabled) return "Paused";
    const parts: string[] = [account.lastSuccess === null ? "Not synced yet" : `Synced ${when(account.lastSuccess, now, locale)}`];
    if (account.lastSuccess !== null) parts.push(account.lastAdded === 0 ? "nothing new" : `${account.lastAdded} new`);
    if (account.held.length > 0) parts.push(`${account.held.length} to review`);
    if (account.bankBalance !== null && account.ledgerBalance !== null) {
        parts.push(account.balanceDiff === null ? "balance matches ✓" : `ledger differs by ${formatMoney(account.balanceDiff.replace(/^-/, ""), currency)}`);
    }
    return parts.join(" · ");
}

/** What the toast after Sync now says, and how many transactions its Review button opens. */
export interface RunToast {
    text: string;
    /** For "Review 4": the new transactions that most want a look. Zero hides the button. */
    review: number;
    /** True when nothing went right — the toast is an error then. */
    failed: boolean;
}

/**
 * `"Synced 3 accounts · 9 new transactions"`, plus "Review 4".
 *
 * Review counts the new rows that landed uncategorized when the engine says
 * so (`uncategorized`, requested of WP-S), and every new row otherwise —
 * either way, the ones Review Recents will show. Held rows are said
 * separately: they are not written, so the card review cannot show them.
 */
export function runToast(run: WireSyncRun): RunToast {
    const synced = run.accounts.filter((a) => a.outcome === "added" || a.outcome === "nothing-new");
    const added = run.accounts.reduce((n, a) => n + a.added, 0);
    const held = run.accounts.reduce((n, a) => n + a.held, 0);
    const failedCount = run.accounts.filter((a) => a.outcome === "failed").length;
    const knowsUncategorized = run.accounts.some((a) => a.uncategorized !== undefined);
    const review = knowsUncategorized ? run.accounts.reduce((n, a) => n + (a.uncategorized ?? 0), 0) : added;
    if (synced.length === 0 && (failedCount > 0 || !run.ok)) {
        const first = run.problems[0]?.message ?? run.accounts.find((a) => a.problem !== null)?.problem?.message ?? null;
        return {text: first === null ? "Sync failed." : `Sync failed: ${first}`, review: 0, failed: true};
    }
    const parts = [`Synced ${plural(synced.length, "account")}`, added === 0 ? "nothing new" : plural(added, "new transaction")];
    if (held > 0) parts.push(`${held} held for a look`);
    if (failedCount > 0) parts.push(`${failedCount} failed`);
    return {text: parts.join(" · "), review: added === 0 ? 0 : review, failed: false};
}

/**
 * The line under Sync now once a connection is getting close to SimpleFIN's
 * daily limit, or null. Past `manualWarn` it warns; at `manualMax` it says why
 * the button is off.
 */
export function quotaNote(connections: readonly WireConnection[]): {text: string; blocked: boolean} | null {
    const busiest = connections.reduce<WireConnection | null>((top, c) => (top === null || c.requestsToday > top.requestsToday ? c : top), null);
    if (busiest === null || busiest.requestsToday < busiest.quota.manualWarn) return null;
    if (busiest.requestsToday >= busiest.quota.manualMax) {
        return {text: `${busiest.requestsToday} syncs today. Sync now is off until midnight UTC, so SimpleFIN doesn't disable access.`, blocked: true};
    }
    return {text: `${busiest.requestsToday} syncs today. SimpleFIN allows about 24 before it disables access.`, blocked: false};
}

/** `"Data stops at Nov 11"` — the problem's own date, in words. */
export function dataThroughLine(iso: string, todayIso: string, locale?: string): string {
    return `Data stops at ${shortDate(iso, todayIso, locale)}`;
}
