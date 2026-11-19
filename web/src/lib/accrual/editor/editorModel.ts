// The D-editor's pure half (plan 45): what the Accrual section shows for a
// card, read from the card's postings and the accounts' roles (`/api/accrual`
// serves them once) and types. Recomputed on every edit, client-side; the
// engine computes the schedules themselves.

import {decToInput, parseAmountInput, type PostingForm} from "#lib/api/editMapping.js";
import {accountTypeOf, type AccountClasses, type AccountType} from "#lib/domain/accountClass.js";
import type {Dec} from "#lib/domain/money.js";
import type {ISODate} from "#lib/domain/types.js";
import type {ReviewForm} from "#lib/review/types.js";
import {
    commentTag,
    CUSTOMER_TAG,
    defaultDescription,
    INVOICE_TAG,
    isDeferralRole,
    isSettlementRole,
    MATCH_TAG,
    partyKindFor,
    RECOGNITION_TAG,
    sourceTags,
    VENDOR_TAG,
    withCommentTags,
    type ScheduleDraft,
} from "../model";
import type {AccrualRole, Party, Term} from "../types";

/** One row of the D-editor table. */
export type AccrualLineKind =
    /** A deferred-revenue credit or prepaid debit with no `recognition:`: the "Recognize over time" form. */
    | "unscheduled"
    /** The same, already scheduled: the summary, Edit schedule, End contract. */
    | "scheduled"
    /** A revenue credit or expense debit beside a cash, AR or AP leg, with no deferral: earned at once, or over a term. */
    | "earned"
    /** An AR debit / AP credit: an invoice or a bill. */
    | "invoice"
    /** An AR credit / AP debit from cash: a payment. */
    | "payment"
    /** A deferred debit (prepaid credit) outside a recognition entry: a credit note. */
    | "credit";

export interface AccrualLine {
    kind: AccrualLineKind;
    /** The posting's position on the card. */
    posting: number;
    account: string;
    role: AccrualRole | null;
    commodity: string;
    /** The typed amount, signed; null when the leg is elided or blank. */
    amount: Dec | null;
    /** `recognition:` on the posting. */
    id: string | null;
    party: Party | null;
    invoice: string | null;
    /** The posting carries a `match:` tag (a linked invoice or payment). */
    matched: boolean;
}

type RoleOf = (account: string) => AccrualRole | null;

const sign = (amount: Dec | null): number => (amount === null ? 0 : amount.m > 0n ? 1 : amount.m < 0n ? -1 : 0);

/** The card's party: a posting's `customer:`/`vendor:` first, else the transaction's tags. */
export function cardParty(form: ReviewForm, posting: PostingForm | null): Party | null {
    const from = (comment: string): Party | null => {
        const customer = commentTag(comment, CUSTOMER_TAG);
        if (customer !== null && customer !== "") return {kind: "customer", name: customer};
        const vendor = commentTag(comment, VENDOR_TAG);
        if (vendor !== null && vendor !== "") return {kind: "vendor", name: vendor};
        return null;
    };
    if (posting !== null) {
        const own = from(posting.comment);
        if (own !== null) return own;
    }
    for (const [name, value] of form.tags) {
        if (name === CUSTOMER_TAG && value !== "") return {kind: "customer", name: value};
        if (name === VENDOR_TAG && value !== "") return {kind: "vendor", name: value};
    }
    return null;
}

function tagOnCard(form: ReviewForm, posting: PostingForm, name: string): string | null {
    return commentTag(posting.comment, name) ?? form.tags.find(([tag]) => tag === name)?.[1] ?? null;
}

/** A posted recognition entry (it carries `recognized:`) is never a source: the section stays away. */
export function isRecognitionEntry(form: ReviewForm, txnTags: readonly (readonly [string, string])[]): boolean {
    return txnTags.some(([name]) => name === "recognized") || form.tags.some(([name]) => name === "recognized");
}

const CASHLIKE: ReadonlySet<AccountType> = new Set(["cash"]);

/**
 * The card's accrual lines, in posting order (the D-editor table). `roleOf`
 * is the accrual role from `/api/accrual`; `classes` gives the account types.
 */
