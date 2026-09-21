// Editing a scenario: the sign flip, the sections, step split/merge, and the
// encoder that puts one on the wire.
//
// Pure module — no Svelte, no DOM, no clock — so every decision below is tested
// by calling it rather than by building a screen to reach it (`web/README.md`).
// `scenarioStore.svelte.ts` is the thin `$state` wrapper that calls into here.
//
// # The sign, stated once
//
// A scenario line carries the JOURNAL's sign: `(income:salary) $-5188.33` is
// negative, `(expenses:rent) $4200` is positive, exactly as
// `projections_api.rs` sends and reads them. The table shows and takes
// MAGNITUDES, the way the budget editor does for goals — a user typing an income
// line types what they earn, not the negation of it.
//
// [`signedQuantity`] is the only place that flip happens, and it decides by the
// account's resolved TYPE, never by its name ([[account-type-not-name]]). That
// is also why [`withAccount`] exists: moving a row from `expenses:rent` to
// `income:rent` has to re-sign the amount it already holds, and a component that
// wrote `line.account = …` directly would leave a positive number under a
// revenue account, which the engine would then project as negative revenue.

import {encodeDec} from "$lib/api/editMapping";
import type {RunProjectionBody, WireScenarioIn} from "$lib/api/native";
import {absDec} from "$lib/format/amounts";
import {neg, type Dec} from "$lib/domain/money";
import {isAccountType, type AccountType} from "$lib/domain/accountTypes";
import type {ISODate} from "$lib/domain/types";
import type {
    AddSection,
    Growth,
    HeldSection,
    LineSection,
    Scenario,
    ScenarioAmount,
    ScenarioEvent,
    ScenarioInterval,
    ScenarioLine,
    ScenarioPeriod,
} from "./types";

// Re-exported because every reader of a section is a reader of this module, and
// the two names were declared here before the hold moved them next to the field
// that carries one (`ScenarioLine.section`).
export type {AddSection, HeldSection, LineSection};

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/** Whether a string is a full ISO date, which is all `type=date` and the engine accept. */
export function isIsoDate(value: string): boolean {
    return ISO_DATE.test(value.trim());
}

// ---------------------------------------------------------------------------
// The sign
// ---------------------------------------------------------------------------

/**
 * Whether postings to this account are written negative — i.e. it is revenue.
 *
 * `isAccountType`, not `resolveAccountType(...) === "revenue"`: a declared
 * `type: G` gain is revenue to the engine (hledger's `type:R` query matches
 * one), and a row the engine books as revenue while this says otherwise is a row
 * whose amount reaches the engine with the wrong sign.
 */
export function isRevenueAccount(account: string, declared: ReadonlyMap<string, AccountType>): boolean {
    return isAccountType(account, declared, "revenue");
}

/**
 * A magnitude the user typed → the signed quantity the journal would carry.
 *
 * THE ONE SIGN FLIP. Everything that writes an amount into a scenario goes
 * through here; nothing else may negate.
 */
export function signedQuantity(magnitude: Dec, account: string, declared: ReadonlyMap<string, AccountType>): Dec {
    const mag = absDec(magnitude);
    return isRevenueAccount(account, declared) ? neg(mag) : mag;
}

/** A signed scenario amount → the magnitude the table shows. */
export function magnitudeOf(amount: ScenarioAmount): Dec {
    return absDec(amount.quantity);
}

/**
 * Re-sign an amount for an account, keeping its commodity.
 *
 * The display precision is RAISED to fit the new quantity and never lowered.
 * Raising it is required — an amount whose precision was below its own places
 * would be rounded away by the growth walk — and lowering it is not wanted:
 * retyping a seeded `$1,875.00` as `1875` would otherwise quietly move that
 * line's growth onto whole dollars, which is a modelling change made by a
 * cosmetic edit.
 */
export function amountFor(amount: ScenarioAmount, magnitude: Dec, account: string, declared: ReadonlyMap<string, AccountType>): ScenarioAmount {
    const quantity = signedQuantity(magnitude, account, declared);
    return {...amount, quantity, precision: Math.max(amount.precision, quantity.p)};
}

