// ledgeline-engine native (/api/*) HTTP client. Fetch + query-string building +
// error taxonomy only; wire-shape knowledge lives in nativeDecode.ts. Mirrors
// HledgerApi (the wire client) but distinguishes a server that simply lacks the
// native routes (a plain hledger-web) from one that is unreachable.

import {ApiUnreachableError, authHeaders, REQUEST_TIMEOUT_MS, withDeadline} from "./client";

/** The configured server answered, but has no /api/* routes (e.g. plain hledger-web). */
export class NativeApiUnavailableError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "NativeApiUnavailableError";
    }
}

/** User-facing copy for the missing-engine case (a 404 or non-JSON on /api/*). */
export const NATIVE_UNAVAILABLE_MESSAGE = "This server doesn't provide Ledgeline's report API — start the Ledgeline engine.";

/**
 * 400 on a READ — the engine answered and REFUSED to compute the report,
 * naming a journal-authoring mistake (an unknown `holdings:` or `issection:`
 * tag value, and their relatives). The read path's counterpart of the write
 * path's [`ValidationError`]: the message is the server's own sentence, and
 * the remedy is editing the journal — never the connection, which is why this
 * is deliberately NOT an [`ApiUnreachableError`] subclass. Before it existed,
 * every consumer that branches on class read a journal typo as network
 * trouble (editFailure.ts mapped it to kind "network").
 */
export class EngineRefusalError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "EngineRefusalError";
    }
}

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

// ---------------------------------------------------------------------------
// CSV import rules write path (`PUT /api/rules/{*id}`).
//
// The body is the COMPLETE intended shape of the document, and the engine
// refuses a plan that does not account for every item it handed out — so an
// omitted item is a 400, never a silent truncation. No variant carries raw text:
// a client can only name typed content the engine's own renderers produce, or
// name an id whose bytes were already read from that file. That is why `trivia`,
// `include`, `opaque` and the `source`/`archive` directives have no variant here
// at all — `keep` is the only form the engine accepts for them.
// ---------------------------------------------------------------------------

/** Emit an existing item's bytes unchanged. Moving one is just listing it somewhere else. */
export interface KeepRulesItem {
    kind: "keep";
    id: number;
}

/** `id` present ⇒ rewrite that item in place; absent ⇒ insert a new one. */
export interface DirectiveRulesItem {
    kind: "directive";
    id?: number;
    name: string;
    value: string;
}

export interface FieldsRulesItem {
    kind: "fields";
    id?: number;
    names: string[];
}

export interface AssignmentRulesItem {
    kind: "assignment";
    id?: number;
    field: string;
    value: string;
}

/** A matcher on the write wire: an absent `field` is a whole-record match. */
export interface RulesMatcherInput {
    field?: string;
    pattern: string;
}

export interface IfBlockRulesItem {
    kind: "ifBlock";
    id?: number;
    matchers: RulesMatcherInput[];
    assignments: {field: string; value: string}[];
}

export type SaveRulesItem = KeepRulesItem | DirectiveRulesItem | FieldsRulesItem | AssignmentRulesItem | IfBlockRulesItem;

/** `PUT /api/rules/{*id}` body. `deny_unknown_fields` server-side: a typo'd key is a 400, not a no-op. */
export interface SaveRulesBody {
    /** The revision the document was read with. A mismatch is a 409 and nothing is written. */
    revision: string;
    items: SaveRulesItem[];
    /** Items dropped on purpose. Omitting an item is NEVER an implicit delete. */
    delete: number[];
}

/**
 * Percent-encode a rules id for a URL path, SEGMENT BY SEGMENT.
 *
 * The route is a greedy `{*id}` wildcard and a real id contains slashes
 * (`import/2026/bank.csv.rules`), so the separators must survive as separators —
 * `encodeURIComponent` over the whole string would turn them into `%2F` and the
 * server would resolve a name no scan ever produced. Encoding each component
 * still escapes a space or a `#` inside one file name, which the id may contain.
 */
function encodeRulesId(id: string): string {
    return id.split("/").map(encodeURIComponent).join("/");
}