export function classifyCard(form: ReviewForm, roleOf: RoleOf, classes: AccountClasses, txnTags: readonly (readonly [string, string])[] = []): AccrualLine[] {
    if (isRecognitionEntry(form, txnTags)) return [];
    const typed = form.postings.map((posting) => {
        const role = posting.account.trim() === "" ? null : roleOf(posting.account.trim());
        const type = posting.account.trim() === "" ? null : accountTypeOf(posting.account.trim(), classes);
        return {posting, role, type, amount: parseAmountInput(posting.amount)};
    });
    const hasCash = typed.some(({type}) => type !== null && CASHLIKE.has(type));
    const hasSettlement = typed.some(({role}) => isSettlementRole(role));
    const hasDeferral = typed.some(({role}) => isDeferralRole(role));
    // A payment's fee or discount is not revenue or a cost to spread.
    const isPayment = typed.some(({role, amount}) => (role === "receivable" && sign(amount) < 0) || (role === "payable" && sign(amount) > 0));
    const lines: AccrualLine[] = [];
    typed.forEach(({posting, role, type, amount}, at) => {
        const s = sign(amount);
        const id = commentTag(posting.comment, RECOGNITION_TAG);
        let kind: AccrualLineKind | null = null;
        if (role === "deferred-revenue") kind = s < 0 ? (id !== null && id !== "" ? "scheduled" : "unscheduled") : s > 0 ? "credit" : null;
        else if (role === "prepaid") kind = s > 0 ? (id !== null && id !== "" ? "scheduled" : "unscheduled") : s < 0 ? "credit" : null;
        else if (role === "receivable") kind = s > 0 ? "invoice" : s < 0 && hasCash ? "payment" : null;
        else if (role === "payable") kind = s < 0 ? "invoice" : s > 0 && hasCash ? "payment" : null;
        else if (!hasDeferral && !isPayment && (hasCash || hasSettlement) && ((type === "revenue" && s < 0) || (type === "expense" && s > 0))) kind = "earned";
        if (kind === null) return;
        lines.push({
            kind,
            posting: at,
            account: posting.account.trim(),
            role,
            commodity: posting.commodity.trim(),
            amount,
            id: id === "" ? null : id,
            party: cardParty(form, posting),
            invoice: tagOnCard(form, posting, INVOICE_TAG),
            matched: commentTag(posting.comment, MATCH_TAG) !== null,
        });
    });
    return lines;
}

/** The deferral role an earned line would move to: deferred revenue for revenue, prepaid for an expense. */
export function deferralRoleFor(line: AccrualLine, classes: AccountClasses): "deferred-revenue" | "prepaid" {
    return accountTypeOf(line.account, classes) === "expense" ? "prepaid" : "deferred-revenue";
}

/** The accounts that have `role`, sorted. */
export function accountsWithRole(roles: ReadonlyMap<string, {role: AccrualRole}>, role: AccrualRole): string[] {
    return [...roles.entries()]
        .filter(([, value]) => value.role === role)
        .map(([account]) => account)
        .sort();
}

/** The declaration the section shows when no account has the role yet (D-editor "Suggesting a deferred account"). */
export function declarationHint(role: "deferred-revenue" | "prepaid"): string {
    return role === "deferred-revenue"
        ? "account liabilities:deferred-revenue  ; type: L, kpi: deferred-revenue"
        : "account assets:prepaid  ; type: A, kpi: prepaid";
}

/**
 * The "Recognize into" default: the renewal prefill's target, else the earned
 * line the deferral replaced (its old account), else the first other
 * revenue (or expense) leg on the card, else "".
 */
export function defaultTarget(form: ReviewForm, line: AccrualLine, classes: AccountClasses, prefillTarget: string | null): string {
    if (prefillTarget !== null && prefillTarget !== "") return prefillTarget;
    const want: AccountType = line.role === "prepaid" ? "expense" : "revenue";
    const other = form.postings.find((posting, at) => at !== line.posting && accountTypeOf(posting.account.trim(), classes) === want);
    return other?.account.trim() ?? "";
}

/**
 * A fresh draft for `lines` (one schedule over lines with the same terms):
 * starts on the transaction date, 12 months, anniversary timing.
 */