/** A line with a new account, its amount RE-SIGNED for the account's type. */
export function withAccount(line: ScenarioLine, account: string, declared: ReadonlyMap<string, AccountType>): ScenarioLine {
    return {...line, account, amount: amountFor(line.amount, magnitudeOf(line.amount), account, declared)};
}

/** A line with a new magnitude, signed for the account it already has. */
export function withMagnitude(line: ScenarioLine, magnitude: Dec, declared: ReadonlyMap<string, AccountType>): ScenarioLine {
    return {...line, amount: amountFor(line.amount, magnitude, line.account, declared)};
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

/**
 * The section a line's ROLE and ACCOUNT TYPE put it in — the reading that
 * ignores any hold.
 *
 * The order of the three tests is the contract:
 *
 * 1. **`role: "asset"` wins outright**, ahead of the period and ahead of the
 *    account. An asset row states a BALANCE, and the only field that can make
 *    one is `role` — which no box on the row edits. So an asset row's section
 *    is constant for the row's whole life, and the section-stability rule below
 *    has nothing to defend it from. (That also settles the dated asset row: a
 *    balance with a single-date contribution is still a balance, and filing it
 *    among the one-offs would hide its rate behind a table with no Growth
 *    column.)
 * 2. A single-date rule (`~ 2027-03-01`) is a LINE in the model —
 *    `seed_scenario` deliberately leaves it one so the file round trip says so —
 *    but it is a one-off to a reader, so it is shown among the events. The
 *    engine treats the two identically (plan 22, amendment 12).
 * 3. Everything else that is not revenue lands under Expenses, including a
 *    recurring transfer to an asset account. That is a two-way split, like the
 *    budget editor's, and it is why the heading says "and other outflows".
 */
export function resolvedSection(line: ScenarioLine, declared: ReadonlyMap<string, AccountType>): LineSection {
    if (line.role === "asset") return "asset";
    if (line.period.simple === null && isIsoDate(line.period.raw)) return "oneoff";
    return isRevenueAccount(line.account, declared) ? "income" : "expense";
}

/**
 * The section the table SHOWS a line in: its hold while it has one, else its
 * account's type.
 *
 * THE SECTION-STABILITY RULE, and the reason `ScenarioLine.section` exists. A
 * section read live from the account text moves a row on a keystroke — `i` is
 * not a revenue account, and neither is `` — and moving a row between two
 * `{#each}` blocks destroys its DOM node and the focus in it. So while a row is
 * being edited it is HELD, and the account type gets to decide again only once
 * editing is over. `ProjectionsTable.svelte` owns when a hold is taken and
 * released; this function owns what a hold means.
 *
 * A hold never beats `oneoff` or `asset`. Those two are the line's PERIOD and
 * its ROLE talking, and the account box cannot argue with either — so a stale
 * hold left on a row that became one must not drag it back into the flow
 * tables.
 */
export function sectionOfLine(line: ScenarioLine, declared: ReadonlyMap<string, AccountType>): LineSection {
    const resolved = resolvedSection(line, declared);
    if (resolved === "oneoff" || resolved === "asset") return resolved;
    return line.section ?? resolved;
}

/** One logical row of the table: a line and, when it is a step change, its later segments. */
export interface LogicalRow {
    id: string;
    /** In span order, earliest first. Length > 1 means this row is a step change. */
    segments: ScenarioLine[];
}

/**
 * Group a section's lines into logical rows, segments in span order.
 *
 * Rows keep the order their FIRST segment appears in, so a table does not
 * reshuffle itself when a step is added to a row halfway down it.
 */
export function logicalRows(lines: readonly ScenarioLine[], declared: ReadonlyMap<string, AccountType>, section: LineSection): LogicalRow[] {
    const rows: LogicalRow[] = [];
    const byId = new Map<string, LogicalRow>();
    for (const line of lines) {
        if (sectionOfLine(line, declared) !== section) continue;
        const existing = byId.get(line.id);
        if (existing === undefined) {
            const row: LogicalRow = {id: line.id, segments: [line]};
            byId.set(line.id, row);
            rows.push(row);
        } else {
            existing.segments.push(line);
        }
    }
    for (const row of rows) row.segments.sort(compareSegments);
    return rows;
}

/**
 * One accessible name per row of a section, each unique WITHIN it.
 *
 * The account, which is what a user calls a row — falling back to the row's
 * POSITION when it has no account yet, and adding the position when two rows
 * share one. Every blank row used to be "a new row", so neither a screen reader
 * nor a test could say which `⋯` it meant, which left the half-typed row a user
 * most wants to delete with no name to reach it by. Two rows freshly added to
 * one section both carry the seeded `income:` prefix, so the duplicate case is
 * not hypothetical either.
 *
 * `sectionTitle` is passed in rather than derived: the words on the headings
 * belong to the component, and this module has no business knowing that the
 * `expense` section is called "Expenses and other outflows".
 */
export function rowNames(rows: readonly LogicalRow[], sectionTitle: string): string[] {
    const counts = new Map<string, number>();
    for (const row of rows) {
        const account = row.segments[0].account.trim();
        counts.set(account, (counts.get(account) ?? 0) + 1);
    }
    return rows.map((row, at) => {
        const account = row.segments[0].account.trim();
        const where = `row ${at + 1} of ${sectionTitle}`;
        if (account === "") return where;
        return (counts.get(account) ?? 0) > 1 ? `${account} (${where})` : account;
    });
}

/** Earliest span first; an unbounded start sorts before every dated one. */
function compareSegments(a: ScenarioLine, b: ScenarioLine): number {
    const left = a.period.from ?? "";
    const right = b.period.from ?? "";
    return left < right ? -1 : left > right ? 1 : 0;
}

// ---------------------------------------------------------------------------
// Periods
// ---------------------------------------------------------------------------

/**
 * `{simple, from, to}` → the period expression a journal would carry.
 *
 * This is the string the engine re-parses through hledger's own grammar, so it
 * has to be hledger's spelling: `to` is EXCLUSIVE and the interval word comes
 * first. A period with no `simple` cannot be rebuilt — its `raw` is the only
 * reading of it — so callers must not reach here with one.
 */
export function periodRaw(simple: ScenarioInterval, from: ISODate | null, to: ISODate | null): string {
    let raw: string = simple;
    if (from !== null) raw += ` from ${from}`;
    if (to !== null) raw += ` to ${to}`;
    return raw;
}

/** A period rebuilt with new bounds, `raw` regenerated to match. */
export function withBounds(period: ScenarioPeriod, from: ISODate | null, to: ISODate | null): ScenarioPeriod {
    if (period.simple === null) return period;
    return {raw: periodRaw(period.simple, from, to), simple: period.simple, from, to};
}

/** A period on a new interval, bounds kept, `raw` regenerated to match. */
export function withInterval(period: ScenarioPeriod, simple: ScenarioInterval): ScenarioPeriod {
    return {raw: periodRaw(simple, period.from, period.to), simple, from: period.from, to: period.to};
}

/** A bare monthly period — what a brand-new row opens as. */
export function monthlyPeriod(): ScenarioPeriod {
    return {raw: "monthly", simple: "monthly", from: null, to: null};
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/**
 * The lowest `<prefix>-<n>` not already taken.
 *
 * Deterministic rather than random so a test can assert the id a split produced,
 * and so two edits in the same tick cannot collide.
 */
export function freshId(prefix: string, taken: ReadonlySet<string>): string {
    for (let n = 1; ; n += 1) {
        const id = `${prefix}-${n}`;
        if (!taken.has(id)) return id;
    }
}

/** Every id and group in use, which is what a fresh one has to avoid. */
export function takenIds(scenario: Scenario): Set<string> {
    const taken = new Set<string>();
    for (const line of scenario.lines) {
        taken.add(line.id);
        taken.add(line.group);
    }
    for (const event of scenario.events) taken.add(event.id);
    return taken;
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/** The account type a section's rows are expected to post to. */
const SECTION_TYPE: Record<AddSection, AccountType> = {income: "revenue", expense: "expense", asset: "asset"};

/** hledger's own top-level names, for a journal that has nothing of the type yet. */
const HLEDGER_DEFAULT_TOP: Record<AddSection, string> = {income: "income", expense: "expenses", asset: "assets"};

/**
 * The account prefix a new row in `section` opens on — `income:` / `expenses:` /
 * `assets:`, or whatever this journal calls them.
 *
 * A TYPING convenience, and only that: it puts the caret in the right tree so
 * the combobox has something to complete. Where the row is SHOWN is the hold's
 * job now (`sectionOfLine`) — this used to be what kept a new Income row out of
 * Expenses, and it could not, because the user's next keystroke could delete it.
 * (An ASSET row is the one section where the prefix carries no such weight at
 * all: that row is held there by its `role`, whatever ends up in the box.)
 *
 * Taking the prefix from the journal's own accounts means a user whose revenue
 * tree is `revenues:` is not handed `income:`. Falls back to the hledger
 * defaults when nothing in the journal has the type yet — which is the
 * first-run case, where there is nothing to learn from.
 */
export function sectionPrefix(section: AddSection, accountNames: readonly string[], declared: ReadonlyMap<string, AccountType>): string {
    const want: AccountType = SECTION_TYPE[section];
    const counts = new Map<string, number>();
    for (const name of accountNames) {
        // `isAccountType`, so a journal whose bank accounts are declared
        // `type: C` still teaches this the name of its asset root.
        if (!isAccountType(name, declared, want)) continue;
        const top = name.split(":")[0];
        counts.set(top, (counts.get(top) ?? 0) + 1);
    }
    let best = HLEDGER_DEFAULT_TOP[section];
    let bestCount = 0;
    // Sorted first, so a tie resolves the same way on every render rather than
    // by whichever name the journal happened to list first.
    for (const [top, count] of [...counts].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
        if (count > bestCount) {
            best = top;
            bestCount = count;
        }
    }
    return `${best}:`;
}

/** A blank monthly line for `section`, at zero in `commodity`. */
export function blankLine(id: string, commodity: string, precision: number): ScenarioLine {
    return {
        id,
        group: id,
        // A new row is a FLOW. An asset row is a different thing the table adds
        // deliberately (plan 23, Phase 3), never something a blank row becomes
        // by having an `assets:` account typed into it.
        role: "flow",
        account: "",
        amount: {commodity, quantity: {m: 0n, p: precision}, precision},
        period: monthlyPeriod(),
        growth: null,
        opening: null,
        note: "",
        // Authored, not estimated: the user is writing it, so it must not wear
        // the "from history" flag a seeded row wears.
        source: "journal",
    };
}

/**
 * A blank monthly ASSET row: a balance that compounds, with no rate and no
 * contribution yet.
 *
 * A separate constructor rather than a flag on `blankLine`, because the default
 * matters. `blankLine` makes a FLOW and must keep making one — an asset row is
 * something the table creates deliberately from the Assets section's own button,
 * never something a blank row becomes by having `assets:` typed into it. The
 * contribution opens at ZERO, which is the `$0` posting the file format writes
 * for a row that only compounds, and `opening` stays null so the Balance column
 * shows the journal's own figure.
 */
export function blankAssetLine(id: string, commodity: string, precision: number): ScenarioLine {
    return {...blankLine(id, commodity, precision), role: "asset"};
}

/** A blank dated event with one empty posting. */
export function blankEvent(id: string, date: ISODate, commodity: string, precision: number): ScenarioEvent {
    return {id, date, description: "", postings: [{account: "", amount: {commodity, quantity: {m: 0n, p: precision}, precision}}]};
}

/**
 * Split the segment of logical row `id` that spans `on` into two bounded
 * segments meeting at that date.
 *
 * This is "make this a step": payroll holds at its old figure up to `on` and
 * takes a new base from `on` forward, both halves still one row to the editor
 * because they share `id`. The two halves are two `~` rules, so the later one
 * takes a FRESH `group` — the engine's cash and net-worth guards are scoped per
 * rule, and two rules sharing a group would have their derived legs counted
 * against each other.
 *
 * Returns `lines` unchanged when there is nothing to split: no such row, a row
 * whose period has no bare interval to rebuild (its `raw` is the only reading of
 * it), or a date already on a segment boundary.
 */
export function splitStep(lines: readonly ScenarioLine[], id: string, on: ISODate, taken: ReadonlySet<string>): ScenarioLine[] {
    if (!isIsoDate(on)) return [...lines];
    const index = lines.findIndex((line) => line.id === id && spans(line.period, on));
    if (index < 0) return [...lines];
    const segment = lines[index];
    if (segment.period.simple === null) return [...lines];

    const before: ScenarioLine = {...segment, period: withBounds(segment.period, segment.period.from, on)};
    const after: ScenarioLine = {
        ...segment,
        group: freshId("group", new Set([...taken, segment.group])),
        period: withBounds(segment.period, on, segment.period.to),
    };
    return [...lines.slice(0, index), before, after, ...lines.slice(index + 1)];
}

/** Whether `on` falls strictly inside a period's span, so splitting there yields two non-empty halves. */
function spans(period: ScenarioPeriod, on: ISODate): boolean {
    return (period.from === null || period.from < on) && (period.to === null || on < period.to);
}

/**
 * Collapse every segment of logical row `id` back into one line spanning their
 * union.
 *
 * The earliest `from` and the latest `to` win, and an absent bound on any
 * segment means the union is unbounded on that side — so merging the two halves
 * a `splitStep` produced gives back exactly the line it split.
 *
 * The FIRST segment's amount, growth and note survive; the later ones are what
 * the step changed and are what the user asked to discard.
 */
export function mergeStep(lines: readonly ScenarioLine[], id: string): ScenarioLine[] {
    const segments = lines.filter((line) => line.id === id).sort(compareSegments);
    if (segments.length < 2) return [...lines];
    const first = segments[0];
    if (first.period.simple === null) return [...lines];

    const from = segments.some((s) => s.period.from === null) ? null : segments[0].period.from;
    const to = segments.some((s) => s.period.to === null) ? null : maxDate(segments.map((s) => s.period.to));
    const merged: ScenarioLine = {...first, period: withBounds(first.period, from, to)};

    const out: ScenarioLine[] = [];
    let placed = false;
    for (const line of lines) {
        if (line.id !== id) {
            out.push(line);
            continue;
        }
        // In the position the row already occupied, so merging does not move it.
        if (!placed) {
            out.push(merged);
            placed = true;
        }
    }
    return out;
}

function maxDate(dates: (ISODate | null)[]): ISODate | null {
    let best: ISODate | null = null;
    for (const date of dates) {
        if (date !== null && (best === null || date > best)) best = date;
    }
    return best;
}

/** Remove one SEGMENT (one `~` rule) by its group. */
export function removeSegment(lines: readonly ScenarioLine[], group: string): ScenarioLine[] {
    return lines.filter((line) => line.group !== group);
}

/** Remove a whole logical row and every segment of it. */
export function removeRow(lines: readonly ScenarioLine[], id: string): ScenarioLine[] {
    return lines.filter((line) => line.id !== id);
}

/**
 * Copy a logical row, segments and all, beneath the original.
 *
 * The copy is AUTHORED even when the original was seeded from history: a user
 * who duplicated an estimate and is about to edit it is no longer being shown an
 * average, and leaving the flag on would say they were.
 *
 * It carries no section HOLD either, for the same kind of reason: a hold belongs
 * to a row being edited, and nobody is editing a row that does not exist yet. A
 * copy that inherited one would keep it until somebody happened to focus and
 * leave the copy, since nothing else ever releases one.
 */
export function duplicateRow(lines: readonly ScenarioLine[], id: string, taken: ReadonlySet<string>): ScenarioLine[] {
    const segments = lines.filter((line) => line.id === id);
    if (segments.length === 0) return [...lines];
    const claimed = new Set(taken);
    const newId = freshId("line", claimed);
    claimed.add(newId);
    const copies = segments.map((segment) => {
        const group = freshId("group", claimed);
        claimed.add(group);
        return {...segment, id: newId, group, source: "journal" as const, section: undefined};
    });
    const last = lines.map((line) => line.id).lastIndexOf(id);
    return [...lines.slice(0, last + 1), ...copies, ...lines.slice(last + 1)];
}

// ---------------------------------------------------------------------------
// Growth
// ---------------------------------------------------------------------------

/** Trailing zeros dropped, so a rate of `3.00 %` shows as `3 %`. */
export function trimDec(d: Dec): Dec {
    let m = d.m;
    let p = d.p;
    while (p > 0 && m % 10n === 0n) {
        m /= 10n;
        p -= 1;
    }
    return {m, p};
}

/**
 * A wire rate (a FRACTION: `0.03`) → the percentage the UI shows (`3`).
 *
 * Exact, not `× 100` in floating point: `{m, p}` × 100 is `{m, p − 2}` whenever
 * there are two places to give up, and `{m × 100, p}` otherwise.
 */
export function percentFromRate(rate: Dec): Dec {
    return trimDec(rate.p >= 2 ? {m: rate.m, p: rate.p - 2} : {m: rate.m * 100n, p: rate.p});
}

/** The percentage a user typed (`2.5`) → the fraction the wire carries (`0.025`). */
export function rateFromPercent(percent: Dec): Dec {
    return {m: percent.m, p: percent.p + 2};
}

/** `week` → `wk`, for the table's narrow Growth column. */
export const GROWTH_UNIT_ABBREV: Record<Growth["unit"], string> = {week: "wk", month: "mo", year: "yr"};

// ---------------------------------------------------------------------------
// Whole scenarios
// ---------------------------------------------------------------------------

/** An empty, unnamed scenario — what the tab holds before a seed answers. */
export function emptyScenario(): Scenario {
    return {name: "", created: null, updated: null, lines: [], events: []};
}

/**
 * A deep, MUTABLE copy.
 *
 * `decodeScenario` freezes what it returns, as every decoder here does, and the
 * editor's `$state` has to be able to write into what it holds. Cloning at the
 * boundary is what keeps both true; `Dec` leaves are rebuilt too, so nothing
 * frozen survives anywhere in the tree the editor owns.
 */
export function cloneScenario(scenario: Scenario): Scenario {
    return {
        name: scenario.name,
        created: scenario.created,
        updated: scenario.updated,
        lines: scenario.lines.map((line) => ({
            ...line,
            amount: cloneAmount(line.amount),
            period: {...line.period},
            growth: line.growth === null ? null : {rate: {...line.growth.rate}, unit: line.growth.unit},
            // The balance override is an amount like any other, and the decoder
            // freezes it: a spread alone would hand the editor a frozen object
            // to write a new quantity into.
            opening: line.opening === null ? null : cloneAmount(line.opening),
        })),
        events: scenario.events.map((event) => ({
            ...event,
            postings: event.postings.map((posting) => ({account: posting.account, amount: cloneAmount(posting.amount)})),
        })),
    };
}

function cloneAmount(amount: ScenarioAmount): ScenarioAmount {
    return {commodity: amount.commodity, quantity: {m: amount.quantity.m, p: amount.quantity.p}, precision: amount.precision};
}

/**
 * The scenario, as `POST /api/projections/run` takes it.
 *
 * `period` goes back as the whole object it arrived as: `PeriodIn` is the one
 * inbound type without `deny_unknown_fields`, precisely so a client need not
 * strip the derived fields first, and `raw` is the only one the server reads.
 */
export function scenarioToWire(scenario: Scenario): WireScenarioIn {
    return {
        name: scenario.name,
        created: scenario.created,
        updated: scenario.updated,
        lines: scenario.lines.map((line) => ({
            id: line.id,
            group: line.group,
            // Sent explicitly, always. The engine defaults an absent `role` to
            // `flow` for the sake of older bodies, and relying on that default
            // would make an asset row project as an outflow the size of its
            // contribution.
            role: line.role,
            account: line.account,
            amount: {commodity: line.amount.commodity, quantity: encodeDec(line.amount.quantity), precision: line.amount.precision},
            period: {raw: line.period.raw},
            growth: line.growth === null ? null : {rate: encodeDec(line.growth.rate), unit: line.growth.unit},
            opening:
                line.opening === null
                    ? null
                    : {commodity: line.opening.commodity, quantity: encodeDec(line.opening.quantity), precision: line.opening.precision},
            note: line.note,
            source: line.source,
        })),
        events: scenario.events.map((event) => ({
            id: event.id,
            date: event.date,
            description: event.description,
            postings: event.postings.map((posting) => ({
                account: posting.account,
                amount: {commodity: posting.amount.commodity, quantity: encodeDec(posting.amount.quantity), precision: posting.amount.precision},
            })),
        })),
    };
}

/**
 * A scenario with the rows the engine would refuse dropped.
 *
 * A half-typed row — no account yet — is a `400` for the whole request, which
 * would blank all three charts while the user is still reaching for the combobox.
 * Dropping it instead projects everything that IS complete, which is the live
 * editing the body-carried scenario exists for.
 */
export function projectable(scenario: Scenario): Scenario {
    return {
        ...scenario,
        lines: scenario.lines.filter((line) => line.account.trim() !== ""),
        events: scenario.events
            .filter((event) => isIsoDate(event.date))
            .map((event) => ({...event, postings: event.postings.filter((posting) => posting.account.trim() !== "")}))
            .filter((event) => event.postings.length > 0),
    };
}

/**
 * The whole request body for a run.
 *
 * No `asOf`: the engine defaults it to ITS today, which is the machine the
 * journal is on and the same clock every other report on this page is read
 * against. Sending the browser's date instead would make a projection differ
 * from a balance sheet by a timezone. The start date the tab shows is
 * `Projection.start`, which the engine derives and returns.
 */
export function runBody(scenario: Scenario, window: {interval: string; count: number; depth: number}): RunProjectionBody {
    return {
        scenario: scenarioToWire(projectable(scenario)),
        interval: window.interval,
        count: window.count,
        depth: window.depth,
    };
}

// ---------------------------------------------------------------------------
// The name, and the file it becomes (plan 22, Phase 3, §"Save As, and the name")
// ---------------------------------------------------------------------------

/** The filename prefix the engine groups a projection file's listing under. */
const PROJECTION_PREFIX = "projection-";

/**
 * `projection-<slug>.journal`, or null when `name` has nothing to slug.
 *
 * A MIRROR of `projections::slug_filename` in the engine, and it has to agree
 * with it exactly: the dialog shows the resulting path before it writes, and a
 * path that turned out to be a different one would make that preview a lie.
 * The rules are the engine's:
 *
 * - lowercased, runs of non-alphanumerics collapsed to one `-`, ends trimmed;
 * - **ASCII alphanumerics only.** Not `\p{Alnum}`: a slug is a filename on
 *   somebody else's filesystem too, and a Unicode class would let the same name
 *   normalize differently on two machines. The DISPLAY name keeps every
 *   character — only the filename is narrowed.
 * - null when nothing survives, rather than `projection-.journal`, which is a
 *   name nobody chose.
 */
export function slugFilename(name: string): string | null {
    let slug = "";
    for (const c of name) {
        if (/[0-9A-Za-z]/.test(c)) slug += c.toLowerCase();
        else if (!slug.endsWith("-")) slug += "-";
    }
    slug = slug
        .replace(/^-+|-+$/g, "")
        .slice(0, 200)
        .replace(/-+$/, "");
    return slug === "" ? null : `${PROJECTION_PREFIX}${slug}.journal`;
}

/**
 * The id a Save As would write to: `<directory>/<filename>`, or the filename
 * alone at the root.
 *
 * Shown to the user BEFORE the write, which is the whole point — "the dialog
 * shows the resulting path before saving". Null when the name does not slug.
 */
export function projectionId(directory: string, name: string): string | null {
    const filename = slugFilename(name);
    if (filename === null) return null;
    const dir = directory.replace(/^\/+|\/+$/g, "");
    return dir === "" ? filename : `${dir}/${filename}`;
}
