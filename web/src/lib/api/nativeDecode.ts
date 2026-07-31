// Native (ledgeline-engine) wire → domain decoder. THE ONLY FILE outside the
// engine that knows the native /api/* JSON field names. It mirrors normalize.ts:
// permissive raw mirrors + pure builders that emit frozen domain objects, with
// Dec built from {mantissa: string, places} decoded via BigInt — the engine
// string-encodes the mantissa because COMPUTED values (e.g. marketValue =
// shares × price, non-normalized) can exceed the JS safe-integer range, which a
// JSON number would silently lose. Nothing here touches Svelte/DOM, so the whole
// module is unit-testable under node.
//
// Wire contract (see crates/ledgeline-server/src/reports_api.rs):
//   - Dec          → {mantissa: <string>, places}  (value = mantissa / 10^places; BigInt-decoded)
//   - MixedAmount  → {"<commodity>": Dec, …}  (zero commodities already dropped)
//   - nulls kept (basis/price/gain/…); camelCase keys map 1:1 onto the domain types.
//
// The Raw* interfaces below mirror 28 Rust `Wire*` structs BY HAND — there is no
// codegen across this seam. What keeps the two halves honest is
// fixtures/native/v1/*.json (regenerate with `just snapshot-native`): the engine
// asserts its live responses against those bytes in
// crates/ledgeline-server/tests/native_wire_golden.rs, and nativeDecode.test.ts
// decodes the same files. Renaming a field on either side fails both suites.
//
// Corollary, and the reason decodeDec/decodeMixed both THROW on an absent value:
// a field this decoder cannot find must never become 0. Every money field in the
// reports path runs through decodeMixed, and "absent" silently rendering $0.00
// was a live bug class (CLEANUP.md DRY-3).

import type {Dec, MixedAmount} from "$lib/domain/money";
import type {ISODate} from "$lib/domain/types";
import type {Holding, HoldingsPoint, HoldingsReport, HoldingsSeries, HoldingsWarning} from "$lib/holdings/types";
import type {
    IfLayout,
    OpaqueReason,
    PreviewUnavailable,
    RulesDocument,
    RulesFieldsPref,
    RulesIndex,
    RulesItem,
    RulesMatcher,
    RulesPref,
    RulesPreview,
    RulesSettings,
    RulesSourcePref,
    RulesWarning,
} from "$lib/imports/types";
import type {
    Cadence,
    ChangeKind,
    ChangeRow,
    CostOfLiving,
    InsightsPeriod,
    InsightsReport,
    InvestmentPerf,
    MetricDelta,
    MoverRow,
    PerfPoint,
    Subscription,
    SubscriptionsReport,
    TopTxn,
} from "$lib/reports/insightsTypes";
import type {BudgetCell, BudgetReport, BudgetRow, PeriodReport, ReportRow, Section, SectionedReport} from "$lib/reports/types";
import {ApiShapeError} from "./client";

// ---------------------------------------------------------------------------
// Permissive raw mirrors (INTERNAL — nothing outside lib/api imports these).
// Every field is optional; the decoders validate what they read.
// ---------------------------------------------------------------------------

interface RawDec {
    // String-encoded significand (decoded via BigInt): computed values can
    // exceed the JS safe-integer range, so the engine sends it as a string.
    mantissa?: string;
    places?: number;
}

type RawMixed = Record<string, RawDec | undefined>;

interface RawReportRow {
    account?: string;
    depth?: number;
    own?: RawMixed;
    inclusive?: RawMixed;
}

interface RawSection {
    title?: string;
    rows?: RawReportRow[];
    total?: RawMixed;
}

interface RawSectionedReport {
    asOf?: string;
    from?: string;
    to?: string;
    sections?: RawSection[];
    grandTotal?: RawMixed;
}

interface RawPeriodRow {
    account?: string;
    depth?: number;
    values?: RawMixed[];
}

interface RawReportMeta {
    unpriced?: unknown[];
}

interface RawPeriodReport {
    buckets?: unknown[];
    rows?: RawPeriodRow[];
    totals?: RawMixed[];
    meta?: RawReportMeta | null;
}

interface RawBudgetCell {
    actual?: RawMixed;
    // `null` = no goal (e.g. <unbudgeted>); an object (possibly {}) = budgeted.
    goal?: RawMixed | null;
}

interface RawBudgetRow {
    account?: string;
    depth?: number;
    cells?: RawBudgetCell[];
}

interface RawBudgetReport {
    buckets?: unknown[];
    rows?: RawBudgetRow[];
    totals?: RawBudgetCell[];
}

interface RawHoldingPrice {
    qty?: RawDec;
    date?: string;
    source?: string;
}

interface RawHolding {
    symbol?: string;
    name?: string;
    accounts?: unknown[];
    shares?: RawDec;
    basis?: RawDec | null;
    firstBasisDate?: string | null;
    price?: RawHoldingPrice | null;
    marketValue?: RawDec | null;
    gain?: RawDec | null;
    gainPct?: number | null;
}

interface RawHoldingsTotals {
    marketValue?: RawDec;
    basis?: RawDec | null;
    gain?: RawDec | null;
    gainPct?: number | null;
}

interface RawWarning {
    symbol?: string;
    kind?: string;
    message?: string;
}

interface RawHoldingsReport {
    asOf?: string;
    base?: string;
    holdings?: RawHolding[];
    totals?: RawHoldingsTotals;
    topGainers?: RawHolding[];
    topLosers?: RawHolding[];
    warnings?: RawWarning[];
}

interface RawHoldingsPoint {
    date?: string;
    bucket?: string;
    label?: string;
    marketValue?: RawDec;
    basis?: RawDec | null;
}

