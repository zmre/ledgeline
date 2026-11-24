// Segment-aware fuzzy matching over account names, for the account combobox.
//
// This replaces a native `<datalist>`, whose matching was whatever the browser
// engine felt like — which differed between the WKWebView this app ships in and
// the browser `just dev` runs in, and could not be Tab-completed at all.
//
// Pure and dependency-free on purpose: the whole thing is a linear scan, and at
// realistic sizes (25 accounts in fixtures/sample.journal, a few hundred in a
// real journal) that is free per keystroke. No memoization, no index — `LIMIT`
// exists to keep the popup a sane length, not for speed.

/** Ranking tiers, best first. A match's tier dominates every other consideration. */
const enum Tier {
    /** The query IS the account. */
    Exact = 0,
    /** The account starts with the query, colons and all: `expenses:gro` → `expenses:groceries:costco`. */
    FullPrefix = 1,
    /** Query segments prefix consecutive account segments from the ROOT: `ex:gr`. */
    AnchoredHead = 2,
    /** Query segments prefix account segments in order AND the last one hits the leaf: `gr:co`, `costco`. */
    AnchoredLeaf = 3,
    /** Query segments prefix account segments in order, anywhere in the name. */
    Ordered = 4,
    /** The leaf merely contains the query: `ostc` → `costco`. */
    LeafSubstring = 5,
    /** Every query character appears in order once colons are ignored: `exgro`. */
    Subsequence = 6,
}

export interface AccountMatch {
    name: string;
    /** Lower is better. Exposed so the combobox can compute a longest-common-prefix over the top tier only. */
    tier: number;
}

/** Default popup length. Not a performance guard — see the file header. */
export const LIMIT = 50;

function segmentsOf(name: string): string[] {
    return name.split(":");
}

/**
 * Do the query's segments prefix the account's segments in order, starting at
 * `from`? Returns the account-segment index the LAST query segment landed on,
 * or -1 for no match.
 */
function matchOrdered(query: readonly string[], account: readonly string[], from: number): number {
    let at = from;
    let landed = -1;
    for (const part of query) {
        let found = -1;
        for (let i = at; i < account.length; i += 1) {
            if (account[i].startsWith(part)) {
                found = i;
                break;
            }
        }
        if (found === -1) return -1;
        landed = found;
        at = found + 1;
    }
    return landed;
}

/** Are all of `query`'s characters present in `text`, in order? */
function isSubsequence(query: string, text: string): boolean {
    let at = 0;
    for (const char of text) {
        if (char === query[at]) at += 1;
        if (at === query.length) return true;
    }
    return at === query.length;
}

/** The tier `name` earns for `query`, or null if it does not match at all. Both must already be lowercased. */
function tierFor(query: string, queryParts: readonly string[], name: string): Tier | null {
    if (name === query) return Tier.Exact;
    if (name.startsWith(query)) return Tier.FullPrefix;

    const parts = segmentsOf(name);
    // Head-anchored: every query segment prefixes the account segment at the
    // same position. `ex:gr` → `expenses:groceries:…`.
    if (queryParts.length <= parts.length && queryParts.every((part, at) => parts[at].startsWith(part))) return Tier.AnchoredHead;

    const landed = matchOrdered(queryParts, parts, 0);
    if (landed !== -1) return landed === parts.length - 1 ? Tier.AnchoredLeaf : Tier.Ordered;

    // Below here the query is treated as one blob: a user typing `ostc` or
    // `exgro` is not thinking in segments.
    const flat = query.replaceAll(":", "");
    if (parts[parts.length - 1].includes(flat)) return Tier.LeafSubstring;
    if (isSubsequence(flat, name.replaceAll(":", ""))) return Tier.Subsequence;
    return null;
}

/**
 * Accounts matching `query`, best first.
 *
 * Case-insensitive. An empty query returns everything alphabetically, so the
 * popup is useful before the first keystroke. A trailing colon falls out
 * naturally: `expenses:` splits to `["expenses", ""]` and an empty segment
 * prefixes anything, so it lists that account's children.
 */
export function matchAccounts(query: string, names: readonly string[], limit: number = LIMIT): AccountMatch[] {
    const needle = query.trim().toLowerCase();
    if (needle === "") {
        return [...names]
            .sort((a, b) => a.localeCompare(b))
            .slice(0, limit)
            .map((name) => ({name, tier: Tier.Exact}));
    }

    const queryParts = segmentsOf(needle);
    const matches: {name: string; tier: Tier}[] = [];
    for (const name of names) {
        const tier = tierFor(needle, queryParts, name.toLowerCase());
        if (tier !== null) matches.push({name, tier});
    }

    // Tier dominates; then shorter names, because a match on `expenses:gas`
    // is more likely what you meant than one on a deeper account that merely
    // contains the same letters; then alphabetical, so the order is stable and
    // does not depend on the journal's account declaration order.
    matches.sort((a, b) => a.tier - b.tier || a.name.length - b.name.length || a.name.localeCompare(b.name));
    return matches.slice(0, limit);
}

/**
 * The longest string every name starts with, character by character.
 *
 * Deliberately NOT segment-aware: raw character LCP already does the useful
 * thing. Over `expenses:groceries:costco` and `expenses:gas` it yields
 * `expenses:g`, which is exactly the shell-completion behaviour — stopping at
 * `expenses:` would throw away a character the user has already earned.
 *
 * Case is taken from the FIRST name, so completing preserves the journal's own
 * capitalization rather than whatever the user typed.
 */
export function longestCommonPrefix(names: readonly string[]): string {
    if (names.length === 0) return "";
    let prefix = names[0];
    for (const name of names.slice(1)) {
        let at = 0;
        while (at < prefix.length && at < name.length && prefix[at].toLowerCase() === name[at].toLowerCase()) at += 1;
        prefix = prefix.slice(0, at);
        if (prefix === "") break;
    }
    return prefix;
}

/**
 * The completion Tab should apply, or null when Tab has nothing to add and
 * should fall through to normal focus traversal.
 *
 * The LCP is taken over the TOP TIER only. Mixing tiers would let a loose
 * subsequence match drag the shared prefix back to nothing, so Tab would
 * silently do nothing exactly when the ranking is most confident.
 */
export function tabCompletion(value: string, matches: readonly AccountMatch[]): string | null {
    if (matches.length === 0) return null;
    const best = matches[0].tier;
    const prefix = longestCommonPrefix(matches.filter((match) => match.tier === best).map((match) => match.name));
    // Only useful if it actually adds characters. Comparing case-insensitively
    // means Tab still normalizes `EXPENSES:g` to the journal's own spelling.
    if (prefix.length > value.length) return prefix;
    if (prefix.length === value.length && prefix !== value) return prefix;
    return null;
}
