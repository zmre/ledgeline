// The receivables dashboard (invoicer PLAN 5a): `GET /api/dashboard`'s shape
// (`invoicer_core::views::dashboard::Dashboard`) and the pure helpers the page
// draws it with.
//
// Every figure links to the invoice list, filtered by query parameters. The
// contract (PLAN 5a R5 and 5b S1; the list's parser is lib/listQuery.ts):
//
// | Parameter            | Meaning                                                              |
// |----------------------|----------------------------------------------------------------------|
// | `status=open`        | still owed (sent, not paid in full), overdue included                |
// | `status=overdue`     | still owed and past due on today                                     |
// | `status=draft`       | drafts                                                               |
// | `status=paid`        | paid in full                                                         |
// | `aged=current`       | still owed, not yet past due                                         |
// | `aged=<from>-<to>`   | still owed, `from` to `to` days past due, inclusive (`aged=31-60`)  |
// | `aged=<from>+`       | still owed, `from` or more days past due (`aged=91+`; `91-` reads too)|
// | `customer=<name>`    | billed to the customer of that name (as the books name it)           |
// | `paid_from=<date>`   | money received on or after that day (R14, the paid figure)           |
// | `paid_to=<date>`     | money received on or before that day                                 |
//
// Parameters combine (`customer=Tailspin&status=overdue`). The bucket edges are
// a setting (`[invoicer] aging-buckets`), so `aged=` takes any range.

/** An amount: plain decimal text (for drawing) and the journal's style. */
export interface AmountView {
    amount: string;
    text: string;
}

/** Part of what is owed, and how many items make it. */
export interface PartView extends AmountView {
    count: number;
}

/** One aging bucket. */
export interface BucketView extends PartView {
    /** `current`, `1-30`, `91+`: the list's `aged=` value. */
    key: string;
    /** `Current`, `1–30`, `91+`. */
    label: string;
    from: number | null;
    to: number | null;
}

/** The overdue item due longest ago. */
export interface OldestView {
    number: string | null;
    /** Its file; null for a receivable booked in the journal alone. */
    path: string | null;
    customer: string | null;
    due: string;
    days_past_due: number;
    outstanding: string;
}

/** Days sales outstanding. */
export interface DsoView {
    value: string | null;
    days: number;
    outstanding: string;
    billed: string;
}

/** What is owed in one commodity. */
export interface CommodityView {
    commodity: string;
    outstanding: PartView;
    overdue: PartView;
    not_due: PartView;
    /** Booked in the journal without an invoice file: in the totals, not in the list. */
    unfiled: PartView;
    /** The bar's full width: the total, or $10,000 when that is more. */
    scale: AmountView;
    buckets: BucketView[];
    oldest_overdue: OldestView | null;
    dso: DsoView | null;
}

/** Money received in one commodity in the paid window. */
export interface PaidCommodityView extends AmountView {
    commodity: string;
    /** Invoices (and other receivables) it paid into. */
    count: number;
    /** Of those, booked without an invoice file here: in the figure, not in the list. */
    unfiled: number;
}

/** Paid in the last N days (R14): money received from `from` through `to` (today). */
export interface PaidView {
    days: number;
    from: string;
    to: string;
    /** The default commodity first; empty when nothing came in. */
    commodities: PaidCommodityView[];
}

/** Why an invoice is on the recently updated row (R15). */
export type RecentEvent = "dated" | "edited" | "sent" | "paid" | "voided";

/** One recently updated invoice. */
export interface RecentView {
    number: string | null;
    path: string;
    customer: string | null;
    date: string | null;
    /** The day of its latest event. */
    updated: string;
    event: RecentEvent;
    /** `draft`, `open`, `overdue`, `paid`, `void` (or another `status:` value). */
    status: string;
    total: string | null;
}

