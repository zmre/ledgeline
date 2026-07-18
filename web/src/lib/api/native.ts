// ledgeline-engine native (/api/*) HTTP client. Fetch + query-string building +
// error taxonomy only; wire-shape knowledge lives in nativeDecode.ts. Mirrors
// HledgerApi (the wire client) but distinguishes a server that simply lacks the
// native routes (a plain hledger-web) from one that is unreachable.

import {ApiUnreachableError} from "./client";

/** The configured server answered, but has no /api/* routes (e.g. plain hledger-web). */
export class NativeApiUnavailableError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "NativeApiUnavailableError";
    }
}

/** User-facing copy for the missing-engine case (a 404 or non-JSON on /api/*). */
export const NATIVE_UNAVAILABLE_MESSAGE = "This server doesn't provide Ledgeline's report API — start the Ledgeline engine.";

type QueryValue = string | number | undefined;

/** Build a `?a=1&b=2` string, dropping undefined and empty-string values (no leading "?" when empty). */
function queryString(values: Record<string, QueryValue>): string {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(values)) {
        if (value === undefined) continue;
        const text = typeof value === "number" ? String(value) : value;
        if (text !== "") params.set(key, text);
    }
    const encoded = params.toString();
    return encoded === "" ? "" : `?${encoded}`;
}

export interface BalanceSheetQuery {
    asOf?: string;
    depth?: number;
}
export interface IncomeStatementQuery {
    from?: string;
    to?: string;
    depth?: number;
}
export interface CashFlowQuery {
    end?: string;
    interval?: string;
    count?: number;
    depth?: number;
}
export interface NetWorthQuery {
    end?: string;
    interval?: string;
    count?: number;
    depth?: number;
    valueIn?: string;
}
export interface HoldingsQuery {
    asOf?: string;
    /** Comma-separated subtree roots; empty = all accounts. */
    accounts?: string;
    mode?: "include" | "exclude";
    /**
     * Optional gain-window start (YYYY-MM-DD). Absent/empty ⇒ all-time gain
     * (marketValue − basis). When set, the engine returns a WINDOWED gain
     * (marketValue − value-at-gainSince); the response JSON keys are unchanged.
     */
    gainSince?: string;
}
export interface HoldingsSeriesQuery extends HoldingsQuery {
    interval?: string;
    count?: number;
}

export class LedgelineApi {
    readonly baseUrl: string;

    constructor(baseUrl: string) {
        this.baseUrl = baseUrl.replace(/\/+$/, "");
    }

    /** GET a native route, returning raw unknown JSON; pass through a nativeDecode.* decoder separately. */
    private async getJson(route: string): Promise<unknown> {
        const url = `${this.baseUrl}${route}`;
        let response: Response;
        try {
            // no-store: report data must always come from the live engine, never the HTTP cache
            response = await fetch(url, {headers: {Accept: "application/json"}, cache: "no-store"});
        } catch (cause) {
            throw new ApiUnreachableError(`Cannot reach the Ledgeline engine at ${this.baseUrl} (network or CORS failure)`, {cause});
        }
        // A server without the native routes (plain hledger-web) 404s here.
        if (response.status === 404) {
            throw new NativeApiUnavailableError(NATIVE_UNAVAILABLE_MESSAGE);
        }
        if (!response.ok) {
            throw new ApiUnreachableError(`GET ${url} responded ${response.status} ${response.statusText}`);
        }
        try {
            return (await response.json()) as unknown;
        } catch (cause) {
            // 200 but not JSON (an HTML page from a non-engine server) — same "not the engine" signal.
            throw new NativeApiUnavailableError(NATIVE_UNAVAILABLE_MESSAGE, {cause});
        }
    }

    balanceSheet(query: BalanceSheetQuery = {}): Promise<unknown> {
        return this.getJson(`/api/reports/balancesheet${queryString({asOf: query.asOf, depth: query.depth})}`);
    }

    incomeStatement(query: IncomeStatementQuery = {}): Promise<unknown> {
        return this.getJson(`/api/reports/incomestatement${queryString({from: query.from, to: query.to, depth: query.depth})}`);
    }

    cashFlow(query: CashFlowQuery = {}): Promise<unknown> {
        return this.getJson(`/api/reports/cashflow${queryString({end: query.end, interval: query.interval, count: query.count, depth: query.depth})}`);
    }

    netWorth(query: NetWorthQuery = {}): Promise<unknown> {
        return this.getJson(
            `/api/reports/networth${queryString({end: query.end, interval: query.interval, count: query.count, depth: query.depth, valueIn: query.valueIn})}`
        );
    }

    holdings(query: HoldingsQuery = {}): Promise<unknown> {
        return this.getJson(`/api/holdings${queryString({asOf: query.asOf, accounts: query.accounts, mode: query.mode, gainSince: query.gainSince})}`);
    }

    holdingsSeries(query: HoldingsSeriesQuery = {}): Promise<unknown> {
        return this.getJson(
            `/api/holdings/series${queryString({asOf: query.asOf, accounts: query.accounts, mode: query.mode, interval: query.interval, count: query.count})}`
        );
    }
}
