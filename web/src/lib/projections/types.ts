// The Projections domain model: a what-if scenario, and what projecting it
// forward produces.
//
// These mirror the Rust wire in `crates/ledgeline-server/src/projections_api.rs`
// one for one, by hand, as every other native type in this app does. Two facts
// from that file are load-bearing everywhere below and are restated here because
// nothing in the type system can enforce them:
//
//  1. A LINE'S `amount` CARRIES THE JOURNAL'S OWN SIGN. A revenue line is
//     NEGATIVE (`(income:salary) $-5188.33`) and an expense line is positive,
//     exactly as the postings are written. The table shows magnitudes; the flip
//     lives in `scenarioModel.ts` and nowhere else.
//  2. The projection's STATEMENTS are the Reports tab's own (plan 28): the
//     balance sheet, income statement and cash flow statement over the real
//     journal before the cutoff plus the scenario's generated transactions
//     after it, in the Reports tab's signs. Nothing here re-signs them.
//
// Pure module: no Svelte, no DOM, so every helper over these is unit-testable
// under node.

import type {Dec, MixedAmount} from "#lib/domain/money.js";
import type {ISODate, Transaction} from "#lib/domain/types.js";
import type {AutoRuleForm} from "#lib/autoRules/ruleForm.js";
import type {AccountType} from "#lib/domain/accountClass.js";
import type {AccountReport, BalanceSheetColumns, CashFlowStatement, IncomeStatementReport} from "#lib/reports/types.js";

/** The unit a growth rate steps on. */
export type GrowthUnit = "week" | "month" | "year";
export const GROWTH_UNITS: readonly GrowthUnit[] = ["week", "month", "year"];

/** Where a line came from: the user's journal, or an average estimated from history. */
export type LineSource = "journal" | "unbudgeted";
export const LINE_SOURCES: readonly LineSource[] = ["journal", "unbudgeted"];

/**
 * What a recurring row IS: a per-period flow, or a balance that compounds.
 *
 * The discriminator is this field and never the account's type. `assets:cash`
 * is a legitimate destination for a one-off flow and a legitimate asset row, so
 * a reader that guessed from the account would reclassify one of them on every
 * round trip — which is why the engine puts it on the wire explicitly.
 *
 * An `asset` row's `amount` is its per-period CONTRIBUTION (zero when the
 * balance only compounds), its `growth` is the rate the BALANCE compounds at,
 * and `opening` may override the journal's balance for that account. See
 * `plans/23-asset-growth.md`.
 */
export type LineRole = "flow" | "asset";
export const LINE_ROLES: readonly LineRole[] = ["flow", "asset"];

/**
 * One of the five fixed intervals a line's period can step by, bounds aside.
 *
 * The engine re-parses `raw` and ignores everything else.
 */
export type ScenarioInterval = "daily" | "weekly" | "monthly" | "quarterly" | "yearly";
export const SCENARIO_INTERVALS: readonly ScenarioInterval[] = ["daily", "weekly", "monthly", "quarterly", "yearly"];

/**
 * A single-commodity amount.
 *
 * `precision` is the DISPLAY precision and is not `quantity.p`. It travels back
 * to the engine untouched because the growth curve rounds to it at every step —
 * a client that dropped it would get a different, and wrong, curve.
 */
export interface ScenarioAmount {
    commodity: string;
    quantity: Dec;
    precision: number;
}

/** A recurrence, in the three readings the table needs. Only `raw` is read back by the engine. */
export interface ScenarioPeriod {
    /** The period expression as it would be written in a journal. The round-trip field. */
    raw: string;
    /**
     * `daily`…`yearly` when `raw` steps by one of the five intervals, BOUNDED
     * OR NOT (`monthly from 2026-11-01` is `monthly`), else null. `raw` is
     * rebuilt as `simple [from D] [to D]`.
     */
    simple: ScenarioInterval | null;
    /**
     * When `simple` is null but the bounds can still be rewritten: the interval
     * words as written (`every 15th day of month`), which `raw` is rebuilt
     * around. Null when `simple` is set, and when the bounds cannot be
     * rewritten at all (a one-off, or a recurrence the engine does not
     * understand) — `raw` is then the only reading of the period.
     */
    base: string | null;
    /** Inclusive start, or null. */
    from: ISODate | null;
    /** EXCLUSIVE end, as hledger writes `to`, or null. */
    to: ISODate | null;
}

/** A stepwise growth rate. `rate` is a FRACTION (0.03); the `%` belongs to the UI. */
export interface Growth {
    rate: Dec;
    unit: GrowthUnit;
}

