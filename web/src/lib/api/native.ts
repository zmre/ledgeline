// ledgeline-engine native (/api/*) HTTP client. Fetch + query-string building +
// error taxonomy only; wire-shape knowledge lives in nativeDecode.ts. Mirrors
// HledgerApi (the wire client) but distinguishes a server that simply lacks the
// native routes (a plain hledger-web) from one that is unreachable.

import {ApiUnreachableError, authHeaders} from "./client";

/** The configured server answered, but has no /api/* routes (e.g. plain hledger-web). */
export class NativeApiUnavailableError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "NativeApiUnavailableError";
    }
}

/** User-facing copy for the missing-engine case (a 404 or non-JSON on /api/*). */
export const NATIVE_UNAVAILABLE_MESSAGE = "This server doesn't provide Ledgeline's report API — start the Ledgeline engine.";

// ---------------------------------------------------------------------------
// Write-path (edit) error taxonomy. The write endpoints answer with PLAIN-TEXT
// error bodies (unlike the JSON reports), so each of these carries the server's
// human message verbatim for the UI to surface.
// ---------------------------------------------------------------------------

/** 409 — the journal file changed on disk under us; the client must refetch and retry. */
export class ConflictError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "ConflictError";
    }
}

/** 400 — the edit was rejected (unbalanced, unparseable, round-trip mismatch); message is the server's. */
export class ValidationError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "ValidationError";
    }
}

/** 404 — the target transaction no longer exists (its index moved or it was deleted). */
export class NotFoundError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "NotFoundError";
    }
}

// ---------------------------------------------------------------------------
// Write-path wire types (native, camelCase). `Dec` is string-mantissa encoded
// exactly like the report endpoints (see nativeDecode.ts) so a large computed
// value never loses precision through a JS number.
// ---------------------------------------------------------------------------

/** An exact decimal on the wire: value = mantissa / 10^places (string mantissa). */
export interface WireDec {
    mantissa: string;
    places: number;
}

export type WireStatus = "cleared" | "pending" | "unmarked";
export type WireCostKind = "unit" | "total";
export type InsertPosition = "append" | "dateOrdered";

/** A `@`/`@@` cost annotation on a posting amount. */
export interface WireCost {
    kind: WireCostKind;
    amount: {commodity: string; quantity: WireDec};
}

/** A single-commodity posting amount, optionally priced with a cost. */
export interface WirePostingAmount {
    commodity: string;
    quantity: WireDec;
    cost?: WireCost;
}

/** Real / unbalanced-virtual `(a)` / balanced-virtual `[a]` on the write wire. */
export type WirePostingType = "regular" | "virtual" | "balancedVirtual";

/** A `=`/`==`/`=*`/`==*` balance assertion: `total` is `==`, `inclusive` is `=*`. */
export interface WireBalanceAssertion {
    amount: {commodity: string; quantity: WireDec};
    inclusive: boolean;
    total: boolean;
}

/**
 * One posting: an account and an OPTIONAL amount — no `amount` marks the elided/inferred leg.
 *
 * `type` and `balanceAssertion` are optional, but omitting them means the
 * posting HAS neither — the engine writes exactly what it is sent. A
 * read-modify-write (the edit popup's PUT) must therefore echo both back or it
 * destroys them, which is what DL-2 was.
 */
export interface WirePostingInput {
    account: string;
    status?: WireStatus;
    comment?: string;
    amount?: WirePostingAmount;
    type?: WirePostingType;
    balanceAssertion?: WireBalanceAssertion;
}

/** `POST /api/transactions` (ADD) / `PUT /api/transactions/{index}` (REPLACE) request body. */
export interface AddTransactionBody {
    date: string;
    date2?: string;
    status?: WireStatus;
    code?: string;
    description?: string;
    comment?: string;
    position?: InsertPosition;
    postings: WirePostingInput[];
}

/** REPLACE uses the identical whole-transaction body shape as ADD. */
export type ReplaceTransactionBody = AddTransactionBody;

/** One surgical posting edit for PATCH: `index` is the 0-based posting position within the transaction. */
export interface PatchPostingEdit {
    index: number;
    account: string;
}

/** `PATCH /api/transactions/{index}` (SURGICAL) body — send only the field(s) that changed. */
export interface PatchTransactionBody {
    description?: string;
    status?: WireStatus;
    postings?: PatchPostingEdit[];
}

/** The transaction as it landed in the journal after the reparse (native response shape). */
export interface WireTransaction {
    index: number;
    date: string;
    date2?: string;
    status: WireStatus;
    code: string;
    description: string;
    postings: {account: string; amounts: WirePostingAmount[]; status: string; type: string; balanceAssertion?: WireBalanceAssertion}[];
}

/** 201 (ADD) / 200 (REPLACE, PATCH) response: the resulting transaction + its (re)assigned index. */
export interface MutationResult {
    index: number;
    transaction: WireTransaction;
}

/** `DELETE /api/transactions/{index}` 200 response. */
export interface DeleteResult {
    deletedIndex: number;
    remaining: number;
}

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
export interface BudgetQuery {
    end?: string;
    interval?: string;
    count?: number;
    depth?: number;
    /** Case-insensitive periodic-rule description filter; absent/empty = all rules. */
    budgetDesc?: string;
}
export interface InsightsQuery {
    /** Inclusive comparison-span start (YYYY-MM-DD). */
    start?: string;
    /** Inclusive comparison-span end (YYYY-MM-DD). */
    end?: string;
    /** Comma-separated account prefixes excluded from cost of living; absent = server default. */
    exclude?: string;
}
export interface SubscriptionsQuery {
    asOf?: string;
    /** Months of history to scan (default 24). */
    lookback?: number;
    /** Charges needed before a monthly cadence is believed (default 5). */
    minMonthly?: number;
    /** Charges needed before an annual cadence is believed (default 2). */
    minAnnual?: number;
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
            response = await fetch(url, {headers: authHeaders({Accept: "application/json"}), cache: "no-store"});
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

