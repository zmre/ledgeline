// Cash balances for the `negative-cash` check rule ONLY (checks/rules.ts).
//
// The Balances view itself is served by the engine (`/api/balances/cash`,
// crates/ledgeline-core/src/view/cash.rs), which was held to goldens captured
// from this module; what is left here goes when the check rules move to the
// engine too (plan 26 B4).
//
// Pure TS, relative imports only — this directory is the engine that ports to
// Rust (see purity.test.ts).
//
// # What this answers, and what it deliberately does not
//
// "How much cash do I have right now, where is it, and how stale is each
// figure." That is a NARROWER question than the balance sheet's, and the
// narrowing is the whole point: no securities, no property, no valuation, no
// FX. Every figure here is a balance as WRITTEN in one commodity, so nothing
// depends on a `P` directive being present or current, and a missing price can
// never make this view read zero.
//
// # Where the classification comes from
//
// Two declared tags, never an account NAME:
//
//   * `type:` — an account is cash when its effective type is hledger's Cash
//     subtype (`type: C`, or the name heuristic when nothing in the ancestry is
//     declared), and a liability when its effective type is Liability. This is
//     the type the ENGINE resolved (`/api/journal` accounts) — the same
//     resolution the cash-flow report and the balance sheet's "Cash and cash
//     equivalents" line use.
//   * `bsterm:` — non-current accounts are excluded, so a mortgage tagged
//     `bsterm: noncurrent` stays out of "short-term liabilities" while the card
//     balance beside it counts. This is ADAPTIVE for free: a journal that
//     declares no term anywhere has nothing resolving to non-current, so
//     nothing is excluded.
//
// [`cashViewClassifier`] is the single seam where that decision is made. It is
// factored out because it is the one thing here that could drift from the
// Balance Sheet tab: an account moved onto or off the Cash line by an explicit
// `bsgroup:` tag is grouped by the Rust engine's five-step
// `AccountGroups::resolve`, which this does not reimplement. Should that matter,
// the fix is to swap this one function for a call to
// `/api/reports/balancesheet/grouped`, which already returns per-account rows
// tagged with their group and term — nothing else in this module changes.
//
// # Signs
//
// None are applied. hledger already records a liability you owe as a negative
// balance and cash you hold as a positive one, which is exactly the display the
// ask calls for ("liabilities should show as negative"). A NEGATIVE cash
// balance therefore means overdrawn, and is worth flagging rather than
// normalizing away — see the `negative-cash` rule in lib/checks/rules.ts, which
// reads these same balances.

import {accountTermOf, accountTypeOf, type AccountClasses} from "../domain/accountClass";
import {add, dec, type Dec, type MixedAmount} from "../domain/money";
import type {ISODate, Transaction} from "../domain/types";

/** Which half of the view an account belongs to. */
export type BalanceKind = "cash" | "liability";

/** One account's as-written balances across every commodity it holds. */
export interface AccountBalance {
    /** Full colon path, exactly as posted to — balances are never rolled up into parents. */
    account: string;
    kind: BalanceKind;
    /** commodity → exact balance. Commodities netting to zero are KEPT (a zeroed account is a fact worth showing). */
    amounts: MixedAmount;
    /**
     * Effective date of the most recent counted posting to this account, in ANY
     * commodity — "when did this account last move", which is the question a
     * staleness figure answers. Null is unreachable for an account that appears
     * here at all (it got here by having a posting) and exists only so the type
     * is honest.
     */
    asOf: ISODate | null;
    /** True when a posting on that latest date carried a balance assertion — the figure was checked, not merely accumulated. */
    confirmed: boolean;
}

/** Exact zero — the accumulator seed and the miss value for a commodity lookup. */
const ZERO: Dec = dec(0n, 0);

/**
 * What the classifier decided about one account: which list it goes in, or why
 * it is out.
 *
 * `"guessed-long-term"` is distinct from plain `null` so the view can SAY it.
 * An account excluded by its own `bsterm:` tag needs no explanation — the owner
 * wrote the tag — but one excluded because a word in its name looked long-term
 * is a guess, and a guess that silently removes a debt from the reader's screen
 * has to be visible and correctable.
 */
export type Membership = BalanceKind | "guessed-long-term" | null;

/**
 * The account-selection seam: full account name → which list it belongs in, or
 * why it is out. See the module header for why this is one function.
 *
 * Order is the point. The declared `type:` decides whether the account is even
 * eligible; then a declared `bsterm:` on the account or any ancestor decides
 * the term; ONLY when nothing in the ancestry declared one does the name
 * heuristic get a vote (the engine's `infer_bs_term`, which documents why the
 * list is as short as it is). All three are resolved by the engine. A tag always beats a guess, so `bsterm: current` on a
 * home-equity line brings it back into the view.
 */