/**
 * Which of the what-if table's four sections a row is shown in.
 *
 * `asset` is decided by the row's `role` and nothing else, so unlike the
 * income/expense split it cannot change while a row is being typed into — see
 * `resolvedSection`.
 */
export type LineSection = "income" | "expense" | "asset" | "oneoff";

/**
 * The two sections a row can be HELD in.
 *
 * Never `oneoff`: that one is decided by the line's PERIOD, which nothing typed
 * into an account box can change. Never `asset` either, and for the stronger
 * version of the same reason — that one is decided by the row's `role`, which no
 * editable field on the row touches at all, so there is nothing for a hold to
 * protect against.
 */
export type HeldSection = "income" | "expense";

/** The sections with an "Add a line" button — the ones a new row can be born into. */
export type AddSection = HeldSection | "asset";

/** One recurring row of the what-if table. */
export interface ScenarioLine {
    /** The LOGICAL row: two bounded segments of one step change share it. */
    id: string;
    /** The SOURCE RULE: every posting of one `~` block shares it, which is what the engine's cash and net-worth guards are scoped to. */
    group: string;
    /** A per-period flow, or a balance that compounds. */
    role: LineRole;
    account: string;
    /**
     * For a `flow`, signed as the journal writes it — see fact (1) at the top of
     * this file. For an `asset` row, the per-period CONTRIBUTION, zero when the
     * balance only compounds.
     */
    amount: ScenarioAmount;
    period: ScenarioPeriod;
    growth: Growth | null;
    /**
     * `asset` rows only: an override of the journal's balance at the projection
     * start. `null` — the normal case — means "use the journal's own", and only
     * the DIFFERENCE between an override and the journal's figure ever reaches
     * the net-worth series.
     */
    opening: ScenarioAmount | null;
    note: string;
    /**
     * The account this line balances against (plan 28 Decision 1). `null` is a
     * pre-plan-28 line: the engine books it to the default counter-account and
     * a save writes that default. The table shows that default greyed
     * (`ForecastDefaults.lines`, keyed by `group`).
     *
     * ONE PER RULE: every line of one `group` that names a counter must name
     * the same one, or the file cannot be written. `withCounter` keeps that
     * true by moving a line whose counter differs into a rule of its own.
     */
    counter: string | null;
    /** `asset` rows only: the gains account; `null` is `equity:unrealized gains`. */
    gains: string | null;
    source: LineSource;
    /**
     * The section the table is HOLDING this row in, or absent for "wherever its
     * account type says".
     *
     * UI-ONLY, like `ScenarioPeriod`'s `simple`/`from`/`to`: `scenarioToWire`
     * names its fields one by one and this is not among them, so it never
     * reaches the engine and a file round trip never carries it.
     *
     * It exists because a row's section USED to be re-derived from the account
     * text on every keystroke, and a half-typed account is not revenue — so a
     * row added under Income jumped to Expenses on the first letter and back on
     * the last, losing focus each time because the two sections are two
     * different `{#each}` blocks. See `ProjectionsTable.svelte`'s header for the
     * rule this field enforces, and plan 22, amendment 37.
     */
    section?: HeldSection;
    /**
     * The MAGNITUDE the user typed, once this line has been edited (plan 26 B6).
     *
     * Absent on a line as the engine sent it, whose `amount.quantity` carries
     * the journal's sign. Present after an amount or account edit: it is what the
     * table shows, and it is sent as the wire's `entry`, which the ENGINE signs by
     * the account's type — so nothing on this side knows that revenue is written
     * negative.
     */
    entry?: Dec;
    /**
     * The section the ENGINE placed this line in, and the role, account and
     * period it placed it for (`projections::line_section`, plan 26 B6). It is
     * the authority while those three are unchanged; once the user edits one,
     * `resolvedSection` previews the answer from the engine's account table
     * until the line round-trips again. Absent from a body that predates it.
     */
    resolved?: Readonly<{role: LineRole; account: string; raw: string; section: LineSection}>;
}

/** One account/amount pair of a dated event. Always one account and one amount. */
export interface EventPosting {
    account: string;
    amount: ScenarioAmount;
}

/** A dated one-off. Its postings decide whether it touches the P&L, the balance sheet, or both. */
export interface ScenarioEvent {
    id: string;
    date: ISODate;
    description: string;
    postings: EventPosting[];
    /** The account the event balances against (plan 28), or `null`. */
    counter: string | null;
}

