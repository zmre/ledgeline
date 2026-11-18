// Wire → domain decoders for the accrual routes (plan 45 Phase 2's table).
//
// The discipline of journalViewDecode.ts: every field is checked, a missing or
// mistyped one throws `ApiShapeError`, decoded objects are frozen, and exact
// decimals travel as `{m, p}`. Every GET answers `{enabled: false}` when the
// helpers are off for the ledger (D-gate); those decoders return null then, so
// a caller cannot mistake "off" for an empty list.

import {ApiShapeError} from "#lib/api/client.js";
import {arr, bool, decodeViewDec, num, obj, str, strings} from "#lib/api/journalViewDecode.js";
import type {Dec} from "#lib/domain/money.js";
import {
    ACCRUAL_ROLES,
    CONTRACT_KINDS,
    type AccrualRole,
    type AccrualStatus,
    type ContractKind,
    type DueEntry,
    type DueMonth,
    type DuePosting,
    type DueTie,
    type DueView,
    type FitKind,
    type Money,
    type NeedsView,
    type Party,
    type PeriodPart,
    type PeriodStatus,
    type PlannedEntry,
    type PostResult,
    type Proration,
    type Restatement,
    type RestatementAction,
    type SavePlan,
    type Schedule,
    type ScheduleSpec,
    type SettlementsView,
    type SettleProposal,
    type SettleSide,
    type Term,
    type TransactionSchedules,
} from "./types";

type Raw = Record<string, unknown>;

const freeze = <T>(items: T[]): readonly T[] => Object.freeze(items);

function oneOf<T extends string>(allowed: readonly T[], raw: unknown, context: string): T {
    const value = str(raw, context);
    if (!(allowed as readonly string[]).includes(value)) throw new ApiShapeError(`${context}: unexpected ${JSON.stringify(value)}`);
    return value as T;
}

const optStr = (raw: unknown, context: string): string | null => (raw === undefined || raw === null ? null : str(raw, context));

function whole(raw: unknown, context: string): number {
    const value = num(raw, context);
    if (!Number.isSafeInteger(value) || value < 0) throw new ApiShapeError(`${context}: expected a whole number`);
    return value;
}

function index(raw: unknown, context: string): number {
    const value = whole(raw, context);
    if (value < 1) throw new ApiShapeError(`${context}: invalid index ${value}`);
    return value;
}

/** `{enabled: false}` → true; `{enabled: true, …}` → false; anything else throws. */
function disabled(body: Raw, context: string): boolean {
    return !bool(body.enabled, `${context}.enabled`);
}

export function decodeMoney(raw: unknown, context: string): Money {
    const value = obj(raw, context);
    return Object.freeze({commodity: str(value.commodity, `${context}.commodity`), quantity: decodeViewDec(value.quantity, `${context}.quantity`)});
}

const moneys = (raw: unknown, context: string): readonly Money[] => freeze(arr(raw, context).map((m, i) => decodeMoney(m, `${context}[${i}]`)));
const decs = (raw: unknown, context: string): readonly Dec[] => freeze(arr(raw, context).map((d, i) => decodeViewDec(d, `${context}[${i}]`)));

export function decodeParty(raw: unknown, context: string): Party {
    const value = obj(raw, context);
    return Object.freeze({kind: oneOf(["customer", "vendor"] as const, value.kind, `${context}.kind`), name: str(value.name, `${context}.name`)});
}

const optParty = (raw: unknown, context: string): Party | null => (raw === undefined || raw === null ? null : decodeParty(raw, context));

function tagPairs(raw: unknown, context: string): readonly (readonly [string, string])[] {
    return freeze(
        arr(raw, context).map((pair, i) => {
            const items = arr(pair, `${context}[${i}]`);
            if (items.length !== 2) throw new ApiShapeError(`${context}[${i}]: expected [name, value]`);
            return Object.freeze([str(items[0], `${context}[${i}][0]`), str(items[1], `${context}[${i}][1]`)] as const);
        })
    );
}