interface RawHoldingsSeries {
    base?: string;
    points?: RawHoldingsPoint[];
    hasBasis?: boolean;
}

interface RawMetricDelta {
    current?: RawMixed;
    previous?: RawMixed;
    delta?: RawMixed;
    pct?: number | null;
}

interface RawCostOfLiving {
    currentTotal?: RawMixed;
    previousTotal?: RawMixed;
    monthsCurrent?: number;
    monthsPrevious?: number;
}

interface RawPerfPoint {
    gain?: RawDec | null;
    gainPct?: number | null;
}

interface RawInvestmentPerf {
    current?: RawPerfPoint;
    previous?: RawPerfPoint;
}

interface RawInsightsPeriod {
    start?: string;
    mid?: string;
    end?: string;
    prevStart?: string;
    prevEnd?: string;
    currStart?: string;
    currEnd?: string;
}

interface RawChangeRow {
    account?: string;
    current?: RawDec;
    previous?: RawDec;
    delta?: RawDec;
    pct?: number | null;
    kind?: string;
}

interface RawMoverRow {
    symbol?: string;
    name?: string;
    gain?: RawDec | null;
    gainPct?: number | null;
    startEstimated?: boolean;
}

interface RawSubscription {
    payee?: string;
    cadence?: string;
    typicalAmount?: RawDec;
    annualizedCost?: RawDec;
    occurrences?: number;
    firstSeen?: string;
    lastSeen?: string;
    nextExpected?: string;
    accounts?: unknown[];
    manual?: boolean;
}

interface RawSubscriptionsReport {
    asOf?: string;
    lookbackStart?: string;
    monthly?: RawSubscription[];
    annual?: RawSubscription[];
}

interface RawTopTxn {
    index?: number;
    date?: string;
    description?: string;
    amount?: RawDec;
}

interface RawInsightsReport {
    period?: RawInsightsPeriod;
    base?: string;
    journalStart?: string | null;
    revenue?: RawMetricDelta;
    expenses?: RawMetricDelta;
    netWorth?: RawMetricDelta;
    costOfLiving?: RawCostOfLiving;
    investment?: RawInvestmentPerf;
    cashBalance?: RawMetricDelta;
    expenseChanges?: RawChangeRow[];
    revenueChanges?: RawChangeRow[];
    movers?: RawMoverRow[];
    topTxns?: RawTopTxn[];
}

// --- CSV import rules (rules_api.rs) ---------------------------------------
// Unlike the report bodies above, most keys here are `skip_serializing_if =
// "Option::is_none"`: an absent setting is a FACT ("the file does not say"), not
// a missing field, so these decode to null instead of throwing. The strictness
// lives where the wire is unconditional — a document's id/revision/newline/items
// and each item's kind tag — which is what the golden decode test leans on.

interface RawRulesPref {
    value?: unknown;
    itemId?: number;
}

interface RawRulesSource {
    value?: string;
    executesShellCommand?: boolean;
    itemId?: number;
}

interface RawRulesFieldsPref {
    names?: unknown[];
    itemId?: number;
}

interface RawRulesSettings {
    source?: RawRulesSource;
    archive?: RawRulesPref;
    encoding?: RawRulesPref;
    separator?: RawRulesPref;
    decimalMark?: RawRulesPref;
    dateFormat?: RawRulesPref;
    timezone?: RawRulesPref;
    newestFirst?: RawRulesPref;
    intraDayReversed?: RawRulesPref;
    skip?: RawRulesPref;
    balanceType?: RawRulesPref;
    account1?: RawRulesPref;
    account2?: RawRulesPref;
    currency?: RawRulesPref;
    fields?: RawRulesFieldsPref;
}

interface RawRulesMatcher {
    field?: string;
    pattern?: string;
}

interface RawRulesAssignment {
    field?: string;
    value?: string;
}

/** One item, flattened: `#[serde(flatten)]` puts the `kind` tag beside id/line/lines. */
interface RawRulesItem {
    id?: number;
    line?: number;
    lines?: number;
    kind?: string;
    // trivia / opaque
    text?: string;
    truncated?: boolean;
    // directive
    name?: string;
    value?: string;
    // include
    target?: string;
    // fields
    names?: unknown[];
    // assignment
    field?: string;
    // ifBlock
    layout?: string;
    matchers?: RawRulesMatcher[];
    assignments?: RawRulesAssignment[];
    // opaque
    reason?: string;
    label?: string;
}

interface RawRulesWarning {
    itemId?: number;
    line?: number;
    message?: string;
}

interface RawRulesDoc {
    id?: string;
    label?: string;
    revision?: string;
    editable?: boolean;
    newline?: string;
    settings?: RawRulesSettings;
    items?: RawRulesItem[];
    warnings?: RawRulesWarning[];
}

interface RawRulesFile {
    id?: string;
    label?: string;
    revision?: string;
    sizeBytes?: number;
    parsed?: boolean;
    account1?: string;
    account2?: string;
    ifBlockCount?: number;
    editableBlockCount?: number;
    opaqueItemCount?: number;
    warnings?: unknown[];
}

interface RawRulesIndex {
    rootLabel?: string;
    editable?: boolean;
    truncated?: boolean;
    files?: RawRulesFile[];
    warnings?: unknown[];
}

interface RawRulesPreview {
    available?: boolean;
    reason?: string;
    dataLabel?: string;
    separator?: string;
    header?: unknown[];
    rows?: unknown[];
    columns?: number;
    truncated?: boolean;
}

// ---------------------------------------------------------------------------
// Scalar decoders (shared)
// ---------------------------------------------------------------------------

