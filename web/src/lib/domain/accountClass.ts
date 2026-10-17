// Account classification, as the ENGINE resolved it (plan 26 B1). Pure TS: no
// Svelte/DOM imports.
//
// `/api/journal` serves every account the journal knows — posted-to, declared,
// and every implied ancestor — with its effective type and balance-sheet term
// (`crates/ledgeline-core/src/reports/account_class.rs`). This module only READS
// that table: there is no `type:`/`bsterm:` tag parsing, no declaration table and
// no ancestor/descendant resolution on this side any more, so the browser and
// the engine cannot classify one account two ways (the 2026-09 sign bug,
// [[account-type-not-name]]).
//
// One residue remains, deliberately: an account the engine has never seen — a
// name being typed into the projections or edit form — is not in the table. For
// that case [`accountTypeOf`] reproduces what the engine's `resolve_account_type`
// would answer: a DECLARED ancestor's type if there is one (the table says which
// types were declared), else hledger's name inference. A brand-new name can have
// no declared descendants (they would have put it in the table), so those two
// steps are the whole resolution for it.

import {categorize} from "./accounts";

/**
 * A resolved account type. `cash`, `conversion` and `gain` are the subtypes
 * hledger tracks beyond the five roots; `isAccountType` folds each into its
 * parent.
 */
export type AccountType = "asset" | "liability" | "equity" | "revenue" | "expense" | "cash" | "conversion" | "gain";

/** The two halves of an Assets or Liabilities box. Equity is never split. */
export type BsTerm = "current" | "noncurrent";

/** One account's classification, exactly as `/api/journal`'s `accounts` serves it. */
export interface AccountClass {
    /** Effective type; null = untyped. */
    readonly type: AccountType | null;
    /** The type came from a `type:` tag on the account or an ancestor (not from its name). */
    readonly typeDeclared: boolean;
    /** Declared/inherited `bsterm:`, else a name guess for a cash or liability account, else null. */
    readonly term: BsTerm | null;
    /** `term` is a name-heuristic GUESS, never a tag — a view excluding on it should say so. */
    readonly termGuessed: boolean;
}

/** Full account name → its classification. */
export type AccountClasses = ReadonlyMap<string, AccountClass>;

export const NO_ACCOUNT_CLASSES: AccountClasses = new Map();

/**
 * The subtype hierarchy, stated ONCE: hledger's three subtypes and the roots
 * they fold into (verified against hledger 1.52: `type:R` matches a declared
 * `type:G` account, `type:E` a `type:V` one).
 */
const PARENT: Partial<Record<AccountType, AccountType>> = {cash: "asset", gain: "revenue", conversion: "equity"};

// hledger's Cash-account name heuristic — exactly the engine's
// `matches_cash_name` (`account_types.rs` CASH_SEGMENTS). `che(ck|qu)ing` is
// `checking`/`chequing`; the `u` flag gives JS Unicode case folding, as the
// engine's lowercasing does.
const CASH_RE = /^assets?(:.+)?:(cash|bank|che(ck|qu)ing|savings?|current)(:|$)/iu;

/** hledger's name inference — the engine's `infer_account_type`. Only consulted for names the engine never classified. */
function inferAccountType(account: string): AccountType | null {
    if (CASH_RE.test(account)) return "cash";
    const root = categorize(account);
    return root === "other" ? null : root;
}

/**
 * The effective type of `account`: the engine's answer when it classified the
 * name, else what it WOULD answer — the nearest known ancestor's type when that
 * type was declared, else name inference. See the module header.
 */
export function accountTypeOf(account: string, classes: AccountClasses): AccountType | null {
    const known = classes.get(account);
    if (known !== undefined) return known.type;
    for (let cut = account.lastIndexOf(":"); cut > 0; cut = account.lastIndexOf(":", cut - 1)) {
        const ancestor = classes.get(account.slice(0, cut));
        if (ancestor === undefined) continue;
        // The NEAREST known ancestor decides: every declared ancestor of this
        // name is that ancestor or above it, so if its type was not declared,
        // none of ours was either.
        return ancestor.typeDeclared ? ancestor.type : inferAccountType(account);
    }
    return inferAccountType(account);
}

/**
 * Whether `account`'s effective type belongs to `category`, subtypes included:
 * `cash` counts as an `asset`, `gain` as a `revenue`, `conversion` as an
 * `equity`. The call every classification should make instead of
 * `accountTypeOf(...) === "x"`; the exact form is right only for a question
 * ABOUT a subtype ("is this Cash").
 */
export function isAccountType(account: string, classes: AccountClasses, category: AccountType): boolean {
    const resolved = accountTypeOf(account, classes);
    if (resolved === null) return false;
    return resolved === category || PARENT[resolved] === category;
}

/** The engine's balance-sheet term for `account` (null term for a name it never classified). */
export function accountTermOf(account: string, classes: AccountClasses): {term: BsTerm | null; guessed: boolean} {
    const known = classes.get(account);
    return known === undefined ? {term: null, guessed: false} : {term: known.term, guessed: known.termGuessed};
}
