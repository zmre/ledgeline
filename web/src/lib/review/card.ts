// Pure decisions behind the review card (plan 32 D6, D7, D13, D20, D21): what
// the hero says, which footer is showing, which suggestion chips a card offers
// and where they land, what a similar row will receive, and which stripe a
// journal row wears. No Svelte, no DOM — the `unit` vitest project tests these.

import {formToOps, similarOps, REVIEWED_TAG, type CardChange} from "#lib/api/editMapping.js";
import {accountTypeOf, type AccountClasses} from "#lib/domain/accountClass.js";
import {add, neg} from "#lib/domain/money.js";
import type {Amount, ISODate, Posting, Transaction} from "#lib/domain/types.js";
import {txnFlowAmounts, writtenPostings} from "#lib/journal/rowModel.js";
import type {QueueItem, ReviewForm, ReviewTxn, Suggestion} from "./types";

// ---------------------------------------------------------------------------
// Unknown accounts (D3)
// ---------------------------------------------------------------------------

/**
 * The client's copy of `checks.rs is_uncategorized`: the last segment is
 * `unknown` or `uncategorized`, or the account is a bare top-level expense or
 * revenue account. The engine's `unknown` flag says whether a TRANSACTION
 * qualifies; the card needs to know WHICH posting, and whether an edit fixed it.
 */
export function isUncategorized(account: string, classes: AccountClasses): boolean {
    const name = account.trim();
    if (name === "") return false;
    const segments = name.toLowerCase().split(":");
    const last = segments[segments.length - 1];
    if (last === "unknown" || last === "uncategorized") return true;
    if (segments.length !== 1) return false;
    const type = accountTypeOf(name, classes);
    return type === "expense" || type === "revenue";
}

/** Whether the card still has an unknown posting: one of its rows, or one an `=` rule generated (which only the rule can fix). */
export function hasUnknown(txn: Transaction, form: ReviewForm, classes: AccountClasses): boolean {
    if (form.postings.some((row) => isUncategorized(row.account, classes))) return true;
    return txn.postings.some((posting) => posting.generated !== undefined && isUncategorized(posting.account, classes));
}

// ---------------------------------------------------------------------------
// The hero (D6)
// ---------------------------------------------------------------------------

/** Money out (neutral), money in (success), or a move between the user's own accounts (info). */
export type HeroTone = "out" | "in" | "transfer";

export interface Hero {
    /** The description before `|`. */
    payee: string;
    /** The note after `|`, or "". */
    note: string;
    /** The displayed magnitudes (`txnFlowAmounts`), one per commodity. */
    amounts: Amount[];
    tone: HeroTone;
    /** "from Chase Sapphire" / "to Checking", or null when no balance-sheet leg says. */
    funding: {preposition: "from" | "to"; account: string} | null;
}

const BALANCE_SHEET: ReadonlySet<string> = new Set(["asset", "cash", "liability", "equity"]);

/** Split `payee | note` the way hledger does (the first `|`). */
export function splitDescription(description: string): {payee: string; note: string} {
    const bar = description.indexOf("|");
    if (bar < 0) return {payee: description.trim(), note: ""};
    return {payee: description.slice(0, bar).trim(), note: description.slice(bar + 1).trim()};
}

/**
 * The hero: who, and how much moved on the user's own (balance-sheet) accounts.
 *
 * The sign and amount come from the NET of the balance-sheet postings per
 * commodity, never from the transaction's mix of account types: a stock sale
 * with a broker fee has an expense leg, yet it is money IN to the cash account.
 * Priced legs (an `@`/`@@` cost, the shares bought or sold) are left out of the
 * net whenever any other balance-sheet leg exists, so the sale reads as the cash
 * it brought in rather than as "−10 AAPL". A move between the user's own
 * accounts nets to zero and reads as a transfer.
 */