/** Shallow-freeze an array without losing its mutable-typed contract (as normalize.ts). */
function frozen<T>(items: T[]): T[] {
    return Object.freeze(items) as T[];
}

/** {mantissa, places} → frozen Dec, guarding the JS safe-integer range like normalize.ts. */
function decodeDec(raw: RawDec | undefined, context: string): Dec {
    if (raw === undefined || raw === null || typeof raw.mantissa !== "string" || typeof raw.places !== "number") {
        throw new ApiShapeError(`${context}: missing mantissa/places`);
    }
    // BigInt handles any magnitude; just validate it's an integer literal.
    if (!/^-?\d+$/.test(raw.mantissa)) {
        throw new ApiShapeError(`${context}: mantissa ${JSON.stringify(raw.mantissa)} is not an integer`);
    }
    if (!Number.isSafeInteger(raw.places) || raw.places < 0) {
        throw new ApiShapeError(`${context}: invalid places ${raw.places}`);
    }
    return Object.freeze({m: BigInt(raw.mantissa), p: raw.places});
}

/** A nullable Dec: null/undefined (a refused/absent value) stays null; anything else must decode. */
function decodeOptDec(raw: RawDec | null | undefined, context: string): Dec | null {
    return raw === null || raw === undefined ? null : decodeDec(raw, context);
}

/**
 * {"<commodity>": Dec, …} → a plain Map (domain MixedAmount). Zero commodities
 * are already dropped server-side, so `{}` is a legitimate empty amount.
 *
 * STRICT, exactly like decodeDec: a missing field throws rather than decoding to
 * an empty Map (CLEANUP.md DRY-3). This used to return `new Map()` for
 * undefined, and since every money field in the reports path comes through here
 * — section `total`, `grandTotal`, period `values`/`totals`, budget `actual` —
 * a renamed or dropped Rust field rendered `$0.00` (`format.ts` fmtBase) or `0`
 * (`ReportTable.svelte`) with nothing raising anywhere. An empty amount and an
 * amount the server never sent are not the same fact, and only one of them is
 * safe to show a human as a balance.
 *
 * All 13 `WireMixed` fields in `reports_api.rs` are plain `BTreeMap` fields with
 * no `skip_serializing_if`, so every one of them is ALWAYS on the wire. The one
 * nullable case (`BudgetCell.goal: Option<WireMixed>`) serializes as `null`, not
 * as an absent key, and has [`decodeOptMixed`].
 */
function decodeMixed(raw: RawMixed | undefined, context: string): MixedAmount {
    if (raw === undefined || raw === null) {
        throw new ApiShapeError(`${context}: missing amount (expected an object of commodity → decimal, {} if empty)`);
    }
    if (typeof raw !== "object") throw new ApiShapeError(`${context}: expected an object of commodity → decimal`);
    const out: MixedAmount = new Map();
    for (const [commodity, value] of Object.entries(raw)) {
        out.set(commodity, decodeDec(value, `${context} "${commodity}"`));
    }
    return out;
}

/**
 * A nullable MixedAmount: `null` (a deliberate "no such amount") stays null;
 * anything else must decode. Mirrors decodeOptDec. Only `BudgetCell.goal` is
 * nullable — `null` = the account has no goal at all (`<unbudgeted>`), `{}` = it
 * is budgeted but has no goal in THIS bucket. Those two render differently, so
 * the distinction has to survive decoding.
 */
function decodeOptMixed(raw: RawMixed | null | undefined, context: string): MixedAmount | null {
    return raw === null || raw === undefined ? null : decodeMixed(raw, context);
}

/** A JSON array of strings, or [] when absent. */
function decodeStrings(raw: unknown[] | undefined, context: string): string[] {
    if (raw === undefined) return [];
    return raw.map((value, i) => {
        if (typeof value !== "string") throw new ApiShapeError(`${context}[${i}]: expected a string`);
        return value;
    });
}

// ---------------------------------------------------------------------------
// SectionedReport (balance sheet / income statement)
// ---------------------------------------------------------------------------

function decodeReportRow(raw: RawReportRow | undefined, context: string): ReportRow {
    if (raw === undefined || typeof raw.account !== "string" || typeof raw.depth !== "number") {
        throw new ApiShapeError(`${context}: missing account/depth`);
    }
    return Object.freeze({
        account: raw.account,
        depth: raw.depth,
        own: decodeMixed(raw.own, `${context} own`),
        inclusive: decodeMixed(raw.inclusive, `${context} inclusive`),
    });
}

function decodeSection(raw: RawSection | undefined, context: string): Section {
    if (raw === undefined || typeof raw.title !== "string" || !Array.isArray(raw.rows)) {
        throw new ApiShapeError(`${context}: missing title/rows`);
    }
    return Object.freeze({
        title: raw.title,
        rows: frozen(raw.rows.map((row, i) => decodeReportRow(row, `${context} row #${i}`))),
        total: decodeMixed(raw.total, `${context} total`),
    });
}

export function decodeSectionedReport(raw: unknown): SectionedReport {
    const report = raw as RawSectionedReport;
    if (typeof report !== "object" || report === null || !Array.isArray(report.sections)) {
        throw new ApiShapeError("sectioned report: expected a sections array");
    }
    const out: SectionedReport = {
        sections: frozen(report.sections.map((section, i) => decodeSection(section, `section #${i}`))),
        grandTotal: decodeMixed(report.grandTotal, "report grandTotal"),
    };
    if (typeof report.asOf === "string") out.asOf = report.asOf;
    if (typeof report.from === "string") out.from = report.from;
    if (typeof report.to === "string") out.to = report.to;
    return Object.freeze(out);
}

