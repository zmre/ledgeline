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

import type {Dec} from "#lib/domain/money.js";
import type {Amount, ISODate, Posting, Transaction, TxnStatus} from "#lib/domain/types.js";
import type {ReviewForm} from "#lib/review/types.js";
import {isIsoDate} from "#lib/url/params.js";
import type {
    AddTransactionBody,
    InsertPosition,
    PatchTransactionBody,
    WireBalanceAssertion,
    WireCost,
    WireDec,
    WireEditOp,
    WirePostingAmount,
    WirePostingInput,
    WirePostingType,
} from "./native";

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
    /**
     * Real / virtual `(a)` / balanced-virtual `[a]`, carried through from an
     * edited transaction (preserved, not edited — same contract as `cost`).
     *
     * Losing this is not cosmetic: a `[budget:env]` envelope leg rewritten as a
     * real posting moves money onto the balance sheet that was never there. New
     * rows start `"regular"`, which is what the popup can express.
     */
    type: WirePostingType;
    /**
     * A `=`/`==` balance assertion carried through from an edited transaction
     * (preserved, not edited). It is the account's reconciliation anchor — once
     * dropped from the file it cannot be recovered.
     */
    balanceAssertion: WireBalanceAssertion | null;
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
    return {account: "", amount: "", commodity: defaultCommodity, status: "unmarked", comment: "", cost: null, type: "regular", balanceAssertion: null};
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

/** Domain assertion → the write wire's shape (the asserted amount never carries a cost). */
function assertionToWire(assertion: NonNullable<Posting["balanceAssertion"]>): WireBalanceAssertion {
    return {
        amount: {commodity: assertion.amount.commodity, quantity: encodeDec(assertion.amount.qty)},
        inclusive: assertion.inclusive,
        total: assertion.total,
    };
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
        type: posting.type ?? "regular",
        balanceAssertion: posting.balanceAssertion !== undefined ? assertionToWire(posting.balanceAssertion) : null,
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
        // Only the postings the file writes. A posting an `=` rule generated is
        // the rule's output, re-made on every read; writing it back would make
        // it a real posting the rule then adds to a second time (plan 30). The
        // modal lists them read-only below the form instead.
        postings: txn.postings.filter((posting) => posting.generated === undefined).map(postingToForm),
    };
}

/**
 * Form → the whole-transaction wire body (shared by ADD and REPLACE). Rows with
 * a blank account are dropped; a row with a blank/invalid amount becomes the
 * elided leg (no `amount` field). Optional string fields are omitted when empty
 * so the body stays minimal.
 *
 * A posting amount NEVER carries a blank commodity: a bare number re-parses
 * under a journal `D` default-commodity directive (or is inferred) to a
 * different commodity/precision, which the engine's round-trip guard would
 * reject. Any row with an amount but a cleared commodity falls back to
 * `defaultCommodity` (the journal's dominant commodity, itself defaulting to
 * `$`).
 *
 * `type` and `balanceAssertion` are echoed back verbatim. The engine writes a
 * posting exactly as it is sent, so NOT sending them is what erased balance
 * assertions and rewrote `[balanced-virtual]` legs as real postings on every
 * save (DL-2). `type` is omitted when `"regular"` (the engine's default) purely
 * to keep the body minimal; the assertion rides only a row that still has an
 * amount, because an assertion cannot be written on the inferred leg —
 * `validateForm` refuses that combination rather than let it drop silently.
 */
