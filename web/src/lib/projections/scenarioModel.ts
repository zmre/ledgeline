// Editing a scenario: magnitudes, the sections, step split/merge, and the
// encoder that puts one on the wire.
//
// Pure module — no Svelte, no DOM, no clock — so every decision below is tested
// by calling it rather than by building a screen to reach it (`web/README.md`).
// `scenarioStore.svelte.ts` is the thin `$state` wrapper that calls into here.
//
// # The sign is the engine's
//
// A scenario line as the engine sends it carries the JOURNAL's sign:
// `(income:salary) $-5188.33` is negative, `(expenses:rent) $4200` positive. The
// table shows and takes MAGNITUDES, the way the budget editor does for goals —
// a user typing an income line types what they earn. An edited line keeps what
// was typed as its `entry` and sends it as such; the ENGINE signs it by the
// account's resolved type (`account_types::signed_entry`, plan 26 B6). Nothing
// here negates an amount, so nothing here can get the sign wrong — moving a row
// from `expenses:rent` to `income:rent` just re-enters its magnitude
// ([`withAccount`]) and the engine re-signs it.
//
// # Sections: the engine's, previewed while typing
//
// Every line the engine sends says which section it belongs in
// (`ScenarioLine.resolved`). While the user retypes an account or a period the
// table still has to place the row, synchronously, for a name the engine has not
// seen — so [`resolvedSection`] previews the same rule from the engine's account
// table until the line round-trips. The two agree by construction (the rule is
// locked to the engine's by `scenario_signs_golden.rs`).