// ---------------------------------------------------------------------------
// PeriodReport (cash flow / net worth)
// ---------------------------------------------------------------------------

function decodePeriodRow(raw: RawPeriodRow | undefined, context: string): PeriodReport["rows"][number] {
    if (raw === undefined || typeof raw.account !== "string" || typeof raw.depth !== "number" || !Array.isArray(raw.values)) {
        throw new ApiShapeError(`${context}: missing account/depth/values`);
    }
    return Object.freeze({
        account: raw.account,
        depth: raw.depth,
        values: frozen(raw.values.map((value, i) => decodeMixed(value, `${context} values[${i}]`))),
    });
}

export function decodePeriodReport(raw: unknown): PeriodReport {
    const report = raw as RawPeriodReport;
    if (typeof report !== "object" || report === null || !Array.isArray(report.buckets) || !Array.isArray(report.rows) || !Array.isArray(report.totals)) {
        throw new ApiShapeError("period report: expected buckets/rows/totals arrays");
    }
    const out: PeriodReport = {
        buckets: frozen(decodeStrings(report.buckets, "report buckets")),
        rows: frozen(report.rows.map((row, i) => decodePeriodRow(row, `period row #${i}`))),
        totals: frozen(report.totals.map((total, i) => decodeMixed(total, `report totals[${i}]`))),
    };
    if (report.meta !== undefined && report.meta !== null) {
        out.meta = Object.freeze({unpriced: frozen(decodeStrings(report.meta.unpriced, "report meta.unpriced"))});
    }
    return Object.freeze(out);
}

// ---------------------------------------------------------------------------
// BudgetReport (actuals vs. periodic-rule goals)
// ---------------------------------------------------------------------------

function decodeBudgetCell(raw: RawBudgetCell | undefined, context: string): BudgetCell {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing cell`);
    return Object.freeze({
        actual: decodeMixed(raw.actual, `${context} actual`),
        // null (no goal) stays null; an object (incl. {} = budgeted-but-zero) decodes to a Map.
        goal: decodeOptMixed(raw.goal, `${context} goal`),
    });
}

function decodeBudgetRow(raw: RawBudgetRow | undefined, context: string): BudgetRow {
    if (raw === undefined || typeof raw.account !== "string" || typeof raw.depth !== "number" || !Array.isArray(raw.cells)) {
        throw new ApiShapeError(`${context}: missing account/depth/cells`);
    }
    return Object.freeze({
        account: raw.account,
        depth: raw.depth,
        cells: frozen(raw.cells.map((cell, i) => decodeBudgetCell(cell, `${context} cells[${i}]`))),
    });
}

export function decodeBudgetReport(raw: unknown): BudgetReport {
    const report = raw as RawBudgetReport;
    if (typeof report !== "object" || report === null || !Array.isArray(report.buckets) || !Array.isArray(report.rows) || !Array.isArray(report.totals)) {
        throw new ApiShapeError("budget report: expected buckets/rows/totals arrays");
    }
    return Object.freeze({
        kind: "budget" as const,
        buckets: frozen(decodeStrings(report.buckets, "budget buckets")),
        rows: frozen(report.rows.map((row, i) => decodeBudgetRow(row, `budget row #${i}`))),
        totals: frozen(report.totals.map((total, i) => decodeBudgetCell(total, `budget totals[${i}]`))),
    });
}

// ---------------------------------------------------------------------------
// HoldingsReport + HoldingsSeries
// ---------------------------------------------------------------------------

function decodeHoldingPrice(raw: RawHoldingPrice, context: string): NonNullable<Holding["price"]> {
    if (typeof raw.date !== "string") throw new ApiShapeError(`${context}: missing date`);
    if (raw.source !== "directive" && raw.source !== "cost") {
        throw new ApiShapeError(`${context}: unknown price source ${JSON.stringify(raw.source)}`);
    }
    return Object.freeze({qty: decodeDec(raw.qty, `${context} qty`), date: raw.date, source: raw.source});
}

function decodeHolding(raw: RawHolding | undefined, context: string): Holding {
    if (raw === undefined || typeof raw.symbol !== "string" || typeof raw.name !== "string") {
        throw new ApiShapeError(`${context}: missing symbol/name`);
    }
    return Object.freeze({
        symbol: raw.symbol,
        name: raw.name,
        accounts: frozen(decodeStrings(raw.accounts, `${context} accounts`)),
        shares: decodeDec(raw.shares, `${context} shares`),
        basis: decodeOptDec(raw.basis, `${context} basis`),
        firstBasisDate: typeof raw.firstBasisDate === "string" ? (raw.firstBasisDate as ISODate) : null,
        price: raw.price === null || raw.price === undefined ? null : decodeHoldingPrice(raw.price, `${context} price`),
        marketValue: decodeOptDec(raw.marketValue, `${context} marketValue`),
        gain: decodeOptDec(raw.gain, `${context} gain`),
        gainPct: typeof raw.gainPct === "number" ? raw.gainPct : null,
    });
}

function decodeWarning(raw: RawWarning | undefined, context: string): HoldingsWarning {
    if (raw === undefined || typeof raw.symbol !== "string" || typeof raw.message !== "string") {
        throw new ApiShapeError(`${context}: missing symbol/message`);
    }
    if (raw.kind !== "missing-basis" && raw.kind !== "negative-shares" && raw.kind !== "unpriced") {
        throw new ApiShapeError(`${context}: unknown warning kind ${JSON.stringify(raw.kind)}`);
    }
    return Object.freeze({symbol: raw.symbol, kind: raw.kind, message: raw.message});
}

