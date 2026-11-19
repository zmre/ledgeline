// Domain types for the accrual recognition helpers (plan 45, Phase 3). Pure
// TS: no Svelte/DOM imports. Decoded from the wire by `./decode.ts`; every
// computation behind them (schedules, due sets, ties, restatements,
// settlement proposals) lives in `crates/ledgeline-core/src/accrual/`. The
// client only displays and asks.

import type {Dec} from "#lib/domain/money.js";
import type {ISODate} from "#lib/domain/types.js";

/** An account's accrual role (D-gate). */
export type AccrualRole = "deferred-revenue" | "prepaid" | "receivable" | "payable";
export const ACCRUAL_ROLES: readonly AccrualRole[] = ["deferred-revenue", "prepaid", "receivable", "payable"];

/** One exact amount in one commodity. */
export interface Money {
    commodity: string;
    quantity: Dec;
}

export type PartyKind = "customer" | "vendor";

/** A customer or a vendor, as its `customer:`/`vendor:` tag names it. */
export interface Party {
    kind: PartyKind;
    name: string;
}

/** `contract:` values (Q5). */
export type ContractKind = "new" | "renewal" | "expansion" | "reactivation";
export const CONTRACT_KINDS: readonly ContractKind[] = ["new", "renewal", "expansion", "reactivation"];

/** Anniversary (unanchored) or calendar months (anchored, prorated). */
export type Proration = "anniversary" | "calendar";

/** How long a schedule runs: N months, or through a date. */
export type Term = {months: number; through?: undefined} | {through: ISODate; months?: undefined};

/** `GET /api/accrual` with the helpers on. */
export interface AccrualStatus {
    generation: string;
    basis: {value: "cash" | "accrual"; source: "books" | "inferred"};
    /** Why the helpers are on: `[books] basis`, a declared `kpi:`, or an account name that looks like one. */
    reason: {kind: "basis" | "declared" | "inferred" | "none"; account: string | null; role: AccrualRole | null};
    /** Every account with a role, by full name. */
    roles: ReadonlyMap<string, {role: AccrualRole; declared: boolean}>;
    /** The recognition file, relative to the journal's folder; null without a backing file. */
    file: {path: string; exists: boolean; included: boolean} | null;
    closedThrough: ISODate | null;
    /**
     * `invoices` is the CLEAR proposals only (what Link all takes), under the
     * user's settlement lookback and tolerance; `invoicesCheck` the ones to check.
     */
    counts: {due: number; dueClosed: number; needsSchedule: number; invoices: number; invoicesCheck: number};
    /** Why the recognition file could not be read, or null. */
    problem: string | null;
    /** `[books]` values that were ignored, and why. */
    warnings: readonly string[];
}

/** One posting of a due entry (the rule as `hledger print --forecast` would print it). */
export interface DuePosting {
    account: string;
    amounts: readonly Money[];
}

/** One due occurrence: schedule `id`'s period dated `recognized`. */
export interface DueEntry {
    id: string;
    recognized: ISODate;
    description: string;
    postings: readonly DuePosting[];
}

/** A month's due entries for one customer/vendor (or none). */
export interface DueGroup {
    party: Party | null;
    entries: readonly DueEntry[];
}

/** One deferred/prepaid account at the month's end: before and after posting, and what the schedules say. */
export interface DueTie {
    account: string;
    commodity: string;
    before: Dec;
    after: Dec;
    /** The schedules' unrecognized remainder at that date. */
    scheduled: Dec;
    ties: boolean;
}

export interface DueMonth {
    /** `YYYY-MM`. */
    month: string;
    count: number;
    totals: readonly Money[];
    groups: readonly DueGroup[];
    ties: readonly DueTie[];
}

/** `GET /api/recognition/due` — the catch-up table (D6). */
export interface DueView {
    generation: string;
    today: ISODate;
    closedThrough: ISODate | null;
    months: readonly DueMonth[];
    /** Due on or before `closed-through`: posted only as one adjustment today. */
    closed: {count: number; totals: readonly Money[]; entries: readonly DueEntry[]};
}

/** `POST /api/recognition/post` → what was written. */
export interface PostResult {
    generation: string;
    posted: readonly {id: string; recognized: ISODate}[];
    /** Schedule ids that got a closed-period adjustment. */
    adjustments: readonly string[];
}

/** One side of a settlement (an invoice or bill posting, or a payment's), or a deferral that needs a schedule. */
export interface SettleSide {
    index: number;
    posting: number;
    /** The transaction's fingerprint when listed; a link needs it. Null when the engine could not say. */
    fingerprint: string | null;
    date: ISODate;
    description: string;
    account: string;
    amount: Money;
    invoice: string | null;
    party: Party | null;
}

