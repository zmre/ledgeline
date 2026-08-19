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
    AliasEffect,
    AliasEntry,
    AliasFile,
    AliasListing,
    AliasLock,
    AliasRefusal,
    AliasRename,
    BalanceCheck,
    CandidateSignals,
    CliParity,
    CommitResult,
    ConfRefusalReason,
    ConfWritten,
    ConvertNote,
    DryRunResult,
    GitReport,
    HledgerStatus,
    HledgerUnavailableReason,
    ImportCapabilities,
    JournalTarget,
    OrderingReport,
    Prefs,
    ProposedTxn,
    RulesCandidate,
    SortMove,
    SortResult,
    StagedFile,
    StageDefaults,
    StagePreview,
    StatementMeta,
} from "$lib/imports/importTypes";
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
import type {
    Amounts,
    BalanceSheetReport,
    BsGroup,
    BsSection,
    BsSectionKind,
    BsValuation,
    BudgetCell,
    BudgetReport,
    BudgetRow,
    DateRange,
    GroupSource,
    IncomeStatementReport,
    IsGroup,
    IsRow,
    IsSection,
    IsSectionKind,
    IsSubtotal,
    IsSubtotalKind,
    PeriodReport,
    ReportRow,
    Section,
    SectionedReport,
} from "$lib/reports/types";
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

interface RawBsGroup {
    name?: string;
    source?: string;
    rows?: RawReportRow[];
    total?: RawMixed;
}

interface RawBsSection {
    kind?: string;
    title?: string;
    groups?: RawBsGroup[];
    total?: RawMixed;
}

interface RawBalanceSheetReport {
    asOf?: string;
    base?: string | null;
    value?: string;
    sections?: RawBsSection[];
    netWorth?: RawMixed;
    check?: RawMixed;
    balanced?: boolean;
    meta?: RawReportMeta | null;
}

/** `{current, prior?}` — `prior` is ABSENT, not null, when `compare=none`. */
interface RawAmounts {
    current?: RawMixed;
    prior?: RawMixed | null;
}

interface RawIsRow {
    account?: string;
    depth?: number;
    amounts?: RawAmounts;
}

interface RawIsGroup {
    name?: string;
    source?: string;
    rows?: RawIsRow[];
    total?: RawAmounts;
}

interface RawIsSubtotal {
    kind?: string;
    label?: string;
    total?: RawAmounts;
}

interface RawIsSection {
    kind?: string;
    title?: string;
    groups?: RawIsGroup[];
    total?: RawAmounts;
    trailing?: RawIsSubtotal[];
}

interface RawDateRange {
    from?: string;
    to?: string;
}

