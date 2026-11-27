// The Accounts tree's pure parts (plan 46 T-accounts): which rows show with
// some branches folded, where ← and → go, and the lists other pages read
// from the tree (the account picker's names, the customers time is linked
// to). The server sends the rows already in tree order (by segment).
import type {AccountRow, AccountsView} from "../api/accounts.js";

/** `clients:acme` for `clients:acme:tax`; null at the top. */
export function parentOf(account: string): string | null {
    const at = account.lastIndexOf(":");
    return at < 0 ? null : account.slice(0, at);
}

/** Every ancestor of `account`, nearest first. */
export function ancestorsOf(account: string): string[] {
    const out: string[] = [];
    for (let p = parentOf(account); p !== null; p = parentOf(p)) out.push(p);
    return out;
}

/** The accounts with a row below them. */
export function withChildren(rows: readonly AccountRow[]): Set<string> {
    const out = new Set<string>();
    for (const row of rows) for (const a of ancestorsOf(row.account)) out.add(a);
    return out;
}

/** The rows that show: none under a folded branch. */
export function visibleRows(rows: readonly AccountRow[], folded: ReadonlySet<string>): AccountRow[] {
    if (folded.size === 0) return [...rows];
    return rows.filter((row) => !ancestorsOf(row.account).some((a) => folded.has(a)));
}

/** Whether the account is archived, itself or through an ancestor. */
export const isArchived = (row: AccountRow): boolean => row.archived !== null && row.archived.value.toLowerCase() === "yes";

/** The account picker's names (Entries' add, move and filter): every account not archived, the undeclared ones too. */
export function pickerAccounts(view: AccountsView): string[] {
    const names = new Set(view.accounts.filter((r) => !isArchived(r)).map((r) => r.account));
    for (const name of view.undeclared) names.add(name);
    return [...names].sort();
}

/** The customers time is linked to, A–Z (Entries' customer filter). */
export function customersOf(view: AccountsView): string[] {
    return [...new Set(view.accounts.flatMap((r) => (r.customer === null ? [] : [r.customer.value])))].sort();
}

/** The indent a row's depth gets, in rem. */
export const indentRem = (depth: number): number => depth * 1.25;

/** A tag's value for the table: "yes", "no", or "—" when nothing in the chain says. */
export const tagText = (value: {value: string} | null): string => (value === null ? "—" : value.value);
