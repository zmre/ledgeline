// Declared balance-sheet TERM (`bsterm:`). Pure TS: no Svelte/DOM imports.
//
// Three account tags, three questions (docs/balance-sheet.md): `type:` picks
// which box a balance lands in, `bsgroup:` picks the line within it, and
// `bsterm:` — this one — picks which HALF of the box that line sits in, current
// or non-current. Like both of the others it INHERITS to sub-accounts, so a
// term declared on `liabilities:mortgage` covers `liabilities:mortgage:escrow`
// without being repeated.
//
// The vocabulary is CLOSED, and this is the TypeScript mirror of
// `reports::account_groups::parse_bs_term_tag`, spelling for spelling — the
// same relationship `accountTypes.ts` already has with `reports::account_types`.
// A term that quietly fell back would file a balance under the wrong subtotal
// and leave the statement looking fine, so an unrecognized value reads as "not
// declared" here rather than as a guess. It is not swallowed either: the engine
// separately reports it as an `account-tag` diagnostic in Problems.

/** The two halves of an Assets or Liabilities box. Equity is never split. */
export type BsTerm = "current" | "noncurrent";

/**
 * Every accepted spelling, matched case-insensitively after trimming. The
 * canonical two are `current` and `noncurrent`; the rest are the synonyms the
 * Rust parser accepts, listed in docs/balance-sheet.md.
 */
const TERM_BY_SPELLING: Readonly<Record<string, BsTerm>> = {
    current: "current",
    short: "current",
    shortterm: "current",
    "short-term": "current",
    noncurrent: "noncurrent",
    "non-current": "noncurrent",
    long: "noncurrent",
    longterm: "noncurrent",
    "long-term": "noncurrent",
};

/** Parse a `bsterm:` tag value; null when the spelling is outside the closed vocabulary. */
export function parseBsTermTag(value: string): BsTerm | null {
    return TERM_BY_SPELLING[value.trim().toLowerCase()] ?? null;
}

/**
 * The declaration shape this module needs — just the name and the parsed term.
 * Structural rather than importing `AccountDecl`, which lives in
 * `accountTypes.ts` and imports [`BsTerm`] from here.
 */
export interface TermDecl {
    readonly name: string;
    readonly bsterm?: BsTerm | null;
}

/** Declared (non-null) terms keyed by account name. */
export function declaredTerms(decls: readonly TermDecl[]): Map<string, BsTerm> {
    const terms = new Map<string, BsTerm>();
    for (const decl of decls) {
        if (decl.bsterm !== undefined && decl.bsterm !== null) terms.set(decl.name, decl.bsterm);
    }
    return terms;
}

/**
 * Effective term of `account`: own declaration → nearest declared ancestor →
 * null for undeclared.
 *
 * Null is returned rather than a default because the default is not this
 * module's to pick: the Rust engine's `resolve_bs_term` defaults to
 * non-current for the built-in Investments group and to current for everything
 * else, and only a caller that knows the GROUP can apply that. Callers dealing
 * in cash and liabilities — where Investments cannot arise — read null as
 * current.
 *
 * The ancestor walk is the same one `resolveAccountType` does; see it for why
 * inheritance is by path prefix at segment boundaries.
 */
export function resolveBsTerm(account: string, declared: ReadonlyMap<string, BsTerm>): BsTerm | null {
    for (let name = account; name !== "";) {
        const term = declared.get(name);
        if (term !== undefined) return term;
        const cut = name.lastIndexOf(":");
        if (cut === -1) break;
        name = name.slice(0, cut);
    }
    return null;
}

