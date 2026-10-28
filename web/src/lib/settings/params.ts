// Settings tab ⇄ URL-query codec. Same shape `imports/params.ts` states for
// its own subnav: `?tab=`, restored once on mount and mirrored back with
// debounced replaceState by the route itself.

export type SettingsTab = "accounts" | "aliases" | "auto";

/** "Accounts" is the first (default) tab. */
export const TAB_ORDER: SettingsTab[] = [
    "accounts",
    "aliases",
    // Plan 30 Phase 5. MERGE NOTE: branch `2026-10-28-hledger-conf` adds an
    // "hledger config" tab here; keep both — "Automatic postings" points at it.
    "auto",
];

export const TAB_LABELS: Record<SettingsTab, string> = {
    accounts: "Accounts",
    // "Account Aliases" rather than "Aliases", for the reason
    // `imports/params.ts` gives: the word alone means nothing to someone who
    // has not read hledger's manual.
    aliases: "Account Aliases",
    // hledger's own name for `= QUERY` rules is "auto postings"; spelled out
    // for the same reason as above.
    auto: "Automatic postings",
};

export interface SettingsParams {
    tab: SettingsTab;
    /**
     * The `=` rule to focus on the Automatic postings tab — its position in the
     * journal's rule list, as a generated posting names it. Absent otherwise.
     */
    rule?: number;
}

export const isTab = (v: string): v is SettingsTab => (TAB_ORDER as string[]).includes(v);

export function defaultSettingsParams(): SettingsParams {
    return {tab: "accounts"};
}

export function paramsToSearch(p: SettingsParams): string {
    const q = new URLSearchParams();
    q.set("tab", p.tab);
    if (p.tab === "auto" && p.rule !== undefined) q.set("rule", String(p.rule));
    return q.toString();
}

export function searchToParams(search: string, dflt: SettingsParams): SettingsParams {
    const q = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const raw = q.get("tab");
    const tab = raw !== null && isTab(raw) ? raw : dflt.tab;
    const rule = q.get("rule");
    if (tab === "auto" && rule !== null && /^\d{1,6}$/.test(rule)) return {tab, rule: Number(rule)};
    return {tab};
}
