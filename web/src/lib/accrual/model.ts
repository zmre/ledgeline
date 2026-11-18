// Pure helpers shared by the accrual UI (plan 45 Phase 3): month labels, the
// Post-through date, entry keys, the source posting's tags, and schedule specs
// on the way to the wire. Display arithmetic only — the engine computes every
// schedule, due set, tie and restatement; nothing here decides what is due.

import {decToInput} from "#lib/api/editMapping.js";
import type {WireScheduleSpecIn} from "#lib/api/native.js";
import type {ISODate} from "#lib/domain/types.js";
import type {AccrualRole, ContractKind, DueEntry, Party, ScheduleSpec, Term} from "./types";

/** The term chips (D-editor): months. */
export const TERM_CHIPS = [1, 3, 6, 12, 24, 36] as const;

/** "Needs a schedule" looks back this many days by default (Risks: adoption noise). */
export const NEEDS_LOOKBACK_DEFAULT = 90;
/** The ▾'s choices for that lookback; 0 is everything. */
export const NEEDS_LOOKBACK_CHOICES = [30, 90, 180, 365, 0] as const;

// The source posting's tags (plan 45 D-vocab), as `accrual/mod.rs` names them.
export const RECOGNITION_TAG = "recognition";
export const SERVICE_START_TAG = "service-start";
export const TERM_MONTHS_TAG = "term-months";
export const SERVICE_END_TAG = "service-end";
export const PRORATION_TAG = "proration";
export const CONTRACT_TAG = "contract";
export const CUSTOMER_TAG = "customer";
export const VENDOR_TAG = "vendor";
export const INVOICE_TAG = "invoice";
export const MATCH_TAG = "match";

const SHORT_MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function monthParts(month: string): [number, number] {
    const year = Number(month.slice(0, 4));
    const m = Number(month.slice(5, 7));
    return [year, m];
}

/** `2026-11` → "Nov 2026". */
export function monthShort(month: string): string {
    const [year, m] = monthParts(month);
    return `${SHORT_MONTHS[m - 1] ?? month} ${year}`;
}

/** `2026-11-17` → "Nov 17". */
export function dayShort(date: ISODate): string {
    const m = Number(date.slice(5, 7));
    return `${SHORT_MONTHS[m - 1] ?? date.slice(5, 7)} ${Number(date.slice(8, 10))}`;
}

/** The last day of `YYYY-MM`. */
export function endOfMonth(month: string): ISODate {
    const [year, m] = monthParts(month);
    const last = new Date(Date.UTC(year, m, 0)).getUTCDate();
    return `${month}-${String(last).padStart(2, "0")}`;
}

/**
 * **Post through ‹date›** (D6): the end of the latest checked month, capped at
 * today. Null when nothing is checked.
 */
export function postThroughDate(checkedMonths: Iterable<string>, today: ISODate): ISODate | null {
    let latest: string | null = null;
    for (const month of checkedMonths) if (latest === null || month > latest) latest = month;
    if (latest === null) return null;
    const end = endOfMonth(latest);
    return end > today ? today : end;
}

/** The key of one due occurrence: schedule id and scheduled date. */
export function entryKey(entry: Pick<DueEntry, "id" | "recognized">): string {
    return `${entry.id}@${entry.recognized}`;
}

/** The tri-state of a checkbox over `keys`: none, some (indeterminate) or all of them checked. */
export function checkState(keys: readonly string[], checked: ReadonlySet<string>): "none" | "some" | "all" {
    const count = keys.filter((key) => checked.has(key)).length;
    if (count === 0) return "none";
    return count === keys.length ? "all" : "some";
}

/** "customer: Customer X" as the party's tag. */
export function partyTag(party: Party): readonly [string, string] {
    return [party.kind === "customer" ? CUSTOMER_TAG : VENDOR_TAG, party.name];
}

/** Whether `role` is a deferral (recognized over time) rather than a settlement account. */
export function isDeferralRole(role: AccrualRole | null | undefined): role is "deferred-revenue" | "prepaid" {
    return role === "deferred-revenue" || role === "prepaid";
}

/** Whether `role` is a settlement (receivable/payable) account. */
export function isSettlementRole(role: AccrualRole | null | undefined): role is "receivable" | "payable" {
    return role === "receivable" || role === "payable";
}

/** The party kind a role implies: revenue sides have customers, expense sides vendors. */
export function partyKindFor(role: AccrualRole): Party["kind"] {
    return role === "deferred-revenue" || role === "receivable" ? "customer" : "vendor";
}

/**
 * A fresh schedule id: six lowercase letters and digits, like the engine's,
 * avoiding `taken`. The client picks it because the source posting carries it
 * (`recognition: ID`) in the same save that the rules do.
 */
export function newScheduleId(taken: ReadonlySet<string> = new Set(), random: () => number = defaultRandom): string {
    const alphabet = "abcdefghijklmnopqrstuvwxyz0123456789";
    for (let attempt = 0; attempt < 100; attempt += 1) {
        let id = "";
        // A letter first, so an id never reads as a number.
        id += alphabet[Math.floor(random() * 26)];
        for (let i = 1; i < 6; i += 1) id += alphabet[Math.floor(random() * alphabet.length)];
        if (!taken.has(id)) return id;
    }
    throw new Error("Could not draw a fresh schedule id.");
}

function defaultRandom(): number {
    const cryptoApi = globalThis.crypto;
    if (cryptoApi?.getRandomValues !== undefined) {
        const buffer = new Uint32Array(1);
        cryptoApi.getRandomValues(buffer);
        return (buffer[0] ?? 0) / 2 ** 32;
    }
    return Math.random();
}

