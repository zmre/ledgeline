// Pure mappers between the journal domain model and the native write-path wire
// bodies (see native.ts). No Svelte/DOM imports so vitest runs these in node.
//
// Two directions:
//   - domain Transaction → an editable TxnForm (popup prefill), and TxnForm →
//     the whole-transaction AddTransactionBody used by BOTH add (POST) and
//     replace (PUT); plus minimal client-side validation.
//   - surgical PATCH bodies from a single changed field (description, or a
//     recategorized account) — the inline-edit path.
//
// Amounts: the popup edits a plain decimal STRING and a commodity; the exact
// Dec is parsed from the string (its fractional-digit count IS the precision),
// so a value the user did not touch round-trips bit-for-bit. Commodity display
// style is NOT sent — the engine infers it from the journal when rendering.

import type {Dec} from "$lib/domain/money";
import type {Amount, Posting, Transaction, TxnStatus} from "$lib/domain/types";
import type {AddTransactionBody, InsertPosition, PatchTransactionBody, WireCost, WireDec, WirePostingAmount, WirePostingInput} from "./native";

// ---------------------------------------------------------------------------
// Dec ⇆ wire / display string
// ---------------------------------------------------------------------------

/** Exact Dec → the wire's string-mantissa form. */
export function encodeDec(d: Dec): WireDec {
    return {mantissa: d.m.toString(), places: d.p};
}

/**
 * Exact Dec → a plain decimal string for an input field (no grouping, no
 * rounding — unlike formatDec which caps at 2 places). `{m: 5624n, p: 2}` → "56.24".
 */
export function decToInput(d: Dec): string {
    const negative = d.m < 0n;
    const digits = (negative ? -d.m : d.m).toString();
    const sign = negative ? "-" : "";
    if (d.p === 0) return sign + digits;
    const padded = digits.padStart(d.p + 1, "0");
    const intPart = padded.slice(0, padded.length - d.p);
    const fracPart = padded.slice(padded.length - d.p);
    return `${sign}${intPart}.${fracPart}`;
}

/**
 * Parse a user-typed amount into an exact Dec, or null when it isn't a number.
 * Spaces and commas (thousands grouping) are stripped; `.` is the decimal mark
 * (the fractional-digit count becomes the Dec's precision). An empty/sign-only
 * string is null — that's how the popup marks a posting's elided/inferred leg.
 */
export function parseAmountInput(raw: string): Dec | null {
    const s = raw.trim().replace(/[\s,]/g, "");
    if (s === "" || s === "-" || s === "+") return null;
    const match = /^([+-]?)(\d*)(?:\.(\d*))?$/.exec(s);
    if (match === null) return null;
    const intPart = match[2] ?? "";
    const fracPart = match[3] ?? "";
    if (intPart === "" && fracPart === "") return null;
    const digits = (intPart + fracPart).replace(/^0+(?=\d)/, "");
    const sign = match[1] === "-" ? -1n : 1n;
    return {m: sign * BigInt(digits === "" ? "0" : digits), p: fracPart.length};
}

// ---------------------------------------------------------------------------
// Popup form model
// ---------------------------------------------------------------------------

/** One editable posting row: account + an optional amount string + commodity. */
export interface PostingForm {
    account: string;
    amount: string;
    commodity: string;
    /** Per-posting status (`*`/`!`), independent of the transaction status. */
    status: TxnStatus;
    comment: string;
    /** A cost annotation carried through from an edited transaction (the popup preserves but doesn't edit it). */
    cost: WireCost | null;
}

/** The whole-transaction popup form (add-blank or edit-prefilled). */
export interface TxnForm {
    date: string;
    /** Optional secondary/auxiliary date (hledger `date2`); "" when unset. */
    date2: string;
    status: TxnStatus;
    code: string;
    description: string;
    /**
     * The transaction's full comment text. hledger tags ARE comment text
     * (`key:value`), so this single field doubles as the tags editor — it
     * carries and round-trips any inline tags verbatim.
     */
    comment: string;
    postings: PostingForm[];
}

/** A blank posting row seeded with the journal's dominant commodity. */
export function emptyPosting(defaultCommodity: string): PostingForm {
    return {account: "", amount: "", commodity: defaultCommodity, status: "unmarked", comment: "", cost: null};
}

/** A blank two-row form for ADD (today's date, unmarked, dominant commodity). */
export function blankForm(today: string, defaultCommodity: string): TxnForm {
    return {
        date: today,
        date2: "",
        status: "unmarked",
        code: "",
        description: "",
        comment: "",
        postings: [emptyPosting(defaultCommodity), emptyPosting(defaultCommodity)],
    };
}

function costToWire(cost: NonNullable<Amount["cost"]>): WireCost {
    return {kind: cost.per ? "unit" : "total", amount: {commodity: cost.commodity, quantity: encodeDec(cost.qty)}};
}