export function formToBody(form: TxnForm, defaultCommodity: string, position?: InsertPosition): AddTransactionBody {
    const fallbackCommodity = defaultCommodity.trim() || "$";
    const postings: WirePostingInput[] = [];
    for (const row of form.postings) {
        const account = row.account.trim();
        if (account === "") continue;
        const posting: WirePostingInput = {account};
        if (row.status !== "unmarked") posting.status = row.status;
        if (row.comment.trim() !== "") posting.comment = row.comment.trim();
        if (row.type !== "regular") posting.type = row.type;
        const qty = parseAmountInput(row.amount);
        if (qty !== null) {
            const commodity = row.commodity.trim() || fallbackCommodity;
            const amount: WirePostingAmount = {commodity, quantity: encodeDec(qty)};
            if (row.cost !== null) amount.cost = row.cost;
            posting.amount = amount;
            if (row.balanceAssertion !== null) posting.balanceAssertion = row.balanceAssertion;
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

/**
 * Minimal client-side gate (the engine does the real balancing/validation and
 * returns a 400 message): a valid date and at least one posting with an
 * account; any non-blank amount must parse. Returns human messages ([] = ok).
 *
 * It also refuses to submit a row whose balance assertion would be dropped.
 * An assertion can only be written next to an amount, so blanking the amount of
 * an asserted posting would silently discard its reconciliation anchor — the
 * DL-2 failure, arrived at from the other direction. Destroying an anchor has
 * to be a deliberate act (the popup's "remove" control), never a side effect of
 * clearing a number.
 */
export function validateForm(form: TxnForm): string[] {
    const errors: string[] = [];
    const date = form.date.trim();
    if (date === "") errors.push("A date is required.");
    else if (!isIsoDate(date)) errors.push("The date must be in YYYY-MM-DD form.");
    const withAccount = form.postings.filter((p) => p.account.trim() !== "");
    if (withAccount.length === 0) errors.push("Add at least one posting with an account.");
    for (const row of withAccount) {
        if (row.amount.trim() !== "" && parseAmountInput(row.amount) === null) {
            errors.push(`"${row.amount}" is not a valid amount.`);
        }
        if (row.balanceAssertion !== null && parseAmountInput(row.amount) === null) {
            errors.push(`"${row.account.trim()}" has a balance assertion, which needs an amount on that posting — give it an amount, or remove the assertion.`);
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

/**
 * 0-based positions of the WRITTEN postings whose account equals `account`.
 *
 * Positions count every posting as served — generated ones included — because
 * that is what the engine's PATCH addresses: it maps a served position to the
 * file's line itself, and refuses (409) one that lands on a generated posting
 * (plan 30). So a generated posting on the same account is skipped here, never
 * renumbered around.
 */
export function postingIndicesForAccount(txn: Transaction, account: string): number[] {
    const indices: number[] = [];
    txn.postings.forEach((posting, index) => {
        if (posting.account === account && posting.generated === undefined) indices.push(index);
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

// ---------------------------------------------------------------------------
// Review card → batch ops (plan 32 D9, D11)
// ---------------------------------------------------------------------------

/** The tag a reviewed transaction carries (D1). */
export const REVIEWED_TAG = "reviewed";
/** The tag a rename from the review card records the ORIGINAL similarity key in (D13a, Q12). */
export const PAYEE_KEY_TAG = "payee-key";

/**
 * Tags the card never shows as chips: hledger's hidden twins (`_name`) and
 * the ones `--auto` and the forecast stamp on a transaction at read time,
 * which are not in the file to edit.
 */
const MACHINE_TAGS: ReadonlySet<string> = new Set(["modified-transaction", "generated-transaction"]);

function isCardTag(name: string): boolean {
    return !name.startsWith("_") && !MACHINE_TAGS.has(name);
}

/**
 * The engine's reading of one comma segment (`parse.rs tag_in_segment`): a
 * tag when any word precedes its first `:` — the last such word is the name.
 */
function isTagSegment(segment: string): boolean {
    const colon = segment.indexOf(":");
    return colon >= 0 && segment.slice(0, colon).trim() !== "";
}

/**
 * The FREE TEXT of a transaction comment: the non-tag comma segments of its
 * first (same-line) line, trimmed and rejoined with `", "` — exactly the part
 * `setTxnComment` owns (`edit.rs free_text`).
 */
export function commentFreeText(comment: string): string {
    const firstLine = (comment.split("\n", 1)[0] ?? "").trim();
    return firstLine
        .split(",")
        .filter((segment) => !isTagSegment(segment))
        .map((segment) => segment.trim())
        .filter((segment) => segment !== "")
        .join(", ");
}

/** The review card's form for `txn` (D8): free-text comment, tag chips, and the written postings. */
export function reviewFormOf(txn: Transaction): ReviewForm {
    return {
        date: txn.date,
        description: txn.description,
        comment: commentFreeText(txn.comment),
        tags: txn.tags.filter(([name]) => isCardTag(name)).map(([name, value]) => [name, value]),
        postings: txnToForm(txn).postings,
    };
}

/** Two typed amounts denote the same number (`5.5` and `5.50` do; blank only equals blank). */
function sameAmount(a: string, b: string): boolean {
    const x = parseAmountInput(a);
    const y = parseAmountInput(b);
    if (x === null || y === null) return x === y && a.trim() === b.trim();
    const places = Math.max(x.p, y.p);
    return x.m * 10n ** BigInt(places - x.p) === y.m * 10n ** BigInt(places - y.p);
}

function sameJson(a: unknown, b: unknown): boolean {
    return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * Whether `form` changes `original`'s postings beyond their accounts — an
 * amount, a commodity, a posting added or removed (D11's "split"). Such a
 * save re-renders the transaction (`replace`), and is never applied to
 * similar transactions. An elided amount left blank is not a change.
 */
export function isSplit(original: Transaction, form: ReviewForm): boolean {
    const before = txnToForm(original).postings;
    if (before.length !== form.postings.length) return true;
    return before.some((was, at) => {
        const now = form.postings[at];
        return (
            now === undefined ||
            !sameAmount(was.amount, now.amount) ||
            (parseAmountInput(now.amount) !== null && was.commodity.trim() !== now.commodity.trim()) ||
            was.status !== now.status ||
            was.comment.trim() !== now.comment.trim() ||
            was.type !== now.type ||
            !sameJson(was.cost, now.cost) ||
            !sameJson(was.balanceAssertion, now.balanceAssertion)
        );
    });
}

/** The first value of each tag name, in order. */
function firstValues(tags: readonly (readonly [string, string])[]): Map<string, string> {
    const values = new Map<string, string>();
    for (const [name, value] of tags) if (!values.has(name)) values.set(name, value);
    return values;
}

/** `setTxnTag` ops turning `before`'s tags into `after`'s: changed or added ones set, missing ones removed. */
function tagOps(before: readonly (readonly [string, string])[], after: readonly (readonly [string, string])[]): WireEditOp[] {
    const was = firstValues(before);
    const now = firstValues(after);
    const ops: WireEditOp[] = [];
    for (const [name, value] of now) if (was.get(name) !== value) ops.push({op: "setTxnTag", name, value});
    for (const name of was.keys()) if (!now.has(name)) ops.push({op: "setTxnTag", name, value: null});
    return ops;
}

/**
 * The batch ops that save `form` over `original` (D9), in the order the
 * engine wants them:
 *
 * - **Description and accounts only:** surgical `setDescription` /
 *   `setPostingAccount`, so the rest of the text — an elided amount, a
 *   hand-aligned column — is untouched.
 * - **A split, or a new date:** `replace` first (there is no surgical date
 *   op), which re-renders the one transaction with its comment as it was.
 * - **Then, either way,** `setTxnTag` for each tag chip changed, added or
 *   removed, and `setTxnComment` when the free text changed — surgical over
 *   whichever text the ops before left.
 *
 * Empty when nothing changed. Positions are the form's rows: the postings the
 * file writes.
 */
export function formToOps(original: Transaction, form: ReviewForm, defaultCommodity = "$"): WireEditOp[] {
    const before = reviewFormOf(original);
    const ops: WireEditOp[] = [];
    if (isSplit(original, form) || form.date.trim() !== original.date) {
        const body = formToBody(
            {
                date: form.date,
                date2: original.date2 ?? "",
                status: original.status,
                code: original.code,
                description: form.description,
                comment: original.comment,
                postings: form.postings,
            },
            defaultCommodity
        );
        ops.push({op: "replace", transaction: body});
    } else {
        if (form.description.trim() !== original.description) ops.push({op: "setDescription", description: form.description.trim()});
        form.postings.forEach((row, posting) => {
            const account = row.account.trim();
            if (account !== "" && account !== before.postings[posting]?.account) ops.push({op: "setPostingAccount", posting, account});
        });
    }
    ops.push(...tagOps(before.tags, form.tags));
    if (form.comment.trim() !== before.comment) ops.push({op: "setTxnComment", comment: form.comment.trim()});
    return ops;
}

/**
 * The ops a review save sends for its own card: `formToOps`, plus — when the
 * description changed — `payee-key:<original key>` so the rename can be
 * suggested once the raw text is gone (Q12), plus `reviewed:<date>` when the
 * card is being marked reviewed.
 */
export function cardOps(
    original: Transaction,
    form: ReviewForm,
    options: {key: string | null; reviewedOn: ISODate | null; defaultCommodity?: string}
): WireEditOp[] {
    const ops = formToOps(original, form, options.defaultCommodity);
    const renamed = form.description.trim() !== original.description;
    const hasKey = (tags: readonly (readonly [string, string])[]): boolean => tags.some(([name]) => name === PAYEE_KEY_TAG);
    if (renamed && options.key !== null && !hasKey(original.tags) && !hasKey(form.tags)) {
        ops.push({op: "setTxnTag", name: PAYEE_KEY_TAG, value: options.key});
    }
    if (options.reviewedOn !== null) ops.push({op: "setTxnTag", name: REVIEWED_TAG, value: options.reviewedOn});
    return ops;
}

/** What a card's save changes that may be applied to similar transactions (D11). */
export interface CardChange {
    /** The new description, or null when it is unchanged. */
    description: string | null;
    /** Old account → new account, for each account the card moved. */
    accounts: ReadonlyMap<string, string>;
}

/**
 * What `form` changes over `original` that similar transactions could take:
 * the description and the accounts. Null for a split (D11: never applied to
 * similar transactions) and when nothing applicable changed.
 */
export function cardChange(original: Transaction, form: ReviewForm): CardChange | null {
    if (isSplit(original, form)) return null;
    const before = txnToForm(original).postings;
    const accounts = new Map<string, string>();
    form.postings.forEach((row, at) => {
        const was = before[at]?.account;
        const now = row.account.trim();
        if (was !== undefined && now !== "" && now !== was && !accounts.has(was)) accounts.set(was, now);
    });
    const description = form.description.trim() !== original.description ? form.description.trim() : null;
    return description === null && accounts.size === 0 ? null : {description, accounts};
}

/**
 * The ops that apply `change` to one similar `candidate`: its description set,
 * and every WRITTEN posting on a moved account moved. Empty when nothing
 * applies (the candidate has none of the accounts and the description is
 * unchanged or already that). Never marks it reviewed (D11).
 */
export function similarOps(candidate: Transaction, change: CardChange): WireEditOp[] {
    const ops: WireEditOp[] = [];
    if (change.description !== null && candidate.description !== change.description) ops.push({op: "setDescription", description: change.description});
    candidate.postings
        .filter((posting) => posting.generated === undefined)
        .forEach((posting, written) => {
            const account = change.accounts.get(posting.account);
            if (account !== undefined) ops.push({op: "setPostingAccount", posting: written, account});
        });
    return ops;
}