export function heroOf(txn: Transaction, classes: AccountClasses): Hero {
    const written = writtenPostings(txn);
    const typed = written.map((posting) => ({posting, type: accountTypeOf(posting.account, classes)}));
    const {payee, note} = splitDescription(txn.description);
    const sheet = typed.filter(({type}) => type !== null && BALANCE_SHEET.has(type)).map(({posting}) => posting);
    if (sheet.length === 0) {
        const isRevenue = typed.some(({type}) => type === "revenue");
        const isExpense = typed.some(({type}) => type === "expense");
        return {payee, note, amounts: txnFlowAmounts(txn), tone: isRevenue && !isExpense ? "in" : "out", funding: null};
    }
    const priced = (posting: Posting): boolean => posting.amounts.length > 0 && posting.amounts.every((amount) => amount.cost !== undefined);
    const unpriced = sheet.filter((posting) => !priced(posting));
    const base = unpriced.length > 0 ? unpriced : sheet;
    const net = new Map<string, Amount>();
    for (const amount of base.flatMap((posting) => posting.amounts)) {
        const seen = net.get(amount.commodity);
        net.set(
            amount.commodity,
            seen === undefined ? {commodity: amount.commodity, qty: amount.qty, style: amount.style} : {...seen, qty: add(seen.qty, amount.qty)}
        );
    }
    const moved = [...net.values()].filter((amount) => amount.qty.m !== 0n);
    const tone: HeroTone = moved.length === 0 ? "transfer" : moved.every((a) => a.qty.m > 0n) ? "in" : moved.every((a) => a.qty.m < 0n) ? "out" : "transfer";
    const sign = (posting: Posting): number => {
        const first = posting.amounts[0];
        return first === undefined ? 0 : first.qty.m < 0n ? -1 : first.qty.m > 0n ? 1 : 0;
    };
    // Money out leaves a balance-sheet account (negative); money in lands in one (positive).
    const leg = tone === "in" ? base.find((posting) => sign(posting) > 0) : base.find((posting) => sign(posting) < 0);
    return {
        payee,
        note,
        amounts: moved.length === 0 ? txnFlowAmounts(txn) : moved.map((amount) => (amount.qty.m < 0n ? {...amount, qty: neg(amount.qty)} : amount)),
        tone,
        funding: leg === undefined ? null : {preposition: tone === "in" ? "to" : "from", account: leg.account},
    };
}

/** An ISO date as UTC midnight, read back only through UTC getters or a UTC formatter. */
function utcDay(date: ISODate): Date {
    const [y, m, d] = date.split("-").map(Number);
    return new Date(Date.UTC(y ?? 1970, (m ?? 1) - 1, d ?? 1));
}

/** `2026-10-12` → "Oct 12, 2026", read as a calendar date (never shifted by the zone). */
export function longDate(date: ISODate): string {
    return new Intl.DateTimeFormat("en-US", {month: "short", day: "numeric", year: "numeric", timeZone: "UTC"}).format(utcDay(date));
}

/** `2026-10-12` → "Oct 12". */
export function shortDate(date: ISODate): string {
    return new Intl.DateTimeFormat("en-US", {month: "short", day: "numeric", timeZone: "UTC"}).format(utcDay(date));
}

// ---------------------------------------------------------------------------
// The footer (D7)
// ---------------------------------------------------------------------------

/** The four footer states, in D7's table order. */
export type FooterState = "clean" | "clean-unknown" | "dirty" | "dirty-unknown";

export function footerState(dirty: boolean, unknown: boolean): FooterState {
    if (dirty) return unknown ? "dirty-unknown" : "dirty";
    return unknown ? "clean-unknown" : "clean";
}

/** Whether a card's form differs from its transaction BY VALUE (typing a change and undoing it is clean again). */
export function isDirty(txn: Transaction, form: ReviewForm): boolean {
    return formToOps(txn, form).length > 0;
}

// ---------------------------------------------------------------------------
// Suggestion chips (D13, D13a)
// ---------------------------------------------------------------------------

export interface Chips {
    /** The suggestions shown, best first (at most 3). */
    chips: Suggestion[];
    /** The form row a chip fills, or null when nothing to fill (no chips). */
    target: number | null;
    /** The chip selected when the card appears: the top one when it is strong, else none. */
    initial: number | null;
    /** "fix": an unknown row awaits a category. "usually": a categorised card that disagrees with strong precedent. */
    mode: "fix" | "usually";
}

const NO_CHIPS: Chips = {chips: [], target: null, initial: null, mode: "fix"};