function postingToForm(posting: Posting): PostingForm {
    const first = posting.amounts[0];
    return {
        account: posting.account,
        amount: first === undefined ? "" : decToInput(first.qty),
        commodity: first === undefined ? "" : first.commodity,
        status: posting.status,
        comment: posting.comment,
        cost: first?.cost !== undefined ? costToWire(first.cost) : null,
    };
}

/**
 * Prefill the popup form from an existing transaction (EDIT-ALL → PUT). `date2`
 * defaults to "" when the txn has no secondary date; `comment` carries the
 * transaction's FULL comment text (tags included) so nothing is lost on save.
 */
export function txnToForm(txn: Transaction): TxnForm {
    return {
        date: txn.date,
        date2: txn.date2 ?? "",
        status: txn.status,
        code: txn.code,
        description: txn.description,
        comment: txn.comment,
        postings: txn.postings.map(postingToForm),
    };
}

/**
 * Form → the whole-transaction wire body (shared by ADD and REPLACE). Rows with
 * a blank account are dropped; a row with a blank/invalid amount becomes the
 * elided leg (no `amount` field). Optional string fields are omitted when empty
 * so the body stays minimal.
 */
export function formToBody(form: TxnForm, position?: InsertPosition): AddTransactionBody {
    const postings: WirePostingInput[] = [];
    for (const row of form.postings) {
        const account = row.account.trim();
        if (account === "") continue;
        const posting: WirePostingInput = {account};
        if (row.status !== "unmarked") posting.status = row.status;
        if (row.comment.trim() !== "") posting.comment = row.comment.trim();
        const qty = parseAmountInput(row.amount);
        if (qty !== null) {
            const amount: WirePostingAmount = {commodity: row.commodity.trim(), quantity: encodeDec(qty)};
            if (row.cost !== null) amount.cost = row.cost;
            posting.amount = amount;
        }
        postings.push(posting);
    }
    const body: AddTransactionBody = {date: form.date.trim(), postings};
    if (form.date2.trim() !== "") body.date2 = form.date2.trim();
    if (form.status !== "unmarked") body.status = form.status;
    if (form.code.trim() !== "") body.code = form.code.trim();
    if (form.description.trim() !== "") body.description = form.description.trim();
    if (form.comment.trim() !== "") body.comment = form.comment.trim();
    if (position !== undefined) body.position = position;
    return body;
}

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/**
 * Minimal client-side gate (the engine does the real balancing/validation and
 * returns a 400 message): a valid date and at least one posting with an
 * account; any non-blank amount must parse. Returns human messages ([] = ok).
 */
export function validateForm(form: TxnForm): string[] {
    const errors: string[] = [];
    const date = form.date.trim();
    if (date === "") errors.push("A date is required.");
    else if (!ISO_DATE.test(date)) errors.push("The date must be in YYYY-MM-DD form.");
    const withAccount = form.postings.filter((p) => p.account.trim() !== "");
    if (withAccount.length === 0) errors.push("Add at least one posting with an account.");
    for (const row of withAccount) {
        if (row.amount.trim() !== "" && parseAmountInput(row.amount) === null) {
            errors.push(`"${row.amount}" is not a valid amount.`);
        }
    }
    return errors;
}

// ---------------------------------------------------------------------------
// Surgical PATCH builders (inline edits)
// ---------------------------------------------------------------------------

/** PATCH body for a description-only change. */
export function descriptionPatch(description: string): PatchTransactionBody {
    return {description};
}

/** PATCH body for a status-only change (the inline cleared/pending toggle). */
export function statusPatch(status: TxnStatus): PatchTransactionBody {
    return {status};
}

/** 0-based positions of the postings whose account equals `account`. */
export function postingIndicesForAccount(txn: Transaction, account: string): number[] {
    const indices: number[] = [];
    txn.postings.forEach((posting, index) => {
        if (posting.account === account) indices.push(index);
    });
    return indices;
}

/**
 * PATCH body recategorizing every posting currently on `oldAccount` to
 * `newAccount` (usually one — clicking an account chip in the journal). Empty
 * `postings` when the old account isn't found (caller should skip the request).
 */
export function accountPatch(txn: Transaction, oldAccount: string, newAccount: string): PatchTransactionBody {
    return {postings: postingIndicesForAccount(txn, oldAccount).map((index) => ({index, account: newAccount}))};
}

/** The journal's most-common commodity (popup default); "$" when the journal is empty. */
export function dominantCommodity(txns: readonly Transaction[]): string {
    const counts = new Map<string, number>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            for (const amount of posting.amounts) {
                if (amount.commodity === "") continue;
                counts.set(amount.commodity, (counts.get(amount.commodity) ?? 0) + 1);
            }
        }
    }
    let best = "$";
    let bestCount = 0;
    for (const [commodity, count] of counts) {
        if (count > bestCount) {
            best = commodity;
            bestCount = count;
        }
    }
    return best;
}
