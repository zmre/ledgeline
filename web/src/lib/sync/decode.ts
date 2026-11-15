// Decoders for the bank sync routes (plan 43): raw JSON in, the typed wire
// shapes of `./types` out, or an `ApiShapeError` naming the field that broke.
//
// Strict like every decoder in `api/nativeDecode.ts`: the server answers every
// field, so an absent or mistyped one is a broken contract rather than a
// default. Kept beside the feature instead of in that 5000-line file because
// WP-W owns `lib/sync/**` and the sync section of `native.ts` only.

import {ApiShapeError} from "#lib/api/client.js";
import type {
    ConnectionState,
    NewAccountType,
    ProblemActionKind,
    ProblemCode,
    RunOutcome,
    Severity,
    WireConnectResult,
    WireConnection,
    WireHeld,
    WireLinkPreview,
    WireLinkedAccount,
    WireManualAccount,
    WireProblem,
    WireRemoteAccount,
    WireRunAccount,
    WireSharePreview,
    WireSyncRun,
    WireSyncStatus,
    Who,
} from "./types";

type Raw = Record<string, unknown>;

const PROBLEM_CODES: readonly ProblemCode[] = [
    "con-auth",
    "act-failed",
    "revoked",
    "payment",
    "quota",
    "network",
    "edge-blocked",
    "token-used",
    "stale",
    "hledger",
    "rules",
    "held",
    "balance",
    "target",
    "not-connected",
    "setup",
    "other",
];
const SEVERITIES: readonly Severity[] = ["info", "warning", "error"];
const WHOS: readonly Who[] = ["bank", "simplefin", "ledgeline", "network", "setup"];
const ACTION_KINDS: readonly ProblemActionKind[] = [
    "open-url",
    "reconnect",
    "retry",
    "edit-rules",
    "review-held",
    "choose-file",
    "connect",
    "set-hledger",
    "show-balance",
    "recheck",
];
const CONNECTION_STATES: readonly ConnectionState[] = ["ok", "syncing", "attention", "broken", "not-connected"];
const OUTCOMES: readonly RunOutcome[] = ["added", "nothing-new", "failed", "skipped"];
const ACCOUNT_TYPES: readonly NewAccountType[] = ["A", "L", "C"];

function obj(value: unknown, ctx: string): Raw {
    if (typeof value !== "object" || value === null || Array.isArray(value)) throw new ApiShapeError(`${ctx}: expected an object`);
    return value as Raw;
}
function arr(value: unknown, ctx: string): unknown[] {
    if (!Array.isArray(value)) throw new ApiShapeError(`${ctx}: expected an array`);
    return value;
}
function str(value: unknown, ctx: string): string {
    if (typeof value !== "string") throw new ApiShapeError(`${ctx}: expected a string`);
    return value;
}
function optStr(value: unknown, ctx: string): string | null {
    return value === null ? null : str(value, ctx);
}
function num(value: unknown, ctx: string): number {
    if (typeof value !== "number" || !Number.isFinite(value)) throw new ApiShapeError(`${ctx}: expected a number`);
    return value;
}
function optNum(value: unknown, ctx: string): number | null {
    return value === null ? null : num(value, ctx);
}
function bool(value: unknown, ctx: string): boolean {
    if (typeof value !== "boolean") throw new ApiShapeError(`${ctx}: expected a boolean`);
    return value;
}
function oneOf<T extends string>(list: readonly T[], value: unknown, ctx: string): T {
    if (typeof value !== "string" || !(list as readonly string[]).includes(value)) throw new ApiShapeError(`${ctx}: unexpected value ${JSON.stringify(value)}`);
    return value as T;
}
function sign(value: unknown, ctx: string): 1 | -1 {
    if (value !== 1 && value !== -1) throw new ApiShapeError(`${ctx}: expected 1 or -1`);
    return value;
}