// ---------------------------------------------------------------------------
// New Transactions import flow (`/api/import/*`, `/api/prefs`).
//
// `stage` is the SPA's first upload: raw bytes, not JSON. Everything else on
// this route family is an ordinary JSON mutation and goes through `mutate`.
// ---------------------------------------------------------------------------

/**
 * The engine's `MAX_UPLOAD_BYTES`, enforced by a `DefaultBodyLimit` on the stage
 * route alone.
 *
 * Duplicated here on purpose: refusing a 40 MiB workbook before spending a
 * minute uploading it is worth one constant, and the server still enforces it —
 * this is a courtesy, never the check.
 */
export const MAX_UPLOAD_BYTES = 16 * 1024 * 1024;

/**
 * The dry-run request body, and the base of the commit body.
 *
 * **`rulesId` and `journalId` are NOT nullable.** The Save-CSV-only path — no
 * rules file fits, keep the converted CSV anyway — is `ImportSaveCsvBody` on its
 * own route, because a dry-run with no rules file has nothing to propose and
 * nothing to reconcile. Nullable handles here would encode a request the engine
 * cannot answer and put a null check in front of every use of either field; the
 * engine's `WireDryRunRequest` says the same in Rust.
 *
 * `balance`/`balanceAccount` are decimal TEXT and an account name — never
 * numbers (convention #1).
 */
export interface ImportRunBody {
    stageId: string;
    rulesId: string;
    csvPath: string;
    journalId: string;
    balance: string | null;
    balanceAccount: string | null;
}

/** `POST /api/import/commit` — the dry-run body plus whether to write the balance assertion. */
export interface ImportCommitBody extends ImportRunBody {
    writeAssertion: boolean;
}

/**
 * `POST /api/import/save-csv` — keep the converted CSV, import nothing.
 *
 * Two fields, and that is the whole request: there is no rules file, so there is
 * no journal, no balance and no assertion. The response is
 * `{csvWritten, git}` and decodes through `decodeCommitResult`, whose absent
 * `journalWritten` already means "no journal was touched".
 */
export interface ImportSaveCsvBody {
    stageId: string;
    csvPath: string;
}

/**
 * One change to one `alias` line, tagged by `kind`.
 *
 * There is no `move`: aliases are POSITIONAL, so reordering them is a semantic
 * change dressed as a cosmetic one, and the engine does not offer it either.
 * `append` carries no position because the end of the file is the only place an
 * alias can be inserted without changing what anything above it means.
 */
export type SaveAliasEdit =
    | {kind: "replace"; index: number; pattern: string; replacement: string; regex: boolean}
    | {kind: "delete"; index: number}
    | {kind: "append"; pattern: string; replacement: string; regex: boolean};

/**
 * `PUT /api/aliases/{*journalId}`.
 *
 * Unlike a rules-file save, omitting a line is NOT a delete: this request cannot
 * reorder and cannot delete by omission, so naming one line changes one line and
 * an empty `edits` writes nothing at all.
 */
export interface SaveAliasesBody {
    revision: string;
    edits: SaveAliasEdit[];
}

/** `PUT /api/prefs`. Both fields are tri-state: null means "unset", not "off". */
export interface PrefsBody {
    hledgerPath: string | null;
    gitAutocommit: boolean | null;
}

/**
 * Longest error body worth showing a user. The engine's are single sentences
 * (~90 characters at their longest); anything past this is a document, not a
 * message.
 */
const MAX_ERROR_BODY_CHARS = 500;

/**
 * The sentence to report for a failed READ, preferring the engine's own body.
 *
 * The read path used to throw `GET …/api/holdings/other?… responded 400 Bad
 * Request` and drop the body unread, which was fine while every 4xx was a
 * malformed query the SPA itself had built — nobody could act on those, so there
 * was nothing to lose. It stopped being fine when the engine started reporting
 * JOURNAL-authoring mistakes this way: an unknown `holdings:` (or `issection:`)
 * tag value answers `400` with "account 'assets:x' declares `holdings: y`, which
 * is not one of stocks, other, none" — a sentence that names the account, the bad
 * value and the fix, and which the status line replaces with nothing.
 *
 * The write path (`send`) has always preferred the body for the same reason, in
 * its own words. Both paths share `usableErrorBody`'s guards, so neither can
 * drift into showing a proxy's error page.
 */