function decodeHoldingsTotals(raw: RawHoldingsTotals | undefined, context: string): HoldingsReport["totals"] {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing totals`);
    return Object.freeze({
        marketValue: decodeDec(raw.marketValue, `${context} marketValue`),
        basis: decodeOptDec(raw.basis, `${context} basis`),
        gain: decodeOptDec(raw.gain, `${context} gain`),
        gainPct: typeof raw.gainPct === "number" ? raw.gainPct : null,
    });
}

export function decodeHoldingsReport(raw: unknown): HoldingsReport {
    const report = raw as RawHoldingsReport;
    if (
        typeof report !== "object" ||
        report === null ||
        typeof report.asOf !== "string" ||
        typeof report.base !== "string" ||
        !Array.isArray(report.holdings)
    ) {
        throw new ApiShapeError("holdings report: expected asOf/base/holdings");
    }
    return Object.freeze({
        asOf: report.asOf as ISODate,
        base: report.base,
        holdings: frozen(report.holdings.map((holding, i) => decodeHolding(holding, `holding #${i}`))),
        totals: decodeHoldingsTotals(report.totals, "holdings totals"),
        topGainers: frozen((report.topGainers ?? []).map((holding, i) => decodeHolding(holding, `topGainer #${i}`))),
        topLosers: frozen((report.topLosers ?? []).map((holding, i) => decodeHolding(holding, `topLoser #${i}`))),
        warnings: frozen((report.warnings ?? []).map((warning, i) => decodeWarning(warning, `warning #${i}`))),
    });
}

function decodeHoldingsPoint(raw: RawHoldingsPoint | undefined, context: string): HoldingsPoint {
    if (raw === undefined || typeof raw.date !== "string" || typeof raw.bucket !== "string" || typeof raw.label !== "string") {
        throw new ApiShapeError(`${context}: missing date/bucket/label`);
    }
    return Object.freeze({
        date: raw.date as ISODate,
        bucket: raw.bucket,
        label: raw.label,
        marketValue: decodeDec(raw.marketValue, `${context} marketValue`),
        basis: decodeOptDec(raw.basis, `${context} basis`),
    });
}

export function decodeHoldingsSeries(raw: unknown): HoldingsSeries {
    const series = raw as RawHoldingsSeries;
    if (typeof series !== "object" || series === null || typeof series.base !== "string" || !Array.isArray(series.points)) {
        throw new ApiShapeError("holdings series: expected base/points");
    }
    return Object.freeze({
        base: series.base,
        points: frozen(series.points.map((point, i) => decodeHoldingsPoint(point, `series point #${i}`))),
        hasBasis: series.hasBasis === true,
    });
}

// ---------------------------------------------------------------------------
// InsightsReport (period-over-period dashboard)
// ---------------------------------------------------------------------------

function decodeMetricDelta(raw: RawMetricDelta | undefined, context: string): MetricDelta {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing metric`);
    return Object.freeze({
        current: decodeMixed(raw.current, `${context} current`),
        previous: decodeMixed(raw.previous, `${context} previous`),
        delta: decodeMixed(raw.delta, `${context} delta`),
        pct: typeof raw.pct === "number" ? raw.pct : null,
    });
}

function decodePerfPoint(raw: RawPerfPoint | undefined, context: string): PerfPoint {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing perf point`);
    return Object.freeze({
        gain: decodeOptDec(raw.gain, `${context} gain`),
        gainPct: typeof raw.gainPct === "number" ? raw.gainPct : null,
    });
}

function decodeCostOfLiving(raw: RawCostOfLiving | undefined, context: string): CostOfLiving {
    if (raw === undefined || raw === null || typeof raw.monthsCurrent !== "number" || typeof raw.monthsPrevious !== "number") {
        throw new ApiShapeError(`${context}: missing month counts`);
    }
    return Object.freeze({
        currentTotal: decodeMixed(raw.currentTotal, `${context} currentTotal`),
        previousTotal: decodeMixed(raw.previousTotal, `${context} previousTotal`),
        monthsCurrent: raw.monthsCurrent,
        monthsPrevious: raw.monthsPrevious,
    });
}

function decodeInvestmentPerf(raw: RawInvestmentPerf | undefined, context: string): InvestmentPerf {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing investment`);
    return Object.freeze({
        current: decodePerfPoint(raw.current, `${context} current`),
        previous: decodePerfPoint(raw.previous, `${context} previous`),
    });
}

function decodeInsightsPeriod(raw: RawInsightsPeriod | undefined, context: string): InsightsPeriod {
    const iso = (value: string | undefined, name: string): ISODate => {
        if (typeof value !== "string") throw new ApiShapeError(`${context}: missing ${name}`);
        return value as ISODate;
    };
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing period`);
    return Object.freeze({
        start: iso(raw.start, "start"),
        mid: iso(raw.mid, "mid"),
        end: iso(raw.end, "end"),
        prevStart: iso(raw.prevStart, "prevStart"),
        prevEnd: iso(raw.prevEnd, "prevEnd"),
        currStart: iso(raw.currStart, "currStart"),
        currEnd: iso(raw.currEnd, "currEnd"),
    });
}

function decodeChangeKind(raw: string | undefined, context: string): ChangeKind {
    if (raw === "changed" || raw === "ended") return raw;
    throw new ApiShapeError(`${context}: unknown change kind ${JSON.stringify(raw)}`);
}

function decodeChangeRow(raw: RawChangeRow | undefined, context: string): ChangeRow {
    if (raw === undefined || typeof raw.account !== "string") throw new ApiShapeError(`${context}: missing account`);
    return Object.freeze({
        account: raw.account,
        current: decodeDec(raw.current, `${context} current`),
        previous: decodeDec(raw.previous, `${context} previous`),
        delta: decodeDec(raw.delta, `${context} delta`),
        pct: typeof raw.pct === "number" ? raw.pct : null,
        kind: decodeChangeKind(raw.kind, context),
    });
}