/** What a schedule form holds (D-editor): one schedule over one or more source lines. */
export interface ScheduleDraft {
    id: string;
    /** Transaction date: `service-start` is written only when it differs. */
    txnDate: ISODate;
    start: ISODate;
    term: Term;
    proration: ScheduleSpec["proration"];
    contract: ContractKind | null;
    party: Party | null;
    description: string;
    lines: readonly {sourceAccount: string; targetAccount: string; commodity: string; quantity: string}[];
}

/**
 * The tags the source posting carries for `draft` (D-vocab), in the order the
 * engine's invoice builder writes them: party, `recognition`, `service-start`
 * (only when it isn't the transaction date), `term-months` or `service-end`,
 * `proration: calendar`, `contract`.
 */
export function sourceTags(draft: ScheduleDraft): (readonly [string, string])[] {
    const tags: (readonly [string, string])[] = [];
    if (draft.party !== null && draft.party.name.trim() !== "") tags.push(partyTag({...draft.party, name: draft.party.name.trim()}));
    tags.push([RECOGNITION_TAG, draft.id]);
    if (draft.start !== draft.txnDate) tags.push([SERVICE_START_TAG, draft.start]);
    if (draft.term.months !== undefined) tags.push([TERM_MONTHS_TAG, String(draft.term.months)]);
    else tags.push([SERVICE_END_TAG, draft.term.through]);
    if (draft.proration === "calendar") tags.push([PRORATION_TAG, "calendar"]);
    if (draft.contract !== null) tags.push([CONTRACT_TAG, draft.contract]);
    return tags;
}

/** The names `sourceTags` may write, which a re-applied form replaces. */
export const SOURCE_TAG_NAMES: ReadonlySet<string> = new Set([
    CUSTOMER_TAG,
    VENDOR_TAG,
    RECOGNITION_TAG,
    SERVICE_START_TAG,
    TERM_MONTHS_TAG,
    SERVICE_END_TAG,
    PRORATION_TAG,
    CONTRACT_TAG,
]);

/** A comment's comma segments, trimmed, empty ones dropped. */
function segments(line: string): string[] {
    return line
        .split(",")
        .map((segment) => segment.trim())
        .filter((segment) => segment !== "");
}

function tagName(segment: string): string | null {
    const colon = segment.indexOf(":");
    if (colon <= 0) return null;
    const words = segment.slice(0, colon).trim().split(/\s+/);
    return words[words.length - 1] ?? null;
}

/** The value of tag `name` in a posting comment, or null. */
export function commentTag(comment: string, name: string): string | null {
    for (const line of comment.split("\n")) {
        for (const segment of segments(line)) {
            if (tagName(segment) === name) return segment.slice(segment.indexOf(":") + 1).trim();
        }
    }
    return null;
}

/**
 * `comment` with `tags` set on its first line: any existing tag of a name in
 * `replace` (or in `tags`) is dropped first, everything else — free text, other
 * tags, later lines — is kept as written.
 */
export function withCommentTags(comment: string, tags: readonly (readonly [string, string])[], replace: ReadonlySet<string> = SOURCE_TAG_NAMES): string {
    const names = new Set([...replace, ...tags.map(([name]) => name)]);
    const [first = "", ...rest] = comment.split("\n");
    const kept = segments(first).filter((segment) => {
        const name = tagName(segment);
        return name === null || !names.has(name);
    });
    const head = [...kept, ...tags.map(([name, value]) => `${name}: ${value}`)].join(", ");
    const keptRest = rest.map((line) =>
        segments(line)
            .filter((segment) => {
                const name = tagName(segment);
                return name === null || !names.has(name);
            })
            .join(", ")
    );
    return [head, ...keptRest].filter((line, at) => at === 0 || line !== "").join("\n");
}

/** A draft's spec on the way to the wire. */
export function draftToWire(draft: ScheduleDraft): WireScheduleSpecIn {
    const party = draft.party !== null && draft.party.name.trim() !== "" ? {kind: draft.party.kind, name: draft.party.name.trim()} : undefined;
    return {
        id: draft.id,
        lines: draft.lines.map((line) => ({
            sourceAccount: line.sourceAccount,
            targetAccount: line.targetAccount,
            commodity: line.commodity,
            quantity: line.quantity,
            ...(party === undefined ? {} : {party}),
        })),
        start: draft.start,
        term: draft.term.months !== undefined ? {months: draft.term.months} : {through: draft.term.through},
        proration: draft.proration,
        description: draft.description,
    };
}

/** A spec the engine served (an existing schedule, a renewal prefill) as a draft. */
export function specToDraft(spec: ScheduleSpec, txnDate: ISODate, contract: ContractKind | null): ScheduleDraft {
    return {
        id: spec.id,
        txnDate,
        start: spec.start,
        term: spec.term,
        proration: spec.proration,
        contract,
        party: spec.lines.find((line) => line.party !== null)?.party ?? null,
        description: spec.description,
        lines: spec.lines.map((line) => ({
            sourceAccount: line.sourceAccount,
            targetAccount: line.targetAccount,
            commodity: line.total.commodity,
            quantity: decToInput(line.total.quantity),
        })),
    };
}

/** "12 months" / "through 2027-03-31". */
export function termLabel(term: Term): string {
    if (term.months !== undefined) return term.months === 1 ? "1 month" : `${term.months} months`;
    return `through ${term.through}`;
}

/** The default rule description: "Customer X Recognized", else the source's own description. */
export function defaultDescription(party: Party | null, sourceDescription: string): string {
    const who = party !== null && party.name.trim() !== "" ? party.name.trim() : sourceDescription.trim();
    return who === "" ? "Recognized" : `${who} Recognized`;
}

/** The journal page's link to a month's recognition group, expanded (Q7, toasts). */
export function journalMonthHref(month: string): string {
    return `/?from=${month}-01&to=${endOfMonth(month)}&expand=${month}`;
}