async function readErrorBody(response: Response, request: string): Promise<string> {
    const fallback = `${request} responded ${response.status} ${response.statusText}`;
    let body: string;
    try {
        body = await response.text();
    } catch {
        return fallback; // a body that cannot be read is not a message
    }
    const message = usableErrorBody(body);
    return message === "" ? fallback : message;
}

/**
 * The server's own sentence from an error body, or `""` when the body is not a
 * message at all — the caller falls back to its own line (the status line, or
 * a mapped status's friendly sentence).
 *
 * Two guards, both REJECTING rather than truncating. A non-engine server (a
 * proxy, a dev server) answers errors with an HTML DOCUMENT, and a page of
 * markup in an alert box or an edit-failure toast is strictly worse than the
 * status line; so is a body long enough to push the Retry button off screen.
 * Half a sentence from an unknown source is not more useful than "responded
 * 502", so neither case is trimmed into service.
 */
function usableErrorBody(body: string): string {
    const trimmed = body.trim();
    if (trimmed === "" || trimmed.length > MAX_ERROR_BODY_CHARS || trimmed.startsWith("<")) return "";
    return trimmed;
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
/**
 * The grouped balance sheet's query (plans/12). `value`/`valueIn` are optional
 * because the engine's defaults — market, valued in `prices.base_commodity()` —
 * are exactly what the screen wants, and there is no control for either; sending
 * them anyway would pin the SPA to a base commodity it had to guess.
 */
export interface GroupedBalanceSheetQuery {
    asOf?: string;
    /**
     * Account-depth clamp for the rows inside an expanded group. OMIT IT for no
     * clamp — that is the endpoint's contract, and `0` cannot express it because
     * `depth=0` already means hledger's totals-only. The reports page omits it.
     */
    depth?: number;
    value?: "market" | "cost" | "none";
    valueIn?: string;
}
export interface IncomeStatementQuery {
    from?: string;
    to?: string;
    depth?: number;
}
/**
 * The grouped income statement's query (plans/13). Note what is NOT here:
 * `depth`. This report has no depth control and the endpoint takes no such
 * param — groups are the reading, and the accounts inside one are a drill-down.
 *
 * `value`/`valueIn`/`compare` are optional for the same reason they are on the
 * grouped balance sheet: the engine's defaults (market, valued in
 * `prices.base_commodity()`, comparing against the previous equal-length window)
 * are exactly what the screen wants, and there is no control for any of them.
 * Sending them anyway would pin the SPA to a base commodity it had to guess.
 */
export interface GroupedIncomeStatementQuery {
    from?: string;
    to?: string;
    value?: "market" | "cost" | "none";
    valueIn?: string;
    compare?: "previous" | "none";
}
/**
 * The flow graphs' query (the Sankey diagrams above the P&L's boxes). Note what
 * is NOT here: `value` and `compare`. A link's width is one number, so the
 * graphs are always market-valued, and neither has a comparison column.
 *
 * `valueIn` is optional for the reason it is on the statement: the engine
 * defaults to `prices.base_commodity()`, which is exactly what the screen wants,
 * and sending one would pin the SPA to a base commodity it had to guess.
 */
export interface IncomeStatementFlowsQuery {
    from?: string;
    to?: string;
    valueIn?: string;
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
/**
 * The Other-holdings query (plans/14): the stock query's params, verbatim, plus
 * `valueIn`.
 *
 * Same scope, same `mode`, same `gainSince` window — deliberately, because the
 * scope bar drives both tabs and a window control that meant two things would be
 * a lie. `valueIn` is optional for the reason the grouped statements give: the
 * engine's default (value in `prices.base_commodity()`) is exactly what the
 * screen wants and there is no control for it, so sending one would pin the SPA
 * to a base commodity it had to guess.
 */
export interface OtherHoldingsQuery extends HoldingsQuery {
    valueIn?: string;
}
export interface OtherHoldingsSeriesQuery extends HoldingsSeriesQuery {
    valueIn?: string;
}

/** Cancellation + deadline for one client's requests; see `REQUEST_TIMEOUT_MS`. */
export interface LedgelineApiOptions {
    signal?: AbortSignal;
    timeoutMs?: number;
}

export class LedgelineApi {
    readonly baseUrl: string;
    private readonly signal?: AbortSignal;
    private readonly timeoutMs: number;

    constructor(baseUrl: string, options?: LedgelineApiOptions) {
        this.baseUrl = baseUrl.replace(/\/+$/, "");
        this.signal = options?.signal;
        this.timeoutMs = options?.timeoutMs ?? REQUEST_TIMEOUT_MS;
    }

    /** GET a native route, returning raw unknown JSON; pass through a nativeDecode.* decoder separately. */
    private async getJson(route: string): Promise<unknown> {
        const url = `${this.baseUrl}${route}`;
        return withDeadline(`GET ${url}`, this.timeoutMs, this.signal, async (signal) => {
            let response: Response;
            try {
                // no-store: report data must always come from the live engine, never the HTTP cache
                response = await fetch(url, {headers: authHeaders({Accept: "application/json"}), cache: "no-store", signal});
            } catch (cause) {
                throw new ApiUnreachableError(`Cannot reach the Ledgeline engine at ${this.baseUrl} (network or CORS failure)`, {cause});
            }
            // A server without the native routes (plain hledger-web) 404s here.
            if (response.status === 404) {
                throw new NativeApiUnavailableError(NATIVE_UNAVAILABLE_MESSAGE);
            }
            if (!response.ok) {
                const message = await readErrorBody(response, `GET ${url}`);
                // The write path types its 400s as ValidationError; this is the
                // same fact on a read — the engine is answering fine, and the
                // JOURNAL is what needs fixing — so it must not wear the class
                // every consumer reads as "check your connection". Everything
                // else non-OK (401, 5xx, a proxy) still does mean the engine
                // cannot usefully be reached from here.
                if (response.status === 400) {
                    throw new EngineRefusalError(message);
                }
                throw new ApiUnreachableError(message);
            }
            try {
                return (await response.json()) as unknown;
            } catch (cause) {
                // 200 but not JSON (an HTML page from a non-engine server) — same "not the engine" signal.
                throw new NativeApiUnavailableError(NATIVE_UNAVAILABLE_MESSAGE, {cause});
            }
        });
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

    /**
     * WHICH journal this engine has open: `{"title": …, "file": …}`, both
     * nullable (decode with `decodeJournalInfo`).
     *
     * `title` is the engine's own derivation — the journal's first-line comment,
     * else the containing folder's name — and `file` is the BARE filename of the
     * main journal file, never a path. The app bar shows the first and hovers the
     * second, so somebody keeping several entities can see which set of books is
     * on screen without reading the port number.
     *
     * Its own route rather than a field on `/transactions`, for the reason
     * `diagnostics` has one: that endpoint is a byte-for-byte hledger-web
     * emulation whose parity comparator rejects any unexpected key. A plain
     * hledger-web therefore 404s here, which the caller reads as "this server
     * cannot tell me which ledger it is" and answers by showing no label.
     */
    journalInfo(): Promise<unknown> {
        return this.getJson("/api/journal");
    }

    /**
     * The flat, unvalued balance sheet. Still here, and still exercised by the
     * hledger parity golden — the screen no longer uses it, but
     * `fixtures/native/v1/balancesheet.json` must stay byte-identical.
     */
    balanceSheet(query: BalanceSheetQuery = {}): Promise<unknown> {
        return this.getJson(`/api/reports/balancesheet${queryString({asOf: query.asOf, depth: query.depth})}`);
    }

    /** The grouped, market-valued balance sheet the Balance Sheet tab renders (decode with `decodeBalanceSheetReport`). */
    balanceSheetGrouped(query: GroupedBalanceSheetQuery = {}): Promise<unknown> {
        return this.getJson(
            `/api/reports/balancesheet/grouped${queryString({asOf: query.asOf, depth: query.depth, value: query.value, valueIn: query.valueIn})}`
        );
    }

    /**
     * The flat, unvalued income statement. Still here, and still exercised by
     * the hledger parity golden — the screen no longer uses it, but
     * `fixtures/native/v1/incomestatement.json` must stay byte-identical.
     */
    incomeStatement(query: IncomeStatementQuery = {}): Promise<unknown> {
        return this.getJson(`/api/reports/incomestatement${queryString({from: query.from, to: query.to, depth: query.depth})}`);
    }

    /** The grouped, market-valued income statement the P&L tab renders (decode with `decodeIncomeStatementReport`). */
    incomeStatementGrouped(query: GroupedIncomeStatementQuery = {}): Promise<unknown> {
        return this.getJson(
            `/api/reports/incomestatement/grouped${queryString({from: query.from, to: query.to, value: query.value, valueIn: query.valueIn, compare: query.compare})}`
        );
    }

    /** The two money-flow graphs the P&L tab draws above its boxes (decode with `decodeFlowReport`). */
    incomeStatementFlows(query: IncomeStatementFlowsQuery = {}): Promise<unknown> {
        return this.getJson(`/api/reports/incomestatement/flows${queryString({from: query.from, to: query.to, valueIn: query.valueIn})}`);
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

    /** Non-stock, non-cash assets — a house, a van, a partnership (decode with `decodeOtherHoldingsReport`). */
    otherHoldings(query: OtherHoldingsQuery = {}): Promise<unknown> {
        return this.getJson(
            `/api/holdings/other${queryString({asOf: query.asOf, accounts: query.accounts, mode: query.mode, gainSince: query.gainSince, valueIn: query.valueIn})}`
        );
    }

    /**
     * The Other tab's value-over-time series. The response is the same
     * `WireHoldingsSeries` the stock series returns, byte for byte, so it decodes
     * with `decodeHoldingsSeries` and renders through `HoldingsTrend` with no new
     * chart code — the whole reason the engine reuses the type.
     *
     * `gainSince` is not sent, for the reason `holdingsSeries` does not send it:
     * the series is always all-time, and only the report's change column is
     * windowed.
     */
    otherHoldingsSeries(query: OtherHoldingsSeriesQuery = {}): Promise<unknown> {
        return this.getJson(
            `/api/holdings/other/series${queryString({asOf: query.asOf, accounts: query.accounts, mode: query.mode, interval: query.interval, count: query.count, valueIn: query.valueIn})}`
        );
    }

    /** Every `*.rules` file beside the open journal, summarized (the imports file list). */
    listRules(): Promise<unknown> {
        return this.getJson("/api/rules");
    }

    /** One parsed rules document, item by item. */
    getRules(id: string): Promise<unknown> {
        return this.getJson(`/api/rules/${encodeRulesId(id)}`);
    }

    /**
     * The first few rows of the data file a rules file describes.
     *
     * A SIBLING prefix, not `/api/rules/{id}/preview`: axum 0.8 refuses to
     * register a suffix after a greedy wildcard, and the id genuinely contains
     * slashes. Same string, same validation, same resolution.
     */
    previewRules(id: string): Promise<unknown> {
        return this.getJson(`/api/rules-preview/${encodeRulesId(id)}`);
    }

    // -----------------------------------------------------------------------
    // Write path (edit endpoints). Success bodies are JSON; error bodies are
    // plain text, so `mutate` reads the body ONCE as text and either JSON-parses
    // it (on the expected status) or maps the status → the edit error taxonomy.
    // -----------------------------------------------------------------------

    /** Save a whole rules document. → 200, the saved document (decode with `decodeRulesDoc`). */
    saveRules(id: string, body: SaveRulesBody): Promise<unknown> {
        return this.mutate<unknown>("PUT", `/api/rules/${encodeRulesId(id)}`, 200, body);
    }

    /** Every `alias` the journal declares (decode with `decodeAliasListing`). */
    listAliases(): Promise<unknown> {
        return this.getJson("/api/aliases");
    }

    /**
     * Rewrite one journal file's alias lines. → 200, that file at its new
     * revision (decode with `decodeAliasFileResponse`); 409 when the file moved
     * underneath the editor.
     *
     * The id is encoded segment by segment for the same reason a rules id is:
     * it genuinely contains slashes (`2026/2026.journal`) and they must survive
     * as path separators rather than as `%2F`.
     */
    saveAliases(journalId: string, body: SaveAliasesBody): Promise<unknown> {
        return this.mutate<unknown>("PUT", `/api/aliases/${encodeRulesId(journalId)}`, 200, body);
    }

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
        return withDeadline(`GET ${url}`, this.timeoutMs, this.signal, async (signal) => {
            let response: Response;
            try {
                response = await fetch(url, {method: "GET", headers: authHeaders({Accept: "application/json"}), cache: "no-store", signal});
            } catch (cause) {
                throw new ApiUnreachableError(`Cannot reach the Ledgeline engine at ${this.baseUrl} (network or CORS failure)`, {cause});
            }
            return response.status !== 404;
        });
    }

    // -----------------------------------------------------------------------
    // New Transactions import flow. Every route here is registered ABOVE the
    // bearer-token layer's sibling routes and answers `Cache-Control: no-store`;
    // none of it derives from the journal snapshot, so none of it is ETagged.
    // -----------------------------------------------------------------------

    /** What the screen may offer at all: hledger's version, the formats, the journal targets, git. */
    importCapabilities(): Promise<unknown> {
        return this.getJson("/api/import/capabilities");
    }

    /**
     * Upload one statement file for conversion, preview and candidate scoring.
     *
     * `filename` must already be a BARE name — `importModel.headerFilename` is
     * what makes it one. The engine sanitises it again (it has to; a client is
     * not a check), and uses it only for format detection and the destination
     * default.
     */
    stageImport(filename: string, bytes: ArrayBuffer | Uint8Array): Promise<unknown> {
        return this.upload("/api/import/stage", filename, bytes);
    }

    /** Proposed transactions + balance reconciliation. `ok:false` is a 200 with hledger's stderr. */
    importDryRun(body: ImportRunBody): Promise<unknown> {
        return this.mutate<unknown>("POST", "/api/import/dry-run", 200, body);
    }

    /** Write the CSV, run the real import, report ordering and git. */
    importCommit(body: ImportCommitBody): Promise<unknown> {
        return this.mutate<unknown>("POST", "/api/import/commit", 200, body);
    }

    /**
     * Write the converted CSV and nothing else — the path taken when no rules
     * file fits the statement.
     *
     * Its own route rather than a commit with null handles: converting a file
     * nobody has rules for is still worth keeping, and none of the import
     * machinery (rules file, journal, dry-run, assertion) applies to it.
     */
    importSaveCsv(body: ImportSaveCsvBody): Promise<unknown> {
        return this.mutate<unknown>("POST", "/api/import/save-csv", 200, body);
    }

    /** The confirmed format-preserving re-sort, offered only after a commit reported `inOrder:false`. */
    importSort(journalId: string): Promise<unknown> {
        return this.mutate<unknown>("POST", "/api/import/sort", 200, {journalId});
    }

    /**
     * Install the journal's aliases into an `hledger.conf` beside it, so a
     * terminal `hledger import` maps the same accounts this screen does.
     *
     * **The body carries a revision and nothing else.** WHAT to write is
     * recomputed by the engine from the journal's own `alias` directives; a body
     * carrying the lines would make this a write-arbitrary-text primitive aimed
     * at a file hledger reads options out of. 409 when the config changed
     * underneath the page.
     */
    writeHledgerConf(revision: string): Promise<unknown> {
        return this.mutate<unknown>("POST", "/api/import/hledger-conf", 200, {revision});
    }

    /** The preferences store (the resolved hledger path, the git-autocommit opt-out). */
    getPrefs(): Promise<unknown> {
        return this.getJson("/api/prefs");
    }

    /** Replace the whole preferences blob. A bad `hledgerPath` is a 400, not a stored-and-fails-later. */
    putPrefs(body: PrefsBody): Promise<unknown> {
        return this.mutate<unknown>("PUT", "/api/prefs", 200, body);
    }

    /**
     * Issue a write request and map the response: JSON-decode the body on
     * `okStatus`, else translate the HTTP status into the edit error taxonomy
     * carrying the server's plain-text message.
     */
    private async mutate<T>(method: string, route: string, okStatus: number, body?: unknown): Promise<T> {
        const headers = authHeaders(body === undefined ? {Accept: "application/json"} : {Accept: "application/json", "Content-Type": "application/json"});
        return this.send<T>(method, route, okStatus, headers, body === undefined ? undefined : JSON.stringify(body));
    }

    /**
     * POST raw bytes with the source filename in a header — the ONE upload
     * primitive, and the reason it cannot go through `mutate`.
     *
     * `mutate` sends `JSON.stringify(body)`; a workbook is not JSON and
     * base64-ing 16 MiB through one would cost a third again in transfer and a
     * decode step on both sides. So this is its own path, sharing `send`'s
     * status → error-taxonomy mapping so the two cannot drift.
     *
     * The size check is local courtesy only: the engine's `DefaultBodyLimit`
     * answers 413 and that is the actual enforcement, mapped below.
     */
    private async upload<T>(route: string, filename: string, bytes: ArrayBuffer | Uint8Array): Promise<T> {
        const size = bytes.byteLength;
        if (size > MAX_UPLOAD_BYTES) {
            throw new ValidationError(`That file is ${Math.round(size / (1024 * 1024))} MB; Ledgeline accepts up to ${MAX_UPLOAD_BYTES / (1024 * 1024)} MB.`);
        }
        if (size === 0) throw new ValidationError("That file is empty.");
        const headers = authHeaders({
            Accept: "application/json",
            "Content-Type": "application/octet-stream",
            "X-Ledgeline-Filename": filename,
        });
        // A Uint8Array view is passed through as-is: BodyInit accepts it, and
        // copying to an ArrayBuffer would double the peak memory for a 16 MiB file.
        return this.send<T>("POST", route, 200, headers, bytes as BodyInit, {
            // 413 only reaches this route, and "your file is too big" is a
            // refusal the user can act on — the generic branch would report it
            // as an unreachable engine.
            413: (message) => new ValidationError(message || `That file is larger than the ${MAX_UPLOAD_BYTES / (1024 * 1024)} MB Ledgeline accepts.`),
        });
    }

    /**
     * The one fetch + status-mapping path both `mutate` and `upload` use.
     *
     * `extraStatuses` lets a caller name a status only its own route can produce
     * (413 for the upload) without every other caller inheriting a message that
     * makes no sense for it.
     */
    private async send<T>(
        method: string,
        route: string,
        okStatus: number,
        headers: Record<string, string>,
        body: BodyInit | undefined,
        extraStatuses: Record<number, (message: string) => Error> = {}
    ): Promise<T> {
        const url = `${this.baseUrl}${route}`;
        return withDeadline(`${method} ${url}`, this.timeoutMs, this.signal, async (signal) => {
            let response: Response;
            try {
                response = await fetch(url, {method, headers, body, cache: "no-store", signal});
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
            // The read path's guards (`usableErrorBody`), applied to the write
            // path too: a POST through a misbehaving proxy answers with an HTML
            // document or a whole error page, and either one used to land raw
            // in the edit-failure toast. Guarded to "", every branch below
            // falls back to its own sentence — the status line for the
            // unmapped default.
            // The read path's guards (`usableErrorBody`), applied to the write
            // path too: a POST through a misbehaving proxy answers with an HTML
            // document or a whole error page, and either one used to land raw
            // in the edit-failure toast. Guarded to "", every branch below
            // falls back to its own sentence — the status line for the
            // unmapped default.
            const message = usableErrorBody(text);
            const extra = extraStatuses[response.status];
            if (extra !== undefined) throw extra(message);
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
                    // Every unmapped status still carries the engine's own
                    // plain-text body, and on a 500 that body is the only thing
                    // that tells a user whether their file was touched (the
                    // engine's write path is ordered so that every check
                    // precedes the single atomic write, and says so: "nothing
                    // was written"). Replacing it with "responded 500 Internal
                    // Server Error" throws the one actionable half away.
                    throw new ApiUnreachableError(message || `${method} ${url} responded ${response.status} ${response.statusText}`);
            }
        });
    }
}