/** `GET /api/accrual` → the status, or null when the helpers are off. */
export function decodeAccrualStatus(raw: unknown): AccrualStatus | null {
    const body = obj(raw, "accrual");
    if (disabled(body, "accrual")) return null;
    const basis = obj(body.basis, "accrual.basis");
    const reason = obj(body.reason, "accrual.reason");
    const rolesRaw = obj(body.roles, "accrual.roles");
    const roles = new Map<string, {role: AccrualRole; declared: boolean}>();
    for (const [account, entry] of Object.entries(rolesRaw)) {
        const value = obj(entry, `accrual.roles[${account}]`);
        roles.set(
            account,
            Object.freeze({
                role: oneOf(ACCRUAL_ROLES, value.role, `accrual.roles[${account}].role`),
                declared: bool(value.declared, `accrual.roles[${account}].declared`),
            })
        );
    }
    const file = body.file === undefined || body.file === null ? null : obj(body.file, "accrual.file");
    const counts = obj(body.counts, "accrual.counts");
    return Object.freeze({
        generation: str(body.generation, "accrual.generation"),
        basis: Object.freeze({
            value: oneOf(["cash", "accrual"] as const, basis.value, "accrual.basis.value"),
            source: oneOf(["books", "inferred"] as const, basis.source, "accrual.basis.source"),
        }),
        reason: Object.freeze({
            kind: oneOf(["basis", "declared", "inferred", "none"] as const, reason.kind, "accrual.reason.kind"),
            account: optStr(reason.account, "accrual.reason.account"),
            role: reason.role === undefined || reason.role === null ? null : oneOf(ACCRUAL_ROLES, reason.role, "accrual.reason.role"),
        }),
        roles,
        file:
            file === null
                ? null
                : Object.freeze({
                      path: str(file.path, "accrual.file.path"),
                      exists: bool(file.exists, "accrual.file.exists"),
                      included: bool(file.included, "accrual.file.included"),
                  }),
        closedThrough: optStr(body.closedThrough, "accrual.closedThrough"),
        counts: Object.freeze({
            due: whole(counts.due, "accrual.counts.due"),
            dueClosed: whole(counts.dueClosed, "accrual.counts.dueClosed"),
            needsSchedule: whole(counts.needsSchedule, "accrual.counts.needsSchedule"),
            invoices: whole(counts.invoices, "accrual.counts.invoices"),
        }),
        problem: optStr(body.problem, "accrual.problem"),
        warnings: strings(body.warnings, "accrual.warnings"),
    });
}

function decodeDuePosting(raw: unknown, context: string): DuePosting {
    const value = obj(raw, context);
    return Object.freeze({account: str(value.account, `${context}.account`), amounts: moneys(value.amounts, `${context}.amounts`)});
}

function decodeDueEntry(raw: unknown, context: string): DueEntry {
    const value = obj(raw, context);
    return Object.freeze({
        id: str(value.id, `${context}.id`),
        recognized: str(value.recognized, `${context}.recognized`),
        description: str(value.description, `${context}.description`),
        postings: freeze(arr(value.postings, `${context}.postings`).map((p, i) => decodeDuePosting(p, `${context}.postings[${i}]`))),
    });
}

function decodeTie(raw: unknown, context: string): DueTie {
    const value = obj(raw, context);
    return Object.freeze({
        account: str(value.account, `${context}.account`),
        commodity: str(value.commodity, `${context}.commodity`),
        before: decodeViewDec(value.before, `${context}.before`),
        after: decodeViewDec(value.after, `${context}.after`),
        scheduled: decodeViewDec(value.scheduled, `${context}.scheduled`),
        ties: bool(value.ties, `${context}.ties`),
    });
}

function decodeMonth(raw: unknown, context: string): DueMonth {
    const value = obj(raw, context);
    const month = str(value.month, `${context}.month`);
    if (!/^\d{4}-\d{2}$/.test(month)) throw new ApiShapeError(`${context}.month: expected YYYY-MM, got ${JSON.stringify(month)}`);
    return Object.freeze({
        month,
        count: whole(value.count, `${context}.count`),
        totals: moneys(value.totals, `${context}.totals`),
        groups: freeze(
            arr(value.groups, `${context}.groups`).map((g, i) => {
                const group = obj(g, `${context}.groups[${i}]`);
                return Object.freeze({
                    party: optParty(group.party, `${context}.groups[${i}].party`),
                    entries: freeze(
                        arr(group.entries, `${context}.groups[${i}].entries`).map((e, j) => decodeDueEntry(e, `${context}.groups[${i}].entries[${j}]`))
                    ),
                });
            })
        ),
        ties: freeze(arr(value.ties, `${context}.ties`).map((t, i) => decodeTie(t, `${context}.ties[${i}]`))),
    });
}

