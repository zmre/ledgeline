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

import type {Dec, MixedAmount} from "$lib/domain/money";
import type {ISODate} from "$lib/domain/types";
import type {Holding, HoldingsPoint, HoldingsReport, HoldingsSeries, HoldingsWarning} from "$lib/holdings/types";
import type {PeriodReport, ReportRow, Section, SectionedReport} from "$lib/reports/types";
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

/** {"<commodity>": Dec, …} → a plain Map (domain MixedAmount). Zero commodities are already dropped server-side. */
function decodeMixed(raw: RawMixed | undefined, context: string): MixedAmount {
    const out: MixedAmount = new Map();
    if (raw === undefined || raw === null) return out;
    if (typeof raw !== "object") throw new ApiShapeError(`${context}: expected an object of commodity → decimal`);
    for (const [commodity, value] of Object.entries(raw)) {
        out.set(commodity, decodeDec(value, `${context} "${commodity}"`));
    }
    return out;
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