interface RawIncomeStatementReport {
    from?: string;
    to?: string;
    prior?: RawDateRange | null;
    base?: string | null;
    value?: string;
    sections?: RawIsSection[];
    netIncome?: RawAmounts;
    multiStep?: boolean;
    meta?: RawReportMeta | null;
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

// --- New Transactions import flow (import_api.rs) ---------------------------
// "The lane E wire contract" in plans/11-enhanced-import.md. Same posture as the
// rules wire above: an OMITTED optional key is the fact "there is none", so it
// decodes to null; the unconditional keys (a stage id, a dry run's `ok`, a
// commit's `csvWritten`) are demanded.

interface RawHledgerStatus {
    available?: boolean;
    version?: string;
    reason?: string;
    message?: string;
}

interface RawJournalTarget {
    id?: string;
    label?: string;
    txnCount?: number;
    lastTxnDate?: string | null;
    isRoot?: boolean;
    writable?: boolean;
}

interface RawGitCapability {
    available?: boolean;
    autocommit?: boolean;
}

interface RawImportCapabilities {
    hledger?: RawHledgerStatus;
    formats?: unknown[];
    journals?: RawJournalTarget[];
    git?: RawGitCapability;
    aliases?: RawAlias[];
    editable?: boolean;
}

interface RawAlias {
    journalId?: string;
    index?: number;
    line?: number;
    pattern?: string;
    replacement?: string;
    regex?: boolean;
    forwarded?: boolean;
    refusal?: string;
    refusalMessage?: string;
    editable?: boolean;
    lock?: string;
    lockMessage?: string;
}

interface RawAliasFile {
    journalId?: string;
    label?: string;
    revision?: string;
    writable?: boolean;
    aliases?: RawAlias[];
}

interface RawAliasListing {
    editable?: boolean;
    files?: RawAliasFile[];
}

interface RawAliasEffect {
    forwarded?: number;
    renames?: {from?: string; to?: string}[];
    cli?: RawCliParity;
}

interface RawCliParity {
    matches?: boolean;
    differences?: {from?: string; to?: string}[];
    confPath?: string | null;
    confOutside?: boolean;
    confHijackedBy?: string | null;
    additions?: unknown[];
    refusals?: RawConfRefusal[];
    revision?: string;
    writable?: boolean;
}

interface RawConfRefusal {
    pattern?: string;
    replacement?: string;
    reason?: string;
    message?: string;
}

interface RawConfWritten {
    confPath?: string;
    created?: boolean;
    added?: unknown[];
    revision?: string;
}

/** One `ConvertNote`, flattened: the `kind` tag sits beside that variant's own fields. */
interface RawConvertNote {
    kind?: string;
    name?: string;
    of?: number;
    count?: number;
    label?: string;
    delimiter?: string;
    lines?: number;
    expected?: string;
    computed?: string;
}

interface RawStagePreview {
    header?: unknown[];
    rows?: unknown[];
    rowCount?: number;
    truncated?: boolean;
}

interface RawStatementMeta {
    accountHint?: string;
    currency?: string;
    ledgerBalance?: string;
    balanceAsOf?: string;
}

interface RawProposedTxn {
    date?: string;
    description?: string;
    postings?: unknown[];
}

interface RawCandidateSignals {
    txns?: number;
    postings?: number;
    amountlessPostings?: number;
    bareCommodityAmounts?: number;
    unknownAccounts?: number;
    emptyDescriptions?: number;
    columnCountMatches?: boolean;
    headerMatchesSource?: boolean;
}

interface RawRulesCandidate {
    id?: string;
    label?: string;
    score?: number;
    signals?: RawCandidateSignals;
    sample?: RawProposedTxn[];
    account1?: string | null;
    account2?: string | null;
}

interface RawStageDefaults {
    csvPath?: string;
    journalId?: string | null;
}

interface RawStagedFile {
    stageId?: string;
    format?: string;
    preview?: RawStagePreview;
    statement?: RawStatementMeta | null;
    notes?: RawConvertNote[];
    candidates?: RawRulesCandidate[];
    defaults?: RawStageDefaults;
}

interface RawSkippedRows {
    olderThan?: string;
    count?: number;
}

interface RawBalanceCheck {
    statement?: string;
    computed?: string;
    matches?: boolean;
    difference?: string | null;
}

interface RawDryRun {
    ok?: boolean;
    entries?: string;
    count?: number;
    status?: string;
    skipped?: RawSkippedRows | null;
    balance?: RawBalanceCheck | null;
    aliases?: RawAliasEffect | null;
    blockedByGit?: unknown[];
    stderr?: string;
}

interface RawSortMove {
    date?: string;
    description?: string;
    fromLine?: number;
    toLine?: number;
}

interface RawOrdering {
    inOrder?: boolean;
    moves?: RawSortMove[];
}

interface RawGitReport {
    committed?: boolean;
    paths?: unknown[];
    skipped?: unknown[];
}

interface RawCommitResult {
    csvWritten?: string;
    journalWritten?: string | null;
    imported?: number;
    ordering?: RawOrdering;
    git?: RawGitReport | null;
}

interface RawSortResult {
    moved?: number;
}

interface RawPrefs {
    hledgerPath?: string | null;
    gitAutocommit?: boolean | null;
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
// BalanceSheetReport (grouped, market-valued — plans/12)
// ---------------------------------------------------------------------------

const BS_SECTION_KINDS: readonly BsSectionKind[] = ["assets", "liabilities", "equity"];
/** Shared by both statements — one resolver on the Rust side, one vocabulary here. */
const GROUP_SOURCES: readonly GroupSource[] = ["tag", "type", "commodity", "segment", "computed"];
const BS_VALUATIONS: readonly BsValuation[] = ["market", "cost", "none"];

/**
 * A closed enum off the wire.
 *
 * THROWS on an unknown member rather than falling back, exactly like
 * `decodeChangeKind`/`decodeCadence`: `source` drives a badge and `kind` drives
 * which of the three boxes a section lands in, so an invented default would put
 * real balances under the wrong heading.
 */
function decodeEnum<T extends string>(allowed: readonly T[], raw: string | undefined, context: string): T {
    const found = allowed.find((member) => member === raw);
    if (found === undefined) throw new ApiShapeError(`${context}: expected one of ${allowed.join("/")}, got ${JSON.stringify(raw)}`);
    return found;
}

/** A REQUIRED boolean. Absent is a broken contract, not a `false` (DRY-3). */
function decodeBoolean(raw: boolean | undefined, context: string): boolean {
    if (typeof raw !== "boolean") throw new ApiShapeError(`${context}: expected a boolean, got ${JSON.stringify(raw)}`);
    return raw;
}

function decodeBsGroup(raw: RawBsGroup | undefined, context: string): BsGroup {
    // `rows` is demanded even though a computed group's is always `[]`: absent
    // and empty are different facts, and only one of them is safe to render as
    // "this group has no accounts" (DRY-3, the same reasoning as decodeMixed).
    if (raw === undefined || typeof raw.name !== "string" || !Array.isArray(raw.rows)) {
        throw new ApiShapeError(`${context}: missing name/rows`);
    }
    return Object.freeze({
        name: raw.name,
        source: decodeEnum(GROUP_SOURCES, raw.source, `${context} source`),
        rows: frozen(raw.rows.map((row, i) => decodeReportRow(row, `${context} row #${i}`))),
        total: decodeMixed(raw.total, `${context} total`),
    });
}

function decodeBsSection(raw: RawBsSection | undefined, context: string): BsSection {
    if (raw === undefined || typeof raw.title !== "string" || !Array.isArray(raw.groups)) {
        throw new ApiShapeError(`${context}: missing title/groups`);
    }
    return Object.freeze({
        kind: decodeEnum(BS_SECTION_KINDS, raw.kind, `${context} kind`),
        title: raw.title,
        groups: frozen(raw.groups.map((group, i) => decodeBsGroup(group, `${context} group #${i}`))),
        total: decodeMixed(raw.total, `${context} total`),
    });
}

/**
 * `GET /api/reports/balancesheet/grouped` → the three-box balance sheet.
 *
 * `base` is `Option<Commodity>` on the Rust side and arrives as `null` for a
 * journal with no base commodity — a fact the UI has to render differently
 * (there is no dominant figure to promote), so it is kept as null rather than
 * coerced to "".
 *
 * `kind: "balanceSheet"` is added HERE and is not on the wire: this report and
 * `SectionedReport` both carry a `sections` array, and the page picks its
 * renderer and its export builder off that tag.
 */
export function decodeBalanceSheetReport(raw: unknown): BalanceSheetReport {
    const report = raw as RawBalanceSheetReport;
    if (typeof report !== "object" || report === null || typeof report.asOf !== "string" || !Array.isArray(report.sections)) {
        throw new ApiShapeError("balance sheet: expected asOf and a sections array");
    }
    if (report.base !== null && report.base !== undefined && typeof report.base !== "string") {
        throw new ApiShapeError("balance sheet base: expected a commodity string or null");
    }
    const out: BalanceSheetReport = {
        kind: "balanceSheet",
        asOf: report.asOf as ISODate,
        base: typeof report.base === "string" ? report.base : null,
        value: decodeEnum(BS_VALUATIONS, report.value, "balance sheet value"),
        sections: frozen(report.sections.map((section, i) => decodeBsSection(section, `balance sheet section #${i}`))),
        netWorth: decodeMixed(report.netWorth, "balance sheet netWorth"),
        // `{}` is the balanced case and IS sent; absent is a broken contract.
        // Defaulting it to empty would turn "the engine stopped answering the
        // integrity question" into "the journal balances", which is the one
        // wrong answer this field must never give.
        check: decodeMixed(report.check, "balance sheet check"),
        // Likewise required, and for the same reason: the engine derives it from
        // the precisions the journal writes, so it is not something the client
        // can reconstruct. Defaulting a missing one to `true` would silently
        // suppress a real imbalance.
        balanced: decodeBoolean(report.balanced, "balance sheet balanced"),
    };
    // Omitted (`skip_serializing_if = "Option::is_none"`) when nothing is unpriced.
    if (report.meta !== undefined && report.meta !== null) {
        out.meta = Object.freeze({unpriced: frozen(decodeStrings(report.meta.unpriced, "balance sheet meta.unpriced"))});
    }
    return Object.freeze(out);
}

// ---------------------------------------------------------------------------
// IncomeStatementReport (grouped, adaptive GAAP — plans/13)
// ---------------------------------------------------------------------------

const IS_SECTION_KINDS: readonly IsSectionKind[] = ["revenue", "cogs", "opex", "depreciation", "interest", "tax", "other"];
const IS_SUBTOTAL_KINDS: readonly IsSubtotalKind[] = ["grossProfit", "ebitda", "operatingIncome", "pretaxIncome"];

/**
 * `{current, prior?}`, checked against whether the REPORT is comparing.
 *
 * That cross-check is the whole point of taking `comparing` as an argument.
 * `prior` is an optional key, and an optional key is exactly where a renamed
 * Rust field disappears without trace: the column would simply stop rendering,
 * which reads as "this report has no comparison" rather than as a bug. So:
 *
 *   - comparing (`report.prior` is a window) → `prior` MUST decode. Absent throws.
 *   - not comparing → `prior` must be absent. `null` is tolerated as absent,
 *     because that is what serde emits for an `Option` without
 *     `skip_serializing_if` and it means the identical thing; an actual amount
 *     throws, since there would be no window to label the column with.
 *
 * `current` is unconditionally required via `decodeMixed`, so a missing figure
 * can never become `$0.00` (DRY-3).
 */
function decodeAmounts(raw: RawAmounts | undefined, context: string, comparing: boolean): Amounts {
    if (raw === undefined || raw === null || typeof raw !== "object") {
        throw new ApiShapeError(`${context}: missing amounts (expected {current, prior?})`);
    }
    const out: Amounts = {current: decodeMixed(raw.current, `${context} current`)};
    if (comparing) {
        out.prior = decodeMixed(raw.prior ?? undefined, `${context} prior`);
    } else if (raw.prior !== undefined && raw.prior !== null) {
        throw new ApiShapeError(`${context} prior: sent a prior figure on a report with no prior period`);
    }
    return Object.freeze(out);
}

function decodeIsRow(raw: RawIsRow | undefined, context: string, comparing: boolean): IsRow {
    if (raw === undefined || typeof raw.account !== "string" || typeof raw.depth !== "number") {
        throw new ApiShapeError(`${context}: missing account/depth`);
    }
    return Object.freeze({account: raw.account, depth: raw.depth, amounts: decodeAmounts(raw.amounts, `${context} amounts`, comparing)});
}

function decodeIsGroup(raw: RawIsGroup | undefined, context: string, comparing: boolean): IsGroup {
    // `rows` demanded even when empty, for the reason `decodeBsGroup` gives:
    // absent and empty are different facts and only one is safe to render.
    if (raw === undefined || typeof raw.name !== "string" || !Array.isArray(raw.rows)) {
        throw new ApiShapeError(`${context}: missing name/rows`);
    }
    return Object.freeze({
        name: raw.name,
        source: decodeEnum(GROUP_SOURCES, raw.source, `${context} source`),
        rows: frozen(raw.rows.map((row, i) => decodeIsRow(row, `${context} row #${i}`, comparing))),
        total: decodeAmounts(raw.total, `${context} total`, comparing),
    });
}

function decodeIsSubtotal(raw: RawIsSubtotal | undefined, context: string, comparing: boolean): IsSubtotal {
    if (raw === undefined || typeof raw.label !== "string") throw new ApiShapeError(`${context}: missing label`);
    return Object.freeze({
        kind: decodeEnum(IS_SUBTOTAL_KINDS, raw.kind, `${context} kind`),
        label: raw.label,
        total: decodeAmounts(raw.total, `${context} total`, comparing),
    });
}

function decodeIsSection(raw: RawIsSection | undefined, context: string, comparing: boolean): IsSection {
    // `trailing` is required for the same reason `rows` is: "this box has no
    // ladder line under it" is a real answer the engine computes from its
    // guards, and it must not be reachable by the field going missing.
    if (raw === undefined || typeof raw.title !== "string" || !Array.isArray(raw.groups) || !Array.isArray(raw.trailing)) {
        throw new ApiShapeError(`${context}: missing title/groups/trailing`);
    }
    return Object.freeze({
        kind: decodeEnum(IS_SECTION_KINDS, raw.kind, `${context} kind`),
        title: raw.title,
        groups: frozen(raw.groups.map((group, i) => decodeIsGroup(group, `${context} group #${i}`, comparing))),
        total: decodeAmounts(raw.total, `${context} total`, comparing),
        trailing: frozen(raw.trailing.map((subtotal, i) => decodeIsSubtotal(subtotal, `${context} subtotal #${i}`, comparing))),
    });
}

/** `{from, to}` — the window the prior figures cover. Both dates required. */
function decodeDateRange(raw: RawDateRange, context: string): DateRange {
    if (typeof raw.from !== "string" || typeof raw.to !== "string") {
        throw new ApiShapeError(`${context}: expected {from, to} ISO dates`);
    }
    return Object.freeze({from: raw.from as ISODate, to: raw.to as ISODate});
}

/**
 * `GET /api/reports/incomestatement/grouped` → the grouped income statement.
 *
 * `kind: "incomeStatement"` is added HERE and is not on the wire. Three report
 * types now carry a `sections` array, so this tag is what lets the page pick a
 * renderer and a workbook builder at all (FE-1).
 *
 * `prior` does double duty: it is the window the comparison column covers AND
 * the switch that says every `Amounts` in the tree must carry one. Reading it
 * first, and passing `comparing` down, is what turns an optional key into a
 * checked one — see `decodeAmounts`.
 */
export function decodeIncomeStatementReport(raw: unknown): IncomeStatementReport {
    const report = raw as RawIncomeStatementReport;
    if (typeof report !== "object" || report === null || typeof report.from !== "string" || typeof report.to !== "string" || !Array.isArray(report.sections)) {
        throw new ApiShapeError("income statement: expected from, to and a sections array");
    }
    if (report.base !== null && report.base !== undefined && typeof report.base !== "string") {
        throw new ApiShapeError("income statement base: expected a commodity string or null");
    }
    // Omitted (or null) for `compare=none`; a window for `compare=previous`.
    const prior = report.prior === undefined || report.prior === null ? null : decodeDateRange(report.prior, "income statement prior");
    const comparing = prior !== null;
    const out: IncomeStatementReport = {
        kind: "incomeStatement",
        from: report.from as ISODate,
        to: report.to as ISODate,
        prior,
        base: typeof report.base === "string" ? report.base : null,
        value: decodeEnum(BS_VALUATIONS, report.value, "income statement value"),
        sections: frozen(report.sections.map((section, i) => decodeIsSection(section, `income statement section #${i}`, comparing))),
        netIncome: decodeAmounts(report.netIncome, "income statement netIncome", comparing),
        // Required, not defaulted: it decides whether `opex` reads "Expenses" or
        // "Operating expenses", so a missing one would silently relabel a box.
        multiStep: decodeBoolean(report.multiStep, "income statement multiStep"),
    };
    // Omitted (`skip_serializing_if = "Option::is_none"`) when nothing is unpriced.
    if (report.meta !== undefined && report.meta !== null) {
        out.meta = Object.freeze({unpriced: frozen(decodeStrings(report.meta.unpriced, "income statement meta.unpriced"))});
    }
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

// ---------------------------------------------------------------------------
// New Transactions: capabilities, stage, dry run, commit, sort, prefs
// ---------------------------------------------------------------------------

/** An optional string key: absent or explicitly null is the fact "there is none". */
function optStr(value: unknown, context: string): string | null {
    return value === undefined || value === null ? null : str(value, context);
}

/** An optional number key. Same null-is-a-fact reading as [`optStr`]. */
function optNum(value: unknown, context: string): number | null {
    return value === undefined || value === null ? null : num(value, context);
}

/** An optional boolean key; anything present must actually BE a boolean. */
function optBool(value: unknown, context: string): boolean | null {
    if (value === undefined || value === null) return null;
    if (typeof value !== "boolean") throw new ApiShapeError(`${context}: expected a boolean`);
    return value;
}

/** A rectangular table of strings — a preview's rows. */
function decodeRows(raw: unknown[], context: string): string[][] {
    return raw.map((row, i) => {
        if (!Array.isArray(row)) throw new ApiShapeError(`${context}[${i}]: expected an array`);
        return frozen(decodeStrings(row, `${context}[${i}]`));
    });
}

const HLEDGER_REASONS: readonly HledgerUnavailableReason[] = ["notFound", "tooOld", "unrunnable", "timedOut"];

/**
 * Whether the engine can run hledger.
 *
 * The unknown-`reason` case decodes to null rather than throwing, unlike every
 * other tagged union in this file. This response is precisely the one a broken
 * install depends on: refusing to decode it because a future engine grew a fifth
 * reason would replace an actionable banner ("hledger 1.31 is older than 1.40")
 * with a generic decode failure, which is the opposite of the point. `message`
 * is the engine's own sentence and always survives.
 */
function decodeHledgerStatus(raw: RawHledgerStatus | undefined, context: string): HledgerStatus {
    const status = raw ?? {};
    return Object.freeze({
        available: status.available === true,
        version: optStr(status.version, `${context} version`),
        reason: HLEDGER_REASONS.find((candidate) => candidate === status.reason) ?? null,
        message: optStr(status.message, `${context} message`),
    });
}

function decodeJournalTarget(raw: RawJournalTarget | undefined, context: string): JournalTarget {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing journal target`);
    return Object.freeze({
        id: str(raw.id, `${context} id`),
        label: str(raw.label, `${context} label`),
        txnCount: num(raw.txnCount, `${context} txnCount`),
        lastTxnDate: optStr(raw.lastTxnDate, `${context} lastTxnDate`),
        isRoot: raw.isRoot === true,
        // Absent must NOT read as writable: offering an unwritable target as
        // writable is how an import lands on a symlink outside the include root.
        writable: raw.writable === true,
    });
}

/** `GET /api/import/capabilities` → what the New Transactions screen may offer. */
export function decodeImportCapabilities(raw: unknown): ImportCapabilities {
    const caps = raw as RawImportCapabilities;
    if (typeof caps !== "object" || caps === null || !Array.isArray(caps.journals)) {
        throw new ApiShapeError("import capabilities: expected a journals array");
    }
    return Object.freeze({
        hledger: decodeHledgerStatus(caps.hledger, "import capabilities hledger"),
        formats: frozen(decodeStrings(caps.formats, "import capabilities formats")),
        journals: frozen(caps.journals.map((target, i) => decodeJournalTarget(target, `import capabilities journals[${i}]`))),
        git: Object.freeze({available: caps.git?.available === true, autocommit: caps.git?.autocommit === true}),
        // Absent reads as "this journal declares none", which is true of almost
        // every journal and is exactly what an older engine meant by omitting it.
        aliases: frozen((caps.aliases ?? []).map((alias, i) => decodeAlias(alias, `import capabilities aliases[${i}]`))),
        editable: caps.editable === true,
    });
}

/** Values the engine may send; anything else reads as "no reason given". */
const ALIAS_REFUSALS: readonly AliasRefusal[] = ["scoped", "empty", "control", "tooLong", "limit", "stale"];
const ALIAS_LOCKS: readonly AliasLock[] = ["commentLike", "empty", "delimiter", "control", "tooLong"];

function decodeAlias(raw: RawAlias | undefined, context: string): AliasEntry {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing alias`);
    return Object.freeze({
        journalId: str(raw.journalId, `${context} journalId`),
        index: num(raw.index, `${context} index`),
        line: num(raw.line, `${context} line`),
        pattern: str(raw.pattern, `${context} pattern`),
        replacement: str(raw.replacement, `${context} replacement`),
        regex: raw.regex === true,
        // Absent must NOT read as forwarded: claiming hledger saw a mapping it
        // did not is the one wrong answer this whole surface exists to prevent.
        forwarded: raw.forwarded === true,
        refusal: ALIAS_REFUSALS.find((candidate) => candidate === raw.refusal) ?? null,
        refusalMessage: optStr(raw.refusalMessage, `${context} refusalMessage`),
        // Same argument, the other way: absent reads as NOT editable, so an
        // engine that stops modelling a shape cannot have it rewritten anyway.
        editable: raw.editable === true,
        lock: ALIAS_LOCKS.find((candidate) => candidate === raw.lock) ?? null,
        lockMessage: optStr(raw.lockMessage, `${context} lockMessage`),
    });
}

function decodeAliasFile(raw: RawAliasFile | undefined, context: string): AliasFile {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing file`);
    return Object.freeze({
        journalId: str(raw.journalId, `${context} journalId`),
        label: str(raw.label, `${context} label`),
        revision: str(raw.revision, `${context} revision`),
        writable: raw.writable === true,
        aliases: frozen((raw.aliases ?? []).map((alias, i) => decodeAlias(alias, `${context} aliases[${i}]`))),
    });
}

/** `GET /api/aliases` → every alias the open journal declares. */
export function decodeAliasListing(raw: unknown): AliasListing {
    const listing = raw as RawAliasListing;
    if (typeof listing !== "object" || listing === null || !Array.isArray(listing.files)) {
        throw new ApiShapeError("alias listing: expected a files array");
    }
    return Object.freeze({
        editable: listing.editable === true,
        files: frozen(listing.files.map((file, i) => decodeAliasFile(file, `alias listing files[${i}]`))),
    });
}

/** `PUT /api/aliases/{*id}` → the file it just wrote, at its new revision. */
export function decodeAliasFileResponse(raw: unknown): AliasFile {
    return decodeAliasFile(raw as RawAliasFile, "alias save");
}

/**
 * What the journal's aliases did to a dry run, or null when none is in force.
 *
 * Null and `{forwarded: 0, renames: []}` are different facts — "no alias
 * applies to this journal" versus "aliases applied and changed nothing here" —
 * and the screen says different things about them, so the absence is preserved
 * rather than normalised into an empty object.
 */
function decodeAliasEffect(raw: RawAliasEffect | null | undefined, context: string): AliasEffect | null {
    if (raw === undefined || raw === null) return null;
    return Object.freeze({
        forwarded: num(raw.forwarded, `${context} forwarded`),
        renames: decodeRenames(raw.renames, `${context} renames`),
        cli: decodeCliParity(raw.cli, `${context} cli`),
    });
}

function decodeRenames(raw: {from?: string; to?: string}[] | undefined, context: string): readonly AliasRename[] {
    return frozen(
        (raw ?? []).map((rename, i) => Object.freeze({from: str(rename?.from, `${context}[${i}] from`), to: str(rename?.to, `${context}[${i}] to`)}))
    );
}

/**
 * Whether a command-line `hledger import` would agree with this one.
 *
 * An ABSENT `cli` decodes to "it matches", not to a thrown shape error. This is
 * an advisory section — the same call {@link decodeConvertNote} makes — and an
 * engine older than this build genuinely has nothing to say about parity, so the
 * quiet reading is the honest one. Claiming a divergence because a field was
 * missing would put a scary notice on a correct import.
 */
function decodeCliParity(raw: RawCliParity | null | undefined, context: string): CliParity {
    if (raw === undefined || raw === null) {
        return Object.freeze({
            matches: true,
            differences: frozen([]),
            confPath: null,
            confOutside: false,
            confHijackedBy: null,
            additions: frozen([]),
            refusals: frozen([]),
            revision: "",
            writable: false,
        });
    }
    return Object.freeze({
        matches: raw.matches === true,
        differences: decodeRenames(raw.differences, `${context} differences`),
        confPath: raw.confPath ?? null,
        confOutside: raw.confOutside === true,
        confHijackedBy: raw.confHijackedBy ?? null,
        additions: frozen(decodeStrings(raw.additions, `${context} additions`)),
        refusals: frozen(
            (raw.refusals ?? []).map((refusal, i) =>
                Object.freeze({
                    pattern: str(refusal?.pattern, `${context} refusals[${i}] pattern`),
                    replacement: str(refusal?.replacement, `${context} refusals[${i}] replacement`),
                    // An unknown reason decodes to null rather than throwing: the
                    // MESSAGE is the engine's own sentence and is what the screen
                    // shows, so a newer engine's new reason code must not cost the
                    // user the explanation that came with it.
                    reason: confRefusalReason(refusal?.reason),
                    message: str(refusal?.message, `${context} refusals[${i}] message`),
                })
            )
        ),
        revision: raw.revision ?? "",
        writable: raw.writable === true,
    });
}

const CONF_REFUSAL_REASONS: readonly string[] = [
    "comment",
    "replacementWhitespace",
    "replacementBackslash",
    "patternBracket",
    "patternBackslash",
    "patternSlash",
];

function confRefusalReason(raw: string | undefined): ConfRefusalReason | null {
    return raw !== undefined && CONF_REFUSAL_REASONS.includes(raw) ? (raw as ConfRefusalReason) : null;
}

/** `POST /api/import/hledger-conf` — what the one-click command-line-parity fix wrote. */
export function decodeConfWritten(raw: unknown): ConfWritten {
    const written = raw as RawConfWritten;
    return Object.freeze({
        confPath: str(written?.confPath, "hledger.conf confPath"),
        created: written?.created === true,
        added: frozen(decodeStrings(written?.added, "hledger.conf added")),
        revision: str(written?.revision, "hledger.conf revision"),
    });
}

/**
 * One `ConvertNote`, or null when this build does not know its `kind`.
 *
 * Null rather than a throw: a note is advisory and carries no echo obligation,
 * so a newer engine's new variant must not cost the user the whole import. The
 * caller counts what it dropped (`StagedFile.unknownNoteCount`) so the screen can
 * still say something was said.
 */
function decodeConvertNote(raw: RawConvertNote | undefined, context: string): ConvertNote | null {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing note`);
    switch (raw.kind) {
        case "sheetChosen":
            return Object.freeze({kind: "sheetChosen" as const, name: str(raw.name, `${context} name`), of: num(raw.of, `${context} of`)});
        case "statementChosen":
            return Object.freeze({kind: "statementChosen" as const, of: num(raw.of, `${context} of`)});
        case "datesFromSerial":
            return Object.freeze({kind: "datesFromSerial" as const, count: num(raw.count, `${context} count`)});
        case "encodingGuessed":
            return Object.freeze({kind: "encodingGuessed" as const, label: str(raw.label, `${context} label`)});
        case "delimiterSniffed":
            return Object.freeze({kind: "delimiterSniffed" as const, delimiter: str(raw.delimiter, `${context} delimiter`)});
        case "preambleSkipped":
            return Object.freeze({kind: "preambleSkipped" as const, lines: num(raw.lines, `${context} lines`)});
        case "trailerSkipped":
            return Object.freeze({kind: "trailerSkipped" as const, lines: num(raw.lines, `${context} lines`)});
        case "blankRowsDropped":
            return Object.freeze({kind: "blankRowsDropped" as const, count: num(raw.count, `${context} count`)});
        case "raggedRows":
            return Object.freeze({kind: "raggedRows" as const, count: num(raw.count, `${context} count`)});
        case "balanceMismatch":
            return Object.freeze({
                kind: "balanceMismatch" as const,
                expected: str(raw.expected, `${context} expected`),
                computed: str(raw.computed, `${context} computed`),
            });
        default:
            return null;
    }
}

function decodeStagePreview(raw: RawStagePreview | undefined, context: string): StagePreview {
    if (raw === undefined || raw === null || !Array.isArray(raw.rows)) throw new ApiShapeError(`${context}: expected a rows array`);
    return Object.freeze({
        // An absent header is a headerless file, which is a different fact from
        // a header of zero columns — hence null rather than [].
        header: raw.header === undefined || raw.header === null ? null : frozen(decodeStrings(raw.header, `${context} header`)),
        rows: frozen(decodeRows(raw.rows, `${context} rows`)),
        rowCount: num(raw.rowCount, `${context} rowCount`),
        truncated: raw.truncated === true,
    });
}

function decodeStatementMeta(raw: RawStatementMeta | null | undefined, context: string): StatementMeta | null {
    if (raw === undefined || raw === null) return null;
    return Object.freeze({
        accountHint: optStr(raw.accountHint, `${context} accountHint`),
        currency: optStr(raw.currency, `${context} currency`),
        // Verbatim decimal text. Never Number()'d here or anywhere downstream.
        ledgerBalance: optStr(raw.ledgerBalance, `${context} ledgerBalance`),
        balanceAsOf: optStr(raw.balanceAsOf, `${context} balanceAsOf`),
    });
}

function decodeProposedTxn(raw: RawProposedTxn | undefined, context: string): ProposedTxn {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing transaction`);
    return Object.freeze({
        date: str(raw.date, `${context} date`),
        description: str(raw.description, `${context} description`),
        postings: frozen(decodeStrings(raw.postings, `${context} postings`)),
    });
}

/** The five counts the contract carries are required; the three `Signals` extras are optional. */
function decodeSignals(raw: RawCandidateSignals | undefined, context: string): CandidateSignals {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing signals`);
    return Object.freeze({
        txns: num(raw.txns, `${context} txns`),
        postings: num(raw.postings, `${context} postings`),
        amountlessPostings: num(raw.amountlessPostings, `${context} amountlessPostings`),
        bareCommodityAmounts: num(raw.bareCommodityAmounts, `${context} bareCommodityAmounts`),
        unknownAccounts: num(raw.unknownAccounts, `${context} unknownAccounts`),
        emptyDescriptions: optNum(raw.emptyDescriptions, `${context} emptyDescriptions`),
        columnCountMatches: optBool(raw.columnCountMatches, `${context} columnCountMatches`),
        headerMatchesSource: optBool(raw.headerMatchesSource, `${context} headerMatchesSource`),
    });
}