function decodeMoverRow(raw: RawMoverRow | undefined, context: string): MoverRow {
    if (raw === undefined || typeof raw.symbol !== "string" || typeof raw.name !== "string") {
        throw new ApiShapeError(`${context}: missing symbol/name`);
    }
    return Object.freeze({
        symbol: raw.symbol,
        name: raw.name,
        gain: decodeOptDec(raw.gain, `${context} gain`),
        gainPct: typeof raw.gainPct === "number" ? raw.gainPct : null,
        startEstimated: raw.startEstimated === true,
    });
}

function decodeTopTxn(raw: RawTopTxn | undefined, context: string): TopTxn {
    if (raw === undefined || typeof raw.index !== "number" || typeof raw.date !== "string" || typeof raw.description !== "string") {
        throw new ApiShapeError(`${context}: missing index/date/description`);
    }
    return Object.freeze({
        index: raw.index,
        date: raw.date as ISODate,
        description: raw.description,
        amount: decodeDec(raw.amount, `${context} amount`),
    });
}

// ---------------------------------------------------------------------------
// SubscriptionsReport (recurring monthly / annual charges)
// ---------------------------------------------------------------------------

function decodeCadence(raw: string | undefined, context: string): Cadence {
    if (raw === "monthly" || raw === "annual") return raw;
    throw new ApiShapeError(`${context}: unknown cadence ${JSON.stringify(raw)}`);
}

function decodeSubscription(raw: RawSubscription | undefined, context: string): Subscription {
    if (
        raw === undefined ||
        typeof raw.payee !== "string" ||
        typeof raw.occurrences !== "number" ||
        typeof raw.firstSeen !== "string" ||
        typeof raw.lastSeen !== "string" ||
        typeof raw.nextExpected !== "string"
    ) {
        throw new ApiShapeError(`${context}: missing payee/occurrences/dates`);
    }
    return Object.freeze({
        payee: raw.payee,
        cadence: decodeCadence(raw.cadence, context),
        typicalAmount: decodeDec(raw.typicalAmount, `${context} typicalAmount`),
        annualizedCost: decodeDec(raw.annualizedCost, `${context} annualizedCost`),
        occurrences: raw.occurrences,
        firstSeen: raw.firstSeen as ISODate,
        lastSeen: raw.lastSeen as ISODate,
        nextExpected: raw.nextExpected as ISODate,
        accounts: frozen(decodeStrings(raw.accounts, `${context} accounts`)),
        manual: raw.manual === true,
    });
}

export function decodeSubscriptionsReport(raw: unknown): SubscriptionsReport {
    const report = raw as RawSubscriptionsReport;
    if (typeof report !== "object" || report === null || typeof report.asOf !== "string" || typeof report.lookbackStart !== "string") {
        throw new ApiShapeError("subscriptions report: expected asOf/lookbackStart");
    }
    return Object.freeze({
        asOf: report.asOf as ISODate,
        lookbackStart: report.lookbackStart as ISODate,
        monthly: frozen((report.monthly ?? []).map((row, i) => decodeSubscription(row, `subscriptions monthly[${i}]`))),
        annual: frozen((report.annual ?? []).map((row, i) => decodeSubscription(row, `subscriptions annual[${i}]`))),
    });
}

export function decodeInsightsReport(raw: unknown): InsightsReport {
    const report = raw as RawInsightsReport;
    if (typeof report !== "object" || report === null || typeof report.base !== "string") {
        throw new ApiShapeError("insights report: expected a base commodity");
    }
    return Object.freeze({
        period: decodeInsightsPeriod(report.period, "insights period"),
        base: report.base,
        journalStart: typeof report.journalStart === "string" ? (report.journalStart as ISODate) : null,
        revenue: decodeMetricDelta(report.revenue, "insights revenue"),
        expenses: decodeMetricDelta(report.expenses, "insights expenses"),
        netWorth: decodeMetricDelta(report.netWorth, "insights netWorth"),
        costOfLiving: decodeCostOfLiving(report.costOfLiving, "insights costOfLiving"),
        investment: decodeInvestmentPerf(report.investment, "insights investment"),
        cashBalance: decodeMetricDelta(report.cashBalance, "insights cashBalance"),
        expenseChanges: frozen((report.expenseChanges ?? []).map((row, i) => decodeChangeRow(row, `insights expenseChanges[${i}]`))),
        revenueChanges: frozen((report.revenueChanges ?? []).map((row, i) => decodeChangeRow(row, `insights revenueChanges[${i}]`))),
        movers: frozen((report.movers ?? []).map((row, i) => decodeMoverRow(row, `insights movers[${i}]`))),
        topTxns: frozen((report.topTxns ?? []).map((row, i) => decodeTopTxn(row, `insights topTxns[${i}]`))),
    });
}

// ---------------------------------------------------------------------------
// CSV import rules: index, document, preview
// ---------------------------------------------------------------------------

function str(value: unknown, context: string): string {
    if (typeof value !== "string") throw new ApiShapeError(`${context}: expected a string`);
    return value;
}

function num(value: unknown, context: string): number {
    if (typeof value !== "number" || !Number.isFinite(value)) throw new ApiShapeError(`${context}: expected a number`);
    return value;
}

/** A `{value, itemId}` entry, or null when the file does not say. `read` narrows the value's own type. */
function decodePref<T>(raw: RawRulesPref | undefined, context: string, read: (value: unknown, context: string) => T): RulesPref<T> | null {
    if (raw === undefined || raw === null) return null;
    return Object.freeze({value: read(raw.value, `${context} value`), itemId: num(raw.itemId, `${context} itemId`)});
}