export function cashViewClassifier(classes: AccountClasses): (account: string) => Membership {
    return (account) => {
        const type = accountTypeOf(account, classes);
        if (type !== "cash" && type !== "liability") return null;
        // The engine resolved the term: a declared/inherited `bsterm:`, else a
        // name guess (`infer_bs_term`) flagged as one. Undeclared reads as
        // current, which is the engine's default for every group except
        // Investments — and an Investments account is never Cash (the type rule
        // outranks the commodity rule) nor a liability.
        const {term, guessed} = accountTermOf(account, classes);
        if (term !== "noncurrent") return type;
        return guessed ? "guessed-long-term" : null;
    };
}

/**
 * Memo for [`accountBalances`], keyed on the identity of the three things it is
 * a pure function of. Same shape, and the same reasoning, as `signCache` in
 * lib/insights/series.ts: this is a whole-journal pass, two independent
 * consumers want it (the Balances view and the `negative-cash` check rule), and
 * neither's answer changes when the journal's FILTERS do.
 *
 * WeakMap on both object keys so an entry dies with the journal it describes
 * rather than pinning a 150k-transaction array for the life of the process. The
 * innermost key is the as-of date, which is a string.
 *
 * Correctness rests on the arrays being immutable, which they are: normalize.ts
 * deep-freezes what it builds and the journal store replaces its payloads
 * wholesale (`$state.raw`).
 */
const cache = new WeakMap<object, WeakMap<object, Map<string, CashBalanceSet>>>();

/** One pass's whole answer: the accounts in the view, and the ones the name heuristic kept out. */
interface CashBalanceSet {
    balances: AccountBalance[];
    /** Accounts a GUESS excluded (never a tag), sorted, so the view can name them. See [`Membership`]. */
    guessedLongTerm: string[];
}

/** Memoized front door for the whole pass; see [`cache`]. */
function cashBalanceSet(txns: Transaction[], classes: AccountClasses, asOf: ISODate): CashBalanceSet {
    let byClasses = cache.get(txns);
    if (byClasses === undefined) {
        byClasses = new WeakMap();
        cache.set(txns, byClasses);
    }
    let byDate = byClasses.get(classes);
    if (byDate === undefined) {
        byDate = new Map();
        byClasses.set(classes, byDate);
    }
    const hit = byDate.get(asOf);
    if (hit !== undefined) return hit;
    const computed = computeAccountBalances(txns, classes, asOf);
    byDate.set(asOf, computed);
    return computed;
}

/**
 * Every cash and short-term-liability account's balance as of `asOf`
 * (inclusive), from the WHOLE journal — balances are cumulative, so this must
 * be passed the unfiltered transactions, not the journal view's filtered ones.
 *
 * The effective posting date is `posting.date ?? txn.date`, matching
 * `reports::aggregate::account_totals`. Postings of every type contribute
 * (regular, virtual and balanced-virtual), which is what hledger's own balance
 * reports do and therefore what the Balance Sheet tab shows. Amounts count as
 * WRITTEN — a cost annotation is ignored, because a balance is what the account
 * holds and not what it cost.
 *
 * Memoized on argument identity; see [`cache`].
 */
export function accountBalances(txns: Transaction[], classes: AccountClasses, asOf: ISODate): AccountBalance[] {
    return cashBalanceSet(txns, classes, asOf).balances;
}

/** The actual pass; [`cashBalanceSet`] is the memoized front door. */
function computeAccountBalances(txns: Transaction[], classes: AccountClasses, asOf: ISODate): CashBalanceSet {
    const classify = cashViewClassifier(classes);
    // Account name → whether it is in the view, so the ancestor walks inside
    // `classify` run once per distinct account rather than once per posting.
    const kinds = new Map<string, Membership>();
    const entries = new Map<string, AccountBalance>();
    // Only accounts with a POSTING in range are named: an account the guess
    // excluded but that has no activity is not something the reader is missing.
    const guessed = new Set<string>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            // ISO dates compare lexically throughout this codebase; never via Date.
            const date = posting.date ?? txn.date;
            if (date > asOf) continue;
            let kind = kinds.get(posting.account);
            if (kind === undefined) {
                kind = classify(posting.account);
                kinds.set(posting.account, kind);
            }
            if (kind === "guessed-long-term") {
                guessed.add(posting.account);
                continue;
            }
            if (kind === null) continue;
            let entry = entries.get(posting.account);
            if (entry === undefined) {
                entry = {account: posting.account, kind, amounts: new Map(), asOf: null, confirmed: false};
                entries.set(posting.account, entry);
            }
            for (const amount of posting.amounts) {
                entry.amounts.set(amount.commodity, add(entry.amounts.get(amount.commodity) ?? ZERO, amount.qty));
            }
            const asserted = posting.balanceAssertion !== undefined;
            if (entry.asOf === null || date > entry.asOf) {
                entry.asOf = date;
                entry.confirmed = asserted;
            } else if (date === entry.asOf && asserted) {
                entry.confirmed = true;
            }
        }
    }
    return {balances: [...entries.values()], guessedLongTerm: [...guessed].sort()};
}