function decodeCandidate(raw: RawRulesCandidate | undefined, context: string): RulesCandidate {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing candidate`);
    return Object.freeze({
        id: str(raw.id, `${context} id`),
        label: str(raw.label, `${context} label`),
        score: num(raw.score, `${context} score`),
        signals: decodeSignals(raw.signals, `${context} signals`),
        sample: frozen((raw.sample ?? []).map((txn, i) => decodeProposedTxn(txn, `${context} sample[${i}]`))),
        // `account1`/`account2` are `skip_serializing_if = "Option::is_none"` on
        // the engine, so an absent key means the rules file declares none —
        // exactly what `optStr` reads as null. It is what the balance-assertion
        // account defaults to, and having it HERE is what removed a second
        // `/api/rules` fetch joined onto this list by id.
        account1: optStr(raw.account1, `${context} account1`),
        account2: optStr(raw.account2, `${context} account2`),
    });
}

function decodeStageDefaults(raw: RawStageDefaults | undefined, context: string): StageDefaults {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing defaults`);
    return Object.freeze({csvPath: str(raw.csvPath, `${context} csvPath`), journalId: optStr(raw.journalId, `${context} journalId`)});
}

/** `POST /api/import/stage` → the converted file, its preview, and the rules files that fit it. */
export function decodeStagedFile(raw: unknown): StagedFile {
    const staged = raw as RawStagedFile;
    if (typeof staged !== "object" || staged === null || !Array.isArray(staged.candidates)) {
        throw new ApiShapeError("staged file: expected a candidates array");
    }
    const rawNotes = staged.notes ?? [];
    const notes = rawNotes.map((note, i) => decodeConvertNote(note, `staged file notes[${i}]`));
    return Object.freeze({
        stageId: str(staged.stageId, "staged file stageId"),
        format: str(staged.format, "staged file format"),
        preview: decodeStagePreview(staged.preview, "staged file preview"),
        statement: decodeStatementMeta(staged.statement, "staged file statement"),
        notes: frozen(notes.filter((note): note is ConvertNote => note !== null)),
        unknownNoteCount: notes.filter((note) => note === null).length,
        candidates: frozen(staged.candidates.map((candidate, i) => decodeCandidate(candidate, `staged file candidates[${i}]`))),
        defaults: decodeStageDefaults(staged.defaults, "staged file defaults"),
    });
}

