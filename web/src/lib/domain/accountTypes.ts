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

import type {BsTerm} from "./accountTerms";
import {categorize} from "./accounts";

/**
 * A resolved account type. `cash`, `conversion` and `gain` are the subtypes
 * hledger tracks beyond the five roots; `isAccountType` folds each into its
 * parent.
 */
export type AccountType = "asset" | "liability" | "equity" | "revenue" | "expense" | "cash" | "conversion" | "gain";

/** One account's declaration as normalized from /accounts (null = the tag is absent or unrecognized). */
export interface AccountDecl {
    name: string;
    type: AccountType | null;
    /**
     * Declared `bsterm:` — the current / non-current half of the balance-sheet
     * box (see `./accountTerms`). Optional rather than required so the many
     * fixtures and tests that build a bare `{name, type}` still type-check;
     * absent and null both mean "not declared".
     */
    bsterm?: BsTerm | null;
}

// hledger's Cash-account name heuristic: an asset account whose path hits a
// cash-like segment (matching a descendant too — assets:bank:wise:eur via
// "bank"). This is the ONLY copy of the regex; cashFlow's isCashLike delegates
// here.
//
// The segments must be exactly `account_types.rs`'s CASH_SEGMENTS. `che(ck|qu)ing`
// is `checking` and `chequing` — NOT `che(ck|que)ing`, which expands to
// `chequeing` and left the ordinary British/Canadian `assets:chequing` reading
// as Cash to the engine and a plain Asset to the browser.
//
// The `u` flag is load-bearing, not decoration: only with it does JS apply
// Unicode case folding, so `assets:BAN\u{212A}` (KELVIN SIGN) matches here as it
// does in `matches_cash_name`, which lowercases non-ASCII and pins that case.
const CASH_RE = /^assets?(:.+)?:(cash|bank|che(ck|qu)ing|savings?|current)(:|$)/iu;

const TYPE_BY_LETTER: Readonly<Record<string, AccountType>> = {
    a: "asset",
    l: "liability",
    e: "equity",
    r: "revenue",
    x: "expense",
    c: "cash",
    v: "conversion",
    g: "gain",
};
const TYPE_BY_WORD: Readonly<Record<string, AccountType>> = {
    asset: "asset",
    assets: "asset",
    liability: "liability",
    liabilities: "liability",
    equity: "equity",
    equities: "equity",
    revenue: "revenue",
    revenues: "revenue",
    income: "revenue",
    incomes: "revenue",
    expense: "expense",
    expenses: "expense",
    cash: "cash",
    conversion: "conversion",
    conversions: "conversion",
    gain: "gain",
    gains: "gain",
};

/**
 * Parse a `type:` tag value (single letter A/L/E/R/X/C/V/G or a full word),
 * case-insensitively; null when unrecognized.
 *
 * MUST accept exactly what `reports::account_types::parse_account_type_tag`
 * accepts. The two vocabularies drifting is not a cosmetic difference: an
 * unrecognized value here is dropped from the declaration table and the account
 * then falls all the way through to ENGLISH NAME INFERENCE, so the engine and
 * the browser end up disagreeing about what the account IS. On the Projections
 * tab that disagreement is a sign flip — `income:contractors ; type: expenses`
 * resolved as revenue here and as expense there makes `signedQuantity` negate a
 * cost, and the engine then projects an expense that pays money IN
 * ([[account-type-not-name]], plan 22 amendment 42).
 *
 * The plurals and `income`/`incomes` are a deliberate superset of hledger's own
 * vocabulary, for the reason the engine's copy documents: hledger REJECTS them
 * with a parse error while we degrade silently, so accepting them gives the
 * obviously-intended meaning instead of a silent misfile. Every journal hledger
 * accepts is classified identically.
 */