/** `GET /api/recognition/due` → the catch-up table, or null when off. */
export function decodeDue(raw: unknown): DueView | null {
    const body = obj(raw, "recognition due");
    if (disabled(body, "recognition due")) return null;
    const closed = obj(body.closed, "recognition due.closed");
    return Object.freeze({
        generation: str(body.generation, "recognition due.generation"),
        today: str(body.today, "recognition due.today"),
        closedThrough: optStr(body.closedThrough, "recognition due.closedThrough"),
        months: freeze(arr(body.months, "recognition due.months").map((m, i) => decodeMonth(m, `recognition due.months[${i}]`))),
        closed: Object.freeze({
            count: whole(closed.count, "recognition due.closed.count"),
            totals: moneys(closed.totals, "recognition due.closed.totals"),
            entries: freeze(arr(closed.entries, "recognition due.closed.entries").map((e, i) => decodeDueEntry(e, `recognition due.closed.entries[${i}]`))),
        }),
    });
}

/** `POST /api/recognition/post` → what was written. */
export function decodePostResult(raw: unknown): PostResult {
    const body = obj(raw, "recognition post");
    return Object.freeze({
        generation: str(body.generation, "recognition post.generation"),
        posted: freeze(
            arr(body.posted, "recognition post.posted").map((p, i) => {
                const entry = obj(p, `recognition post.posted[${i}]`);
                return Object.freeze({
                    id: str(entry.id, `recognition post.posted[${i}].id`),
                    recognized: str(entry.recognized, `recognition post.posted[${i}].recognized`),
                });
            })
        ),
        adjustments: strings(body.adjustments, "recognition post.adjustments"),
    });
}

export function decodeSide(raw: unknown, context: string): SettleSide {
    const value = obj(raw, context);
    return Object.freeze({
        index: index(value.index, `${context}.index`),
        posting: whole(value.posting, `${context}.posting`),
        fingerprint: optStr(value.fingerprint, `${context}.fingerprint`),
        date: str(value.date, `${context}.date`),
        description: str(value.description, `${context}.description`),
        account: str(value.account, `${context}.account`),
        amount: decodeMoney(value.amount, `${context}.amount`),
        invoice: optStr(value.invoice, `${context}.invoice`),
        party: optParty(value.party, `${context}.party`),
    });
}

const sides = (raw: unknown, context: string): readonly SettleSide[] => freeze(arr(raw, context).map((s, i) => decodeSide(s, `${context}[${i}]`)));

/** `GET /api/recognition/needs` → Needs a schedule, or null when off. */
export function decodeNeeds(raw: unknown): NeedsView | null {
    const body = obj(raw, "recognition needs");
    if (disabled(body, "recognition needs")) return null;
    return Object.freeze({
        generation: str(body.generation, "recognition needs.generation"),
        lookback: whole(body.lookback, "recognition needs.lookback"),
        items: sides(body.items, "recognition needs.items"),
    });
}

function decodeProposal(raw: unknown, context: string): SettleProposal {
    const value = obj(raw, context);
    const fit = obj(value.fit, `${context}.fit`);
    return Object.freeze({
        invoice: decodeSide(value.invoice, `${context}.invoice`),
        payment: decodeSide(value.payment, `${context}.payment`),
        fit: Object.freeze({
            kind: oneOf<FitKind>(["exact", "short", "over"], fit.kind, `${context}.fit.kind`),
            difference: fit.difference === undefined || fit.difference === null ? null : decodeViewDec(fit.difference, `${context}.fit.difference`),
        }),
        byNumber: bool(value.byNumber, `${context}.byNumber`),
        byParty: bool(value.byParty, `${context}.byParty`),
        gapDays: whole(value.gapDays, `${context}.gapDays`),
        ambiguous: bool(value.ambiguous, `${context}.ambiguous`),
        clear: bool(value.clear, `${context}.clear`),
        feeAccount: optStr(value.feeAccount, `${context}.feeAccount`),
    });
}

/** `GET /api/settlements` (or `/for/{index}`) → proposals and open sides, or null when off. */
export function decodeSettlements(raw: unknown): SettlementsView | null {
    const body = obj(raw, "settlements");
    if (disabled(body, "settlements")) return null;
    const counts = obj(body.counts, "settlements.counts");
    const open = obj(body.open, "settlements.open");
    return Object.freeze({
        generation: str(body.generation, "settlements.generation"),
        counts: Object.freeze({clear: whole(counts.clear, "settlements.counts.clear"), check: whole(counts.check, "settlements.counts.check")}),
        proposals: freeze(arr(body.proposals, "settlements.proposals").map((p, i) => decodeProposal(p, `settlements.proposals[${i}]`))),
        open: Object.freeze({invoices: sides(open.invoices, "settlements.open.invoices"), payments: sides(open.payments, "settlements.open.payments")}),
    });
}

