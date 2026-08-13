// Imports tab ⇄ URL-query codec (WP-11). Pure module (no Svelte/DOM imports) so
// the round-trip is unit-testable under node — the vitest project here is
// `node`-only and excludes `*.svelte.test.ts`, so anything living in a component
// is untested by construction. The imports route owns the replaceState glue,
// exactly as the reports route does over `lib/reports/ui/params.ts`.
//
// Scheme: `?tab=new|rules|aliases`, and nothing else. Which rules FILE is open
// is deliberately NOT in the URL: it is picked from a listing the page must
// fetch before it can honour a name, the id is the engine's own opaque handle
// rather than something a user would type, and today's screen has never restored
// it. Adding it later is additive — a new key here and a fallback in the panel.

export type ImportTab = "new" | "rules" | "aliases";

/** "New Transactions" is the first (default) tab — the screen Imports opens on. */
export const TAB_ORDER: ImportTab[] = ["new", "rules", "aliases"];

export const TAB_LABELS: Record<ImportTab, string> = {
    new: "New Transactions",
    rules: "Edit Rules",
    // "Account Aliases" rather than "Aliases": the word alone means nothing to
    // someone who has not read hledger's manual, and this tab is the one place
    // in the app where the reader might not have.
    aliases: "Account Aliases",
};

/** Everything the Imports screen restores from the URL. */
export interface ImportParams {
    tab: ImportTab;
}

/** Narrow an arbitrary query value to a tab id — an unknown one is not a tab. */
export const isTab = (v: string): v is ImportTab => (TAB_ORDER as string[]).includes(v);

/** Defaults: dropping a statement, not editing rules. */
export function defaultImportParams(): ImportParams {
    return {tab: "new"};
}

/** Serialize to a query string (no leading "?"). */
export function paramsToSearch(p: ImportParams): string {
    const q = new URLSearchParams();
    q.set("tab", p.tab);
    return q.toString();
}

/** Parse a query string (with or without leading "?"); absent/malformed params fall back to `dflt`. */
export function searchToParams(search: string, dflt: ImportParams): ImportParams {
    const q = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const tab = q.get("tab");
    return {tab: tab !== null && isTab(tab) ? tab : dflt.tab};
}