export function decodeProblem(raw: unknown, ctx = "sync problem"): WireProblem {
    const p = obj(raw, ctx);
    let action: WireProblem["action"] = null;
    if (p.action !== null) {
        const a = obj(p.action, `${ctx} action`);
        action = {
            kind: oneOf(ACTION_KINDS, a.kind, `${ctx} action kind`),
            label: str(a.label, `${ctx} action label`),
            ...(a.url === undefined ? {} : {url: str(a.url, `${ctx} action url`)}),
            ...(a.ref === undefined ? {} : {ref: str(a.ref, `${ctx} action ref`)}),
        };
    }
    let secondary: WireProblem["secondary"] = null;
    if (p.secondary !== null) {
        const s = obj(p.secondary, `${ctx} secondary`);
        secondary = {kind: oneOf(["recheck"] as const, s.kind, `${ctx} secondary kind`), label: str(s.label, `${ctx} secondary label`)};
    }
    return {
        code: oneOf(PROBLEM_CODES, p.code, `${ctx} code`),
        severity: oneOf(SEVERITIES, p.severity, `${ctx} severity`),
        who: oneOf(WHOS, p.who, `${ctx} who`),
        message: str(p.message, `${ctx} message`),
        since: optNum(p.since, `${ctx} since`),
        dataThrough: optStr(p.dataThrough, `${ctx} dataThrough`),
        action,
        secondary,
        detail: optStr(p.detail, `${ctx} detail`),
        docAnchor: str(p.docAnchor, `${ctx} docAnchor`),
    };
}

const problems = (raw: unknown, ctx: string): WireProblem[] => arr(raw, ctx).map((p, i) => decodeProblem(p, `${ctx}[${i}]`));

function decodeConnection(raw: unknown, ctx: string): WireConnection {
    const c = obj(raw, ctx);
    const q = obj(c.quota, `${ctx} quota`);
    return {
        id: str(c.id, `${ctx} id`),
        name: str(c.name, `${ctx} name`),
        kind: oneOf(["simplefin"] as const, c.kind, `${ctx} kind`),
        host: optStr(c.host, `${ctx} host`),
        connected: bool(c.connected, `${ctx} connected`),
        state: oneOf(CONNECTION_STATES, c.state, `${ctx} state`),
        lastAttempt: optNum(c.lastAttempt, `${ctx} lastAttempt`),
        lastSuccess: optNum(c.lastSuccess, `${ctx} lastSuccess`),
        requestsToday: num(c.requestsToday, `${ctx} requestsToday`),
        quota: {autoMax: num(q.autoMax, `${ctx} quota`), manualWarn: num(q.manualWarn, `${ctx} quota`), manualMax: num(q.manualMax, `${ctx} quota`)},
        manageUrl: str(c.manageUrl, `${ctx} manageUrl`),
        problems: problems(c.problems, `${ctx} problems`),
    };
}

function decodeHeld(raw: unknown, ctx: string): WireHeld {
    const h = obj(raw, ctx);
    const e = obj(h.existing, `${ctx} existing`);
    return {
        heldId: str(h.heldId, `${ctx} heldId`),
        date: str(h.date, `${ctx} date`),
        amount: str(h.amount, `${ctx} amount`),
        payee: str(h.payee, `${ctx} payee`),
        existing: {
            index: num(e.index, `${ctx} existing index`),
            date: str(e.date, `${ctx} existing date`),
            description: str(e.description, `${ctx} existing description`),
            amount: str(e.amount, `${ctx} existing amount`),
        },
    };
}

function decodeLinked(raw: unknown, ctx: string): WireLinkedAccount {
    const a = obj(raw, ctx);
    const bank = a.bankBalance === null ? null : obj(a.bankBalance, `${ctx} bankBalance`);
    return {
        key: str(a.key, `${ctx} key`),
        connectionId: str(a.connectionId, `${ctx} connectionId`),
        remoteId: str(a.remoteId, `${ctx} remoteId`),
        label: str(a.label, `${ctx} label`),
        org: optStr(a.org, `${ctx} org`),
        journalAccount: str(a.journalAccount, `${ctx} journalAccount`),
        target: str(a.target, `${ctx} target`),
        rules: str(a.rules, `${ctx} rules`),
        categories: optStr(a.categories, `${ctx} categories`),
        startDate: str(a.startDate, `${ctx} startDate`),
        balanceSign: sign(a.balanceSign, `${ctx} balanceSign`),
        enabled: bool(a.enabled, `${ctx} enabled`),
        lastSuccess: optNum(a.lastSuccess, `${ctx} lastSuccess`),
        dataThrough: optStr(a.dataThrough, `${ctx} dataThrough`),
        lastAdded: num(a.lastAdded, `${ctx} lastAdded`),
        bankBalance: bank === null ? null : {amount: str(bank.amount, `${ctx} bankBalance amount`), date: str(bank.date, `${ctx} bankBalance date`)},
        ledgerBalance: optStr(a.ledgerBalance, `${ctx} ledgerBalance`),
        balanceDiff: optStr(a.balanceDiff, `${ctx} balanceDiff`),
        held: arr(a.held, `${ctx} held`).map((h, i) => decodeHeld(h, `${ctx} held[${i}]`)),
        history: arr(a.history, `${ctx} history`).map((raw, i) => {
            const h = obj(raw, `${ctx} history[${i}]`);
            return {
                at: num(h.at, `${ctx} history[${i}] at`),
                outcome: oneOf(OUTCOMES, h.outcome, `${ctx} history[${i}] outcome`),
                added: num(h.added, `${ctx} history[${i}] added`),
                held: num(h.held, `${ctx} history[${i}] held`),
            };
        }),
        problems: problems(a.problems, `${ctx} problems`),
        rulesChoices: arr(a.rulesChoices, `${ctx} rulesChoices`).map((raw, i) => {
            const c = obj(raw, `${ctx} rulesChoices[${i}]`);
            return {id: str(c.id, `${ctx} rulesChoices[${i}] id`), fits: bool(c.fits, `${ctx} rulesChoices[${i}] fits`)};
        }),
    };
}