function decodeTerm(raw: unknown, context: string): Term {
    const value = obj(raw, context);
    if (value.months !== undefined && value.through === undefined) {
        const months = whole(value.months, `${context}.months`);
        if (months < 1) throw new ApiShapeError(`${context}.months: at least 1`);
        return Object.freeze({months});
    }
    if (value.through !== undefined && value.months === undefined) return Object.freeze({through: str(value.through, `${context}.through`)});
    throw new ApiShapeError(`${context}: expected {months} or {through}`);
}

export function decodeSpec(raw: unknown, context: string): ScheduleSpec {
    const value = obj(raw, context);
    return Object.freeze({
        id: str(value.id, `${context}.id`),
        lines: freeze(
            arr(value.lines, `${context}.lines`).map((l, i) => {
                const line = obj(l, `${context}.lines[${i}]`);
                return Object.freeze({
                    sourceAccount: str(line.sourceAccount, `${context}.lines[${i}].sourceAccount`),
                    targetAccount: str(line.targetAccount, `${context}.lines[${i}].targetAccount`),
                    total: decodeMoney(line.total, `${context}.lines[${i}].total`),
                    party: optParty(line.party, `${context}.lines[${i}].party`),
                });
            })
        ),
        start: str(value.start, `${context}.start`),
        term: decodeTerm(value.term, `${context}.term`),
        proration: oneOf<Proration>(["anniversary", "calendar"], value.proration, `${context}.proration`),
        description: str(value.description, `${context}.description`),
        headerTags: tagPairs(value.headerTags, `${context}.headerTags`),
    });
}

function decodeSchedule(raw: unknown, context: string): Schedule {
    const value = obj(raw, context);
    return Object.freeze({
        spec: decodeSpec(value.spec, `${context}.spec`),
        periods: freeze(
            arr(value.periods, `${context}.periods`).map((p, i) => {
                const period = obj(p, `${context}.periods[${i}]`);
                return Object.freeze({
                    date: str(period.date, `${context}.periods[${i}].date`),
                    part: oneOf<PeriodPart>(["opening", "regular", "closing"], period.part, `${context}.periods[${i}].part`),
                    amounts: decs(period.amounts, `${context}.periods[${i}].amounts`),
                    status:
                        period.status === undefined
                            ? null
                            : oneOf<PeriodStatus>(["posted", "due", "closed", "covered", "future"], period.status, `${context}.periods[${i}].status`),
                    entry: period.entry === undefined ? null : index(period.entry, `${context}.periods[${i}].entry`),
                });
            })
        ),
        rules: freeze(
            arr(value.rules, `${context}.rules`).map((r, i) => {
                const rule = obj(r, `${context}.rules[${i}]`);
                return Object.freeze({
                    role: oneOf(["opening", "main", "final"] as const, rule.role, `${context}.rules[${i}].role`),
                    period: str(rule.period, `${context}.rules[${i}].period`),
                    text: str(rule.text, `${context}.rules[${i}].text`),
                });
            })
        ),
        renewalDate: str(value.renewalDate, `${context}.renewalDate`),
    });
}

/** `GET /api/recognition/for/{index}` → its schedules, or null when off. */
export function decodeTransactionSchedules(raw: unknown): TransactionSchedules | null {
    const body = obj(raw, "recognition for");
    if (disabled(body, "recognition for")) return null;
    const renewal = body.renewal === undefined || body.renewal === null ? null : obj(body.renewal, "recognition for.renewal");
    return Object.freeze({
        generation: str(body.generation, "recognition for.generation"),
        schedules: freeze(arr(body.schedules, "recognition for.schedules").map((s, i) => decodeSchedule(s, `recognition for.schedules[${i}]`))),
        unscheduled: freeze(arr(body.unscheduled, "recognition for.unscheduled").map((p, i) => whole(p, `recognition for.unscheduled[${i}]`))),
        renewal:
            renewal === null
                ? null
                : Object.freeze({
                      prefill: decodeSpec(renewal.prefill, "recognition for.renewal.prefill"),
                      contract: oneOf<ContractKind>(CONTRACT_KINDS, renewal.contract, "recognition for.renewal.contract"),
                  }),
        partySuggestion: optParty(body.partySuggestion, "recognition for.partySuggestion"),
    });
}