    /**
     * Engine-computed journal diagnostics: `{"diagnostics": [...]}`, every
     * unbalanced transaction and failed balance assertion.
     *
     * Its own route rather than a field on `/transactions` because that endpoint
     * is a byte-for-byte hledger-web emulation whose parity comparator rejects
     * any unexpected key. A plain hledger-web 404s here, which the caller reads
     * as "no diagnostics".
     */
    diagnostics(): Promise<unknown> {
        return this.getJson("/api/diagnostics");
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

    /** Budget report (actuals vs. periodic-rule goals). Note: top-level /api/budget, NOT under /api/reports/. */
    budget(query: BudgetQuery = {}): Promise<unknown> {
        return this.getJson(
            `/api/budget${queryString({end: query.end, interval: query.interval, count: query.count, depth: query.depth, budgetDesc: query.budgetDesc})}`
        );
    }

    /** Insights dashboard (period-over-period core metrics). */
    insights(query: InsightsQuery = {}): Promise<unknown> {
        return this.getJson(`/api/insights${queryString({start: query.start, end: query.end, exclude: query.exclude})}`);
    }

    /** Recurring monthly/annual charges inferred from expense history. */
    subscriptions(query: SubscriptionsQuery = {}): Promise<unknown> {
        return this.getJson(
            `/api/subscriptions${queryString({asOf: query.asOf, lookback: query.lookback, minMonthly: query.minMonthly, minAnnual: query.minAnnual})}`
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

    // -----------------------------------------------------------------------
    // Write path (edit endpoints). Success bodies are JSON; error bodies are
    // plain text, so `mutate` reads the body ONCE as text and either JSON-parses
    // it (on the expected status) or maps the status → the edit error taxonomy.
    // -----------------------------------------------------------------------

    /** ADD a whole transaction. → 201 `{index, transaction}`. */
    addTransaction(body: AddTransactionBody): Promise<MutationResult> {
        return this.mutate<MutationResult>("POST", "/api/transactions", 201, body);
    }

    /** REPLACE the whole transaction at `index`. → 200 `{index, transaction}`. */
    replaceTransaction(index: number, body: ReplaceTransactionBody): Promise<MutationResult> {
        return this.mutate<MutationResult>("PUT", `/api/transactions/${index}`, 200, body);
    }

    /** SURGICAL partial edit of `index` (send only changed fields). → 200 `{index, transaction}`. */
    patchTransaction(index: number, patch: PatchTransactionBody): Promise<MutationResult> {
        return this.mutate<MutationResult>("PATCH", `/api/transactions/${index}`, 200, patch);
    }

    /** DELETE the transaction at `index`. → 200 `{deletedIndex, remaining}`. */
    deleteTransaction(index: number): Promise<DeleteResult> {
        return this.mutate<DeleteResult>("DELETE", `/api/transactions/${index}`, 200);
    }

    /**
     * Cheap capability probe for the write path: the engine registers the
     * mutating verbs (POST/PUT/PATCH/DELETE) on `/api/transactions` but not GET,
     * so a GET yields 405 (route present ⇒ editing available). A plain
     * hledger-web / SPA fallback has no such route ⇒ 404 (not available). Any
     * other reachable status still means the route exists, so we treat it as
     * available; an unreachable server rejects (the caller degrades to no-edit).
     */
    async probeEditing(): Promise<boolean> {
        const url = `${this.baseUrl}/api/transactions`;
        let response: Response;
        try {
            response = await fetch(url, {method: "GET", headers: authHeaders({Accept: "application/json"}), cache: "no-store"});
        } catch (cause) {
            throw new ApiUnreachableError(`Cannot reach the Ledgeline engine at ${this.baseUrl} (network or CORS failure)`, {cause});
        }
        return response.status !== 404;
    }

    /**
     * Issue a write request and map the response: JSON-decode the body on
     * `okStatus`, else translate the HTTP status into the edit error taxonomy
     * carrying the server's plain-text message.
     */
    private async mutate<T>(method: string, route: string, okStatus: number, body?: unknown): Promise<T> {
        const url = `${this.baseUrl}${route}`;
        const headers = authHeaders(body === undefined ? {Accept: "application/json"} : {Accept: "application/json", "Content-Type": "application/json"});
        let response: Response;
        try {
            response = await fetch(url, {method, headers, body: body === undefined ? undefined : JSON.stringify(body), cache: "no-store"});
        } catch (cause) {
            throw new ApiUnreachableError(`Cannot reach the Ledgeline engine at ${this.baseUrl} (network or CORS failure)`, {cause});
        }
        const text = await response.text();
        if (response.status === okStatus) {
            try {
                return JSON.parse(text) as T;
            } catch (cause) {
                // Expected status but an unparseable body — a non-engine server answering the route.
                throw new NativeApiUnavailableError(NATIVE_UNAVAILABLE_MESSAGE, {cause});
            }
        }
        const message = text.trim();
        switch (response.status) {
            case 400:
                throw new ValidationError(message || "The transaction is invalid.");
            case 404:
                throw new NotFoundError(message || "That transaction no longer exists — refresh the journal.");
            case 409:
                throw new ConflictError(message || "The journal changed on disk — refresh and try again.");
            case 501:
                throw new NativeApiUnavailableError(message || "Editing is not enabled on this server.");
            default:
                throw new ApiUnreachableError(`${method} ${url} responded ${response.status} ${response.statusText}`);
        }
    }
}