export function freshDraft(
    id: string,
    form: ReviewForm,
    lines: readonly AccrualLine[],
    targets: readonly string[],
    party: Party | null,
    contract: ScheduleDraft["contract"]
): ScheduleDraft {
    const first = lines[0];
    const kind = first?.role === undefined || first.role === null ? "customer" : partyKindFor(first.role);
    const who = party === null ? null : {...party, kind};
    return {
        id,
        txnDate: form.date,
        start: form.date,
        term: {months: 12},
        proration: "anniversary",
        contract,
        party: who,
        description: defaultDescription(who, form.description),
        lines: lines.map((line, at) => ({
            sourceAccount: line.account,
            targetAccount: targets[at] ?? "",
            commodity: line.commodity,
            quantity: line.amount === null ? "" : decToInput(line.amount),
        })),
    };
}

/** What the draft is missing before it can be previewed or saved, or null when it is complete. */
export function draftProblem(draft: ScheduleDraft): string | null {
    if (draft.lines.length === 0) return "No deferred line to schedule.";
    if (draft.lines.some((line) => line.targetAccount.trim() === "")) return "Choose the account to recognize into.";
    if (draft.lines.some((line) => parseAmountInput(line.quantity) === null)) return "The deferred line needs an amount.";
    if (!/^\d{4}-\d{2}-\d{2}$/.test(draft.start)) return "Choose a service start date.";
    const term: Term = draft.term;
    if (term.months !== undefined ? !(Number.isSafeInteger(term.months) && term.months >= 1) : !/^\d{4}-\d{2}-\d{2}$/.test(term.through)) {
        return "Choose a term.";
    }
    if (term.months === undefined && term.through <= draft.start) return "The end date must be after the service start.";
    return null;
}

/**
 * `form` with each draft's source tags on its lines' postings (D-vocab), the
 * draft's lines found by source account. Every other byte of every comment is
 * kept. Pure: a new form.
 */
export function applyDrafts(form: ReviewForm, drafts: readonly {draft: ScheduleDraft; postings: readonly number[]}[]): ReviewForm {
    const postings = form.postings.map((posting) => ({...posting}));
    for (const {draft, postings: at} of drafts) {
        const tags = sourceTags(draft);
        for (const index of at) {
            const posting = postings[index];
            if (posting !== undefined) posting.comment = withCommentTags(posting.comment, tags);
        }
    }
    return {...form, postings};
}

/** The draft's lines, one per posting, refreshed from the card (amounts follow the form). */
export function linesFromCard(draft: ScheduleDraft, lines: readonly AccrualLine[]): ScheduleDraft {
    return {
        ...draft,
        txnDate: draft.txnDate,
        lines: lines.map((line, at) => ({
            sourceAccount: line.account,
            targetAccount: draft.lines[at]?.targetAccount ?? draft.lines[0]?.targetAccount ?? "",
            commodity: line.commodity,
            quantity: line.amount === null ? "" : decToInput(line.amount),
        })),
    };
}

/** Every schedule id already in use on the card or its transaction: a fresh one avoids them. */
export function idsTaken(...lists: readonly (readonly (string | null)[])[]): ReadonlySet<string> {
    return new Set(lists.flat().filter((id): id is string => id !== null));
}

/** "Schedule A", "Schedule B"… */
export function scheduleLetter(at: number): string {
    return `Schedule ${String.fromCharCode(65 + (at % 26))}`;
}

/** Summary counts of a served schedule: "12 × $1,000 · 4 posted · 1 due · next Dec 17" parts. */
export interface ScheduleSummary {
    periods: number;
    posted: number;
    due: number;
    closed: number;
    next: ISODate | null;
}

export function summarize(periods: readonly {date: ISODate; status: string | null}[], today: ISODate): ScheduleSummary {
    return {
        periods: periods.length,
        posted: periods.filter((p) => p.status === "posted").length,
        due: periods.filter((p) => p.status === "due").length,
        closed: periods.filter((p) => p.status === "closed").length,
        next: periods.find((p) => p.date > today)?.date ?? null,
    };
}
