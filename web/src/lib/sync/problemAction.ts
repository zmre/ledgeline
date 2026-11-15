// What a bank sync problem's buttons DO (plan 43 §6): the server says which
// action fits ("Fix at SimpleFIN ↗", "Reconnect…", "Edit rules"), and this
// turns it into an effect the screen can carry out, scoped to the connection
// or account the problem hangs off.
//
// Pure, so the whole §6 table is unit-tested. Relative imports only.

import type {Severity, WireConnection, WireLinkedAccount, WireProblem, WireSyncStatus, Who} from "./types";

/** Where every problem's details link points: `docs/imports/bank-sync.md`, as the docs site publishes it. */
export const SYNC_DOCS = "https://zmre.github.io/ledgeline/imports/bank-sync/";

/** SimpleFIN Bridge's own site — where accounts, institutions and app connections are managed. */
export const SIMPLEFIN_URL = "https://beta-bridge.simplefin.org/";

/** What the problem is about: the connection, and the account when it is one account's. */
export interface ProblemScope {
    connectionId: string | null;
    accountKey: string | null;
}

export type ProblemEffect =
    /** Open a web page in the browser. Always https (see `safeUrl`). */
    | {type: "url"; url: string}
    /** Paste a new setup token for this connection, keeping its links. */
    | {type: "reconnect"; connectionId: string | null}
    /** Sync again — one account when the problem is one account's, else the connection. */
    | {type: "sync"; connectionId: string | null; accountKey: string | null}
    | {type: "edit-rules"; rulesId: string | null}
    /** Open the account's drawer at its held list. */
    | {type: "review-held"; accountKey: string | null}
    /** Open the account's drawer to pick another target file. */
    | {type: "choose-file"; accountKey: string | null}
    /** Start the wizard. */
    | {type: "connect"}
    /** The hledger path lives on New Transactions' banner. */
    | {type: "set-hledger"}
    /** The account's running balance in the journal (plan 36). */
    | {type: "show-balance"; accountKey: string | null};

export interface ProblemButton {
    label: string;
    effect: ProblemEffect;
}

/**
 * The URL, when it is one Ledgeline will open: https only, no credentials.
 *
 * The server builds these from what the Bridge sent (an institution's
 * domain), so a `javascript:` or `http:` value would be the Bridge's text
 * reaching an `href`. Refusing it here drops the button rather than the
 * problem.
 */
export function safeUrl(raw: string | undefined): string | null {
    if (raw === undefined) return null;
    try {
        const url = new URL(raw);
        if (url.protocol !== "https:" || url.username !== "" || url.password !== "") return null;
        return url.href;
    } catch {
        return null;
    }
}

const external = (label: string): string => (label.endsWith("↗") ? label : `${label} ↗`);

/** The buttons for `problem` in `scope`: one primary and the optional "I've fixed it". */
export function problemButtons(problem: WireProblem, scope: ProblemScope): {primary: ProblemButton | null; secondary: ProblemButton | null} {
    const resync: ProblemEffect = {type: "sync", connectionId: scope.connectionId, accountKey: scope.accountKey};
    const secondary = problem.secondary === null ? null : {label: problem.secondary.label, effect: resync};
    const action = problem.action;
    if (action === null) return {primary: null, secondary};
    const ref = action.ref ?? null;
    let primary: ProblemButton | null;
    switch (action.kind) {
        case "open-url": {
            const url = safeUrl(action.url);
            primary = url === null ? null : {label: external(action.label), effect: {type: "url", url}};
            break;
        }
        case "reconnect":
            primary = {label: action.label, effect: {type: "reconnect", connectionId: ref ?? scope.connectionId}};
            break;
        case "retry":
        case "recheck":
            primary = {label: action.label, effect: resync};
            break;
        case "edit-rules":
            primary = {label: action.label, effect: {type: "edit-rules", rulesId: ref}};
            break;
        case "review-held":
            primary = {label: action.label, effect: {type: "review-held", accountKey: ref ?? scope.accountKey}};
            break;
        case "choose-file":
            primary = {label: action.label, effect: {type: "choose-file", accountKey: ref ?? scope.accountKey}};
            break;
        case "connect":
            primary = {label: action.label, effect: {type: "connect"}};
            break;
        case "set-hledger":
            primary = {label: action.label, effect: {type: "set-hledger"}};
            break;
        case "show-balance":
            primary = {label: action.label, effect: {type: "show-balance", accountKey: ref ?? scope.accountKey}};
            break;
        default:
            primary = null;
    }
    return {primary, secondary};
}

const WHO: Record<Who, string> = {
    bank: "Your bank",
    simplefin: "SimpleFIN",
    ledgeline: "Ledgeline",
    network: "Network",
    setup: "Your setup",
};

/** Who has to act, as the notice's small heading. */
export const whoLabel = (who: Who): string => WHO[who];

/** The anchored docs section for a problem's details. */
export const docUrl = (anchor: string): string => `${SYNC_DOCS}#${encodeURIComponent(anchor)}`;

const RANK: Record<Severity, number> = {info: 0, warning: 1, error: 2};

/** The worst severity among `problems`, or null for none. */
export function worstSeverity(problems: readonly WireProblem[]): Severity | null {
    return problems.reduce<Severity | null>((worst, p) => (worst === null || RANK[p.severity] > RANK[worst] ? p.severity : worst), null);
}

/** One problem, where it was found, for a flat list (the Problems drawer). */
export interface PlacedProblem {
    problem: WireProblem;
    scope: ProblemScope;
    /** What it is about: the connection's name or the account's label. */
    where: string;
    /** Unique within one status, for `{#each}`. */
    key: string;
}

/**
 * Every problem in `status`, connection-wide ones first, worst first within
 * each. Info-level ones are left out: the drawer is for things to fix.
 */
export function placedProblems(status: WireSyncStatus): PlacedProblem[] {
    const out: PlacedProblem[] = [];
    const byConn = new Map(status.connections.map((c): [string, WireConnection] => [c.id, c]));
    for (const c of status.connections) {
        c.problems.forEach((problem, i) => out.push({problem, scope: {connectionId: c.id, accountKey: null}, where: c.name, key: `c:${c.id}:${i}`}));
    }
    for (const a of status.accounts as readonly WireLinkedAccount[]) {
        a.problems.forEach((problem, i) =>
            out.push({
                problem,
                scope: {connectionId: byConn.has(a.connectionId) ? a.connectionId : null, accountKey: a.key},
                where: a.label,
                key: `a:${a.key}:${i}`,
            })
        );
    }
    return out.filter((p) => p.problem.severity !== "info").sort((x, y) => RANK[y.problem.severity] - RANK[x.problem.severity]);
}