/**
 * The chips a card offers, from its ORIGINAL transaction (so a chip does not
 * vanish the moment it is accepted).
 *
 * - An unknown card offers its suggestions on the first unknown row; the top
 *   one is pre-selected (⌘⇧↵ takes it) only when it is strong (≥ 50%).
 * - A categorised card (Recents) offers a chip only when a strong suggestion
 *   disagrees with its category ("usually food:coffee"), never pre-selected,
 *   so the one-keystroke path cannot recategorise a card by accident.
 */
export function chipsFor(item: QueueItem, original: ReviewForm, classes: AccountClasses): Chips {
    const rows = original.postings;
    const unknownRow = rows.findIndex((row) => isUncategorized(row.account, classes));
    if (unknownRow >= 0) {
        const chips = item.suggestions.filter((suggestion) => !isUncategorized(suggestion.account, classes)).slice(0, 3);
        if (chips.length === 0) return NO_CHIPS;
        return {chips, target: unknownRow, initial: chips[0]?.strong === true ? 0 : null, mode: "fix"};
    }
    const categoryRow = rows.findIndex((row) => {
        const type = accountTypeOf(row.account, classes);
        return type === "expense" || type === "revenue";
    });
    const top = item.suggestions[0];
    if (categoryRow < 0 || top === undefined || !top.strong || rows.some((row) => row.account === top.account)) return NO_CHIPS;
    return {chips: [top], target: categoryRow, initial: null, mode: "usually"};
}

/** Tab on the chip row's account field: the chip it moves to (top → alt 1 → alt 2 → top). */
export function cycleChip(chips: readonly Suggestion[], selected: number | null, fieldValue: string): number | null {
    if (chips.length === 0) return null;
    if (selected === null) return 0;
    // Only advance once the field already shows the selected chip: the first
    // Tab fills it in, the next ones cycle (the account combobox's own rule).
    return fieldValue.trim() === chips[selected]?.account ? (selected + 1) % chips.length : selected;
}

/**
 * The form with `chip` accepted on row `target`: its account, and — when the
 * card has not been renamed yet — the rename precedent carries (D13a).
 */
export function acceptChip(original: Transaction, form: ReviewForm, target: number, chip: Suggestion): ReviewForm {
    const postings = form.postings.map((row, at) => (at === target ? {...row, account: chip.account} : row));
    const renamed = form.description.trim() !== original.description;
    const description = chip.description !== null && !renamed ? chip.description : form.description;
    return {...form, postings, description};
}

/** "food:coffee" for "expenses:food:coffee": the chip's short name. */
export function shortAccount(account: string): string {
    const at = account.indexOf(":");
    return at < 0 ? account : account.slice(at + 1);
}

/** "83%" from basis points. */
export function percent(confidence: number): string {
    return `${Math.round(confidence / 100)}%`;
}

// ---------------------------------------------------------------------------
// Similar rows (D10, D11)
// ---------------------------------------------------------------------------

/** What one similar row will receive, for its chips. */
export interface SimilarRow {
    txn: ReviewTxn;
    /** The ops that apply the card's change to it (never a reviewed tag). */
    ops: ReturnType<typeof similarOps>;
    /** The new description, when it gets one. */
    rename: string | null;
    /** `[from, to]` for every account it moves. */
    moves: [string, string][];
    /** It is also in this run's queue (its card will arrive already fixed). */
    inQueue: boolean;
}

/**
 * The similar section's rows for the card's current change: each candidate
 * that `change` actually applies to, with exactly what it would receive.
 * A null change (nothing applicable edited yet) lists every candidate with
 * nothing to receive, so the section can show its count before an edit.
 */
export function similarRows(candidates: readonly ReviewTxn[], inQueue: readonly number[], change: CardChange | null): SimilarRow[] {
    const queued = new Set(inQueue);
    const rows: SimilarRow[] = [];
    for (const txn of candidates) {
        const base = {txn, inQueue: queued.has(txn.txn.index)};
        if (change === null) {
            rows.push({...base, ops: [], rename: null, moves: []});
            continue;
        }
        const ops = similarOps(txn.txn, change);
        if (ops.length === 0) continue;
        const rename = ops.some((op) => op.op === "setDescription") ? change.description : null;
        const moves: [string, string][] = [];
        for (const posting of writtenPostings(txn.txn)) {
            const to = change.accounts.get(posting.account);
            if (to !== undefined && !moves.some(([from]) => from === posting.account)) moves.push([posting.account, to]);
        }
        rows.push({...base, ops, rename, moves});
    }
    return rows;
}

