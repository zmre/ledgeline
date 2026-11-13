// Stock price updates (TODO.md "Stocks" — a Rust port of the user's own
// `update-prices.sh`, generalized to any hledger journal): the Holdings tab's
// "Update prices" button.
//
// Every field mirrors `ledgeline-server/src/prices_api.rs`'s wire types.
//
// A relative import, not the `#lib/domain/money.js` alias: this file lives in
// `lib/holdings/`, whose purity test (`purity.test.ts`) requires engine
// sources to use only relative imports.

import type {Dec} from "../domain/money";

/** One currently-held symbol a quote will be fetched for. */
export interface PriceSymbol {
    /** The hledger commodity symbol. */
    symbol: string;
    /** The ticker it is looked up as on Yahoo Finance: the commodity's `yahoo:` tag, else the symbol itself. */
    yahooTicker: string;
}

/** One candidate (or, after an update, the actual) target file. */
export interface PricesFile {
    journalId: string;
    label: string;
    writable: boolean;
    priceCount: number;
}

/**
 * How the engine's last automatic price run ended (plan 41; `RunOutcome` in
 * `auto_prices.rs`).
 *  - `updated`: new `P` lines were written (`lastAdded` of them).
 *  - `up-to-date`: the journal already had what it wanted; nothing was fetched.
 *  - `nothing-new`: fetched, but nothing new came back (a holiday, a delisted ticker).
 *  - `no-prices-file`: no file holds prices yet, so nothing was done.
 *  - `rate-limited`: Yahoo asked it to slow down; see `backoffUntil`.
 *  - `conflict`: the file changed while it was writing; the next run retries.
 *  - `failed`: a fetch or the write failed; the next run retries.
 */
export type AutoPricesOutcome = "updated" | "up-to-date" | "nothing-new" | "no-prices-file" | "rate-limited" | "conflict" | "failed";

export const AUTO_PRICES_OUTCOMES: readonly AutoPricesOutcome[] = [
    "updated",
    "up-to-date",
    "nothing-new",
    "no-prices-file",
    "rate-limited",
    "conflict",
    "failed",
];

/** `GET /api/prices/status`'s `auto`: the background updater's state. Times are Unix seconds. */
export interface AutoPricesStatus {
    /** The effective `autoPrices` setting for this ledger. */
    enabled: boolean;
    /** When the last run that did anything finished, or null if none has yet. */
    lastRun: number | null;
    lastOutcome: AutoPricesOutcome | null;
    /** How many `P` lines that run wrote. */
    lastAdded: number;
    /** Until when automatic runs are paused after a rate limit, or null. */
    backoffUntil: number | null;
}

/** `GET /api/prices/status`. */
export interface PricesStatus {
    editable: boolean;
    /** The commodity fetched prices are recorded in. */
    quoteCommodity: string;
    symbols: PriceSymbol[];
    defaultTarget: string | null;
    canCreateFile: boolean;
    createFileName: string;
    files: PricesFile[];
    auto: AutoPricesStatus;
}

/** `POST /api/prices/file` → the file it created and the include it wrote. */
export interface CreatedPricesFile {
    journalId: string;
    label: string;
    includedAs: string;
    mainJournalId: string;
}

/** What happened to one symbol during an update — the structured equivalent of what the bash scripts printed per symbol. */
export type PriceOutcome = "updated" | "duplicate" | "not-found" | "fetch-error";

export interface PriceResult {
    symbol: string;
    yahooTicker: string;
    outcome: PriceOutcome;
    /** Present for `updated` and `duplicate`. */
    date: string | null;
    /** Present for `updated` and `duplicate`. */
    price: Dec | null;
}

/** `POST /api/prices/update`. */
export interface PricesUpdateResponse {
    file: PricesFile;
    results: PriceResult[];
}