/** `GET /api/dashboard`. */
export interface Dashboard {
    basis: "accrual" | "cash";
    today: string;
    /** Invoices in the books folder, every status: zero shows the empty state. */
    invoices: number;
    commodities: CommodityView[];
    average_days_to_pay: {invoices: number; average: string | null};
    drafts: {count: number; totals: AmountView[]; oldest: string | null};
    paid: PaidView;
    /** Latest first, at most six. */
    recent: RecentView[];
    /** Null when the books have no recognition schedules. */
    recognition: {due: number; closed: number; earliest: string | null} | null;
    aging_buckets: number[];
    dso_days: number;
    /** `[invoicer] paid-window-days`: the paid window the page starts on. */
    paid_window_days: number;
    /** Whether this process can open the books in Ledgeline (the window, macOS). */
    open_ledgeline: boolean;
}

/** Whether `body` is shaped like a dashboard. */
export function isDashboard(body: unknown): body is Dashboard {
    if (typeof body !== "object" || body === null) return false;
    const d = body as Record<string, unknown>;
    return (
        typeof d.basis === "string" &&
        typeof d.invoices === "number" &&
        Array.isArray(d.commodities) &&
        typeof d.drafts === "object" &&
        d.drafts !== null &&
        typeof d.paid === "object" &&
        d.paid !== null &&
        Array.isArray(d.recent)
    );
}

/** The invoice list's filters the dashboard links with (see the table above). */
export interface ListFilter {
    status?: "open" | "overdue" | "draft" | "paid";
    aged?: string;
    customer?: string;
    paid_from?: string;
    paid_to?: string;
}

/** The invoice list at `base` (resolved `/invoices`), filtered: `?status=overdue`. */
export function listHref(base: string, filter: ListFilter): string {
    const params = new URLSearchParams();
    // A fixed order, so a link reads the same however it was built.
    for (const key of ["status", "aged", "customer", "paid_from", "paid_to"] as const) {
        const value = filter[key];
        if (value !== undefined && value !== "") params.set(key, value);
    }
    const query = params.toString();
    return query === "" ? base : `${base}?${query}`;
}

const number = (text: string): number => {
    const n = Number(text);
    return Number.isFinite(n) ? n : 0;
};

/** `part` as a share of `whole`, in percent (0–100); 0 when either is not positive. */
export function share(part: string, whole: string): number {
    const p = number(part);
    const w = number(whole);
    if (p <= 0 || w <= 0) return 0;
    return Math.min(100, (p / w) * 100);
}

/** The outstanding bar's two segments: overdue, then not yet due. */
export interface BarSegment {
    id: "overdue" | "not-due";
    /** Where it starts, in percent of the axis. */
    start: number;
    /** Its width, in percent of the axis. */
    width: number;
    /** Its share of what is owed, in percent. */
    share: number;
}

/**
 * The bar's axis: round ticks from 0 to the first round step at or past
 * `scale` (the total, or $10,000 when that is more), at most about six steps of
 * 1, 2, 2.5 or 5 × 10ⁿ. Its last tick is the axis's full width.
 */
export function axisTicks(scale: string | number, steps = 6): number[] {
    const top = typeof scale === "number" ? scale : number(scale);
    if (!(top > 0)) return [0];
    const rough = top / steps;
    const magnitude = 10 ** Math.floor(Math.log10(rough));
    const step = [1, 2, 2.5, 5, 10].map((m) => m * magnitude).find((s) => s >= rough * (1 - 1e-9)) ?? 10 * magnitude;
    const count = Math.ceil(top / step - 1e-9);
    return Array.from({length: count + 1}, (_, i) => Number((i * step).toPrecision(12)));
}

/** An axis label: "0", "500", "2k", "2.5k", "12.5k", "1.5M". */
export function compactNumber(n: number): string {
    const trim = (v: number): string => String(Number(v.toFixed(1)));
    if (Math.abs(n) >= 1_000_000) return `${trim(n / 1_000_000)}M`;
    if (Math.abs(n) >= 1_000) return `${trim(n / 1_000)}k`;
    return trim(n);
}

