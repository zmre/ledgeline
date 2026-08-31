// Stock price updates (TODO.md "Stocks" — a Rust port of the user's own
// `update-prices.sh`, generalized to any hledger journal): the Holdings tab's
// "Update prices" button.
//
// Every field mirrors `ledgeline-server/src/prices_api.rs`'s wire types.
//
// A relative import, not the `$lib/domain/money` alias: this file lives in
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
