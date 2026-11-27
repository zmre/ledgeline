// Billing's pure parts (plan 46 B-unbilled, B-invoice, B-ap): the words the
// Unbilled page, the Bill dialog and the Pay dialog show (in plain words,
// never a raw field name), the request each dialog sends from what was
// edited, the timesheet table's preview, and which picked lines Entries can
// bill. No runes, no DOM: tested in the node project.
import {formatDuration} from "../duration.js";
import {isMonthEnd, isMonthStart, longMonthName, longMonthYear, rangeText} from "../dates.js";
import type {EntryView} from "../api/entries.js";
import type {
    BillGroup,
    BillRequest,
    CustomerUnbilled,
    Detail,
    GroupChoice,
    Grouping,
    ItemSource,
    NeedField,
    PayPreview,
    PayRequest,
    PersonUnpaid,
} from "../api/billing.js";
import type {Qty} from "../types.js";

/** `October 2026` for a whole month, else `Oct 6 – 20, 2026`. */
export function periodLabel(from: string, to: string): string {
    return isMonthStart(from) && isMonthEnd(to) && from.slice(0, 7) === to.slice(0, 7) ? longMonthYear(from) : rangeText(from, to);
}

/** `5 hours`, `1 hour`, `0.83 hours`: what a draft's delete gives back. */
export function hoursWords(quantity: Pick<Qty, "seconds">): string {
    const hours = Math.round((quantity.seconds / 3600) * 100) / 100;
    return `${hours} ${hours === 1 ? "hour" : "hours"}`;
}

/** The rounding's effect on a row: `+4m`, `-2m`, or `—` for none. */
export function differenceText(actual: Pick<Qty, "seconds">, rounded: Pick<Qty, "seconds">): string {
    const seconds = rounded.seconds - actual.seconds;
    if (Math.round(seconds / 60) === 0) return "—";
    return seconds > 0 ? `+${formatDuration(seconds)}` : formatDuration(seconds);
}

/** An estimate as money: `$1,583.33`; null when there is none. */
export function moneyText(amount: string | null, commodity: string | null): string | null {
    if (amount === null) return null;
    const value = Number(amount);
    if (!Number.isFinite(value)) return `${commodity ?? ""}${amount}`;
    return `${commodity ?? ""}${value.toLocaleString("en-US", {minimumFractionDigits: 2, maximumFractionDigits: 2})}`;
}

/** A customer's name as people know it. */
export const customerName = (c: Pick<CustomerUnbilled, "customer" | "title">): string => c.title ?? c.customer;

/** How often a customer is billed, in words. */
export function cadenceWords(cadence: string | null): string {
    switch ((cadence ?? "").trim().toLowerCase()) {
        case "":
            return "Billed when you choose";
        case "weekly":
            return "Billed weekly";
        case "monthly":
            return "Billed monthly";
        case "quarterly":
            return "Billed quarterly";
        default:
            return `Billed ${cadence}`;
    }
}

/**
 * The Unbilled banner: *3 customers have unbilled time through October:
 * Acme Corp 50m · Globex 3h · Initech 40m*, each customer's main suggestion;
 * null when none has one.
 */
export function bannerText(customers: readonly CustomerUnbilled[]): string | null {
    const due = customers.filter((c) => c.periods.length > 0);
    if (due.length === 0) return null;
    const through = due.map((c) => c.periods[0].to).reduce((a, b) => (b > a ? b : a));
    const who = due.length === 1 ? "1 customer has" : `${due.length} customers have`;
    const month = isMonthEnd(through) ? longMonthName(through) : rangeText(through, through);
    return `${who} unbilled time through ${month}: ${due.map((c) => `${customerName(c)} ${c.periods[0].quantity.text}`).join(" · ")}`;
}

/** Where a row's suggested item came from, in plain words. */
export function sourceWords(source: ItemSource): string {
    switch (source) {
        case "choice":
            return "Your choice";
        case "time-items":
            return "This customer's item for the person";
        case "account":
            return "Set on the time account";
        case "most-used":
            return "This customer's most-used item";
    }
}

export const GROUPINGS: readonly {value: Grouping; label: string}[] = [
    {value: "account", label: "By account"},
    {value: "account-person", label: "By account and person"},
    {value: "account-note", label: "By account and note"},
];

export const DETAILS: readonly {value: Detail; label: string}[] = [
    {value: "none", label: "No timesheet"},
    {value: "daily", label: "A row per day and person"},
    {value: "entries", label: "A row per entry"},
];

/** What Remember does for this grouping. */
export const rememberLabel = (group: Grouping): string => (group === "account-person" ? "Remember for this person" : "Remember for this account");

