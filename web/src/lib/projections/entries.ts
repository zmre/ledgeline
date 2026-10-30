// The scenario as the list editor reads it: one entry per piece of the file, in
// FILE ORDER (plan 31 C1), and every edit the list makes, as a pure function of
// the scenario.
//
// The scenario's `layout` is the order. Every rule (a `~` block, by its lines'
// `group`), one-off, `=` rule, free-standing comment and piece of file text is
// named there exactly once, and the engine writes the file in that order — so
// "move this entry" is "move its layout item", whatever kind it is. Nothing here
// renumbers rows per kind, holds a row in a section or swaps postings between
// rules: those were the table's (`moveRow`, `gatherRules`, the section hold),
// and they are gone with it because nothing here moves while you type.
//
// Every posting is badged by its account's TYPE, the counter-account included
// (explicit or the engine's default), and the type filters match an entry by
// ANY of those accounts: a deferred-revenue rule is both Income and Liability
// (Patrick's 2026-10-29 feedback). The header keeps only the entry's KIND —
// one-off, auto — never a section read off its first posting.
//
// Keys are one namespace across every kind (`takenIds` covers them all, and the
// engine's own keys — `rule:K`, `off:K`, `auto:B`, `comment:N`, `fixed:N` — never
// collide), so a key alone names one entry.
//
// Pure module: no Svelte, no DOM. `entryDraft.svelte.ts` builds the dialog's
// drafts from these, and the list component calls them and hands the result to
// `scenarioStore.commit`, so every edit is one revision and one run.

import {accountTypeOf, type AccountClasses} from "#lib/domain/accountClass.js";
import type {ISODate} from "#lib/domain/types.js";
import {
    blankAssetLine,
    blankAutoRule,
    blankEvent,
    blankLine,
    freshId,
    isIsoDate,
    eventBalances,
    linesBalance,
    mergeStep,
    periodWords,
    takenIds,
    withBounds,
    withLayout,
} from "./scenarioModel";
import type {AutoRule, CommentStyle, FixedPiece, ForecastDefaults, ItemKind, LayoutItem, Scenario, ScenarioComment, ScenarioEvent, ScenarioLine} from "./types";

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/** An entry-KIND badge in a card's header. Text, never colour alone. */
export type Badge = "oneoff" | "auto";

/**
 * An account-TYPE badge on a posting: the account's resolved type
 * (`accountTypeOf`), with `revenue` called "income" as the rest of the app
 * calls it. Text, never colour alone.
 */
export type TypeBadge = "income" | "expense" | "asset" | "cash" | "liability" | "equity" | "gain" | "conversion";

/** The order type badges and their filter chips are listed in. */
export const TYPE_BADGES: readonly TypeBadge[] = ["income", "expense", "asset", "cash", "liability", "equity", "gain", "conversion"];

/** `account`'s type badge; null for no account, or an account with no type. */
export function typeBadgeOf(account: string, classes: AccountClasses): TypeBadge | null {
    const name = account.trim();
    if (name === "") return null;
    const type = accountTypeOf(name, classes);
    return type === null ? null : type === "revenue" ? "income" : type;
}

/**
 * The counter-account line a card draws: the block's own, else the engine's
 * default (`inferred`, drawn greyed; `account` null when the engine has not
 * named it yet). Null when there is none to draw — the postings balance on
 * their own, so nothing posts to a counter.
 *
 * Balance is asked twice, and either answer hides the line: of the postings
 * as they are now (`linesBalance`/`eventBalances`, so an edit shows at once),
 * and of the engine, which names a default for every enabled entry that does
 * not balance (`ForecastDefaults`). A turned-off entry is not in the run, so
 * only the first applies to it.
 */
export type CounterLine = {account: string | null; inferred: boolean} | null;

