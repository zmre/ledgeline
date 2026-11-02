// Completion ranking for the review card's description and tag fields (plan
// 32 D15). Pure and linear, like `accountMatch.ts`: a journal has hundreds of
// payees and a few dozen tags, which is free to scan per keystroke.

import type {ISODate} from "#lib/domain/types.js";
import type {PayeeUse, TagUse} from "./types";

/** How long a payee's recency counts for: its weight halves every this many days since it was last used. */
const HALF_LIFE_DAYS = 180;
/** Default popup length. */
export const COMPLETION_LIMIT = 8;

/** Ranking tiers, best first. */
const enum Tier {
    Prefix = 0,
    WordStart = 1,
}

function daysBetween(from: ISODate, to: ISODate): number {
    return Math.max(0, (Date.parse(`${to}T00:00:00Z`) - Date.parse(`${from}T00:00:00Z`)) / 86_400_000);
}

/** Frequency × recency: a payee used often and lately outranks one used often years ago. */
export function payeeWeight(use: PayeeUse, today: ISODate): number {
    return use.count * 0.5 ** (daysBetween(use.last, today) / HALF_LIFE_DAYS);
}

function tierOf(needle: string, payee: string): Tier | null {
    const hay = payee.toLowerCase();
    if (hay.startsWith(needle)) return Tier.Prefix;
    // A word start: after a space or punctuation (`*BLUE` in `SQ *BLUE BOTTLE`).
    for (let at = hay.indexOf(needle); at > 0; at = hay.indexOf(needle, at + 1)) {
        if (!/[\p{L}\p{N}]/u.test(hay[at - 1] ?? "")) return Tier.WordStart;
    }
    return null;
}

/**
 * Payees matching `query`, best first: those it prefixes, then those with a
 * word it prefixes, each tier by frequency × recency. An empty query lists
 * the heaviest payees. The query itself, typed exactly, is not offered back.
 */
export function rankPayees(query: string, payees: readonly PayeeUse[], today: ISODate, limit = COMPLETION_LIMIT): PayeeUse[] {
    const needle = query.trim().toLowerCase();
    const scored: {use: PayeeUse; tier: Tier; weight: number}[] = [];
    for (const use of payees) {
        if (use.payee.toLowerCase() === needle) continue;
        const tier = needle === "" ? Tier.Prefix : tierOf(needle, use.payee);
        if (tier !== null) scored.push({use, tier, weight: payeeWeight(use, today)});
    }
    scored.sort((a, b) => a.tier - b.tier || b.weight - a.weight || a.use.payee.localeCompare(b.use.payee));
    return scored.slice(0, limit).map((entry) => entry.use);
}

/**
 * The commodities a posting's commodity field offers (plan 32 D33), for what
 * is typed: those it prefixes, then those it appears in, each in the order
 * given (the caller lists the journal's, most used first). Case-insensitive,
 * because `eur` should find `EUR`. An empty query lists them all; the query
 * itself, typed exactly, is not offered back.
 */
export function rankCommodities(query: string, commodities: readonly string[], limit = COMPLETION_LIMIT): string[] {
    const typed = query.trim();
    const needle = typed.toLowerCase();
    const unique = [...new Set(commodities)].filter((commodity) => commodity !== "" && commodity !== typed);
    const prefix = unique.filter((commodity) => commodity.toLowerCase().startsWith(needle));
    const inner = unique.filter((commodity) => !commodity.toLowerCase().startsWith(needle) && commodity.toLowerCase().includes(needle));
    return [...prefix, ...inner].slice(0, limit);
}

/** One tag completion: what the popup shows and what choosing it puts in the field. */
export interface TagCompletion {
    label: string;
    /** The field's new text: `name:` for a name (so the value comes next), `name:value` for a value. */
    insert: string;
    /** How often it is used. */
    count: number;
    kind: "name" | "value";
}

/**
 * Tag completions for the chip input's text: before a `:` it completes the
 * NAME (most used first), after it the VALUE among that tag's commonest ones.
 * Names already on the card (`taken`) are not offered again.
 */
export function tagCompletions(text: string, tags: readonly TagUse[], taken: ReadonlySet<string> = new Set(), limit = COMPLETION_LIMIT): TagCompletion[] {
    const colon = text.indexOf(":");
    if (colon < 0) {
        const needle = text.trim().toLowerCase();
        return tags
            .filter((tag) => !taken.has(tag.name) && tag.name.toLowerCase().startsWith(needle) && tag.name.toLowerCase() !== needle)
            .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name))
            .slice(0, limit)
            .map((tag) => ({label: tag.name, insert: `${tag.name}:`, count: tag.count, kind: "name"}));
    }
    const name = text.slice(0, colon).trim();
    const needle = text
        .slice(colon + 1)
        .trim()
        .toLowerCase();
    const tag = tags.find((candidate) => candidate.name === name);
    if (tag === undefined) return [];
    return tag.values
        .filter((value) => value.value.toLowerCase().startsWith(needle) && value.value.toLowerCase() !== needle)
        .sort((a, b) => b.count - a.count || a.value.localeCompare(b.value))
        .slice(0, limit)
        .map((value) => ({label: value.value, insert: `${name}:${value.value}`, count: value.count, kind: "value"}));
}

/**
 * The chip input's text as a tag, or null when it is not one the engine can
 * write: a name with no whitespace, `:`, `,` or `;`, and a value with no
 * comma or newline (p1 amendment 11). Surrounding spaces are trimmed.
 */
export function parseTag(text: string): [string, string] | null {
    const colon = text.indexOf(":");
    const name = (colon < 0 ? text : text.slice(0, colon)).trim();
    const value = colon < 0 ? "" : text.slice(colon + 1).trim();
    if (name === "" || /[\s:,;]/.test(name) || /[,\n]/.test(value)) return null;
    return [name, value];
}