function decodeBalanceCheck(raw: RawBalanceCheck | null | undefined, context: string): BalanceCheck | null {
    if (raw === undefined || raw === null) return null;
    return Object.freeze({
        statement: str(raw.statement, `${context} statement`),
        computed: str(raw.computed, `${context} computed`),
        // `matches` is the ENGINE's verdict, computed by concatenation (fact 3).
        // Nothing on this side re-derives it from the two strings.
        matches: raw.matches === true,
        // Null when the engine could not subtract the two — a multi-commodity
        // balance has no single gap to report. The mismatch is still a fact.
        difference: optStr(raw.difference, `${context} difference`),
    });
}

/**
 * `POST /api/import/dry-run` → what hledger would write, or why it would not.
 *
 * `ok: false` is a VALUE, not a transport error: hledger's stderr is the most
 * useful thing on the screen (it echoes the offending CSV record), so it is
 * carried through to be rendered verbatim rather than thrown.
 */
export function decodeDryRun(raw: unknown): DryRunResult {
    const run = raw as RawDryRun;
    if (typeof run !== "object" || run === null || typeof run.ok !== "boolean") {
        throw new ApiShapeError("dry run: expected an ok flag");
    }
    if (!run.ok) return Object.freeze({ok: false as const, stderr: str(run.stderr, "dry run stderr")});
    return Object.freeze({
        ok: true as const,
        entries: str(run.entries, "dry run entries"),
        count: num(run.count, "dry run count"),
        status: str(run.status, "dry run status"),
        skipped:
            run.skipped === undefined || run.skipped === null
                ? null
                : Object.freeze({olderThan: str(run.skipped.olderThan, "dry run skipped olderThan"), count: num(run.skipped.count, "dry run skipped count")}),
        balance: decodeBalanceCheck(run.balance, "dry run balance"),
        aliases: decodeAliasEffect(run.aliases, "dry run aliases"),
        blockedByGit: frozen(decodeStrings(run.blockedByGit, "dry run blockedByGit")),
    });
}

