// Settings tab ⇄ URL-query codec. Same shape `imports/params.ts` states for
// its own subnav: `?tab=`, restored once on mount and mirrored back with
// debounced replaceState by the route itself.

export type SettingsTab = "accounts" | "aliases";

/** "Accounts" is the first (default) tab. */
export const TAB_ORDER: SettingsTab[] = ["accounts", "aliases"];

export const TAB_LABELS: Record<SettingsTab, string> = {
    accounts: "Accounts",
    // "Account Aliases" rather than "Aliases", for the reason
    // `imports/params.ts` gives: the word alone means nothing to someone who
    // has not read hledger's manual.
    aliases: "Account Aliases",
};

export interface SettingsParams {
    tab: SettingsTab;
}

export const isTab = (v: string): v is SettingsTab => (TAB_ORDER as string[]).includes(v);

export function defaultSettingsParams(): SettingsParams {
    return {tab: "accounts"};
}

export function paramsToSearch(p: SettingsParams): string {
    const q = new URLSearchParams();
    q.set("tab", p.tab);
    return q.toString();
}

export function searchToParams(search: string, dflt: SettingsParams): SettingsParams {
    const q = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const tab = q.get("tab");
    return {tab: tab !== null && isTab(tab) ? tab : dflt.tab};
}
