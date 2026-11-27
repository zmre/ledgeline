// `GET /api/time/summary` (plan 46 T-dash, `app/time.rs`): time grouped by
// any of account, branch, customer, person, billable, billed, note or a
// period bucket, over the same filters as Entries (from, to, person,
// customer, billable, billed, account, q), with an optional depth cut, top N (the
// rest summed as `other`) and per-row period series. `view=tree` is
// `hledger bal --tree`'s rows instead; the dashboard builds its tree from
// grouped rows, so it isn't wrapped here.
import {send} from "#lib/editorApi.js";

/** A duration, exactly: whole seconds, hours as plain decimal text, and `1h 05m` for people. */
export interface Qty {
    seconds: number;
    hours: string;
    text: string;
}

/** One summary row: its key (one part per grouping), time and entry count. */
export interface SummaryRow {
    key: string[];
    quantity: Qty;
    entries: number;
    /** With `series`: `[bucket, quantity]` for every bucket of `Summary.buckets`, in order. */
    series?: [string, Qty][];
}

export interface Summary {
    /** Largest first with `top`, else by key (accounts in tree order). */
    rows: SummaryRow[];
    /** The rows past `top`, summed (key `["other"]`). */
    other: SummaryRow | null;
    total: Qty;
    /** The series' buckets (`2026-11-26`, `2026-W48`, `2026-11`), in order. */
    buckets: string[];
    /** The roll-up journal, relative to the books (`time.journal`): for `hledger -f`. */
    file: string;
}

export const SUMMARY_GROUPS = ["account", "branch", "customer", "person", "billable", "billed", "note", "day", "week", "month", "quarter", "year"] as const;
export type SummaryGroup = (typeof SUMMARY_GROUPS)[number];

export type SummaryInterval = "day" | "week" | "month" | "quarter" | "year";

/** What to ask for. Absent filters are "everything". */
export interface SummaryParams {
    /** First date, inclusive. */
    from?: string;
    /** Last date, inclusive. */
    to?: string;
    /** One person; `""` is unattributed time. */
    person?: string;
    /** One customer (the `customer:` tag's value). */
    customer?: string;
    billable?: boolean;
    /** Entries' billing state: `unbilled`, `invoiced`, `nobill`, `not-billable`, `billed`. */
    billed?: string;
    /** This account or below. */
    account?: string;
    /** Text in the note or account. */
    q?: string;
    group: readonly SummaryGroup[];
    depth?: number;
    top?: number;
    series?: SummaryInterval;
}

/** The request's query string, without the `?`, in a fixed order. */
export function summaryQueryString(params: SummaryParams): string {
    const out = new URLSearchParams();
    if (params.from !== undefined) out.set("from", params.from);
    if (params.to !== undefined) out.set("to", params.to);
    if (params.person !== undefined) out.set("person", params.person);
    if (params.customer !== undefined) out.set("customer", params.customer);
    if (params.billable !== undefined) out.set("billable", String(params.billable));
    if (params.billed !== undefined) out.set("billed", params.billed);
    if (params.account !== undefined) out.set("account", params.account);
    if (params.q !== undefined) out.set("q", params.q);
    if (params.group.length > 0) out.set("group", params.group.join(","));
    if (params.depth !== undefined) out.set("depth", String(params.depth));
    if (params.top !== undefined) out.set("top", String(params.top));
    if (params.series !== undefined) out.set("series", params.series);
    return out.toString();
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

export function isQty(v: unknown): v is Qty {
    return isRecord(v) && typeof v.seconds === "number" && typeof v.hours === "string" && typeof v.text === "string";
}

function isRow(v: unknown): v is SummaryRow {
    if (!isRecord(v)) return false;
    if (!Array.isArray(v.key) || !v.key.every((k) => typeof k === "string")) return false;
    if (!isQty(v.quantity) || typeof v.entries !== "number") return false;
    if (v.series === undefined) return true;
    return Array.isArray(v.series) && v.series.every((s) => Array.isArray(s) && s.length === 2 && typeof s[0] === "string" && isQty(s[1]));
}

/** Whether `body` is a summary (and not the `view=tree` shape). */
export function isSummary(body: unknown): body is Summary {
    return (
        isRecord(body) &&
        Array.isArray(body.rows) &&
        body.rows.every(isRow) &&
        (body.other === null || isRow(body.other)) &&
        isQty(body.total) &&
        Array.isArray(body.buckets) &&
        body.buckets.every((b) => typeof b === "string") &&
        typeof body.file === "string"
    );
}

/** `GET /api/time/summary`. A refusal (`not-set-up`, a bad grouping) is an `EditorApiError` with the server's code. */
export function fetchSummary(params: SummaryParams): Promise<Summary> {
    return send<Summary>("GET", `/api/time/summary?${summaryQueryString(params)}`, isSummary);
}