function decodeSortMove(raw: RawSortMove | undefined, context: string): SortMove {
    if (raw === undefined || raw === null) throw new ApiShapeError(`${context}: missing move`);
    return Object.freeze({
        date: str(raw.date, `${context} date`),
        description: str(raw.description, `${context} description`),
        fromLine: num(raw.fromLine, `${context} fromLine`),
        toLine: num(raw.toLine, `${context} toLine`),
    });
}

/** Absent ordering = nothing was imported (the Save-CSV-only path), which is vacuously in order. */
function decodeOrdering(raw: RawOrdering | null | undefined, context: string): OrderingReport {
    if (raw === undefined || raw === null) return Object.freeze({inOrder: true, moves: frozen([])});
    return Object.freeze({
        inOrder: raw.inOrder === true,
        moves: frozen((raw.moves ?? []).map((move, i) => decodeSortMove(move, `${context} moves[${i}]`))),
    });
}

function decodeGitReport(raw: RawGitReport | null | undefined, context: string): GitReport | null {
    if (raw === undefined || raw === null) return null;
    return Object.freeze({
        committed: raw.committed === true,
        paths: frozen(decodeStrings(raw.paths, `${context} paths`)),
        skipped: frozen(decodeStrings(raw.skipped, `${context} skipped`)),
    });
}