/** A valueless directive: presence IS the meaning, so the engine sends `true`. */
function decodeFlagPref(raw: RawRulesPref | undefined, context: string): RulesPref<boolean> | null {
    return decodePref(raw, context, (value, where) => {
        if (typeof value !== "boolean") throw new ApiShapeError(`${where}: expected a boolean`);
        return value;
    });
}

function decodeSourcePref(raw: RawRulesSource | undefined, context: string): RulesSourcePref | null {
    if (raw === undefined || raw === null) return null;
    return Object.freeze({
        value: str(raw.value, `${context} value`),
        // Absent would mean "we do not know whether hledger shells out", and the
        // safe reading of not-knowing is not `false` — but the field is
        // unconditional on the wire, so demand it rather than guess.
        executesShellCommand: typeof raw.executesShellCommand === "boolean" ? raw.executesShellCommand : true,
        itemId: num(raw.itemId, `${context} itemId`),
    });
}

function decodeFieldsPref(raw: RawRulesFieldsPref | undefined, context: string): RulesFieldsPref | null {
    if (raw === undefined || raw === null) return null;
    return Object.freeze({names: frozen(decodeStrings(raw.names, `${context} names`)), itemId: num(raw.itemId, `${context} itemId`)});
}

function decodeRulesSettings(raw: RawRulesSettings | undefined, context: string): RulesSettings {
    const settings = raw ?? {};
    return Object.freeze({
        source: decodeSourcePref(settings.source, `${context} source`),
        archive: decodeFlagPref(settings.archive, `${context} archive`),
        encoding: decodePref(settings.encoding, `${context} encoding`, str),
        separator: decodePref(settings.separator, `${context} separator`, str),
        decimalMark: decodePref(settings.decimalMark, `${context} decimalMark`, str),
        dateFormat: decodePref(settings.dateFormat, `${context} dateFormat`, str),
        timezone: decodePref(settings.timezone, `${context} timezone`, str),
        newestFirst: decodeFlagPref(settings.newestFirst, `${context} newestFirst`),
        intraDayReversed: decodeFlagPref(settings.intraDayReversed, `${context} intraDayReversed`),
        skip: decodePref(settings.skip, `${context} skip`, num),
        balanceType: decodePref(settings.balanceType, `${context} balanceType`, str),
        account1: decodePref(settings.account1, `${context} account1`, str),
        account2: decodePref(settings.account2, `${context} account2`, str),
        currency: decodePref(settings.currency, `${context} currency`, str),
        fields: decodeFieldsPref(settings.fields, `${context} fields`),
    });
}

const OPAQUE_REASONS: readonly OpaqueReason[] = [
    "ifTable",
    "combinedMatcher",
    "matchGroup",
    "commentLikeMatcher",
    "controlFlowInBlock",
    "unparsedBlockBody",
    "unparsedDirective",
    "unclassified",
];

function decodeOpaqueReason(raw: string | undefined, context: string): OpaqueReason {
    const found = OPAQUE_REASONS.find((reason) => reason === raw);
    if (found === undefined) throw new ApiShapeError(`${context}: unknown opaque reason ${JSON.stringify(raw)}`);
    return found;
}

function decodeLayout(raw: string | undefined, context: string): IfLayout {
    if (raw === "inline" || raw === "stacked") return raw;
    throw new ApiShapeError(`${context}: unknown if-block layout ${JSON.stringify(raw)}`);
}

function decodeMatcher(raw: RawRulesMatcher | undefined, context: string): RulesMatcher {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing matcher`);
    // An absent `field` is a WHOLE-RECORD matcher, not a missing one — the
    // engine omits the key for `MatchScope::WholeRecord`.
    return Object.freeze({field: raw.field === undefined ? null : str(raw.field, `${context} field`), pattern: str(raw.pattern, `${context} pattern`)});
}

/**
 * One item, by its `kind` tag.
 *
 * An unknown tag THROWS rather than degrading to a carry-through: the editor's
 * contract is that every item comes back in the save request, and an item whose
 * shape this decoder invented would be echoed as an id it has no right to claim.
 * Refusing to open the document is the honest failure.
 */
function decodeRulesItem(raw: RawRulesItem | undefined, context: string): RulesItem {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing item`);
    const base = {id: num(raw.id, `${context} id`), line: num(raw.line, `${context} line`), lines: num(raw.lines, `${context} lines`)};
    switch (raw.kind) {
        case "trivia":
            return Object.freeze({...base, kind: "trivia" as const, text: str(raw.text, `${context} text`), truncated: raw.truncated === true});
        case "directive":
            return Object.freeze({...base, kind: "directive" as const, name: str(raw.name, `${context} name`), value: str(raw.value, `${context} value`)});
        case "include":
            return Object.freeze({...base, kind: "include" as const, target: str(raw.target, `${context} target`)});
        case "fields":
            return Object.freeze({...base, kind: "fields" as const, names: frozen(decodeStrings(raw.names, `${context} names`))});
        case "assignment":
            return Object.freeze({...base, kind: "assignment" as const, field: str(raw.field, `${context} field`), value: str(raw.value, `${context} value`)});
        case "ifBlock":
            if (!Array.isArray(raw.matchers) || !Array.isArray(raw.assignments)) {
                throw new ApiShapeError(`${context}: an ifBlock needs matchers and assignments arrays`);
            }
            return Object.freeze({
                ...base,
                kind: "ifBlock" as const,
                layout: decodeLayout(raw.layout, context),
                matchers: frozen(raw.matchers.map((matcher, i) => decodeMatcher(matcher, `${context} matchers[${i}]`))),
                assignments: frozen(
                    raw.assignments.map((assignment, i) =>
                        Object.freeze({
                            field: str(assignment?.field, `${context} assignments[${i}] field`),
                            value: str(assignment?.value, `${context} assignments[${i}] value`),
                        })
                    )
                ),
            });
        case "opaque":
            return Object.freeze({
                ...base,
                kind: "opaque" as const,
                reason: decodeOpaqueReason(raw.reason, context),
                label: str(raw.label, `${context} label`),
                text: str(raw.text, `${context} text`),
                truncated: raw.truncated === true,
            });
        default:
            throw new ApiShapeError(`${context}: unknown item kind ${JSON.stringify(raw.kind)}`);
    }
}

