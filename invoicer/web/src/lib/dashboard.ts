// The receivables dashboard (invoicer PLAN 5a): `GET /api/dashboard`'s shape
// (`invoicer_core::views::dashboard::Dashboard`) and the pure helpers the page
// draws it with.
//
// Every figure links to the invoice list, filtered by query parameters. The
// contract (PLAN 5a, R5), for the list's parser:
//
// | Parameter            | Meaning                                                              |
// |----------------------|----------------------------------------------------------------------|
// | `status=open`        | still owed (sent, not paid in full), overdue included                |
// | `status=overdue`     | still owed and past due on today                                     |
// | `status=draft`       | drafts                                                               |
// | `status=paid`        | paid in full                                                         |
// | `aged=current`       | still owed, not yet past due                                         |
// | `aged=<from>-<to>`   | still owed, `from` to `to` days past due, inclusive (`aged=31-60`)  |
// | `aged=<from>-`       | still owed, `from` or more days past due (`aged=91-`)                |
// | `customer=<name>`    | billed to the customer of that name (as the books name it)           |
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
    /** `current`, `1-30`, `91-`: the list's `aged=` value. */
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

/** `GET /api/dashboard`. */
export interface Dashboard {
    basis: "accrual" | "cash";
    today: string;
    /** Invoices in the books folder, every status: zero shows the empty state. */
    invoices: number;
    commodities: CommodityView[];
    average_days_to_pay: {invoices: number; average: string | null};
    drafts: {count: number; totals: AmountView[]; oldest: string | null};
    /** Null when the books have no recognition schedules. */
    recognition: {due: number; closed: number; earliest: string | null} | null;
    aging_buckets: number[];
    dso_days: number;
    /** Whether this process can open the books in Ledgeline (the window, macOS). */
    open_ledgeline: boolean;
}

/** Whether `body` is shaped like a dashboard. */
export function isDashboard(body: unknown): body is Dashboard {
    if (typeof body !== "object" || body === null) return false;
    const d = body as Record<string, unknown>;
    return typeof d.basis === "string" && typeof d.invoices === "number" && Array.isArray(d.commodities) && typeof d.drafts === "object" && d.drafts !== null;
}

/** The invoice list's filters the dashboard links with (see the table above). */
export interface ListFilter {
    status?: "open" | "overdue" | "draft" | "paid";
    aged?: string;
    customer?: string;
}

/** The invoice list at `base` (resolved `/invoices`), filtered: `?status=overdue`. */
export function listHref(base: string, filter: ListFilter): string {
    const params = new URLSearchParams();
    // A fixed order, so a link reads the same however it was built.
    for (const key of ["status", "aged", "customer"] as const) {
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

/** The outstanding bar's blocks: overdue, then not yet due, then the grey rest of the scale. */
export interface BarBlock {
    id: "overdue" | "not-due" | "rest";
    /** Width, in percent of the bar. */
    width: number;
}

/** The bar's blocks for one commodity; a block at zero is left out (it draws nothing). */
export function barBlocks(view: Pick<CommodityView, "overdue" | "not_due" | "scale" | "outstanding">): BarBlock[] {
    const overdue = share(view.overdue.amount, view.scale.amount);
    const notDue = share(view.not_due.amount, view.scale.amount);
    const rest = Math.max(0, 100 - overdue - notDue);
    const blocks: BarBlock[] = [
        {id: "overdue", width: overdue},
        {id: "not-due", width: notDue},
        // Under a hundredth of a percent is rounding, not a remainder.
        {id: "rest", width: rest < 0.01 ? 0 : rest},
    ];
    return blocks.filter((b) => b.width > 0);
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