import {encodeDec} from "#lib/api/editMapping.js";
import type {RunProjectionBody, WireScenarioIn} from "#lib/api/native.js";
import {absDec} from "#lib/format/amounts.js";
import {cmp, type Dec} from "#lib/domain/money.js";
import {isAccountType, type AccountClasses, type AccountType} from "#lib/domain/accountClass.js";
import type {ISODate} from "#lib/domain/types.js";
import {isIsoDate as isIsoDateExact} from "#lib/url/params.js";
import type {
    AccountsRequest,
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

/**
 * Whether a string is a full ISO date, which is all `type=date` and the engine
 * accept.
 *
 * TRIMS first, which the shared predicate deliberately does not: this is also
 * asked of a journal's period `raw`, and `~ 2027-03-01 ` with a trailing space
 * is the same single-date rule as one without.
 */
export function isIsoDate(value: string): boolean {
    return isIsoDateExact(value.trim());
}

// ---------------------------------------------------------------------------
// Magnitudes
// ---------------------------------------------------------------------------

/**
 * Whether `account` is revenue by the engine's account table — the preview
 * half of [`resolvedSection`], for a name the engine has not placed yet.
 *
 * `isAccountType`, not `accountTypeOf(...) === "revenue"`: a declared
 * `type: G` gain is revenue to the engine (hledger's `type:R` query matches one).
 */
export function isRevenueAccount(account: string, declared: AccountClasses): boolean {
    return isAccountType(account, declared, "revenue");
}

/** The magnitude the table shows: what was typed, else the engine's signed amount's size. */
export function magnitudeOf(line: ScenarioLine): Dec {
    return absDec(line.entry ?? line.amount.quantity);
}

/**
 * The line re-entered at `magnitude`, which the engine will sign by account type.
 *
 * The display precision is RAISED to fit the new quantity and never lowered.
 * Raising it is required — an amount whose precision was below its own places
 * would be rounded away by the growth walk — and lowering it is not wanted:
 * retyping a seeded `$1,875.00` as `1875` would otherwise quietly move that
 * line's growth onto whole dollars, which is a modelling change made by a
 * cosmetic edit.
 */
export function withMagnitude(line: ScenarioLine, magnitude: Dec): ScenarioLine {
    const entry = absDec(magnitude);
    return {...line, entry, amount: {...line.amount, precision: Math.max(line.amount.precision, entry.p)}};
}

/** A line with a new account, its magnitude re-entered so the engine re-signs it for the account's type. */
export function withAccount(line: ScenarioLine, account: string): ScenarioLine {
    return {...withMagnitude(line, magnitudeOf(line)), account};
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

/**
 * The section a line's ROLE, PERIOD and ACCOUNT TYPE put it in — the reading
 * that ignores any hold.
 *
 * The ENGINE's answer (`ScenarioLine.resolved`, from `projections::line_section`)
 * while the line still has the role, account and period it was answered for.
 * After an edit, a preview of the same rule from the engine's account table, in
 * the same order:
 *
 * 1. **`role: "asset"` wins outright** — a balance's section is its role, which
 *    no box on the row edits.
 * 2. **A single-date rule (`~ 2027-03-01`) is a one-off** to a reader, though the
 *    model keeps it a line (plan 22, amendment 12).
 * 3. **Revenue is Income; everything else is Expenses**, a recurring transfer to
 *    an asset included — hence "and other outflows".
 */
export function resolvedSection(line: ScenarioLine, declared: AccountClasses): LineSection {
    const engine = line.resolved;
    if (engine !== undefined && engine.role === line.role && engine.account === line.account && engine.raw === line.period.raw) return engine.section;
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
export function sectionOfLine(line: ScenarioLine, declared: AccountClasses): LineSection {
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

/** Every section's logical rows at once — what [`groupLines`] answers. */
export type SectionRows = Record<LineSection, LogicalRow[]>;

/**
 * Every line placed in its section and grouped into logical rows, segments in
 * span order — the whole table in ONE pass.
 *
 * Rows keep the order their FIRST segment appears in, so a table does not
 * reshuffle itself when a step is added to a row halfway down it.
 *
 * All four sections at once because the table wants all four, and asking one at
 * a time walked the scenario once per section — each walk resolving
 * `sectionOfLine` → `isRevenueAccount` → `accountTypeOf` for every line in
 * it, including the ones it was about to discard. That cost is per KEYSTROKE:
 * the scenario is deep `$state` and every one of those reads touches
 * `line.account`, so a single character invalidated all four derivations.
 *
 * The grouping itself is unchanged, which is what leaves THE SECTION-STABILITY
 * RULE exactly where it was: a row's identity is still `line.id`, and its
 * section is still whatever `sectionOfLine` says — hold and all.
 */
export function groupLines(lines: readonly ScenarioLine[], declared: AccountClasses): SectionRows {
    const sections: SectionRows = {income: [], expense: [], asset: [], oneoff: []};
    // Keyed by section AND id, not by id alone. Grouping WITHIN a section is
    // the behaviour a single pass has to keep: two segments of one row can
    // resolve into different sections, and joining them on id alone would put a
    // row in a table its own segment said it was not in.
    const byKey = new Map<string, LogicalRow>();
    for (const line of lines) {
        const section = sectionOfLine(line, declared);
        const key = `${section} ${line.id}`;
        const existing = byKey.get(key);
        if (existing === undefined) {
            const row: LogicalRow = {id: line.id, segments: [line]};
            byKey.set(key, row);
            sections[section].push(row);
        } else {
            existing.segments.push(line);
        }
    }
    for (const rows of Object.values(sections)) {
        for (const row of rows) row.segments.sort(compareSegments);
    }
    return sections;
}

/** One section's logical rows. The table itself reads [`groupLines`], once for all four. */
export function logicalRows(lines: readonly ScenarioLine[], declared: AccountClasses, section: LineSection): LogicalRow[] {
    return groupLines(lines, declared)[section];
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
 * `{interval words, from, to}` → the period expression a journal would carry.
 *
 * This is the string the engine re-parses through hledger's own grammar, so it
 * has to be hledger's spelling: `to` is EXCLUSIVE and the interval words come
 * first. `words` is a `simple` interval or an engine-supplied `base`; a period
 * with neither cannot be rebuilt, so callers must not reach here with one.
 */
export function periodRaw(words: string, from: ISODate | null, to: ISODate | null): string {
    let raw = words;
    if (from !== null) raw += ` from ${from}`;
    if (to !== null) raw += ` to ${to}`;
    return raw;
}

/**
 * The interval words a period's bounds are written after, or `null` when its
 * bounds cannot be rewritten (a one-off, or a recurrence the engine does not
 * understand — its `raw` is then the only reading of it).
 */
export function periodWords(period: ScenarioPeriod): string | null {
    return period.simple ?? period.base;
}

/**
 * Why a period's From/To cannot be edited, or `null` when they can. The text is
 * the boxes' tooltip, so a disabled box says why it is disabled.
 */
export function boundsLockedReason(period: ScenarioPeriod): string | null {
    if (periodWords(period) !== null) return null;
    return `The dates of “${period.raw}” can't be edited here: it isn't a recurrence this editor can rebuild. Edit the rule in the file instead.`;
}

/** A period rebuilt with new bounds, `raw` regenerated to match. Unchanged when its bounds cannot be rewritten. */
export function withBounds(period: ScenarioPeriod, from: ISODate | null, to: ISODate | null): ScenarioPeriod {
    const words = periodWords(period);
    if (words === null) return period;
    return {raw: periodRaw(words, from, to), simple: period.simple, base: period.base, from, to};
}

/** A period on a new interval, bounds kept, `raw` regenerated to match. */
export function withInterval(period: ScenarioPeriod, simple: ScenarioInterval): ScenarioPeriod {
    return {raw: periodRaw(simple, period.from, period.to), simple, base: null, from: period.from, to: period.to};
}

/** A bare monthly period — what a brand-new row opens as. */
export function monthlyPeriod(): ScenarioPeriod {
    return {raw: "monthly", simple: "monthly", base: null, from: null, to: null};
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
export function sectionPrefix(section: AddSection, accountNames: readonly string[], declared: AccountClasses): string {
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

/**
 * The display precision a row the USER adds opens at.
 *
 * Two, because a hand-typed row is money and money's unit of account is the
 * cent; a SEEDED row takes the engine's precision instead, which is the
 * journal's own. It is a module constant rather than a parameter of the three
 * constructors below because every call site passed exactly this — three
 * parameters threaded through seven calls to say "2" — and `withMagnitude` raises
 * it the moment a user types a third decimal anyway.
 */
const NEW_ROW_PRECISION = 2;

/** A blank monthly line for `section`, at zero in `commodity`. */
export function blankLine(id: string, commodity: string): ScenarioLine {
    const precision = NEW_ROW_PRECISION;
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
        // Plan 28: no counter-account yet, so the engine picks the default.
        counter: null,
        gains: null,
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
export function blankAssetLine(id: string, commodity: string): ScenarioLine {
    return {...blankLine(id, commodity), role: "asset"};
}

/** A blank dated event with one empty posting. */
export function blankEvent(id: string, date: ISODate, commodity: string): ScenarioEvent {
    const precision = NEW_ROW_PRECISION;
    return {id, date, description: "", postings: [{account: "", amount: {commodity, quantity: {m: 0n, p: precision}, precision}}], counter: null};
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
 * whose period has no interval words to rebuild (its `raw` is the only reading of
 * it), or a date already on a segment boundary.
 */
export function splitStep(lines: readonly ScenarioLine[], id: string, on: ISODate, taken: ReadonlySet<string>): ScenarioLine[] {
    if (!isIsoDate(on)) return [...lines];
    const index = lines.findIndex((line) => line.id === id && spans(line.period, on));
    if (index < 0) return [...lines];
    const segment = lines[index];
    if (periodWords(segment.period) === null) return [...lines];

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
    if (periodWords(first.period) === null) return [...lines];

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

// ---------------------------------------------------------------------------
// Counter-accounts and gains accounts (plan 28)
// ---------------------------------------------------------------------------

/** A typed account, or null for "the engine's default" — blank means none, as on the wire. */
export function accountOrNull(raw: string | null): string | null {
    const trimmed = (raw ?? "").trim();
    return trimmed === "" ? null : trimmed;
}

/**
 * Set ONE line's counter-account, keeping every rule writable.
 *
 * A `~` rule has ONE counter-account: its amountless posting (plan 28,
 * amendment 5), and the file writer refuses a rule whose lines name two. So
 * when the line shares its rule (`group`) with any other line whose counter
 * would now DIFFER — a different account, or none — the line moves into a rule
 * of its own, with a fresh `group`, rather than leaving a scenario that cannot
 * be saved. "None" counts as different on purpose: a counter-less line beside
 * a countered one is written as a virtual `(account)` posting, which hledger
 * leaves out of balancing, so the file would stop meaning what the projection
 * shows.
 *
 * A line alone in its rule, or one whose rule-mates already agree, keeps its
 * group, so a no-op edit changes nothing.
 */
export function withCounter(lines: readonly ScenarioLine[], id: string, group: string, counter: string | null, taken: ReadonlySet<string>): ScenarioLine[] {
    const value = accountOrNull(counter);
    const split = lines.some((line) => line.group === group && line.id !== id && line.counter !== value);
    const nextGroup = split ? freshId("group", new Set([...taken, group])) : group;
    return lines.map((line) => (line.id === id && line.group === group ? {...line, counter: value, group: nextGroup} : line));
}

/**
 * Set an asset row's gains account. A posting tag (`gains:`), so unlike a
 * counter it is per line and never splits a rule.
 */
export function withGains(lines: readonly ScenarioLine[], group: string, gains: string | null): ScenarioLine[] {
    const value = accountOrNull(gains);
    return lines.map((line) => (line.group === group && line.role === "asset" ? {...line, gains: value} : line));
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
// Order (Move up / Move down, and drag)
// ---------------------------------------------------------------------------

/**
 * The lines with every rule's postings gathered together, at the position of
 * the rule's FIRST line, otherwise in order.
 *
 * This is the order the file writer produces — one `~` block per `group`, the
 * blocks in first-appearance order, postings in line order — so after this
 * the table shows exactly what a save and a reload would. It is not a no-op on
 * every scenario: `withCounter` moves a line into a rule of its own without
 * moving it, which can leave a rule's lines apart.
 */
export function gatherRules(lines: readonly ScenarioLine[]): ScenarioLine[] {
    const byGroup = new Map<string, ScenarioLine[]>();
    for (const line of lines) {
        const run = byGroup.get(line.group);
        if (run === undefined) byGroup.set(line.group, [line]);
        else run.push(line);
    }
    return [...byGroup.values()].flat();
}

/**
 * Move logical row `id` one place up (`-1`) or down (`+1`) within `section`.
 *
 * # A row moves as the file can store it
 *
 * The order persists as the order of the scenario file's `~` blocks and of the
 * postings inside each one, so a move has to be one a file can state:
 *
 * - **Two rows that are postings of ONE rule** swap their postings within it.
 * - **Otherwise** the row's whole rule — every segment's rule, for a step
 *   change — moves past the neighbour's rule. A rule is one block, so a row
 *   that shares its rule with other rows (a budget rule with several
 *   postings) carries them along: the file has no way to put another rule
 *   between two postings of one.
 *
 * Step-change segments always move together: they are one row. Nothing here
 * touches a line's `section` hold or its account, so THE SECTION-STABILITY RULE
 * is untouched — a move changes a row's place within its section, never which
 * section it is in.
 *
 * Unchanged when there is nowhere to go (the first row moving up, the last
 * moving down) or when the two rows share a rule in a way no single move can
 * untangle (a step whose earlier segment shares a rule with the neighbour).
 */
export function moveRow(lines: readonly ScenarioLine[], declared: AccountClasses, section: LineSection, id: string, delta: -1 | 1): ScenarioLine[] {
    const ordered = gatherRules(lines);
    const rows = groupLines(ordered, declared)[section];
    const at = rows.findIndex((row) => row.id === id);
    const to = at + delta;
    if (at < 0 || to < 0 || to >= rows.length) return [...lines];
    const row = rows[at];
    const other = rows[to];
    const mine = new Set(row.segments.map((line) => line.group));
    const theirs = new Set(other.segments.map((line) => line.group));

    if ([...mine].some((group) => theirs.has(group))) {
        if (mine.size !== 1 || theirs.size !== 1) return [...lines];
        // One rule: swap the two rows' postings inside it. The later row's
        // postings take the earlier positions.
        const [earlier, later] = delta < 0 ? [other, row] : [row, other];
        const isOf = (target: LogicalRow) => (line: ScenarioLine) => line.id === target.id && mine.has(line.group);
        const slots = ordered.flatMap((line, index) => (isOf(earlier)(line) || isOf(later)(line) ? [index] : []));
        const content = [...ordered.filter(isOf(later)), ...ordered.filter(isOf(earlier))];
        const out = [...ordered];
        slots.forEach((slot, n) => (out[slot] = content[n]));
        return out;
    }

    // Different rules: lift every line of this row's rules out, and set them
    // down just before the neighbour's first rule (up) or after its last (down).
    const moving = ordered.filter((line) => mine.has(line.group));
    const rest = ordered.filter((line) => !mine.has(line.group));
    const anchor = delta < 0 ? rest.findIndex((line) => theirs.has(line.group)) : rest.findLastIndex((line) => theirs.has(line.group)) + 1;
    return [...rest.slice(0, anchor), ...moving, ...rest.slice(anchor)];
}

/**
 * Move logical row `id` to where row `target` is in `section` — a drag and
 * drop, as the sequence of single moves the menu would make.
 *
 * Built on [`moveRow`] rather than beside it so a drag can never produce an
 * order the menu could not, which is what keeps the two paths saving alike.
 * Stops early if a step makes no progress (a move no file could state).
 */
export function moveRowTo(lines: readonly ScenarioLine[], declared: AccountClasses, section: LineSection, id: string, target: string): ScenarioLine[] {
    const indexOf = (current: readonly ScenarioLine[], rowId: string) => groupLines(current, declared)[section].findIndex((row) => row.id === rowId);
    let current = [...lines];
    const goal = indexOf(current, target);
    let at = indexOf(current, id);
    if (goal < 0 || at < 0) return current;
    while (at !== goal) {
        const next = moveRow(current, declared, section, id, at < goal ? 1 : -1);
        const moved = indexOf(next, id);
        if (moved === at) break;
        // A move that carried a whole rule can overshoot the target; stopping
        // there is still the nearest order a file can store.
        const overshot = at < goal ? moved > goal : moved < goal;
        current = next;
        at = moved;
        if (overshot) break;
    }
    return current;
}

/** Move event `id` one place up or down. Events are one block each, so any order is storable. */
export function moveEvent(events: readonly ScenarioEvent[], id: string, delta: -1 | 1): ScenarioEvent[] {
    const at = events.findIndex((event) => event.id === id);
    const to = at + delta;
    if (at < 0 || to < 0 || to >= events.length) return [...events];
    const out = [...events];
    [out[at], out[to]] = [out[to], out[at]];
    return out;
}

/** Move event `id` to where event `target` is — a drag and drop. */
export function moveEventTo(events: readonly ScenarioEvent[], id: string, target: string): ScenarioEvent[] {
    const at = events.findIndex((event) => event.id === id);
    const to = events.findIndex((event) => event.id === target);
    if (at < 0 || to < 0 || at === to) return [...events];
    const out = events.filter((event) => event.id !== id);
    out.splice(to, 0, events[at]);
    return out;
}

// ---------------------------------------------------------------------------
// Descriptions (a `~` rule's header text)
// ---------------------------------------------------------------------------

/**
 * Set the description of rule `group` — every line of it, because a rule has
 * ONE header and the file writer takes it from the rule's first line. Setting
 * it on one line alone would be a description the file silently drops.
 */
export function withNote(lines: readonly ScenarioLine[], group: string, note: string): ScenarioLine[] {
    return lines.map((line) => (line.group === group ? {...line, note} : line));
}

/** How many lines share rule `group`, and so share its description. */
export function ruleSize(lines: readonly ScenarioLine[], group: string): number {
    return lines.reduce((n, line) => (line.group === group ? n + 1 : n), 0);
}

/** The engine's longest rule description (`periodic::MAX_DESCRIPTION_BYTES`). */
export const MAX_NOTE_BYTES = 256;

/**
 * Why `note` cannot be a rule's description, or `null` when it can.
 *
 * A MIRROR of what the file writer refuses (`serialize::rule_header`), so the
 * editor can say so while the user is typing rather than the save failing
 * later. The header is `~ PERIOD  DESCRIPTION` on one line: a line break would
 * end it, and `;` or `#` would start a comment that swallows the rest.
 */
export function noteProblem(note: string): string | null {
    // eslint-disable-next-line no-control-regex -- control characters are exactly what is being refused
    if (/[\u0000-\u001f\u007f]/.test(note)) return "A description can't contain a line break or a control character.";
    if (note.includes(";") || note.includes("#")) return "A description can't contain “;” or “#”: the file would read the rest as a comment.";
    const bytes = new TextEncoder().encode(note.trim()).length;
    if (bytes > MAX_NOTE_BYTES) return `A description can be at most ${MAX_NOTE_BYTES} bytes; this one is ${bytes}.`;
    return null;
}

/** The start of the description the seed writes on an estimated row (`projections::seed_scenario`). */
const SEEDED_NOTE_PREFIX = "unbudgeted — average over ";

/**
 * Whether a line's description is the seed's automatic "average over …" text
 * AND its amount has since been changed, so the text describes a figure the
 * row no longer holds.
 *
 * Flagged, never rewritten: the description is the user's to edit, and a
 * stale note is still a true statement about where the row started.
 */
export function noteIsStale(line: ScenarioLine): boolean {
    if (!line.note.startsWith(SEEDED_NOTE_PREFIX) || line.entry === undefined) return false;
    return cmp(absDec(line.entry), absDec(line.amount.quantity)) !== 0;
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
    return {name: "", created: null, updated: null, lines: [], events: [], cash: null};
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
            ...(line.entry === undefined ? {} : {entry: {m: line.entry.m, p: line.entry.p}}),
        })),
        events: scenario.events.map((event) => ({
            ...event,
            postings: event.postings.map((posting) => ({account: posting.account, amount: cloneAmount(posting.amount)})),
        })),
        cash: scenario.cash,
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
            // Only for an edited line: the engine signs it by account type.
            ...(line.entry === undefined ? {} : {entry: encodeDec(line.entry)}),
            period: {raw: line.period.raw},
            growth: line.growth === null ? null : {rate: encodeDec(line.growth.rate), unit: line.growth.unit},
            opening:
                line.opening === null
                    ? null
                    : {commodity: line.opening.commodity, quantity: encodeDec(line.opening.quantity), precision: line.opening.precision},
            note: line.note,
            // Plan 28: blank is "none" on the wire too, but trimmed here so a
            // half-cleared box is never sent as an account named " ".
            counter: accountOrNull(line.counter),
            gains: accountOrNull(line.gains),
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
            counter: accountOrNull(event.counter),
        })),
        cash: accountOrNull(scenario.cash),
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
export function runBody(
    scenario: Scenario,
    window: {interval: string; count: number; lookback: number; depth: number; accounts?: AccountsRequest | null; journal?: boolean}
): RunProjectionBody {
    return {
        scenario: scenarioToWire(projectable(scenario)),
        interval: window.interval,
        count: window.count,
        lookback: window.lookback,
        depth: window.depth,
        // Sent only when asked for, so a run for the other tabs is the body it always was.
        ...(window.accounts != null ? {accounts: {mode: window.accounts.mode, empty: window.accounts.empty}} : {}),
        ...(window.journal === true ? {journal: true} : {}),
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
