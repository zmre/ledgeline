// Domain types for the engine's journal-view routes (plan 26 B3–B5). Pure TS:
// no Svelte/DOM imports. Decoded from the wire by `api/journalViewDecode.ts`;
// the computations behind them live in `crates/ledgeline-core/src/view/`.

import type {Dec} from "#lib/domain/money.js";
import type {AmountStyle, ISODate} from "#lib/domain/types.js";

/** One row of the Balances view: one account, one commodity. */
export interface BalanceRow {
    /** Full colon path, exactly as posted to — never rolled up into parents. */
    account: string;
    qty: Dec;
    /** Effective date of the account's most recent counted posting. */
    asOf: ISODate;
    /** A posting on that date carried a balance assertion. */
    confirmed: boolean;
}

/** The Balances view for one commodity: two lists and the three headline figures. */
export interface BalanceSummary {
    commodity: string;
    /** The style every figure in this commodity displays with. */
    style: AmountStyle;
    /** Cash accounts, largest magnitude first (an overdrawn account sorts by magnitude too). */
    cash: BalanceRow[];
    /** Liability accounts, largest debt first. */
    liabilities: BalanceRow[];
    totalCash: Dec;
    /** Negative when money is owed, matching how the rows read. */
    totalLiabilities: Dec;
    /** totalCash + totalLiabilities. */
    net: Dec;
}

/** `GET /api/balances/cash?asOf=` — every commodity's summary, so the currency selector costs no request. */
export interface CashBalancesView {
    asOf: ISODate;
    /** Most-used first: the selector's order, and the first is the default. */
    commodities: string[];
    /** One per entry of `commodities`, same order. */
    summaries: BalanceSummary[];
    /** Accounts left out because a word in their NAME looked long-term (never a tag), sorted. */
    guessedLongTerm: string[];
}