export function decodeRemoteAccount(raw: unknown, ctx = "sync remote account"): WireRemoteAccount {
    const r = obj(raw, ctx);
    const s = obj(r.suggestion, `${ctx} suggestion`);
    const manual = s.manualRules === null ? null : obj(s.manualRules, `${ctx} manualRules`);
    return {
        connectionId: str(r.connectionId, `${ctx} connectionId`),
        remoteId: str(r.remoteId, `${ctx} remoteId`),
        label: str(r.label, `${ctx} label`),
        org: optStr(r.org, `${ctx} org`),
        balance: str(r.balance, `${ctx} balance`),
        balanceDate: optStr(r.balanceDate, `${ctx} balanceDate`),
        currency: str(r.currency, `${ctx} currency`),
        oldestAvailable: optStr(r.oldestAvailable, `${ctx} oldestAvailable`),
        suggestion: {
            journalAccount: optStr(s.journalAccount, `${ctx} suggestion journalAccount`),
            score: num(s.score, `${ctx} suggestion score`),
            ledgerBalance: optStr(s.ledgerBalance, `${ctx} suggestion ledgerBalance`),
            signFlipped: bool(s.signFlipped, `${ctx} suggestion signFlipped`),
            startDate: str(s.startDate, `${ctx} suggestion startDate`),
            target: optStr(s.target, `${ctx} suggestion target`),
            newAccountName: str(s.newAccountName, `${ctx} suggestion newAccountName`),
            newAccountType: oneOf(ACCOUNT_TYPES, s.newAccountType, `${ctx} suggestion newAccountType`),
            syncRules: arr(s.syncRules, `${ctx} syncRules`).map((raw, i) => {
                const f = obj(raw, `${ctx} syncRules[${i}]`);
                return {id: str(f.id, `${ctx} syncRules[${i}] id`), fits: bool(f.fits, `${ctx} syncRules[${i}] fits`)};
            }),
            manualRules:
                manual === null ? null : {id: str(manual.id, `${ctx} manualRules id`), ruleCount: num(manual.ruleCount, `${ctx} manualRules ruleCount`)},
        },
    };
}

export function decodeSyncStatus(raw: unknown): WireSyncStatus {
    const s = obj(raw, "sync status");
    const count = obj(s.problemCount, "sync status problemCount");
    return {
        autoSync: bool(s.autoSync, "sync status autoSync"),
        available: bool(s.available, "sync status available"),
        unavailableReason: optStr(s.unavailableReason, "sync status unavailableReason"),
        running: bool(s.running, "sync status running"),
        lastRun: optNum(s.lastRun, "sync status lastRun"),
        nextAuto: optNum(s.nextAuto, "sync status nextAuto"),
        connections: arr(s.connections, "sync status connections").map((c, i) => decodeConnection(c, `sync connection[${i}]`)),
        accounts: arr(s.accounts, "sync status accounts").map((a, i) => decodeLinked(a, `sync account[${i}]`)),
        unlinked: arr(s.unlinked, "sync status unlinked").map((r, i) => decodeRemoteAccount(r, `sync unlinked[${i}]`)),
        problemCount: {warning: num(count.warning, "sync problemCount warning"), error: num(count.error, "sync problemCount error")},
    };
}

