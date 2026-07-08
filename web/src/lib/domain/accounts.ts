// Account tree + name utilities (WP-02). Pure TS: no Svelte/DOM imports.

export interface AccountNode {
    name: string;
    fullName: string;
    children: AccountNode[];
}

/**
 * Build a tree from flat account names (e.g. /accountnames). Intermediate
 * ancestors are created even when absent from the input; siblings are sorted.
 */
export function buildAccountTree(names: string[]): AccountNode[] {
    const roots: AccountNode[] = [];
    const byFullName = new Map<string, AccountNode>();
    for (const fullName of [...names].sort()) {
        if (fullName === "") continue;
        let path = "";
        let siblings = roots;
        for (const segment of fullName.split(":")) {
            path = path === "" ? segment : `${path}:${segment}`;
            let node = byFullName.get(path);
            if (node === undefined) {
                node = {name: segment, fullName: path, children: []};
                byFullName.set(path, node);
                siblings.push(node);
            }
            siblings = node.children;
        }
    }
    return roots;
}

/** Clamp an account name to `depth` segments: ("a:b:c", 2) → "a:b". */
export function clampAccount(name: string, depth: number): string {
    return name.split(":").slice(0, depth).join(":");
}

/** True when `account` is `selected` itself or any of its sub-accounts. */
export function accountMatches(selected: string, account: string): boolean {
    return account === selected || account.startsWith(selected + ":");
}

export type RootCategory = "asset" | "liability" | "equity" | "revenue" | "expense" | "other";

/** Categorize by hledger-convention root account name (assets*, liabilities*, equity*, revenues|income*, expenses*). */
export function categorize(account: string): RootCategory {
    const root = account.split(":", 1)[0].toLowerCase();
    if (root.startsWith("asset")) return "asset";
    if (root.startsWith("liabilit")) return "liability";
    if (root.startsWith("equity")) return "equity";
    if (root.startsWith("revenue") || root.startsWith("income")) return "revenue";
    if (root.startsWith("expense")) return "expense";
    return "other";
}