function decodeRulesWarning(raw: RawRulesWarning | undefined, context: string): RulesWarning {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing warning`);
    return Object.freeze({
        itemId: raw.itemId === undefined ? null : num(raw.itemId, `${context} itemId`),
        line: num(raw.line, `${context} line`),
        message: str(raw.message, `${context} message`),
    });
}

/** `GET /api/rules` → the discovery listing. */
export function decodeRulesIndex(raw: unknown): RulesIndex {
    const index = raw as RawRulesIndex;
    if (typeof index !== "object" || index === null || !Array.isArray(index.files)) {
        throw new ApiShapeError("rules index: expected a files array");
    }
    return Object.freeze({
        rootLabel: str(index.rootLabel, "rules index rootLabel"),
        editable: index.editable === true,
        truncated: index.truncated === true,
        files: frozen(
            index.files.map((file, i) => {
                const context = `rules file #${i}`;
                return Object.freeze({
                    id: str(file?.id, `${context} id`),
                    label: str(file?.label, `${context} label`),
                    revision: str(file?.revision, `${context} revision`),
                    sizeBytes: num(file?.sizeBytes, `${context} sizeBytes`),
                    parsed: file?.parsed === true,
                    // account1/account2 are omitted for a file that declares
                    // neither, which is a different fact from "declares an empty one".
                    account1: file?.account1 === undefined ? null : str(file.account1, `${context} account1`),
                    account2: file?.account2 === undefined ? null : str(file.account2, `${context} account2`),
                    ifBlockCount: num(file?.ifBlockCount, `${context} ifBlockCount`),
                    editableBlockCount: num(file?.editableBlockCount, `${context} editableBlockCount`),
                    opaqueItemCount: num(file?.opaqueItemCount, `${context} opaqueItemCount`),
                    warnings: frozen(decodeStrings(file?.warnings, `${context} warnings`)),
                });
            })
        ),
        warnings: frozen(decodeStrings(index.warnings, "rules index warnings")),
    });
}

/** `GET /api/rules/{*id}` (and the body a save answers with) → one parsed document. */
export function decodeRulesDoc(raw: unknown): RulesDocument {
    const doc = raw as RawRulesDoc;
    if (typeof doc !== "object" || doc === null || !Array.isArray(doc.items)) {
        throw new ApiShapeError("rules document: expected an items array");
    }
    if (doc.newline !== "lf" && doc.newline !== "crlf") {
        throw new ApiShapeError(`rules document: unknown newline ${JSON.stringify(doc.newline)}`);
    }
    return Object.freeze({
        id: str(doc.id, "rules document id"),
        label: str(doc.label, "rules document label"),
        revision: str(doc.revision, "rules document revision"),
        editable: doc.editable === true,
        newline: doc.newline,
        settings: decodeRulesSettings(doc.settings, "rules settings"),
        items: frozen(doc.items.map((item, i) => decodeRulesItem(item, `rules item #${i}`))),
        warnings: frozen((doc.warnings ?? []).map((warning, i) => decodeRulesWarning(warning, `rules warning #${i}`))),
    });
}

const PREVIEW_REASONS: readonly PreviewUnavailable[] = [
    "noDataFile",
    "sourceIsCommand",
    "sourceOutsideRoot",
    "notRegularFile",
    "unreadable",
    "notUtf8",
    "empty",
];

/**
 * `GET /api/rules-preview/{*id}` → the first few rows of the described data file.
 *
 * An unavailable preview is a VALUE, not an error: "your `source` is a shell
 * command we will not run" is information the mapping panel shows.
 */
export function decodeRulesPreview(raw: unknown): RulesPreview {
    const preview = raw as RawRulesPreview;
    if (typeof preview !== "object" || preview === null || !Array.isArray(preview.rows)) {
        throw new ApiShapeError("rules preview: expected a rows array");
    }
    const reason = PREVIEW_REASONS.find((candidate) => candidate === preview.reason) ?? null;
    if (preview.reason !== undefined && reason === null) {
        throw new ApiShapeError(`rules preview: unknown reason ${JSON.stringify(preview.reason)}`);
    }
    return Object.freeze({
        available: preview.available === true,
        reason,
        dataLabel: preview.dataLabel === undefined ? null : str(preview.dataLabel, "rules preview dataLabel"),
        separator: str(preview.separator, "rules preview separator"),
        header: preview.header === undefined ? null : frozen(decodeStrings(preview.header, "rules preview header")),
        rows: frozen(
            preview.rows.map((row, i) => {
                if (!Array.isArray(row)) throw new ApiShapeError(`rules preview rows[${i}]: expected an array`);
                return frozen(decodeStrings(row, `rules preview rows[${i}]`));
            })
        ),
        columns: num(preview.columns, "rules preview columns"),
        truncated: preview.truncated === true,
    });
}
