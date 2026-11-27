// Time tracking's Accounts calls (plan 46 T-accounts, `app/time.rs`): the
// tree with each tag resolved nearest-ancestor-wins, and the page's writes
// (declare, set a tag, archive, rename). Every write edits the accounts
// file (and, for a rename, every time file), so the caller fetches the tree
// again after it.
import {send} from "#lib/editorApi.js";
import {isQty, type Qty, type Written} from "./entries.js";

/** A tag's resolved value and the account it was set on. */
export interface Inherited {
    value: string;
    /** The account that says it (itself, or an ancestor). */
    from: string;
    inherited: boolean;
}

/** One account of the tree. */
export interface AccountRow {
    account: string;
    /** Colons in the name: `clients:acme` is 1. */
    depth: number;
    declared: boolean;
    used: boolean;
    tags: [string, string][];
    customer: Inherited | null;
    /** Whether an Invoicer customer has that name. */
    customer_found: boolean;
    billable: Inherited | null;
    item: Inherited | null;
    archived: Inherited | null;
    /** Its time (and below) in the month asked for. */
    month: Qty;
    unbilled: Qty;
    total: Qty;
    warnings: string[];
}

/** `GET /api/time/accounts`. */
export interface AccountsView {
    accounts: AccountRow[];
    /** Accounts the time files use that the accounts file doesn't declare. */
    undeclared: string[];
    /** The accounts file, relative to the books. */
    file: string;
    file_exists: boolean;
}

/** One line a rename changes. */
export interface RenamedLine {
    file: string;
    line: number;
    before: string;
    after: string;
}

/** A rename's preview (or, applied, what it did). */
export interface RenamePreview {
    from: string;
    to: string;
    lines: RenamedLine[];
    files: string[];
    applied: boolean;
}

/** The tags an account takes through `set-tag`. */
export type AccountTag = "customer" | "billable" | "item" | "archived";

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);

const isInherited = (body: unknown): boolean => body === null || (isObject(body) && typeof body.value === "string" && typeof body.from === "string");

/** One row's shape. */
export function isAccountRow(body: unknown): body is AccountRow {
    return (
        isObject(body) &&
        typeof body.account === "string" &&
        typeof body.depth === "number" &&
        isInherited(body.customer ?? null) &&
        isInherited(body.billable ?? null) &&
        isInherited(body.item ?? null) &&
        isInherited(body.archived ?? null) &&
        isQty(body.month) &&
        isQty(body.unbilled) &&
        Array.isArray(body.warnings)
    );
}

/** The tree's shape. */
export function isAccountsView(body: unknown): body is AccountsView {
    return (
        isObject(body) && Array.isArray(body.accounts) && body.accounts.every(isAccountRow) && Array.isArray(body.undeclared) && typeof body.file === "string"
    );
}

const isWritten = (body: unknown): boolean => isObject(body) && "done" in body && Array.isArray(body.files);

const isRenameWritten = (body: unknown): boolean =>
    isWritten(body) && isObject((body as {done: unknown}).done) && Array.isArray((body as {done: {lines?: unknown}}).done.lines);

/** `GET /api/time/accounts?month=YYYY-MM` (this month when absent). */
export function fetchAccounts(month?: string): Promise<AccountsView> {
    return send<AccountsView>("GET", `/api/time/accounts${month === undefined ? "" : `?month=${encodeURIComponent(month)}`}`, isAccountsView);
}

/** `POST /api/time/accounts/declare`: Add child, Declare. */
export function declareAccount(account: string, tags: Record<string, string> = {}): Promise<Written<unknown>> {
    return send<Written<unknown>>("POST", "/api/time/accounts/declare", isWritten, {account, tags});
}

/** `POST /api/time/accounts/set-tag`: `value` null removes the tag (the account then inherits). */
export function setAccountTag(account: string, tag: AccountTag, value: string | null): Promise<Written<unknown>> {
    return send<Written<unknown>>("POST", "/api/time/accounts/set-tag", isWritten, {account, tag, value});
}

/** `POST /api/time/accounts/archive`. */
export function archiveAccount(account: string, archived: boolean): Promise<Written<unknown>> {
    return send<Written<unknown>>("POST", "/api/time/accounts/archive", isWritten, {account, archived});
}

/** `POST /api/time/accounts/rename`: a preview, or with `apply` the rewrite. */
export function renameAccount(from: string, to: string, apply: boolean): Promise<Written<RenamePreview>> {
    return send<Written<RenamePreview>>("POST", "/api/time/accounts/rename", isRenameWritten, {from, to, apply});
}