/** A whole what-if. `name` is empty on a seed: naming it is Phase 3's Save As dialog's job. */
export interface Scenario {
    name: string;
    created: ISODate | null;
    updated: ISODate | null;
    lines: ScenarioLine[];
    events: ScenarioEvent[];
    /** `; cash:` — the scenario's default counter-account, or `null`. */
    cash: string | null;
    /** The scenario file's `= QUERY` auto-posting rules, applied to every projected transaction (plan 30). */
    autoRules: AutoRule[];
    /**
     * The account types the scenario FILE itself declares (`account hmrc:paye  ; type: L`); empty for a seed.
     * The engine read every sign it served — a rule form's percentages, a line's section — with these winning
     * over the main journal's, so they go back with the scenario and the engine reads edits the same way.
     * Optional so a scenario built here (or by an older engine) needs none.
     */
    accountTypes?: Readonly<Record<string, AccountType>>;
}

/**
 * One `= QUERY` auto-posting rule.
 *
 * `raw` is the fact for a rule nobody has touched: it goes back as it came, so
 * the file keeps its bytes. Once `edited`, the FORM is what is sent, and the
 * engine renders it. `form` is `null` for a rule the editor cannot state —
 * shown raw and locked (plan 21's contract).
 */
export interface AutoRule {
    /** The whole block as the file writes it, each line ending in a newline. Empty for a rule added here. */
    raw: string;
    /** The query, as written. */
    query: string;
    /** Why the rule never fires, or `null` when it does. */
    unsupported: string | null;
    /** The rule as a form, or `null` when the form cannot state it. */
    form: AutoRuleForm | null;
    /** Its position among the file's `=` blocks, or `null` for a rule the file does not hold yet. A save rewrites that block. */
    block: number | null;
    /** UI-only: the form was edited (or the rule added) here, so the form, not `raw`, is sent. Never on the wire. */
    edited?: boolean;
}

/** An opening balance and each bucket's closing balance. `opening` is a real figure from the journal. */
export interface BalanceSeries {
    opening: MixedAmount;
    values: MixedAmount[];
    /** The REAL closing balance at the end of each lookback period, oldest → newest; empty with no lookback. */
    history: MixedAmount[];
}

/** Where the cash crosses zero. `label` is the engine's, so a sentence never has to index back into `buckets`. */
export interface Runway {
    /** 0-based index into `Projection.buckets`. */
    bucket: number;
    /** The bucket's human label ("Mar 2028"). */
    label: string;
    /** The bucket's LAST day — the date the closing balance is negative as of. */
    date: ISODate;
    /** Whole periods from the start of the projection (`bucket + 1`). */
    periods: number;
}

/**
 * What one asset row did, attributed back to the row.
 *
 * The Balance column's source and the net-worth tab's breakdown. Both need a
 * figure the SCENARIO does not carry — an opening balance is not part of a
 * what-if — and both have to agree with the curve beside them, so both come
 * from the walk that drew the curve rather than from a balance-sheet report read
 * alongside it. Keyed by `group`, which is the row's source rule.
 *
 * A row the engine refused to model (a non-asset account) is ABSENT here and
 * present in `warnings`: it contributed nothing, and reporting a balance beside
 * it would say it had been modelled.
 */
export interface AssetRow {
    /** The `ScenarioLine.group` this was placed from. */
    group: string;
    account: string;
    /** The journal's own SUBTREE balance for that account, as of today. The greyed figure. */
    journalOpening: MixedAmount;
    /** What the row actually compounded from: the `opening` override, or `journalOpening`. */
    opening: MixedAmount;
    /** Total appreciation over the span. Never a contribution, never the override's adjustment. */
    growth: MixedAmount;
}

/**
 * The accounts the engine books to where the scenario names none — what the
 * table shows GREYED until the user types over it (plan 28, Decisions 4 and 7).
 */
export interface ForecastDefaults {
    /** What a scenario with no `cash:` header falls back to: the cash control's greyed value. */
    cash: string;
    /** The gains account of an asset row that names none. */
    gains: string;
    /** `ScenarioLine.group` → the default counter-account of that rule's counter-less lines. */
    lines: ReadonlyMap<string, string>;
    /** `ScenarioEvent.id` → its default counter-account, for events that name none. */
    events: ReadonlyMap<string, string>;
}

/**
 * The answer.
 *
 * Two kinds of column: `buckets`, `cash` and `runway` cover the FORECAST
 * periods only, while the three statements cover the lookback's full actual
 * periods and then the forecast. `projectedFrom` is the first forecast column's
 * index in the statements — where the actuals/projected divider is drawn.
 */