function counterLine(
    counter: string | null,
    balanced: boolean,
    fallback: string | undefined,
    defaults: ForecastDefaults | null,
    disabled: boolean
): CounterLine {
    if (counter !== null) return {account: counter, inferred: false};
    if (balanced) return null;
    if (fallback !== undefined) return {account: fallback, inferred: true};
    // The engine has run this entry and named no default: it balances.
    if (defaults !== null && !disabled) return null;
    return {account: null, inferred: true};
}

/** Where a rule sits in a step change: two or more `~` blocks whose postings share a `; line:` id (plan 31 C2). */
export interface StepInfo {
    /** 1-based, in span order. */
    index: number;
    of: number;
    /** Every segment's group, this one's included, in span order. */
    groups: string[];
}

interface EntryBase {
    key: string;
    /** Position in the layout, which is the file's order. */
    index: number;
    item: LayoutItem;
    /** A name for the entry, unique in the list: what every control on its card is labelled after. */
    name: string;
    disabled: boolean;
    badges: Badge[];
    /** Every type badge its accounts carry, the counter-account's included, in `TYPE_BADGES` order: what the type filters match. */
    types: TypeBadge[];
}

export type Entry =
    | (EntryBase & {kind: "rule"; lines: ScenarioLine[]; step: StepInfo | null; counter: CounterLine})
    | (EntryBase & {kind: "event"; event: ScenarioEvent; counter: CounterLine})
    | (EntryBase & {kind: "auto"; rule: AutoRule})
    | (EntryBase & {kind: "comment"; comment: ScenarioComment})
    | (EntryBase & {kind: "fixed"; fixed: FixedPiece});

/** The type badges of `accounts`, each once, in `TYPE_BADGES` order. */
function typesOf(accounts: readonly (string | null)[], classes: AccountClasses): TypeBadge[] {
    const found = new Set(accounts.map((account) => (account === null ? null : typeBadgeOf(account, classes))));
    return TYPE_BADGES.filter((badge) => found.has(badge));
}

/** A single-date rule (`~ 2027-03-01`) reads as a one-off, though the model keeps it a line (plan 22, amendment 12); a balance never does. */
function isOneOffRule(lines: readonly ScenarioLine[]): boolean {
    const first = lines[0];
    return first !== undefined && first.role !== "asset" && first.period.simple === null && isIsoDate(first.period.raw);
}