/** `GET /api/recognition/needs` — Needs a schedule. */
export interface NeedsView {
    generation: string;
    /** Days looked back; 0 is everything. */
    lookback: number;
    /** Newest first. */
    items: readonly SettleSide[];
}

export type FitKind = "exact" | "short" | "over";

/** One invoice ↔ payment proposal (D-settle). */
export interface SettleProposal {
    invoice: SettleSide;
    payment: SettleSide;
    fit: {kind: FitKind; difference: Dec | null};
    byNumber: boolean;
    byParty: boolean;
    gapDays: number;
    ambiguous: boolean;
    /** Exact, unambiguous, and with an invoice-number or party signal: Link all takes it. */
    clear: boolean;
    /** For a short payment, the account last used for fees on this receivable/payable. */
    feeAccount: string | null;
}

/** `GET /api/settlements` (and `/for/{index}`). */
export interface SettlementsView {
    generation: string;
    counts: {clear: number; check: number};
    proposals: readonly SettleProposal[];
    open: {invoices: readonly SettleSide[]; payments: readonly SettleSide[]};
}

/** A schedule line: one source posting's deferred amount and where it is recognized. */
export interface ScheduleLine {
    sourceAccount: string;
    targetAccount: string;
    /** Signed as the source posting writes it: negative for deferred revenue. */
    total: Money;
    party: Party | null;
}

/** A schedule's inputs (`ScheduleSpec`). */
export interface ScheduleSpec {
    id: string;
    lines: readonly ScheduleLine[];
    start: ISODate;
    term: Term;
    proration: Proration;
    description: string;
    headerTags: readonly (readonly [string, string])[];
}

export type PeriodPart = "opening" | "regular" | "closing";
/** `posted`, `due`, `closed` (due in a closed period), `covered` (past, before a restated cut), or `future`. */
export type PeriodStatus = "posted" | "due" | "closed" | "covered" | "future";

export interface SchedulePeriod {
    date: ISODate;
    part: PeriodPart;
    /** Per line, signed like the line's total. */
    amounts: readonly Dec[];
    /** Absent in a preview. */
    status: PeriodStatus | null;
    /** The posted entry's transaction index. */
    entry: number | null;
}

export interface ScheduleRule {
    role: "opening" | "main" | "final";
    period: string;
    /** Exactly as it is (or will be) written in the recognition file. */
    text: string;
}

/** A built schedule. */
export interface Schedule {
    spec: ScheduleSpec;
    periods: readonly SchedulePeriod[];
    rules: readonly ScheduleRule[];
    renewalDate: ISODate;
}

/** `GET /api/recognition/for/{index}` — one source transaction's schedules. */
export interface TransactionSchedules {
    generation: string;
    schedules: readonly Schedule[];
    /** Posting positions of its deferral lines with no schedule yet. */
    unscheduled: readonly number[];
    /** The party's latest earlier schedule, to prefill a renewal from. */
    renewal: {prefill: ScheduleSpec; contract: ContractKind} | null;
    /** A customer/vendor guessed from the payee: offered, never applied. */
    partySuggestion: Party | null;
}

export type RestatementAction = "kept" | "unchanged" | "rewrite" | "delete";

export interface RestatementRow {
    index: number;
    date: ISODate;
    recognized: ISODate;
    recognizedAfter: ISODate | null;
    closed: boolean;
    before: readonly Dec[];
    after: readonly Dec[] | null;
    action: RestatementAction;
}

/** A transaction a write will add (an adjustment). */
export interface PlannedEntry {
    date: ISODate;
    description: string;
    tags: readonly (readonly [string, string])[];
    postings: readonly DuePosting[];
}

/** D4's preview of one restated schedule. */
export interface Restatement {
    id: string;
    ruleChange: {before: string; after: string} | null;
    rows: readonly RestatementRow[];
    deletes: readonly number[];
    adjustment: PlannedEntry | null;
    /** Per line, the schedule's total after the restatement. */
    totalAfter: readonly Dec[];
}

/** `POST /api/recognition/preview` and `/save`. */
export interface SavePlan {
    /** Hand back to `save` so a stale preview is refused. */
    accrualGeneration: string;
    appended: readonly Schedule[];
    restatements: readonly Restatement[];
    createsFile: boolean;
    /** The snapshot generation after a save; null for a preview. */
    generation: string | null;
}