export interface Projection {
    /** Forecast bucket keys, oldest → newest. */
    buckets: string[];
    /** The full ACTUAL periods the statements show before the forecast. */
    lookback: string[];
    /** The cutoff: the first projected day. Actuals are strictly before it. */
    actualsEnd: ISODate;
    /** The first forecast column's index in the statements (`lookback.length`). */
    projectedFrom: number;
    cash: BalanceSeries;
    runway: Runway | null;
    /** One entry per asset row the engine modelled, in scenario order. `[]` when there are none. */
    assets: AssetRow[];
    /** Everything the projection could not do. A dropped line always says so here. */
    warnings: string[];
    balanceSheet: BalanceSheetColumns;
    /** Interval mode: one column per lookback and forecast period, plus the Total. */
    incomeStatement: IncomeStatementReport;
    cashFlow: CashFlowStatement;
    defaults: ForecastDefaults;
    /** Every account per column, each section read its own way, when the run asked for it; null otherwise. */
    accounts: AccountReport | null;
    /** The generated transactions, when the run asked for them; null otherwise. */
    journal: ForecastJournal | null;
}

/** What the run is asked to compute beyond the four readings every run carries. */
export interface AccountsRequest {
    /** Rows for declared and known accounts with nothing in any column (`-E --declared`). */
    empty: boolean;
}

/** The part of a scenario one generated transaction came from: a rule's `group`, an event's `id`, or an asset row's `group`. */
export interface ForecastOrigin {
    kind: "rule" | "event" | "growth";
    key: string;
}

export const FORECAST_ORIGIN_KINDS: readonly ForecastOrigin["kind"][] = ["rule", "event", "growth"];

/** One generated transaction, in the journal view's own domain shape. */
export interface ForecastEntry {
    origin: ForecastOrigin;
    txn: Transaction;
    /** Lowercase search text from the engine (core's `haystack`), the same text the Journal tab's free-text filter searches. */
    haystack: string;
}

/** The forecast journal: generated transactions only, date-ascending, capped by the engine. */
export interface ForecastJournal {
    /** How many the forecast generated, before the cap. */
    total: number;
    entries: ForecastEntry[];
    /** Whether `entries` stops short of `total`. */
    truncated: boolean;
}

// ---------------------------------------------------------------------------
// Scenario FILES (plan 22, Phase 3)
//
// A scenario lives in an ordinary journal file that nothing includes, so it can
// be edited, diffed, committed and read by hledger itself. These types are the
// picker's and the Save As dialog's; the scenario inside is the same `Scenario`
// the seed and the run already use.
// ---------------------------------------------------------------------------

/** One `*.journal` file the engine found beside the open journal. */
export interface ProjectionFile {
    /** The file's path relative to the journal directory. The only handle there is — never an absolute path. */
    id: string;
    /** The display fallback, from the filename, when the file has no `; projection:` name. */
    label: string;
    /** `; projection: <name>`, for a `projection-*.journal` that has one. Null otherwise. */
    name: string | null;
    created: ISODate | null;
    updated: ISODate | null;
    /** Whether this is a `projection-*.journal` — the group the picker shows first. */
    isProjection: boolean;
    sizeBytes: number;
    /**
     * Whether saving over it could succeed.
     *
     * False for a file the main journal includes: decision 5 forbids the
     * Projections tab from writing to a plan of record, so the picker offers it
     * for LOADING and refuses to Save over it rather than letting the engine
     * refuse after the fact.
     */
    writable: boolean;
}

/** `GET /api/projections` — the picker's list. */
export interface ProjectionIndex {
    /** The journal directory's own final component, for a heading. Never a path. */
    rootLabel: string;
    /** Whether this server has an editor bound at all. False means every save is a 501. */
    editable: boolean;
    /** A scan cap was hit and the list is a subset. Shown, so a missing file is never a silent one. */
    truncated: boolean;
    files: ProjectionFile[];
    /** The directories that hold a journal, relative, `""` for the root — what Save As offers instead of a free-text path. */
    directories: string[];
    warnings: string[];
}

/** `GET`/`PUT /api/projections/{*id}` — one file, read as a scenario. */
export interface ScenarioFile {
    id: string;
    label: string;
    /**
     * The optimistic-concurrency token, over the file's raw bytes. Sent back
     * with a save; a mismatch is a 409 and nothing is written.
     */
    revision: string;
    /** See `ProjectionFile.writable`. */
    writable: boolean;
    scenario: Scenario;
}