/** A row's name: the account, with its person or note. */
export function groupLabel(group: Pick<BillGroup, "account" | "person" | "note">): string {
    const extra = group.person ?? (group.note === null || group.note === "" ? null : `“${group.note}”`);
    return extra === null ? group.account : `${group.account} · ${extra}`;
}

/** How the customer's rounding reads (B-round). */
export function roundingWords(rounding: {increment: number | null; direction: string; scope: string} | undefined): string {
    if (rounding === undefined || rounding.increment === null) return "Not rounded: the hours are billed as tracked.";
    const way = rounding.direction === "nearest" ? "to the nearest" : "up to";
    const per = rounding.scope === "day" ? "each account's day" : rounding.scope === "entry" ? "each entry" : "each row";
    return `Rounded ${way} ${rounding.increment} minutes, ${per}. The stored time doesn't change.`;
}

/** What a row still needs before the draft can be made, in plain words. */
export function rowNeeds(group: Pick<BillGroup, "item" | "rate" | "amount" | "excluded">, choice: GroupChoice | undefined): string[] {
    if (group.excluded) return [];
    const out: string[] = [];
    if (group.item !== null && !group.item.found && choice?.rate === undefined)
        out.push(`The catalog has no item ${group.item.code}: choose another, or enter a rate.`);
    if (group.rate === null) out.push("Choose an item or enter a rate.");
    else if (group.amount === null) out.push(`${group.rate} doesn't read as an amount of money.`);
    return out;
}

/** A choice with `patch` applied; a field set back to undefined is the server's default again (an empty text is kept while it is edited). */
export function withChoice(choices: readonly GroupChoice[], key: string, patch: Omit<GroupChoice, "key">): GroupChoice[] {
    const at = choices.findIndex((c) => c.key === key);
    const merged: GroupChoice = {...(at < 0 ? {key} : choices[at]), ...patch};
    for (const name of Object.keys(merged) as (keyof GroupChoice)[]) if (merged[name] === undefined) delete merged[name];
    const next = [...choices];
    if (at < 0) next.push(merged);
    else next[at] = merged;
    return next.filter((c) => Object.keys(c).length > 1);
}

/** What the Bill dialog edits beyond the rows. */
export interface BillOptions {
    group: Grouping;
    detail: Detail | null;
    date: string;
    number: string;
    terms: string;
    reference: string;
}

/** The request for `target` (a period or the picked lines) with what was edited; empty fields keep the server's default. */
export function billRequest(
    target: Pick<BillRequest, "customer" | "from" | "to" | "entries">,
    options: BillOptions,
    choices: readonly GroupChoice[]
): BillRequest {
    const request: BillRequest = {...target, group: options.group};
    // A field emptied while editing is the server's default again (the field shows it once it loses its text).
    const groups = choices
        .map((c) => Object.fromEntries(Object.entries(c).filter(([, v]) => !(typeof v === "string" && v.trim() === ""))) as GroupChoice)
        .filter((c) => Object.keys(c).length > 1);
    if (groups.length > 0) request.groups = groups;
    if (options.detail !== null) request.detail = options.detail;
    if (options.date.trim() !== "") request.date = options.date.trim();
    if (options.number.trim() !== "") request.number = options.number.trim();
    if (options.reference.trim() !== "") request.reference = options.reference.trim();
    const terms = Number(options.terms);
    if (options.terms.trim() !== "" && Number.isInteger(terms) && terms >= 0) request.terms = terms;
    return request;
}

/** A markdown table (the timesheet the server writes), read back for the dialog's preview. */
export interface TableView {
    title: string | null;
    headers: string[];
    rows: string[][];
    /** Per column: right-aligned (`---:`). */
    right: boolean[];
}

const cells = (line: string): string[] =>
    line
        .trim()
        .replace(/^\|/, "")
        .replace(/\|$/, "")
        .split("|")
        .map((c) => c.trim());

