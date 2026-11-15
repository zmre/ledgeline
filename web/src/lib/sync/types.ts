// Bank sync (plan 43) wire shapes, mirroring `plans/43-bank-sync-contract.md`
// "Wire shapes" field for field. camelCase JSON; times are epoch SECONDS; dates
// are ISO `YYYY-MM-DD`; money is decimal TEXT, never a JS number.
//
// Pure types, no imports: the store, the helpers, the fixtures and the
// components all name these, and the server (WP-S, `sync/wire.rs`) owns the
// same list. A field added here without a note under "Deviations" in the
// contract is a field the server will never send.

export type Who = "bank" | "simplefin" | "ledgeline" | "network" | "setup";
export type Severity = "info" | "warning" | "error";
export type ProblemCode =
    | "con-auth"
    | "act-failed"
    | "revoked"
    | "payment"
    | "quota"
    | "network"
    | "edge-blocked"
    | "token-used"
    | "stale"
    | "hledger"
    | "rules"
    | "held"
    | "balance"
    | "target"
    | "not-connected"
    | "setup"
    | "other";

export type ProblemActionKind =
    "open-url" | "reconnect" | "retry" | "edit-rules" | "review-held" | "choose-file" | "connect" | "set-hledger" | "show-balance" | "recheck";

export interface WireProblemAction {
    kind: ProblemActionKind;
    label: string;
    /** For `open-url`. */
    url?: string;
    /** What the action is about: a rules id, an account key, a connection id. */
    ref?: string;
}

export interface WireProblem {
    code: ProblemCode;
    severity: Severity;
    who: Who;
    /** A plain sentence, ready to show. */
    message: string;
    since: number | null;
    /** "Data stops at Nov 11" — the ISO date the data stops at. */
    dataThrough: string | null;
    action: WireProblemAction | null;
    /** "I've fixed it". */
    secondary: null | {kind: "recheck"; label: string};
    /** Raw code + server message, sanitized. */
    detail: string | null;
    /** e.g. "con-auth" → docs/imports/bank-sync.md#con-auth */
    docAnchor: string;
}

export type ConnectionState = "ok" | "syncing" | "attention" | "broken" | "not-connected";

export interface WireQuota {
    autoMax: number;
    manualWarn: number;
    manualMax: number;
}

export interface WireConnection {
    id: string;
    name: string;
    kind: "simplefin";
    host: string | null;
    /** The keychain item is present on this machine. */
    connected: boolean;
    state: ConnectionState;
    lastAttempt: number | null;
    lastSuccess: number | null;
    requestsToday: number;
    quota: WireQuota;
    /** https://beta-bridge.simplefin.org/ for the Bridge, else https://<host>/ */
    manageUrl: string;
    problems: WireProblem[];
}

export type RunOutcome = "added" | "nothing-new" | "failed" | "skipped";

export interface WireHistoryEntry {
    at: number;
    outcome: RunOutcome;
    added: number;
    held: number;
}

export interface WireHeld {
    heldId: string;
    date: string;
    amount: string;
    payee: string;
    existing: {index: number; date: string; description: string; amount: string};
}

export interface WireLinkedAccount {
    key: string;
    connectionId: string;
    remoteId: string;
    label: string;
    org: string | null;
    journalAccount: string;
    target: string;
    rules: string;
    categories: string | null;
    startDate: string;
    balanceSign: 1 | -1;
    enabled: boolean;
    lastSuccess: number | null;
    dataThrough: string | null;
    lastAdded: number;
    bankBalance: {amount: string; date: string} | null;
    ledgerBalance: string | null;
    /** Null when equal or unknown. */
    balanceDiff: string | null;
    held: WireHeld[];
    history: WireHistoryEntry[];
    problems: WireProblem[];
}

export type NewAccountType = "A" | "L" | "C";

export interface WireRemoteSuggestion {
    journalAccount: string | null;
    score: number;
    ledgerBalance: string | null;
    signFlipped: boolean;
    startDate: string;
    target: string | null;
    /** e.g. "assets:bank:ally:savings" */
    newAccountName: string;
    newAccountType: NewAccountType;
    /** Existing sync-shaped files. */
    syncRules: {id: string; fits: boolean}[];
    /** A manual file covering the account. */
    manualRules: {id: string; ruleCount: number} | null;
}

export interface WireRemoteAccount {
    connectionId: string;
    remoteId: string;
    label: string;
    org: string | null;
    balance: string;
    balanceDate: string | null;
    currency: string;
    /** Earliest transaction date seen in the fetch. */
    oldestAvailable: string | null;
    suggestion: WireRemoteSuggestion;
}

export interface WireSyncStatus {
    autoSync: boolean;
    available: boolean;
    unavailableReason: string | null;
    running: boolean;
    lastRun: number | null;
    nextAuto: number | null;
    connections: WireConnection[];
    accounts: WireLinkedAccount[];
    unlinked: WireRemoteAccount[];
    problemCount: {warning: number; error: number};
}

export type WireLinkRules = {mode: "new"; name: string} | {mode: "existing"; id: string};

export interface WireLinkRequest {
    connectionId: string;
    remoteId: string;
    label: string;
    journalAccount: string;
    newAccount: null | {type: NewAccountType; openingBalance: string; openingDate: string};
    startDate: string;
    target: string;
    balanceSign: 1 | -1;
    rules: WireLinkRules;
    /** Run the categories refactor first. */
    share: null | {manualRulesId: string};
}

export interface WireLinkPreview {
    remoteId: string;
    ok: boolean;
    error: WireProblem | null;
    new: number;
    already: number;
    held: number;
    uncategorized: number;
    droppedBeforeStart: number;
    /** Proposed journal text. */
    entries: string;
}

export interface WireSharePreview {
    manualRulesId: string;
    categoriesId: string;
    moved: number;
    kept: {header: string; reason: string}[];
    /** Unified diffs. */
    diffs: {manual: string; categories: string};
    /** The `hledger print` before/after check. */
    identical: boolean;
    differences: string | null;
}

export interface WireRunAccount {
    key: string;
    label: string;
    added: number;
    held: number;
    already: number;
    outcome: RunOutcome;
    problem: WireProblem | null;
    /**
     * New rows that landed on the fallback account. REQUESTED of WP-S (see the
     * contract's Deviations): optional until the server sends it, and the
     * toast falls back to `added` without it.
     */
    uncategorized?: number;
}

export interface WireSyncRun {
    ok: boolean;
    requestsToday: number;
    quotaWarning: boolean;
    accounts: WireRunAccount[];
    problems: WireProblem[];
}

export interface WireManualAccount {
    account: string;
    lastTransaction: string | null;
    lastImport: string | null;
    downloadUrl: string | null;
    nudge: boolean;
    /** nudge && > 35 days. */
    stale: boolean;
}

/** `POST /api/sync/connect` and `/connections/{id}/token` → 200. */
export interface WireConnectResult {
    connectionId: string;
    accounts: WireRemoteAccount[];
}

/** `PATCH /api/sync/accounts/{key}`: any subset. */
export interface WireAccountPatch {
    journalAccount?: string;
    target?: string;
    rules?: string;
    startDate?: string;
    balanceSign?: 1 | -1;
    enabled?: boolean;
}

/** `PUT /api/sync/manual`. */
export interface WireManualUpdate {
    account: string;
    downloadUrl?: string | null;
    nudge: boolean;
}
