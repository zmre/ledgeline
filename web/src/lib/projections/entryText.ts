// How an entry READS on its card (plan 31 C3 "UX requirements"): the period as
// the file writes it, amounts signed as the file holds them, growth as a chip,
// an `=` rule as a sentence. Pure, so the wording is tested without a mount.

import type {Dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {absDec, fmt} from "#lib/format/amounts.js";
import type {AutoRule, Growth, ScenarioLine} from "./types";
import {GROWTH_UNIT_ABBREV, percentFromRate} from "./scenarioModel";
import type {AutoRuleFormPosting} from "#lib/autoRules/ruleForm.js";

/** A signed amount in its commodity's display style, with a real minus. */
export function amountText(commodity: string, quantity: Dec, styles: ReadonlyMap<string, AmountStyle>): string {
    const text = fmt(commodity, absDec(quantity), styles);
    return quantity.m < 0n ? `−${text}` : text;
}

/** A line's amount as the file holds it: signed as typed, served or edited alike. */
export function lineAmountText(line: ScenarioLine, styles: ReadonlyMap<string, AmountStyle>): string {
    return amountText(line.amount.commodity, line.amount.quantity, styles);
}

/** A decimal as typed, trailing zeros kept as the value states them. */
export function decText(d: Dec): string {
    const negative = d.m < 0n;
    const digits = (negative ? -d.m : d.m).toString().padStart(d.p + 1, "0");
    const whole = digits.slice(0, digits.length - d.p);
    const fraction = d.p === 0 ? "" : `.${digits.slice(digits.length - d.p)}`;
    return `${negative ? "−" : ""}${whole}${fraction}`;
}

/** `↗ 3%/yr`; `↘ −2%/mo` for a shrinking one. */
export function growthText(growth: Growth): string {
    const percent = percentFromRate(growth.rate);
    return `${percent.m < 0n ? "↘" : "↗"} ${decText(percent)}%/${GROWTH_UNIT_ABBREV[growth.unit]}`;
}

/** One posting of an `=` rule, as its card reads it: `3% of it`, or a fixed amount. */
export function autoPostingText(posting: AutoRuleFormPosting, styles: ReadonlyMap<string, AmountStyle>): string {
    if (posting.kind === "percent") return posting.percent === null ? "…% of it" : `${decText(posting.percent)}% of it`;
    return posting.amount === null ? "a fixed amount" : amountText(posting.amount.commodity, posting.amount.quantity, styles);
}

/** Why an `=` rule is locked (shown as its text), or null when its form can be edited. */
export function autoLockedReason(rule: AutoRule): string | null {
    if (rule.unsupported !== null) return `${rule.unsupported}, so it generates nothing.`;
    if (rule.form === null) return "This rule says something the editor can't show as a form. It is kept, applied and saved exactly as written.";
    return null;
}