/** The tri-state header checkbox: none, some (indeterminate) or all of the rows checked. */
export function checkState(rows: readonly SimilarRow[], checked: ReadonlySet<number>): "none" | "some" | "all" {
    const count = rows.filter((row) => checked.has(row.txn.txn.index)).length;
    if (count === 0) return "none";
    return count === rows.length ? "all" : "some";
}

/** "apply: description + account" — what the section's header says the change is. */
export function changeSummary(change: CardChange | null): string {
    if (change === null) return "";
    const parts = [];
    if (change.description !== null) parts.push("description");
    if (change.accounts.size > 0) parts.push(change.accounts.size === 1 ? "account" : "accounts");
    return parts.join(" + ");
}

// ---------------------------------------------------------------------------
// The reviewed tag, read back (D1, D21)
// ---------------------------------------------------------------------------

export type ReviewBadge = {kind: "reviewed"; on: ISODate | null; label: string} | {kind: "ignored"; label: string} | {kind: "unreviewed"; label: string};

/** The modal's badge (D21): "Reviewed Oct 30", "Ignored" or "Unreviewed". Header tags only. */
export function reviewBadge(tags: readonly (readonly [string, string])[]): ReviewBadge {
    const tag = tags.find(([name]) => name === REVIEWED_TAG);
    if (tag === undefined) return {kind: "unreviewed", label: "Unreviewed"};
    const value = tag[1].trim();
    if (value === "ignore") return {kind: "ignored", label: "Ignored"};
    if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return {kind: "reviewed", on: value, label: `Reviewed ${shortDate(value)}`};
    return {kind: "reviewed", on: null, label: "Reviewed"};
}

// ---------------------------------------------------------------------------
// Journal row stripes (D20)
// ---------------------------------------------------------------------------

export type RowMarker = "unknown" | "unreviewed" | null;

/** `date` minus `days` calendar days, as an ISO date. */
export function minusDays(date: ISODate, days: number): ISODate {
    const at = utcDay(date);
    at.setUTCDate(at.getUTCDate() - days);
    return at.toISOString().slice(0, 10);
}

/**
 * A journal row's stripe: amber when unknown, faint when unreviewed — each
 * only inside the review window (`today − maxDays` … `today`, inclusive; 0 =
 * no lower bound), so old history and future-dated rows stay clean and the
 * faint stripe marks exactly what Review Recents would pick up.
 */
export function rowMarker(row: {txn: {date: ISODate}; unknown: boolean; reviewed: boolean}, today: ISODate, maxDays: number, enabled: boolean): RowMarker {
    if (!enabled) return null;
    const date = row.txn.date;
    if (date > today || (maxDays > 0 && date < minusDays(today, maxDays))) return null;
    if (row.unknown) return "unknown";
    return row.reviewed ? null : "unreviewed";
}

const STRIPE: Record<Exclude<RowMarker, null>, string> = {
    unknown: "color-mix(in oklab, var(--color-warning) 80%, transparent)",
    unreviewed: "color-mix(in oklab, var(--color-base-content) 15%, transparent)",
};

/**
 * The first cell's `box-shadow` for a row: the 3px review stripe at the very
 * edge, and the cursor's 3px primary rule INSIDE it — two stacked inset
 * layers, so neither hides the other (D20). A real border would not paint:
 * daisyUI tables are `border-collapse: separate`.
 */
export function rowShadow(marker: RowMarker, cursor: boolean): string {
    const layers: string[] = [];
    if (marker !== null) layers.push(`inset 3px 0 0 0 ${STRIPE[marker]}`);
    if (cursor) layers.push(`inset ${marker === null ? 3 : 6}px 0 0 0 var(--color-primary)`);
    return layers.length === 0 ? "none" : layers.join(", ");
}

/** The stripe's colour alone, for the narrow layout's card edge. */
export function stripeColor(marker: Exclude<RowMarker, null>): string {
    return STRIPE[marker];
}