/** `POST /api/import/commit` → what was written. */
export function decodeCommitResult(raw: unknown): CommitResult {
    const result = raw as RawCommitResult;
    if (typeof result !== "object" || result === null) throw new ApiShapeError("commit result: expected an object");
    return Object.freeze({
        csvWritten: str(result.csvWritten, "commit result csvWritten"),
        // Null on the Save-CSV-only path: no rules file was chosen, so no
        // `hledger import` ran and no journal was touched.
        journalWritten: optStr(result.journalWritten, "commit result journalWritten"),
        imported: optNum(result.imported, "commit result imported") ?? 0,
        ordering: decodeOrdering(result.ordering, "commit result ordering"),
        git: decodeGitReport(result.git, "commit result git"),
    });
}

/** `POST /api/import/sort` → how many transactions the confirmed re-sort moved. */
export function decodeSortResult(raw: unknown): SortResult {
    const result = raw as RawSortResult;
    if (typeof result !== "object" || result === null) throw new ApiShapeError("sort result: expected an object");
    return Object.freeze({moved: num(result.moved, "sort result moved")});
}

/** `GET`/`PUT /api/prefs` → the preferences store. */
export function decodePrefs(raw: unknown): Prefs {
    const prefs = raw as RawPrefs;
    if (typeof prefs !== "object" || prefs === null) throw new ApiShapeError("prefs: expected an object");
    return Object.freeze({
        hledgerPath: optStr(prefs.hledgerPath, "prefs hledgerPath"),
        // Tri-state and it matters: null = "commit when a repo is present",
        // false = the user turned it off. Collapsing null to false would silently
        // disable a feature the user never opted out of.
        gitAutocommit: optBool(prefs.gitAutocommit, "prefs gitAutocommit"),
    });
}