export function parseAccountTypeTag(value: string): AccountType | null {
    const v = value.trim().toLowerCase();
    const table = v.length === 1 ? TYPE_BY_LETTER : TYPE_BY_WORD;
    // `hasOwn`, not a bare index: these are object literals, so `type:
    // constructor`, `type: toString` and `type: __proto__` would otherwise read
    // straight through `Object.prototype` and return something that is not an
    // AccountType at all. That is worse than null — `declaredTypes` keeps it
    // (it isn't null), every `isAccountType` answers false, and the account
    // belongs to no section instead of falling back to name inference.
    return table[v] !== undefined && Object.hasOwn(table, v) ? table[v] : null;
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

/**
 * [`resolveAccountType`] memoized per declaration table.
 *
 * The descendant step scans every declaration, and `series.categoryOf` asks this
 * once per POSTING over only a couple of hundred DISTINCT names — the engine's
 * copy measured 840k asks at 200k transactions and memoized for exactly this
 * reason (PERF-5e). Keyed on the table's IDENTITY, which `declaredTypes`
 * rebuilds whenever the journal changes, so a stale entry cannot outlive its
 * input and the whole memo is collected with the table.
 *
 * `declared` is a `ReadonlyMap` and must genuinely be treated as one: mutating a
 * table in place after resolving against it would leave this holding the old
 * answer. Nothing does — every caller passes a table `declaredTypes` just built.
 */
const MEMO = new WeakMap<ReadonlyMap<string, AccountType>, Map<string, AccountType | null>>();

/**
 * Fold two types into the one that describes both, or null when they genuinely
 * disagree. Subtypes collapse into their parent, mirroring `isAccountType`'s
 * hierarchy. Port of the engine's `unify`.
 */
function unify(a: AccountType, b: AccountType): AccountType | null {
    if (a === b) return a;
    if ((a === "cash" && b === "asset") || (a === "asset" && b === "cash")) return "asset";
    if ((a === "gain" && b === "revenue") || (a === "revenue" && b === "gain")) return "revenue";
    if ((a === "conversion" && b === "equity") || (a === "equity" && b === "conversion")) return "equity";
    return null;
}

/**
 * Effective type of `account`: own declared → nearest declared ancestor → name
 * inference → an agreeing declared DESCENDANT's type (null when untyped).
 *
 * Port of `reports::account_types::resolve_account_type`, step for step. The
 * descendant step is last for the reason that copy gives: it exists for the
 * parent rows a depth-clamped report invents — clamping `activo:banco` to depth
 * 1 yields `activo`, declared nowhere and meaningless to the English name
 * heuristic — and running it AFTER inference keeps every conventional chart
 * behaving exactly as before. The projection SEED clamps to depth 2, so these
 * synthetic parents are rows a user really does edit.
 *
 * Descendants must AGREE (up to the subtype hierarchy): a genuinely mixed
 * subtree has no single type, so it gets null and is simply not a member of any
 * section. Taking the lexically first would classify a subtree by whichever
 * child happened to sort first, and renaming a child would silently reclassify
 * every ancestor.
 */
export function resolveAccountType(account: string, declared: ReadonlyMap<string, AccountType>): AccountType | null {
    let memo = MEMO.get(declared);
    if (memo === undefined) {
        memo = new Map<string, AccountType | null>();
        MEMO.set(declared, memo);
    }
    // `has`, not `?? null`: null is a real answer here — "this account is
    // untyped" — and a memo that could not tell it from a miss would re-scan
    // every declaration for exactly the accounts the scan is slowest on.
    if (memo.has(account)) return memo.get(account) ?? null;
    const resolved = resolveUncached(account, declared);
    memo.set(account, resolved);
    return resolved;
}

function resolveUncached(account: string, declared: ReadonlyMap<string, AccountType>): AccountType | null {
    for (let name = account; name !== "";) {
        const declaredType = declared.get(name);
        if (declaredType !== undefined) return declaredType;
        const cut = name.lastIndexOf(":");
        if (cut === -1) break;
        name = name.slice(0, cut);
    }
    const inferred = inferAccountType(account);
    if (inferred !== null) return inferred;

    const prefix = `${account}:`;
    let found: AccountType | null = null;
    for (const [name, type] of declared) {
        if (!name.startsWith(prefix)) continue;
        if (found === null) {
            found = type;
            continue;
        }
        found = unify(found, type);
        if (found === null) return null;
    }
    return found;
}

/**
 * Whether `account`'s effective type belongs to `category`, subtypes included.
 *
 * Port of the engine's `is_account_type`/`is_category`, and the call every
 * classification should make instead of `resolveAccountType(...) === "x"`.
 * `cash` counts as an `asset`, `gain` as a `revenue` and `conversion` as an
 * `equity` — verified against hledger 1.52, whose `type:R` query matches a
 * declared `type:G` account and whose `type:E` matches a `type:V` one.
 *
 * The exact-equality form is still right for the one question that is ABOUT a
 * subtype: `cashPredicate` asks "is this Cash", not "is this an asset".
 */
export function isAccountType(account: string, declared: ReadonlyMap<string, AccountType>, category: AccountType): boolean {
    const resolved = resolveAccountType(account, declared);
    if (resolved === null) return false;
    if (resolved === "cash") return category === "asset" || category === "cash";
    if (resolved === "gain") return category === "revenue" || category === "gain";
    if (resolved === "conversion") return category === "equity" || category === "conversion";
    return resolved === category;
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
