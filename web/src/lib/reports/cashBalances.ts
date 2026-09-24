// Cash and short-term liability balances for the journal tab's Balances view.
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
//     the same `resolveAccountType` the cash-flow report and the balance
//     sheet's "Cash and cash equivalents" line use.
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

import {declaredTerms, inferBsTerm, resolveBsTerm} from "../domain/accountTerms";
import {declaredTypes, resolveAccountType, type AccountDecl} from "../domain/accountTypes";
import {add, cmp, dec, isZero, neg, rankCommodities, type Dec, type MixedAmount} from "../domain/money";
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

/** One row of the rendered view: one account, one commodity. */
export interface BalanceRow {
    account: string;
    qty: Dec;
    asOf: ISODate | null;
    confirmed: boolean;
}

/** The whole view for one commodity: two lists and the three headline figures. */
export interface BalanceSummary {
    commodity: string;
    /** Cash accounts, largest holding first; a negative (overdrawn) balance sorts by magnitude like any other. */
    cash: BalanceRow[];
    /** Liability accounts, largest debt first. */
    liabilities: BalanceRow[];
    totalCash: Dec;
    /** Negative when money is owed, matching how the rows read. */
    totalLiabilities: Dec;
    /** totalCash + totalLiabilities — what is left after clearing short-term debt. */
    net: Dec;
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
 * heuristic get a vote (see `inferBsTerm`, which documents why the list is as
 * short as it is). A tag always beats a guess, so `bsterm: current` on a
 * home-equity line brings it back into the view.
 */
export function cashViewClassifier(decls: readonly AccountDecl[]): (account: string) => Membership {
    const types = declaredTypes(decls);
    const terms = declaredTerms(decls);
    return (account) => {
        const type = resolveAccountType(account, types);
        if (type !== "cash" && type !== "liability") return null;
        const declared = resolveBsTerm(account, terms);
        // Undeclared reads as current, which is the engine's default for every
        // group except Investments — and an Investments account is never Cash
        // (the type rule outranks the commodity rule) nor a liability.
        if (declared !== null) return declared === "noncurrent" ? null : type;
        return inferBsTerm(account, type) === "noncurrent" ? "guessed-long-term" : type;
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
function cashBalanceSet(txns: Transaction[], decls: readonly AccountDecl[], asOf: ISODate): CashBalanceSet {
    let byDecls = cache.get(txns);
    if (byDecls === undefined) {
        byDecls = new WeakMap();
        cache.set(txns, byDecls);
    }
    let byDate = byDecls.get(decls);
    if (byDate === undefined) {
        byDate = new Map();
        byDecls.set(decls, byDate);
    }
    const hit = byDate.get(asOf);
    if (hit !== undefined) return hit;
    const computed = computeAccountBalances(txns, decls, asOf);
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
export function accountBalances(txns: Transaction[], decls: readonly AccountDecl[], asOf: ISODate): AccountBalance[] {
    return cashBalanceSet(txns, decls, asOf).balances;
}

/**
 * Accounts left out of the view because a word in their NAME looked long-term,
 * with no `bsterm:` tag anywhere in the ancestry to say so. Shares the pass and
 * the memo with [`accountBalances`].
 *
 * This exists so the guess is never silent: the view names them, and one
 * `bsterm: current` tag brings any of them back.
 */
export function guessedLongTerm(txns: Transaction[], decls: readonly AccountDecl[], asOf: ISODate): string[] {
    return cashBalanceSet(txns, decls, asOf).guessedLongTerm;
}

/** The actual pass; [`cashBalanceSet`] is the memoized front door. */
function computeAccountBalances(txns: Transaction[], decls: readonly AccountDecl[], asOf: ISODate): CashBalanceSet {
    const classify = cashViewClassifier(decls);
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

/**
 * Commodities held by any account in the view, most-used first (ties
 * alphabetical) — drives the currency selector.
 *
 * "Most used" is per ACCOUNT, which is what an `AccountBalance`'s amounts
 * already are: one entry per commodity, however many postings built it.
 */
export function balanceCommodities(balances: readonly AccountBalance[]): string[] {
    return rankCommodities(balances.map((balance) => balance.amounts));
}

function abs(d: Dec): Dec {
    return d.m < 0n ? neg(d) : d;
}

/** Largest magnitude first; exact zeroes last, then alphabetical, so the ordering is total and stable. */
function byMagnitude(a: BalanceRow, b: BalanceRow): number {
    const magnitude = cmp(abs(b.qty), abs(a.qty));
    return magnitude !== 0 ? magnitude : a.account < b.account ? -1 : 1;
}

/**
 * Project one commodity out of [`accountBalances`] into the two rendered lists
 * and the three headline figures.
 *
 * An account with no posting at all in `commodity` is omitted; one whose
 * postings NET to zero is kept. "This account is empty" and "this account has
 * never held this currency" are different statements, and only the first is
 * worth a row the reader can choose to hide.
 */
export function summarize(balances: readonly AccountBalance[], commodity: string): BalanceSummary {
    const cash: BalanceRow[] = [];
    const liabilities: BalanceRow[] = [];
    let totalCash = ZERO;
    let totalLiabilities = ZERO;
    for (const balance of balances) {
        const qty = balance.amounts.get(commodity);
        if (qty === undefined) continue;
        const row: BalanceRow = {account: balance.account, qty, asOf: balance.asOf, confirmed: balance.confirmed};
        if (balance.kind === "cash") {
            cash.push(row);
            totalCash = add(totalCash, qty);
        } else {
            liabilities.push(row);
            totalLiabilities = add(totalLiabilities, qty);
        }
    }
    cash.sort(byMagnitude);
    liabilities.sort(byMagnitude);
    return {commodity, cash, liabilities, totalCash, totalLiabilities, net: add(totalCash, totalLiabilities)};
}

/** Rows worth drawing: every row when `hideZero` is false, else only those with a non-zero balance. */
export function visibleRows(rows: readonly BalanceRow[], hideZero: boolean): BalanceRow[] {
    return hideZero ? rows.filter((row) => !isZero(row.qty)) : [...rows];
}
