// An `= QUERY` auto-posting rule as the rule editor edits it (plan 30 Phase 4).
//
// The FORM is the engine's (`crates/ledgeline-core/src/auto_form.rs`): this
// module only holds it, copies it, checks it is complete enough to send, and
// says in one line what it would do. It never renders rule TEXT and never signs
// a number — the engine renders a form into the `=` block and infers each
// multiplier's sign from the accounts' types, so the user types positive
// percentages ("3.5" is 3.5% of the matched amount, growing the target).
//
// Independent of any page store on purpose: the Projections tab uses it for a
// scenario's rules, and Settings › Automatic postings will use it for the main
// journal's.
//
// Pure TS: no Svelte/DOM imports.

import {encodeDec} from "#lib/api/editMapping.js";
import type {WireDec} from "#lib/api/native.js";
import {absDec, fmt} from "#lib/format/amounts.js";
import {mul, neg, type Dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";

/** A commodity and quantity, as a fixed posting or a preview's sample. */
export interface RuleAmount {
    commodity: string;
    quantity: Dec;
    precision: number;
}

/** One posting a rule adds below each match. Exactly one of `percent` / `amount` is set, per `kind`; `null` is "not typed yet". */
export interface AutoRuleFormPosting {
    account: string;
    kind: "percent" | "fixed";
    /** `3.5` for 3.5% of the matched amount. */
    percent: Dec | null;
    amount: RuleAmount | null;
}

/** The engine's `AutoRuleForm`. */
export interface AutoRuleForm {
    /** The account (and subtree) whose postings fire the rule. */
    matchAccount: string;
    /** Further hledger query terms; empty for none. */
    extraQuery: string;
    postings: AutoRuleFormPosting[];
    /** The account that takes the opposite of the postings, or `null`. */
    counter: string | null;
    /** The `=` line's comment; empty for none. */
    comment: string;
}

/** `AutoRuleFormIn` on the wire. */
export interface WireAutoRuleFormIn {
    matchAccount: string;
    extraQuery: string;
    postings: {
        account: string;
        kind: "percent" | "fixed";
        percent: WireDec | null;
        amount: {commodity: string; quantity: WireDec; precision: number} | null;
    }[];
    counter: string | null;
    comment: string;
}

/** A new rule: nothing matched yet, one percentage posting, balanced against `counter`. */
export function blankRuleForm(counter: string | null): AutoRuleForm {
    return {matchAccount: "", extraQuery: "", postings: [blankPosting()], counter, comment: ""};
}

/** A new percentage posting. */
export function blankPosting(): AutoRuleFormPosting {
    return {account: "", kind: "percent", percent: null, amount: null};
}

/** A deep, mutable copy (the decoder freezes what it returns). */
export function cloneRuleForm(form: AutoRuleForm): AutoRuleForm {
    return {
        matchAccount: form.matchAccount,
        extraQuery: form.extraQuery,
        postings: form.postings.map((posting) => ({
            account: posting.account,
            kind: posting.kind,
            percent: posting.percent === null ? null : {m: posting.percent.m, p: posting.percent.p},
            amount:
                posting.amount === null
                    ? null
                    : {
                          commodity: posting.amount.commodity,
                          quantity: {m: posting.amount.quantity.m, p: posting.amount.quantity.p},
                          precision: posting.amount.precision,
                      },
        })),
        counter: form.counter,
        comment: form.comment,
    };
}

/**
 * Whether the form says enough for the engine to render it: an account to
 * match, and every posting with an account and its number. A half-typed rule
 * is left out of a RUN (as a half-typed line is) rather than failing it.
 */
export function ruleFormComplete(form: AutoRuleForm): boolean {
    return (
        form.matchAccount.trim() !== "" &&
        form.postings.length > 0 &&
        form.postings.every((posting) => posting.account.trim() !== "" && (posting.kind === "percent" ? posting.percent !== null : posting.amount !== null))
    );
}

/** The form as `AutoRuleIn.form` takes it. Blank counter is `null`. */
export function ruleFormToWire(form: AutoRuleForm): WireAutoRuleFormIn {
    const counter = form.counter?.trim() ?? "";
    return {
        matchAccount: form.matchAccount.trim(),
        extraQuery: form.extraQuery,
        postings: form.postings.map((posting) => ({
            account: posting.account.trim(),
            kind: posting.kind,
            percent: posting.kind === "percent" && posting.percent !== null ? encodeDec(posting.percent) : null,
            amount:
                posting.kind === "fixed" && posting.amount !== null
                    ? {commodity: posting.amount.commodity.trim(), quantity: encodeDec(posting.amount.quantity), precision: posting.amount.precision}
                    : null,
        })),
        counter: counter === "" ? null : counter,
        comment: form.comment,
    };
}

/** How many postings a preview names before it says "and N more". */
const PREVIEW_POSTINGS = 2;

/**
 * One line saying what the rule does to `sample` — a typical amount on the
 * matched account, when the caller knows one: "≈ $1,200 to
 * expenses:commission on each $40,000 to revenue:sales". Without a sample, the
 * percentages themselves. `null` while the form is too incomplete to say.
 *
 * Magnitudes throughout: the sign is the engine's, and it means "grows". A
 * negative percentage reads as a minus.
 */
export function rulePreview(form: AutoRuleForm, sample: RuleAmount | null, styles: ReadonlyMap<string, AmountStyle>): string | null {
    const match = form.matchAccount.trim();
    const parts: string[] = [];
    for (const posting of form.postings) {
        const account = posting.account.trim();
        if (account === "") continue;
        if (posting.kind === "percent" && posting.percent !== null) {
            const percent = posting.percent;
            if (sample === null) {
                parts.push(`${showPercent(percent)} to ${account}`);
            } else {
                const share = mul(absDec(sample.quantity), {m: percent.m, p: percent.p + 2});
                parts.push(`${signed(share, sample.commodity, styles)} to ${account}`);
            }
        } else if (posting.kind === "fixed" && posting.amount !== null) {
            parts.push(`${signed(posting.amount.quantity, posting.amount.commodity, styles)} to ${account}`);
        }
    }
    if (match === "" || parts.length === 0) return null;
    const shown = parts.slice(0, PREVIEW_POSTINGS).join(", ");
    const more = parts.length > PREVIEW_POSTINGS ? ` and ${parts.length - PREVIEW_POSTINGS} more` : "";
    const on = sample === null ? `of each posting to ${match}` : `on each ${fmt(sample.commodity, absDec(sample.quantity), styles)} to ${match}`;
    return `≈ ${shown}${more} ${on}`;
}

function signed(quantity: Dec, commodity: string, styles: ReadonlyMap<string, AmountStyle>): string {
    const text = fmt(commodity, absDec(quantity), styles);
    return quantity.m < 0n ? `−${text}` : text;
}

/** `3.5%`, `−2%` — exact, as typed. */
function showPercent(percent: Dec): string {
    const negative = percent.m < 0n;
    const magnitude = negative ? neg(percent) : percent;
    const digits = magnitude.m.toString().padStart(magnitude.p + 1, "0");
    const whole = digits.slice(0, digits.length - magnitude.p);
    const fraction = magnitude.p === 0 ? "" : `.${digits.slice(digits.length - magnitude.p)}`;
    return `${negative ? "−" : ""}${whole}${fraction}%`;
}
