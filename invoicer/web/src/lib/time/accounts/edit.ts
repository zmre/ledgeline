// What the account dialog writes (T-accounts: Link customer, Billable,
// Default item, Archive): the account's own tags as a form, and the
// `set-tag` calls that turn one form into another. A value the account only
// inherits is not its own: the form shows it empty ("inherit"), and leaving
// it empty writes nothing.
import type {AccountRow, AccountTag, Inherited} from "../api/accounts.js";

/** The account's own tags, as the dialog edits them; "" is "inherit". */
export interface AccountForm {
    customer: string;
    billable: "" | "yes" | "no";
    item: string;
    archived: boolean;
}

/** The value the account sets itself, "" when it only inherits one (or none). */
export const ownValue = (tag: Inherited | null): string => (tag === null || tag.inherited ? "" : tag.value);

/** The form for `row`. */
export function formOf(row: AccountRow): AccountForm {
    const billable = ownValue(row.billable).toLowerCase();
    return {
        customer: ownValue(row.customer),
        billable: billable === "yes" || billable === "no" ? billable : "",
        item: ownValue(row.item),
        archived: ownValue(row.archived).toLowerCase() === "yes",
    };
}

/** One `set-tag` call; `value` null removes the tag. */
export interface TagChange {
    tag: AccountTag;
    value: string | null;
}

/** The calls that take `row` from its tags to `form`'s, in a fixed order. */
export function tagChanges(row: AccountRow, form: AccountForm): TagChange[] {
    const before = formOf(row);
    const out: TagChange[] = [];
    const text = (tag: "customer" | "item"): void => {
        const wanted = form[tag].trim();
        if (wanted !== before[tag]) out.push({tag, value: wanted === "" ? null : wanted});
    };
    text("customer");
    if (form.billable !== before.billable) out.push({tag: "billable", value: form.billable === "" ? null : form.billable});
    text("item");
    if (form.archived !== before.archived) out.push({tag: "archived", value: form.archived ? "yes" : null});
    return out;
}

/** "from clients:acme" for an inherited value; "" for the account's own. */
export const inheritedText = (tag: Inherited | null): string => (tag !== null && tag.inherited ? `from ${tag.from}` : "");