/** One line of a comment or attached comment, its comment character and one space stripped. */
function stripPrefix(line: string): string {
    return line.replace(/^[;#*]\s?/, "");
}

/**
 * An attached comment's display text: each line's `; ` stripped, no trailing
 * newline. What the dialog's "Comment above this entry" box holds.
 */
export function attachedDisplay(raw: string): string {
    if (raw === "") return "";
    return raw
        .replace(/\r?\n$/, "")
        .split(/\r?\n/)
        .map(stripPrefix)
        .join("\n");
}

/** The attached comment as the list shows it: the edited text when there is one, else the file's. */
export function attachedText(item: LayoutItem): string {
    return item.attachedText ?? attachedDisplay(item.attached);
}

/**
 * Every entry of `scenario`, in layout order, with its badges, account
 * types, counter line, step and name. `classes` should be the scenario's own
 * reading (`scenarioClasses`); `defaults` are the last run's, null before one.
 *
 * A layout item naming nothing the scenario holds is skipped rather than
 * thrown on: the engine refuses such a layout on save with its own sentence,
 * and the list should still draw everything else.
 */
export function entriesOf(full: Scenario, classes: AccountClasses, defaults: ForecastDefaults | null = null): Entry[] {
    const scenario = withLayout(full);
    const groups = new Map<string, ScenarioLine[]>();
    for (const line of scenario.lines) {
        const run = groups.get(line.group);
        if (run === undefined) groups.set(line.group, [line]);
        else run.push(line);
    }
    const events = new Map(scenario.events.map((event) => [event.id, event]));
    const autos = new Map(scenario.autoRules.map((rule) => [rule.key, rule]));
    const comments = new Map(scenario.comments.map((comment) => [comment.key, comment]));
    const fixed = new Map(scenario.fixed.map((piece) => [piece.key, piece]));
    const steps = stepsOf(scenario.lines);

    const out: Entry[] = [];
    scenario.layout.forEach((item, index) => {
        const base = {key: item.key, index, item, disabled: item.disabled};
        switch (item.kind) {
            case "rule": {
                const lines = groups.get(item.key);
                if (lines === undefined) return;
                const counter = counterLine(lines[0].counter, linesBalance(lines), defaults?.lines.get(item.key), defaults, item.disabled);
                out.push({
                    ...base,
                    kind: "rule",
                    lines,
                    step: steps.get(item.key) ?? null,
                    counter,
                    name: "",
                    badges: isOneOffRule(lines) ? ["oneoff"] : [],
                    types: typesOf([...lines.map((line) => line.account), counter?.account ?? null], classes),
                });
                return;
            }
            case "event": {
                const event = events.get(item.key);
                if (event === undefined) return;
                const counter = counterLine(event.counter, eventBalances(event), defaults?.events.get(event.id), defaults, item.disabled);
                out.push({
                    ...base,
                    kind: "event",
                    event,
                    counter,
                    name: "",
                    badges: ["oneoff"],
                    types: typesOf([...event.postings.map((posting) => posting.account), counter?.account ?? null], classes),
                });
                return;
            }
            case "auto": {
                const rule = autos.get(item.key);
                if (rule === undefined) return;
                const form = rule.form;
                const accounts = form === null ? [] : [form.matchAccount, ...form.postings.map((posting) => posting.account), form.counter];
                out.push({...base, kind: "auto", rule, name: "", badges: ["auto"], types: typesOf(accounts, classes)});
                return;
            }
            case "comment": {
                const comment = comments.get(item.key);
                if (comment !== undefined) out.push({...base, kind: "comment", comment, name: "", badges: [], types: []});
                return;
            }
            case "fixed": {
                const piece = fixed.get(item.key);
                if (piece !== undefined) out.push({...base, kind: "fixed", fixed: piece, name: "", badges: [], types: []});
                return;
            }
        }
    });
    return nameEntries(out);
}

/** Every step change's segments, keyed by group. A group sharing no line id with another is not a step. */
function stepsOf(lines: readonly ScenarioLine[]): Map<string, StepInfo> {
    const groupsById = new Map<string, Set<string>>();
    const fromOf = new Map<string, string>();
    for (const line of lines) {
        const set = groupsById.get(line.id) ?? new Set<string>();
        set.add(line.group);
        groupsById.set(line.id, set);
        if (!fromOf.has(line.group)) fromOf.set(line.group, line.period.from ?? "");
    }
    const out = new Map<string, StepInfo>();
    for (const set of groupsById.values()) {
        if (set.size < 2) continue;
        // Span order: an unbounded start first.
        const groups = [...set].sort((a, b) => {
            const left = fromOf.get(a) ?? "";
            const right = fromOf.get(b) ?? "";
            return left < right ? -1 : left > right ? 1 : 0;
        });
        groups.forEach((group, at) => {
            if (!out.has(group)) out.set(group, {index: at + 1, of: groups.length, groups});
        });
    }
    return out;
}

/** The first line of `text`, clipped for a label. */
function firstLine(text: string, max = 60): string {
    const line = text.split("\n").find((candidate) => candidate.trim() !== "") ?? "";
    const trimmed = line.trim();
    return trimmed.length > max ? `${trimmed.slice(0, max - 1)}…` : trimmed;
}

/** What a user calls an entry, before it is made unique. */
function baseName(entry: Entry): string {
    switch (entry.kind) {
        case "rule": {
            const note = entry.lines[0]?.note.trim() ?? "";
            const accounts = entry.lines.map((line) => line.account.trim()).filter((account) => account !== "");
            if (note !== "") return note;
            return accounts.length === 0 ? "a new rule" : accounts.join(", ");
        }
        case "event":
            return entry.event.description.trim() === "" ? `the one-off on ${entry.event.date}` : entry.event.description.trim();
        case "auto":
            return entry.rule.query.trim() === "" ? "a new automatic posting" : `automatic posting = ${entry.rule.query.trim()}`;
        case "comment": {
            const text = firstLine(entry.comment.text);
            const what = entry.comment.style === "section" ? "section" : "note";
            return text === "" ? `an empty ${what}` : `${what} ${text}`;
        }
        case "fixed":
            return `file text ${firstLine(entry.fixed.raw)}`;
    }
}

/**
 * One accessible name per entry, unique in the list: the entry's own words,
 * with its position added when two entries share them. A name that two `Edit`
 * buttons share addresses neither, for a screen reader or for a test.
 */
function nameEntries(entries: Entry[]): Entry[] {
    const bases = entries.map(baseName);
    const counts = new Map<string, number>();
    for (const base of bases) counts.set(base, (counts.get(base) ?? 0) + 1);
    return entries.map((entry, at) => {
        const base = bases[at];
        const name = (counts.get(base) ?? 0) > 1 ? `${base} (entry ${at + 1})` : base;
        return {...entry, name};
    });
}

// ---------------------------------------------------------------------------
// Filters (plan 31 C12)
// ---------------------------------------------------------------------------

/** All, one per account type, then the kinds. A type's URL value is its badge's word (`?show=income`). */
export type EntryFilter = "all" | TypeBadge | "oneoff" | "auto" | "off" | "notes";
export const ENTRY_FILTERS: readonly EntryFilter[] = ["all", ...TYPE_BADGES, "oneoff", "auto", "off", "notes"];

/** Whether `entry` is shown under `filter`, ignoring the section-heading exception (see `entryVisible`). */
export function entryMatches(entry: Entry, filter: EntryFilter): boolean {
    switch (filter) {
        case "all":
            return true;
        case "off":
            return entry.disabled;
        case "notes":
            return entry.kind === "comment";
        case "auto":
            return entry.kind === "auto";
        case "oneoff":
            return entry.badges.includes("oneoff");
        default:
            // An account type: ANY of the entry's accounts, its counter's included.
            return entry.types.includes(filter);
    }
}

/** Whether a comment is a section heading. Headings stay visible under every filter, so the context survives. */
export function isSection(entry: Entry): boolean {
    return entry.kind === "comment" && entry.comment.style === "section";
}

/** Whether `entry` is drawn under `filter`. */
export function entryVisible(entry: Entry, filter: EntryFilter): boolean {
    return entryMatches(entry, filter) || isSection(entry);
}

/** How many entries each filter would show, headings left out of every count but All and Notes. */
export function filterCounts(entries: readonly Entry[]): Record<EntryFilter, number> {
    const counts = Object.fromEntries(ENTRY_FILTERS.map((filter) => [filter, 0])) as Record<EntryFilter, number>;
    for (const entry of entries) {
        for (const filter of ENTRY_FILTERS) {
            if (entryMatches(entry, filter)) counts[filter] += 1;
        }
    }
    return counts;
}

// ---------------------------------------------------------------------------
// Payloads: what an entry holds besides its layout item
// ---------------------------------------------------------------------------

/** An entry's content, without its place in the file. What a draft edits and what an insert adds. */
export type Payload =
    | {kind: "rule"; lines: ScenarioLine[]}
    | {kind: "event"; event: ScenarioEvent}
    | {kind: "auto"; rule: AutoRule}
    | {kind: "comment"; comment: ScenarioComment};

/** A new, unplaced layout item. */
export function newItem(kind: ItemKind, key: string, attached = ""): LayoutItem {
    return {kind, key, origin: null, disabled: false, attached, raw: ""};
}

/** The key a payload is named by in the layout. */
export function payloadKey(payload: Payload): string {
    switch (payload.kind) {
        case "rule":
            return payload.lines[0]?.group ?? "";
        case "event":
            return payload.event.id;
        case "auto":
            return payload.rule.key;
        case "comment":
            return payload.comment.key;
    }
}

/** The payload `key` names in `scenario`, as the scenario holds it (not copied), or null. */
export function payloadOf(scenario: Scenario, key: string): Payload | null {
    const item = scenario.layout.find((candidate) => candidate.key === key);
    if (item === undefined) return null;
    switch (item.kind) {
        case "rule": {
            const lines = scenario.lines.filter((line) => line.group === key);
            return lines.length === 0 ? null : {kind: "rule", lines};
        }
        case "event": {
            const event = scenario.events.find((candidate) => candidate.id === key);
            return event === undefined ? null : {kind: "event", event};
        }
        case "auto": {
            const rule = scenario.autoRules.find((candidate) => candidate.key === key);
            return rule === undefined ? null : {kind: "auto", rule};
        }
        case "comment": {
            const comment = scenario.comments.find((candidate) => candidate.key === key);
            return comment === undefined ? null : {kind: "comment", comment};
        }
        case "fixed":
            return null;
    }
}

/**
 * `scenario` with `key`'s payload replaced by `payload`, in place in its
 * array: a rule's lines where its first line was, so the postings of other
 * rules keep their positions.
 */
export function replacePayload(scenario: Scenario, key: string, payload: Payload): Scenario {
    switch (payload.kind) {
        case "rule": {
            const at = scenario.lines.findIndex((line) => line.group === key);
            const rest = scenario.lines.filter((line) => line.group !== key);
            const where = at < 0 ? rest.length : scenario.lines.slice(0, at).filter((line) => line.group !== key).length;
            return {...scenario, lines: [...rest.slice(0, where), ...payload.lines, ...rest.slice(where)]};
        }
        case "event":
            return {...scenario, events: scenario.events.map((event) => (event.id === key ? payload.event : event))};
        case "auto":
            return {...scenario, autoRules: scenario.autoRules.map((rule) => (rule.key === key ? payload.rule : rule))};
        case "comment":
            return {...scenario, comments: scenario.comments.map((comment) => (comment.key === key ? payload.comment : comment))};
    }
}

/** `scenario` with `payload` added to its arrays (not yet placed in the layout). */
function addPayload(scenario: Scenario, payload: Payload): Scenario {
    switch (payload.kind) {
        case "rule":
            return {...scenario, lines: [...scenario.lines, ...payload.lines]};
        case "event":
            return {...scenario, events: [...scenario.events, payload.event]};
        case "auto":
            return {...scenario, autoRules: [...scenario.autoRules, payload.rule]};
        case "comment":
            return {...scenario, comments: [...scenario.comments, payload.comment]};
    }
}

// ---------------------------------------------------------------------------
// Mutations. Each returns a NEW scenario, or the same one when it refuses.
// ---------------------------------------------------------------------------

const indexOf = (scenario: Scenario, key: string): number => scenario.layout.findIndex((item) => item.key === key);

/**
 * Move `key`'s entry to `to` in the layout (its index once moved), clamped.
 *
 * Any entry may go anywhere the file can store it, past file text included.
 * File text itself never moves: its order relative to other file text is the
 * one order the engine refuses to change, and the list never drags it.
 */
export function moveEntry(full: Scenario, key: string, to: number): Scenario {
    const scenario = withLayout(full);
    const at = indexOf(scenario, key);
    if (at < 0 || scenario.layout[at].kind === "fixed") return full;
    const rest = scenario.layout.filter((_, index) => index !== at);
    const target = Math.max(0, Math.min(to, rest.length));
    if (target === at) return full;
    return {...scenario, layout: [...rest.slice(0, target), scenario.layout[at], ...rest.slice(target)]};
}

/** Move one place up (`-1`) or down (`+1`) — the menu's Move up/down and J/K. */
export function moveEntryBy(scenario: Scenario, key: string, delta: -1 | 1): Scenario {
    const at = indexOf(withLayout(scenario), key);
    return at < 0 ? scenario : moveEntry(scenario, key, at + delta);
}

/** Delete an entry and everything it holds. File text is never deleted here. */
export function removeEntry(full: Scenario, key: string): Scenario {
    const scenario = withLayout(full);
    const item = scenario.layout.find((candidate) => candidate.key === key);
    if (item === undefined || item.kind === "fixed") return full;
    const layout = scenario.layout.filter((candidate) => candidate.key !== key);
    switch (item.kind) {
        case "rule":
            return {...scenario, layout, lines: scenario.lines.filter((line) => line.group !== key)};
        case "event":
            return {...scenario, layout, events: scenario.events.filter((event) => event.id !== key)};
        case "auto":
            return {...scenario, layout, autoRules: scenario.autoRules.filter((rule) => rule.key !== key)};
        case "comment":
            return {...scenario, layout, comments: scenario.comments.filter((comment) => comment.key !== key)};
    }
}

/** Whether an entry of this kind can be turned off (commented out, plan 31 C7). */
export function canDisable(kind: ItemKind): boolean {
    return kind === "rule" || kind === "event" || kind === "auto";
}

/** `key`'s layout item with `disabled` set. Refused for comments and file text. */
export function setDisabled(full: Scenario, key: string, disabled: boolean): Scenario {
    const scenario = withLayout(full);
    const item = scenario.layout.find((candidate) => candidate.key === key);
    if (item === undefined || !canDisable(item.kind) || item.disabled === disabled) return full;
    return {...scenario, layout: scenario.layout.map((candidate) => (candidate.key === key ? {...candidate, disabled} : candidate))};
}

/** Turn an entry off, or back on. */
export function toggleDisabled(scenario: Scenario, key: string): Scenario {
    const item = withLayout(scenario).layout.find((candidate) => candidate.key === key);
    return item === undefined ? scenario : setDisabled(scenario, key, !item.disabled);
}

/**
 * Insert `payload` as a new entry directly after `afterKey`, or at the end
 * when `afterKey` is null (or names nothing). `item` defaults to a fresh one.
 */
export function insertEntry(
    full: Scenario,
    payload: Payload,
    afterKey: string | null,
    item: LayoutItem = newItem(payload.kind, payloadKey(payload))
): Scenario {
    const scenario = addPayload(withLayout(full), payload);
    const at = afterKey === null ? -1 : indexOf(scenario, afterKey);
    const where = at < 0 ? scenario.layout.length : at + 1;
    return {...scenario, layout: [...scenario.layout.slice(0, where), item, ...scenario.layout.slice(where)]};
}

/** The attached comment as edited text, or back to the file's own when it reads the same. */
export function setAttached(full: Scenario, key: string, text: string): Scenario {
    const scenario = withLayout(full);
    return {
        ...scenario,
        layout: scenario.layout.map((item) => {
            if (item.key !== key || !canDisable(item.kind)) return item;
            const rest = {...item};
            delete rest.attachedText;
            return text === attachedDisplay(item.attached) ? rest : {...rest, attachedText: text};
        }),
    };
}

/** A free-standing comment's text and style, marked edited so a save sends it for the engine to render. */
export function setComment(scenario: Scenario, key: string, text: string, style: CommentStyle): Scenario {
    return {
        ...scenario,
        comments: scenario.comments.map((comment) => (comment.key === key ? {...comment, text, style, edited: true} : comment)),
    };
}

// ---------------------------------------------------------------------------
// Copies and blanks
// ---------------------------------------------------------------------------

/**
 * A copy of `key`'s entry under fresh keys, and its new layout item: never
 * placed, never `origin`ed, so it is always rendered.
 *
 * A copied rule's lines get fresh ids as well as a fresh group: a copy is never
 * a step of the original, and sharing a `; line:` id would make it one. It is
 * AUTHORED (`source: journal`) even when the original was estimated. Its
 * attached comment comes along, as the file's own text.
 */
export function copyEntry(full: Scenario, key: string): {payload: Payload; item: LayoutItem} | null {
    const scenario = withLayout(full);
    const item = scenario.layout.find((candidate) => candidate.key === key);
    const payload = payloadOf(scenario, key);
    if (item === undefined || payload === null) return null;
    const taken = takenIds(scenario);
    const fresh = (prefix: string): string => {
        const id = freshId(prefix, taken);
        taken.add(id);
        return id;
    };
    const copied = clonePayload(payload);
    let copy: Payload;
    switch (copied.kind) {
        case "rule": {
            const group = fresh("group");
            copy = {kind: "rule", lines: copied.lines.map((line) => ({...line, id: fresh("line"), group, source: "journal", resolved: undefined}))};
            break;
        }
        case "event":
            copy = {kind: "event", event: {...copied.event, id: fresh("event")}};
            break;
        case "auto":
            copy = {kind: "auto", rule: {...copied.rule, key: fresh("auto"), block: null}};
            break;
        case "comment":
            copy = {kind: "comment", comment: {...copied.comment, key: fresh("comment")}};
            break;
    }
    const attached = item.attachedText === undefined ? {attached: item.attached} : {attached: "", attachedText: item.attachedText};
    return {payload: copy, item: {...newItem(item.kind, payloadKey(copy)), ...attached}};
}

/** A deep, mutable copy of a payload: what a draft edits without reaching the scenario. */
export function clonePayload(payload: Payload): Payload {
    return plainCopy(payload);
}

/**
 * A deep, plain copy of a possibly-proxied value. Svelte's `$state` proxies
 * refuse `structuredClone`, and JSON would lose the `bigint` mantissas; a
 * recursive copy does neither. Payloads hold only plain data.
 */
export function plainCopy<T>(value: T): T {
    if (Array.isArray(value)) return value.map((item: unknown) => plainCopy(item)) as T;
    if (value !== null && typeof value === "object") {
        return Object.fromEntries(Object.entries(value).map(([name, inner]) => [name, plainCopy(inner)])) as T;
    }
    return value;
}

/** Duplicate: the copy inserted directly below. Returns the new scenario and the copy's key. */
export function duplicateEntry(scenario: Scenario, key: string): {scenario: Scenario; key: string} | null {
    const copy = copyEntry(scenario, key);
    if (copy === null) return null;
    return {scenario: insertEntry(scenario, copy.payload, key, copy.item), key: copy.item.key};
}

/** Duplicate as alternative: the copy inserted below AND the original turned off, in one scenario. */
export function duplicateAsAlternative(scenario: Scenario, key: string): {scenario: Scenario; key: string} | null {
    const done = duplicateEntry(scenario, key);
    if (done === null) return null;
    return {scenario: setDisabled(done.scenario, key, true), key: done.key};
}

/** What the Add menu adds (plan 31 C3): Recurring flow · Asset · One-off · Automatic posting · Note · Section heading. */
export type AddKind = "flow" | "asset" | "oneoff" | "auto" | "note" | "section";

export interface BlankContext {
    /** The commodity a new amount opens in: the journal's dominant one. */
    commodity: string;
    /** The date a new one-off opens on. */
    date: ISODate;
    /** The account prefix a new asset opens on (`sectionPrefix`). */
    assetPrefix: string;
    /** The counter-account a new `=` rule balances against: the scenario's cash account. */
    cash: string | null;
}

/** A blank entry of `kind`, under a key `scenario` does not use. */
export function blankPayload(scenario: Scenario, kind: AddKind, context: BlankContext): Payload {
    const taken = takenIds(scenario);
    switch (kind) {
        case "flow":
            return {kind: "rule", lines: [blankLine(freshId("line", taken), context.commodity)]};
        case "asset":
            return {kind: "rule", lines: [{...blankAssetLine(freshId("line", taken), context.commodity), account: context.assetPrefix}]};
        case "oneoff":
            return {kind: "event", event: blankEvent(freshId("event", taken), context.date, context.commodity)};
        case "auto":
            return {kind: "auto", rule: {...blankAutoRule(context.cash), key: freshId("auto", taken)}};
        case "note":
        case "section":
            return {kind: "comment", comment: {key: freshId("comment", taken), style: kind, raw: "", text: "", edited: true}};
    }
}

/** Add a blank entry of `kind` after `afterKey` (or at the end). */
export function addEntry(scenario: Scenario, kind: AddKind, afterKey: string | null, context: BlankContext): {scenario: Scenario; key: string} {
    const payload = blankPayload(scenario, kind, context);
    return {scenario: insertEntry(scenario, payload, afterKey), key: payloadKey(payload)};
}

// ---------------------------------------------------------------------------
// Step changes (plan 31 C2)
// ---------------------------------------------------------------------------

/**
 * Why rule `group` cannot be split into a step at `on`, or null when it can.
 *
 * An asset rule is never stepped: two segments would each seed the journal's
 * balance and compound it over the whole span — the same money twice. A rate
 * change is a second rule with a `from`, not a step.
 */
export function stepProblem(lines: readonly ScenarioLine[], on: ISODate): string | null {
    if (lines.length === 0) return "There is no rule to split.";
    if (lines.some((line) => line.role === "asset")) return "An asset rule can't be stepped: each segment would compound the same balance.";
    const period = lines[0].period;
    if (periodWords(period) === null) return `“${period.raw}” isn't a recurrence this editor can split.`;
    if (!isIsoDate(on)) return "Choose the date the step starts on.";
    if ((period.from !== null && on <= period.from) || (period.to !== null && on >= period.to))
        return `The step must fall inside the rule's span (${period.raw}).`;
    return null;
}

/**
 * Split rule `group` at `on` into two blocks meeting there: every posting
 * bounded `to on` in the original, and copied, `from on`, into a new block
 * with a FRESH group. Each copy keeps its posting's id, which is what makes
 * the pair a step (the engine writes a shared `; line:` tag).
 *
 * The whole BLOCK splits, not one posting: a card is a block now, and a block
 * whose postings had different spans could not be written.
 */
export function splitBlock(full: Scenario, group: string, on: ISODate): {before: ScenarioLine[]; after: ScenarioLine[]} | null {
    const scenario = withLayout(full);
    const lines = scenario.lines.filter((line) => line.group === group);
    if (stepProblem(lines, on) !== null) return null;
    const next = freshId("group", takenIds(scenario));
    const copied = (clonePayload({kind: "rule", lines}) as {kind: "rule"; lines: ScenarioLine[]}).lines;
    return {
        before: copied.map((line) => ({...line, period: withBounds(line.period, line.period.from, on)})),
        after: copied.map((line) => ({
            ...line,
            group: next,
            source: "journal" as const,
            resolved: undefined,
            period: withBounds(line.period, on, line.period.to),
        })),
    };
}

/** Add a step change at `on`: the later segment inserted directly below. */
export function addStep(scenario: Scenario, group: string, on: ISODate): {scenario: Scenario; key: string} | null {
    const split = splitBlock(scenario, group, on);
    if (split === null) return null;
    const bounded = replacePayload(withLayout(scenario), group, {kind: "rule", lines: split.before});
    const key = split.after[0].group;
    return {scenario: insertEntry(bounded, {kind: "rule", lines: split.after}, group), key};
}

/**
 * Merge a step change back into one rule spanning every segment
 * (`mergeStep`, per posting id). The EARLIEST segment survives, in its own
 * place in the file; the later segments' entries go. Returns the surviving key.
 */
export function mergeSteps(full: Scenario, group: string): {scenario: Scenario; key: string} | null {
    const scenario = withLayout(full);
    const ids = [...new Set(scenario.lines.filter((line) => line.group === group).map((line) => line.id))];
    let lines = scenario.lines;
    for (const id of ids) lines = mergeStep(lines, id);
    if (lines.length === scenario.lines.length) return null;
    const groups = new Set(lines.map((line) => line.group));
    const merged = lines.find((line) => ids.includes(line.id));
    if (merged === undefined) return null;
    return {
        scenario: {...scenario, lines, layout: scenario.layout.filter((item) => item.kind !== "rule" || groups.has(item.key))},
        key: merged.group,
    };
}
