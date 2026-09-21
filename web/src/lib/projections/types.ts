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
//  2. `Projection.netIncome` IS THE OTHER WAY UP — cash-flow orientation,
//     revenue positive and expenses negative, so `totals` is the sum of the
//     depth-1 rows and IS net income. A `$4200` rent line arrives as `-4200`.
//     Do not flip it again.
//
// Pure module: no Svelte, no DOM, so every helper over these is unit-testable
// under node.

import type {Dec, MixedAmount} from "$lib/domain/money";
import type {ISODate} from "$lib/domain/types";
import type {PeriodReport} from "$lib/reports/types";

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
 * A BARE fixed interval, when the line's period is one.
 *
 * `null` on a bounded-but-irregular or single-date rule, in which case `raw` is
 * the only reading — the engine re-parses `raw` and ignores everything else.
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
    /** `daily`…`yearly` when `raw` is a bare fixed interval, else null. */
    simple: ScenarioInterval | null;
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

/** Which of the what-if table's three sections a row is shown in. */
export type LineSection = "income" | "expense" | "oneoff";

/**
 * The two sections a row can be HELD in.
 *
 * Never `oneoff`: that one is decided by the line's PERIOD, which nothing typed
 * into an account box can change.
 */
export type HeldSection = "income" | "expense";

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
}

/** A whole what-if. `name` is empty on a seed: naming it is Phase 3's Save As dialog's job. */
export interface Scenario {
    name: string;
    created: ISODate | null;
    updated: ISODate | null;
    lines: ScenarioLine[];
    events: ScenarioEvent[];
}

/** An opening balance and each bucket's closing balance. `opening` is a real figure from the journal. */
export interface BalanceSeries {
    opening: MixedAmount;
    values: MixedAmount[];
}

/** Where the cash crosses zero. `bucketKey` and `label` are the engine's, so a sentence never has to index back into `buckets`. */
export interface Runway {
    /** 0-based index into `Projection.buckets`. */
    bucket: number;
    bucketKey: string;
    /** The bucket's human label ("Mar 2028"). */
    label: string;
    /** The bucket's LAST day — the date the closing balance is negative as of. */
    date: ISODate;
    /** Whole periods from the start of the projection (`bucket + 1`). */
    periods: number;
}

/** The answer. */
export interface Projection {
    buckets: string[];
    /** The first day of the first bucket — the next WHOLE bucket after `asOf`. */
    start: ISODate;
    /** CASH-FLOW ORIENTATION — see fact (2) at the top of this file. */
    netIncome: PeriodReport;
    cash: BalanceSeries;
    netWorth: BalanceSeries;
    runway: Runway | null;
    /** Everything the projection could not do. A dropped line always says so here. */
    warnings: string[];
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
