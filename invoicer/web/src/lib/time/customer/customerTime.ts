// A customer page's time accounts (item 4): the rows of
// `GET /api/time/accounts` whose `customer:` link (its own, or inherited
// from the nearest ancestor that sets one) names this customer, the way the
// server matches it (`customer_found`): the note's ledger customer or its
// title. Archived accounts are left out, as the pickers leave them out.
//
// Each row's hours are its own and everything below it (the Accounts
// page's), so a parent and its children overlap; `indent` lays them out as
// a tree under the shallowest linked account. Pure: no runes, no DOM.
import type {AccountRow, AccountsView} from "../api/accounts.js";
import {isIsoDate} from "../dates.js";

const ARCHIVED = /^(yes|y|true|1)$/i;

export interface LinkedAccount {
    account: string;
    /** Levels below the shallowest linked account. */
    indent: number;
    /** This month, seconds (and below). */
    month: number;
    /** Unbilled, seconds (and below). */
    unbilled: number;
    /** Whether the link is inherited from an ancestor (and which). */
    inheritedFrom: string | null;
    /** The `customer:` value, for Entries' customer filter. */
    customer: string;
}

/** The accounts linked to a customer known by `names` (ledger customer, title), in tree order. */
export function linkedAccounts(view: Pick<AccountsView, "accounts">, names: readonly string[]): LinkedAccount[] {
    const wanted = new Set(names.map((n) => n.trim()).filter((n) => n !== ""));
    const rows = view.accounts.filter(
        (row: AccountRow) => row.customer !== null && wanted.has(row.customer.value) && (row.archived === null || !ARCHIVED.test(row.archived.value))
    );
    const top = Math.min(...rows.map((r) => r.depth));
    return rows.map((row) => ({
        account: row.account,
        indent: row.depth - top,
        month: row.month.seconds,
        unbilled: row.unbilled.seconds,
        inheritedFrom: row.customer?.inherited === true ? row.customer.from : null,
        customer: row.customer?.value ?? "",
    }));
}

/** `YYYY-MM` of an ISO date. */
export const monthOf = (today: string): string => (isIsoDate(today) ? today.slice(0, 7) : "");

/** Entries' query (no `?`): an account's time this month. */
export const monthQuery = (account: string): string => new URLSearchParams({period: "this-month", account}).toString();

/** Entries' query: an account's unbilled time, whenever it was. */
export const unbilledQuery = (account: string): string => new URLSearchParams({period: "all", billed: "unbilled", account}).toString();

/** Entries' query: all of a customer's time. */
export const customerQuery = (customer: string): string => new URLSearchParams({period: "all", customer}).toString();