// ---------- the name heuristic ----------
//
// Everything below GUESSES a term from the account's name, and it exists under
// protest. This codebase's standing rule is that membership is decided by a
// declared tag and never by an English word (docs/balance-sheet.md, "Why
// grouping never matches account names"), because a chart of accounts may be in
// any language. That rule is not repealed here — the guess runs only where
// hledger puts its own name heuristic, as the LAST resort after the account and
// every ancestor have been asked for a `bsterm:` tag, and it is confined to the
// journal tab's Balances view. The Rust balance sheet does not do this.
//
// The word list is short on purpose. It comes from a survey of 108 public
// plain-text-accounting repositories (2,057 distinct account paths) plus the
// GnuCash, QuickBooks and Xero default charts, and the selection rule was
// PRECISION, not coverage: a false positive silently drops a real debt out of
// the user's short-term picture, which is worse than missing one. So words that
// are *usually* long-term are all excluded — `loan` (a personal loan may be due
// this month), `car`/`auto`/`vehicle`, `note`, `bond`, `lease`, `deposit`,
// `property`, `capital`, `fixed`, `principal`, and every short abbreviation
// (`ltd` is a company suffix, `sep` is September, `re` is a substring of
// retained-earnings). A car loan really will be missed. The fix for a miss is
// one tag; there is no fix for a balance the reader never saw.

/** Explicit statements of intent. These are the tag's own vocabulary, written into the name instead. */
const EXPLICIT_LONG_TERM = new Set(["noncurrent", "longterm"]);

/**
 * Liability words that essentially only ever name a long-dated debt.
 *
 * `mortage` is not a typo here — it is the observed misspelling, common enough
 * to appear in the surveyed corpus, and a mortgage filed under short-term debt
 * is exactly the wrong answer. `student` rather than `studentloan` because the
 * corpus never once spelled it as one word: real paths are
 * `liabilities:loans:student` and `Liabilities:Loan:Student`.
 */
const LONG_TERM_LIABILITY_WORDS = new Set(["mortgage", "mortage", "heloc", "student", "pension"]);

/**
 * Phrases that INVERT the reading, checked before anything else.
 *
 * "Long-term debt, current maturities" is a real FASB line item and it is a
 * CURRENT liability — the slice of the long-term loan due inside a year. A
 * plain search for "long-term" gets it exactly backwards, and the same shape
 * turns up in other standards' charts. Finding one of these means the account
 * is deliberately the current portion, so the guess stands down entirely.
 */
const CURRENT_PORTION_PHRASES = new Set(["currentmaturities", "currentportion", "duewithin"]);

/**
 * Every word in the path, plus every adjacent pair joined.
 *
 * Words rather than segments because the concept is buried at any depth and
 * written every which way — `Assets:Retirement:Vanguard:Company401k`,
 * `liabilities:long-term-debt`, `Liabilities:Loans:nz_student_loan`. Joined
 * pairs are what make one entry cover `noncurrent`, `non-current` and
 * `non current` at once.
 */
function pathWords(account: string): Set<string> {
    const words = account
        .toLowerCase()
        .split(/[^a-z0-9]+/)
        .filter((word) => word !== "");
    const found = new Set(words);
    for (let i = 0; i + 1 < words.length; i++) found.add(words[i] + words[i + 1]);
    return found;
}

/**
 * Guess whether `account` is non-current from its NAME, for an account whose
 * effective type is already known. Null means "no opinion" — which is the
 * answer for almost every account, and the answer callers must treat as
 * current.
 *
 * `kind` gates the vocabulary: the liability words are only consulted for a
 * liability, so `expenses:pension-contributions` and a savings account called
 * `assets:student` are untouched. A `cash` account gets only the explicit
 * words, because declaring an account `type: C` is the owner saying it is a
 * cash equivalent and a word in its name is not grounds to overrule them.
 */
export function inferBsTerm(account: string, kind: "cash" | "liability"): BsTerm | null {
    const words = pathWords(account);
    for (const phrase of CURRENT_PORTION_PHRASES) {
        if (words.has(phrase)) return null;
    }
    for (const word of EXPLICIT_LONG_TERM) {
        if (words.has(word)) return "noncurrent";
    }
    if (kind !== "liability") return null;
    for (const word of LONG_TERM_LIABILITY_WORDS) {
        if (words.has(word)) return "noncurrent";
    }
    return null;
}
