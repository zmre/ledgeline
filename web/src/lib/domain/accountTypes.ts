// Declared account types (post-MVP). Pure TS: no Svelte/DOM imports — ports to
// Rust later.
//
// hledger lets an `account` directive carry a `type:` tag (a single letter
// A/L/E/R/X/C/V, or a full word). An account's EFFECTIVE type is its own
// declared type, else the nearest declared ancestor's, else inferred from the
// name (hledger's default regexes). `Cash` is a subtype of Asset and is exactly
// what `hledger cashflow` selects on, so the cash-flow report resolves types
// this way rather than guessing "cash-like" from names alone — a name like
// `assets:bankofamerica` under an `assets ; type: A` declaration is an Asset,
// not Cash, even though "bank" appears in it.

import {categorize} from "./accounts";

export type AccountType = "asset" | "liability" | "equity" | "revenue" | "expense" | "cash" | "conversion";

/** One account's declared type as normalized from /accounts (null = no `type:` tag). */
export interface AccountDecl {
    name: string;
    type: AccountType | null;
}

// hledger's Cash-account name heuristic: an asset account whose path hits a
// cash-like segment (matching a descendant too — assets:bank:wise:eur via
// "bank"). This is the ONLY copy of the regex; cashFlow's isCashLike delegates
// here.
const CASH_RE = /^assets?(:.+)?:(cash|bank|che(ck|que)ing|savings?|current)(:|$)/i;

const TYPE_BY_LETTER: Readonly<Record<string, AccountType>> = {a: "asset", l: "liability", e: "equity", r: "revenue", x: "expense", c: "cash", v: "conversion"};
const TYPE_BY_WORD: Readonly<Record<string, AccountType>> = {
    asset: "asset",
    liability: "liability",
    equity: "equity",
    revenue: "revenue",
    income: "revenue",
    expense: "expense",
    cash: "cash",
    conversion: "conversion",
};

/** Parse a `type:` tag value (single letter A/L/E/R/X/C/V or a full word), case-insensitively; null when unrecognized. */
export function parseAccountTypeTag(value: string): AccountType | null {
    const v = value.trim().toLowerCase();
    return (v.length === 1 ? TYPE_BY_LETTER[v] : TYPE_BY_WORD[v]) ?? null;
}

/** hledger's name-based type inference — the fallback when nothing in the ancestry is declared. null = untyped (no convention match). */
export function inferAccountType(account: string): AccountType | null {
    if (CASH_RE.test(account)) return "cash";
    switch (categorize(account)) {
        case "asset":
            return "asset";
        case "liability":
            return "liability";
        case "equity":
            return "equity";
        case "revenue":
            return "revenue";
        case "expense":
            return "expense";
        default:
            return null;
    }
}

/** Declared (non-null) types keyed by account name. */
export function declaredTypes(decls: readonly AccountDecl[]): Map<string, AccountType> {
    const m = new Map<string, AccountType>();
    for (const d of decls) {
        if (d.type !== null) m.set(d.name, d.type);
    }
    return m;
}

/** Effective type of `account`: own declared → nearest declared ancestor → name inference (null when untyped). */
export function resolveAccountType(account: string, declared: ReadonlyMap<string, AccountType>): AccountType | null {
    for (let name = account; name !== "";) {
        const declaredType = declared.get(name);
        if (declaredType !== undefined) return declaredType;
        const cut = name.lastIndexOf(":");
        if (cut === -1) break;
        name = name.slice(0, cut);
    }
    return inferAccountType(account);
}

/**
 * Cash predicate for the cash-flow report: an account's effective type is Cash.
 * With NO declared types at all this reduces to the pure name heuristic (every
 * account falls straight to inferAccountType), so journals without `type:`
 * declarations behave exactly as before.
 */
export function cashPredicate(decls: readonly AccountDecl[]): (account: string) => boolean {
    const declared = declaredTypes(decls);
    return (account) => resolveAccountType(account, declared) === "cash";
}