export function decodeConnectResult(raw: unknown): WireConnectResult {
    const r = obj(raw, "sync connect");
    return {
        connectionId: str(r.connectionId, "sync connect connectionId"),
        accounts: arr(r.accounts, "sync connect accounts").map((a, i) => decodeRemoteAccount(a, `sync connect accounts[${i}]`)),
    };
}

export function decodeRemoteAccounts(raw: unknown): WireRemoteAccount[] {
    const r = obj(raw, "sync accounts");
    return arr(r.accounts, "sync accounts").map((a, i) => decodeRemoteAccount(a, `sync accounts[${i}]`));
}

export function decodeLinkPreviews(raw: unknown): WireLinkPreview[] {
    const r = obj(raw, "sync link preview");
    return arr(r.accounts, "sync link preview accounts").map((raw, i) => {
        const ctx = `sync link preview[${i}]`;
        const p = obj(raw, ctx);
        return {
            remoteId: str(p.remoteId, `${ctx} remoteId`),
            ok: bool(p.ok, `${ctx} ok`),
            error: p.error === null ? null : decodeProblem(p.error, `${ctx} error`),
            new: num(p.new, `${ctx} new`),
            already: num(p.already, `${ctx} already`),
            held: num(p.held, `${ctx} held`),
            uncategorized: num(p.uncategorized, `${ctx} uncategorized`),
            droppedBeforeStart: num(p.droppedBeforeStart, `${ctx} droppedBeforeStart`),
            entries: str(p.entries, `${ctx} entries`),
        };
    });
}

export function decodeSharePreview(raw: unknown): WireSharePreview {
    const p = obj(raw, "sync share preview");
    const diffs = obj(p.diffs, "sync share preview diffs");
    return {
        manualRulesId: str(p.manualRulesId, "sync share preview manualRulesId"),
        categoriesId: str(p.categoriesId, "sync share preview categoriesId"),
        moved: num(p.moved, "sync share preview moved"),
        kept: arr(p.kept, "sync share preview kept").map((raw, i) => {
            const k = obj(raw, `sync share preview kept[${i}]`);
            return {header: str(k.header, "sync share kept header"), reason: str(k.reason, "sync share kept reason")};
        }),
        diffs: {manual: str(diffs.manual, "sync share diffs manual"), categories: str(diffs.categories, "sync share diffs categories")},
        identical: bool(p.identical, "sync share preview identical"),
        differences: optStr(p.differences, "sync share preview differences"),
    };
}

function decodeRunAccount(raw: unknown, ctx: string): WireRunAccount {
    const a = obj(raw, ctx);
    return {
        key: str(a.key, `${ctx} key`),
        label: str(a.label, `${ctx} label`),
        added: num(a.added, `${ctx} added`),
        held: num(a.held, `${ctx} held`),
        already: num(a.already, `${ctx} already`),
        outcome: oneOf(OUTCOMES, a.outcome, `${ctx} outcome`),
        problem: a.problem === null ? null : decodeProblem(a.problem, `${ctx} problem`),
        ...(a.uncategorized === undefined ? {} : {uncategorized: num(a.uncategorized, `${ctx} uncategorized`)}),
    };
}

export function decodeSyncRun(raw: unknown): WireSyncRun {
    const r = obj(raw, "sync run");
    return {
        ok: bool(r.ok, "sync run ok"),
        requestsToday: num(r.requestsToday, "sync run requestsToday"),
        quotaWarning: bool(r.quotaWarning, "sync run quotaWarning"),
        accounts: arr(r.accounts, "sync run accounts").map((a, i) => decodeRunAccount(a, `sync run accounts[${i}]`)),
        problems: problems(r.problems, "sync run problems"),
    };
}

export function decodeManualAccounts(raw: unknown): WireManualAccount[] {
    const r = obj(raw, "sync manual");
    return arr(r.accounts, "sync manual accounts").map((raw, i) => {
        const ctx = `sync manual[${i}]`;
        const m = obj(raw, ctx);
        return {
            account: str(m.account, `${ctx} account`),
            lastTransaction: optStr(m.lastTransaction, `${ctx} lastTransaction`),
            lastImport: optStr(m.lastImport, `${ctx} lastImport`),
            downloadUrl: optStr(m.downloadUrl, `${ctx} downloadUrl`),
            nudge: bool(m.nudge, `${ctx} nudge`),
            stale: bool(m.stale, `${ctx} stale`),
        };
    });
}