/** The first pipe table in `markdown`, with the heading above it; null when there is none. */
export function parseTable(markdown: string | null): TableView | null {
    if (markdown === null) return null;
    const lines = markdown.split("\n");
    const start = lines.findIndex((l, i) => l.trim().startsWith("|") && /^\s*\|?\s*:?-+/.test(lines[i + 1] ?? ""));
    if (start < 0) return null;
    const heading = lines
        .slice(0, start)
        .reverse()
        .find((l) => /^#+\s/.test(l));
    const rows: string[][] = [];
    for (const line of lines.slice(start + 2)) {
        if (!line.trim().startsWith("|")) break;
        rows.push(cells(line));
    }
    return {
        title: heading === undefined ? null : heading.replace(/^#+\s*/, ""),
        headers: cells(lines[start]),
        rows,
        right: cells(lines[start + 1]).map((c) => c.endsWith(":")),
    };
}

/** Entries' Bill…: the picked lines' one customer, or why they can't be billed together. */
export function entriesBillTarget(picked: readonly EntryView[]): {customer: string; entries: string[]} | {error: string} {
    if (picked.length === 0) return {error: "Pick the time to bill first."};
    const billed = picked.filter((e) => e.billed !== "unbilled");
    if (billed.length > 0) {
        const n = billed.length;
        return {error: `${n === 1 ? "One picked line is" : `${n} picked lines are`} already billed, written off or not billable. Pick only unbilled time.`};
    }
    const customers = [...new Set(picked.map((e) => e.customer))];
    if (customers.includes(null)) return {error: "Some picked time has no customer. Link its account to a customer on the time Accounts page first."};
    if (customers.length > 1)
        return {error: `The picked time is for ${customers.length} customers (${customers.join(", ")}). An invoice is for one: pick one customer's time.`};
    return {customer: customers[0] as string, entries: picked.map((e) => e.id)};
}

/** A timekeeper's name as people know it. */
export const personName = (p: Pick<PersonUnpaid, "person" | "title">): string => p.title ?? (p.person === "" ? "Someone not named" : p.person);

/** The Pay dialog's field for a need. */
export function needLabel(field: NeedField): string {
    switch (field) {
        case "note":
            return "Whose note";
        case "rate":
            return "Hourly rate";
        case "expense_account":
            return "Expense account";
        case "payable_account":
            return "Payable account";
        case "reference":
            return "Reference";
        case "time":
            return "Time";
    }
}

/** What the Pay dialog edits. Text fields start as the preview's values; only a change is sent. */
export interface PayFields {
    lines: "account" | "total";
    rate: string;
    expense: string;
    payable: string;
    terms: string;
    date: string;
    reference: string;
    saveToNote: boolean;
    /** No note links the person: pick one (`pick`) or create one (`create`). */
    noteMode: "pick" | "create";
    pickedNote: string;
    newNote: {title: string; kind: "person" | "organization"};
}

/** The fields as the first preview has them (what the note and the books already say). */
export function payFieldsOf(preview: PayPreview): PayFields {
    return {
        lines: "account",
        rate: preview.rate ?? "",
        expense: preview.expense_account ?? "",
        payable: preview.payable_account ?? "",
        terms: preview.terms === null ? "" : String(preview.terms),
        date: "",
        reference: preview.reference,
        saveToNote: true,
        noteMode: "pick",
        pickedNote: "",
        newNote: {title: preview.person, kind: "person"},
    };
}

/** The Pay request for `target` with what changed since `initial`; `needsNote`: no note links the person. */
export function payRequest(
    target: Pick<PayRequest, "person" | "from" | "to" | "entries">,
    fields: PayFields,
    initial: PayFields,
    needsNote: boolean
): PayRequest {
    const request: PayRequest = {...target, lines: fields.lines, save_to_note: fields.saveToNote};
    const changed = (now: string, was: string): string | undefined => (now.trim() !== "" && now.trim() !== was.trim() ? now.trim() : undefined);
    const rate = changed(fields.rate, initial.rate);
    if (rate !== undefined) request.rate = rate;
    const expense = changed(fields.expense, initial.expense);
    if (expense !== undefined) request.expense_account = expense;
    const payable = changed(fields.payable, initial.payable);
    if (payable !== undefined) request.payable_account = payable;
    const terms = changed(fields.terms, initial.terms);
    if (terms !== undefined && /^\d+$/.test(terms)) request.terms = Number(terms);
    const reference = changed(fields.reference, initial.reference);
    if (reference !== undefined) request.reference = reference;
    if (fields.date.trim() !== "") request.date = fields.date.trim();
    if (needsNote && fields.noteMode === "pick" && fields.pickedNote.trim() !== "") request.note = fields.pickedNote.trim();
    if (needsNote && fields.noteMode === "create" && fields.newNote.title.trim() !== "") {
        request.create_note = {title: fields.newNote.title.trim(), kind: fields.newNote.kind};
    }
    return request;
}

const NOTE_KEYS: Record<string, string> = {
    pay_rate: "the hourly rate",
    expense_account: "the expense account",
    payable_account: "the payable account",
    pay_terms: "the terms",
};

/** `the hourly rate and the expense account`: what was saved to the note, in words. */
export function savedWords(keys: readonly string[]): string {
    const words = keys.map((k) => NOTE_KEYS[k] ?? k.replaceAll("_", " "));
    if (words.length <= 1) return words.join("");
    return `${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`;
}