/** The segments for one commodity against the axis's top (`axisTicks`'s last); a zero part draws nothing. */
export function barSegments(view: Pick<CommodityView, "overdue" | "not_due" | "outstanding">, axisTop: number): BarSegment[] {
    const top = axisTop > 0 ? axisTop : 1;
    const width = (part: string): number => Math.min(100, Math.max(0, (number(part) / top) * 100));
    const overdue = width(view.overdue.amount);
    const notDue = Math.min(100 - overdue, width(view.not_due.amount));
    const segments: BarSegment[] = [
        {id: "overdue", start: 0, width: overdue, share: share(view.overdue.amount, view.outstanding.amount)},
        {id: "not-due", start: overdue, width: notDue, share: share(view.not_due.amount, view.outstanding.amount)},
    ];
    return segments.filter((b) => b.width > 0);
}

/** A share for people: "56%", "under 1%", "100%". */
export function percent(value: number): string {
    if (value <= 0) return "0%";
    if (value < 1) return "under 1%";
    return `${Math.round(value)}%`;
}

/** The paid windows the dashboard offers (R14), with the setting's own when it isn't one of them. */
export const PAID_WINDOWS = [7, 30, 90, 365] as const;

/** The longest paid window the server takes. */
export const MAX_PAID_DAYS = 3660;

/** The choices for the period selector: the fixed four plus `setting`, ascending. */
export function paidWindows(setting: number): number[] {
    return [...new Set([...PAID_WINDOWS, setting])].filter((d) => d >= 1 && d <= MAX_PAID_DAYS).sort((a, b) => a - b);
}

/** "Last 30 days", "Last day", "Last 365 days". */
export function paidWindowLabel(days: number): string {
    return days === 1 ? "Last day" : `Last ${days} days`;
}

/** The dashboard URL's `paid_days`: a whole number of days the server takes, else null (the setting). */
export function paidDaysParam(params: Pick<URLSearchParams, "get">): number | null {
    const text = (params.get("paid_days") ?? "").trim();
    if (!/^\d+$/.test(text)) return null;
    const days = Number(text);
    return days >= 1 && days <= MAX_PAID_DAYS ? days : null;
}

/** The dashboard's query for a paid window: none for the setting's own (the bare `/app/`). */
export function paidDaysQuery(days: number, setting: number): string {
    return days === setting ? "" : `paid_days=${days}`;
}

/** A recent invoice's status, as its card's band says it, and the theme colour it wears (the list's badges' tones). */
export function recentStatus(status: string): {label: string; tone: "neutral" | "warning" | "error" | "success"} {
    switch (status) {
        case "draft":
            return {label: "Draft", tone: "neutral"};
        case "open":
            return {label: "Open", tone: "warning"};
        case "overdue":
            return {label: "Overdue", tone: "error"};
        case "paid":
            return {label: "Paid", tone: "success"};
        case "void":
            return {label: "Void", tone: "neutral"};
        default:
            return {label: status, tone: "neutral"};
    }
}

/** What a card says happened last: "Paid 2026-11-05", "Edited 2026-11-21". */
export function recentEventText(view: Pick<RecentView, "event" | "updated">): string {
    const words: Record<RecentEvent, string> = {dated: "Dated", edited: "Edited", sent: "Sent", paid: "Paid", voided: "Voided"};
    return `${words[view.event] ?? "Updated"} ${view.updated}`;
}

/** "1 day", "125 days"; a decimal figure as given ("25.3 days"). */
export function days(value: number | string): string {
    return `${value} ${value === 1 || value === "1" || value === "1.0" ? "day" : "days"}`;
}

/** "1 invoice", "3 invoices" (any noun with a plain plural). */
export function count(n: number, one: string, many = `${one}s`): string {
    return `${n} ${n === 1 ? one : many}`;
}

/** A bucket's name for people: "Current" (not yet due), "1–30 days past due", "91+ days past due". */
export function bucketName(bucket: Pick<BucketView, "key" | "label">): string {
    return bucket.key === "current" ? "Not yet due" : `${bucket.label} days past due`;
}