function decodePlannedEntry(raw: unknown, context: string): PlannedEntry {
    const value = obj(raw, context);
    return Object.freeze({
        date: str(value.date, `${context}.date`),
        description: str(value.description, `${context}.description`),
        tags: tagPairs(value.tags, `${context}.tags`),
        postings: freeze(arr(value.postings, `${context}.postings`).map((p, i) => decodeDuePosting(p, `${context}.postings[${i}]`))),
    });
}

function decodeRestatement(raw: unknown, context: string): Restatement {
    const value = obj(raw, context);
    const change = value.ruleChange === undefined || value.ruleChange === null ? null : obj(value.ruleChange, `${context}.ruleChange`);
    return Object.freeze({
        id: str(value.id, `${context}.id`),
        ruleChange:
            change === null
                ? null
                : Object.freeze({before: str(change.before, `${context}.ruleChange.before`), after: str(change.after, `${context}.ruleChange.after`)}),
        rows: freeze(
            arr(value.rows, `${context}.rows`).map((r, i) => {
                const row = obj(r, `${context}.rows[${i}]`);
                return Object.freeze({
                    index: index(row.index, `${context}.rows[${i}].index`),
                    date: str(row.date, `${context}.rows[${i}].date`),
                    recognized: str(row.recognized, `${context}.rows[${i}].recognized`),
                    recognizedAfter: optStr(row.recognizedAfter, `${context}.rows[${i}].recognizedAfter`),
                    closed: bool(row.closed, `${context}.rows[${i}].closed`),
                    before: decs(row.before, `${context}.rows[${i}].before`),
                    after: row.after === undefined || row.after === null ? null : decs(row.after, `${context}.rows[${i}].after`),
                    action: oneOf<RestatementAction>(["kept", "unchanged", "rewrite", "delete"], row.action, `${context}.rows[${i}].action`),
                });
            })
        ),
        deletes: freeze(arr(value.deletes, `${context}.deletes`).map((d, i) => index(d, `${context}.deletes[${i}]`))),
        adjustment: value.adjustment === undefined || value.adjustment === null ? null : decodePlannedEntry(value.adjustment, `${context}.adjustment`),
        totalAfter: decs(value.totalAfter, `${context}.totalAfter`),
    });
}

/** `POST /api/recognition/preview` / `save` → the plan, or null when off. */
export function decodeSavePlan(raw: unknown): SavePlan | null {
    const body = obj(raw, "recognition plan");
    if (disabled(body, "recognition plan")) return null;
    return Object.freeze({
        accrualGeneration: str(body.accrualGeneration, "recognition plan.accrualGeneration"),
        appended: freeze(arr(body.appended, "recognition plan.appended").map((s, i) => decodeSchedule(s, `recognition plan.appended[${i}]`))),
        restatements: freeze(
            arr(body.restatements, "recognition plan.restatements").map((r, i) => decodeRestatement(r, `recognition plan.restatements[${i}]`))
        ),
        createsFile: bool(body.createsFile, "recognition plan.createsFile"),
        generation: optStr(body.generation, "recognition plan.generation"),
    });
}

/** `POST /api/recognition/file` → `{generation, path}`. */
export function decodeCreatedFile(raw: unknown): {generation: string; path: string} {
    const body = obj(raw, "recognition file");
    return Object.freeze({generation: str(body.generation, "recognition file.generation"), path: str(body.path, "recognition file.path")});
}

/** `POST /api/books/closed-through` → `{generation, closedThrough}`. */
export function decodeClosedThrough(raw: unknown): {generation: string; closedThrough: string | null} {
    const body = obj(raw, "closed-through");
    return Object.freeze({
        generation: str(body.generation, "closed-through.generation"),
        closedThrough: optStr(body.closedThrough, "closed-through.closedThrough"),
    });
}

/** `POST /api/settlements/link` → `{generation, matchId}`. */
export function decodeSettleLink(raw: unknown): {generation: string; matchId: string} {
    const body = obj(raw, "settlement link");
    return Object.freeze({generation: str(body.generation, "settlement link.generation"), matchId: str(body.matchId, "settlement link.matchId")});
}

/** `POST /api/settlements/link-all` → `{generation, linked, matchIds}`. */
export function decodeSettleLinkAll(raw: unknown): {generation: string; linked: number; matchIds: readonly string[]} {
    const body = obj(raw, "settlement link-all");
    return Object.freeze({
        generation: str(body.generation, "settlement link-all.generation"),
        linked: whole(body.linked, "settlement link-all.linked"),
        matchIds: strings(body.matchIds, "settlement link-all.matchIds"),
    });
}
