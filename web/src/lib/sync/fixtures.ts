// Bank sync fixtures (plan 43, contract WP-W): typed wire values the
// component tests render, and the dev-only mock (`devMock.ts`) serves while the
// server (WP-S) is not there yet. Everything is invented — no real bank, no
// real person — and dated around one fixed moment, `FIXTURE_NOW`.
//
// Each export is a FUNCTION returning a fresh copy, so a test that changes
// what it was handed cannot leak into the next one.
//
// Nothing in a production path imports this file: the store reaches it only
// through `import.meta.env.DEV`-guarded dynamic imports.

import type {
    WireConnection,
    WireLinkPreview,
    WireLinkedAccount,
    WireManualAccount,
    WireProblem,
    WireRemoteAccount,
    WireSharePreview,
    WireSyncRun,
    WireSyncStatus,
} from "./types";

/** Sat 2026-11-14, 1:00 PM in America/Denver (the zone the test suite pins). */
export const FIXTURE_NOW = Date.UTC(2026, 10, 14, 20, 0, 0) / 1000;
export const FIXTURE_TODAY = "2026-11-14";
/** 7:12 AM that morning, Denver. */
export const FIXTURE_LAST_SYNC = Date.UTC(2026, 10, 14, 14, 12, 0) / 1000;
/** 1:15 PM that afternoon, Denver. */
export const FIXTURE_NEXT_AUTO = Date.UTC(2026, 10, 14, 20, 15, 0) / 1000;

export const CONNECTION_ID = "4f6c2a7e-0000-4000-8000-000000000001";

export function conAuthProblem(): WireProblem {
    return {
        code: "con-auth",
        severity: "warning",
        who: "bank",
        message: "Chase needs you to sign in again at SimpleFIN.",
        since: FIXTURE_LAST_SYNC - 3 * 86_400,
        dataThrough: "2026-11-11",
        action: {kind: "open-url", label: "Fix at SimpleFIN", url: "https://beta-bridge.simplefin.org/"},
        secondary: {kind: "recheck", label: "I've fixed it"},
        detail: "con.auth: Connection to Chase may need attention",
        docAnchor: "con-auth",
    };
}

export function heldProblem(key = "a1b2c3d4e5"): WireProblem {
    return {
        code: "held",
        severity: "warning",
        who: "ledgeline",
        message: "2 transactions need a look before they're added.",
        since: FIXTURE_LAST_SYNC,
        dataThrough: null,
        action: {kind: "review-held", label: "Review", ref: key},
        secondary: null,
        detail: null,
        docAnchor: "held",
    };
}

export function revokedProblem(): WireProblem {
    return {
        code: "revoked",
        severity: "error",
        who: "simplefin",
        message: "Ledgeline's access to SimpleFIN was turned off or revoked.",
        since: FIXTURE_LAST_SYNC,
        dataThrough: "2026-11-13",
        action: {kind: "reconnect", label: "Reconnect…", ref: CONNECTION_ID},
        secondary: null,
        detail: "HTTP 403 from /accounts",
        docAnchor: "revoked",
    };
}

export function tokenUsedProblem(): WireProblem {
    return {
        code: "token-used",
        severity: "error",
        who: "simplefin",
        message:
            "This token was already used. If you didn't use it in another app, someone else may have, so disable it on SimpleFIN. Then create a new one and paste it here.",
        since: null,
        dataThrough: null,
        action: {kind: "open-url", label: "Open SimpleFIN", url: "https://beta-bridge.simplefin.org/"},
        secondary: null,
        detail: "HTTP 403 from the claim endpoint",
        docAnchor: "token-used",
    };
}

export function fixtureConnection(): WireConnection {
    return {
        id: CONNECTION_ID,
        name: "SimpleFIN Bridge",
        kind: "simplefin",
        host: "beta-bridge.simplefin.org",
        connected: true,
        state: "attention",
        lastAttempt: FIXTURE_LAST_SYNC,
        lastSuccess: FIXTURE_LAST_SYNC,
        requestsToday: 3,
        quota: {autoMax: 6, manualWarn: 12, manualMax: 16},
        manageUrl: "https://beta-bridge.simplefin.org/",
        problems: [],
    };
}

export function firstTechLinked(): WireLinkedAccount {
    return {
        key: "a1b2c3d4e5",
        connectionId: CONNECTION_ID,
        remoteId: "ACT-firsttech-1234",
        label: "First Tech · Checking ••1234",
        org: "First Tech",
        journalAccount: "assets:bank:firsttech:checking",
        target: "2026.journal",
        rules: "imports/rules/firsttech-checking.sync.csv.rules",
        categories: "imports/rules/firsttech-checking.categories.rules",
        startDate: "2026-11-01",
        balanceSign: 1,
        enabled: true,
        lastSuccess: FIXTURE_LAST_SYNC,
        dataThrough: "2026-11-13",
        lastAdded: 4,
        bankBalance: {amount: "4210.55", date: "2026-11-13"},
        ledgerBalance: "4210.55",
        balanceDiff: null,
        held: [
            {
                heldId: "a1b2c3d4e5-TRN-0007",
                date: "2026-11-12",
                amount: "-54.20",
                payee: "SAFEWAY #1234",
                existing: {index: 812, date: "2026-11-11", description: "Safeway", amount: "-54.20"},
            },
            {
                heldId: "a1b2c3d4e5-TRN-0009",
                date: "2026-11-13",
                amount: "-12.00",
                payee: "NETFLIX.COM",
                existing: {index: 815, date: "2026-11-13", description: "Netflix", amount: "-12.00"},
            },
        ],
        history: [
            {at: FIXTURE_LAST_SYNC, outcome: "added", added: 4, held: 2},
            {at: FIXTURE_LAST_SYNC - 6 * 3600, outcome: "nothing-new", added: 0, held: 0},
            {at: FIXTURE_LAST_SYNC - 86_400, outcome: "added", added: 7, held: 0},
        ],
        problems: [heldProblem()],
    };
}

export function chaseLinked(): WireLinkedAccount {
    return {
        key: "f6e5d4c3b2",
        connectionId: CONNECTION_ID,
        remoteId: "ACT-chase-9921",
        label: "Chase · Sapphire ••9921",
        org: "Chase",
        journalAccount: "liabilities:cc:chase",
        target: "2026.journal",
        rules: "imports/rules/chase.sync.csv.rules",
        categories: "imports/rules/chase.categories.rules",
        startDate: "2026-10-15",
        balanceSign: -1,
        enabled: true,
        lastSuccess: FIXTURE_LAST_SYNC - 3 * 86_400,
        dataThrough: "2026-11-11",
        lastAdded: 0,
        bankBalance: {amount: "1032.18", date: "2026-11-11"},
        ledgerBalance: "-1032.18",
        balanceDiff: null,
        held: [],
        history: [{at: FIXTURE_LAST_SYNC - 3 * 86_400, outcome: "added", added: 2, held: 0}],
        problems: [conAuthProblem()],
    };
}

export function firstTechRemote(): WireRemoteAccount {
    return {
        connectionId: CONNECTION_ID,
        remoteId: "ACT-firsttech-1234",
        label: "First Tech · Checking ••1234",
        org: "First Tech",
        balance: "4210.55",
        balanceDate: "2026-11-13",
        currency: "USD",
        oldestAvailable: "2026-08-20",
        suggestion: {
            journalAccount: "assets:bank:firsttech:checking",
            score: 0.92,
            ledgerBalance: "4198.15",
            signFlipped: false,
            startDate: "2026-11-01",
            target: "2026.journal",
            newAccountName: "assets:bank:firsttech:checking",
            newAccountType: "C",
            syncRules: [],
            manualRules: null,
        },
    };
}

export function chaseRemote(): WireRemoteAccount {
    return {
        connectionId: CONNECTION_ID,
        remoteId: "ACT-chase-9921",
        label: "Chase · Sapphire ••9921",
        org: "Chase",
        balance: "1032.18",
        balanceDate: "2026-11-13",
        currency: "USD",
        oldestAvailable: "2026-08-17",
        suggestion: {
            journalAccount: "liabilities:cc:chase",
            score: 0.88,
            ledgerBalance: "-1032.18",
            signFlipped: true,
            startDate: "2026-11-02",
            target: "2026.journal",
            newAccountName: "liabilities:cc:chase",
            newAccountType: "L",
            syncRules: [{id: "imports/rules/old-chase.sync.csv.rules", fits: true}],
            manualRules: {id: "imports/rules/chase.csv.rules", ruleCount: 41},
        },
    };
}

export function allyRemote(): WireRemoteAccount {
    return {
        connectionId: CONNECTION_ID,
        remoteId: "ACT-ally-0077",
        label: "Ally · Savings ••0077",
        org: "Ally",
        balance: "20000.00",
        balanceDate: "2026-11-13",
        currency: "USD",
        oldestAvailable: "2026-08-15",
        suggestion: {
            journalAccount: null,
            score: 0,
            ledgerBalance: null,
            signFlipped: false,
            startDate: "2026-08-15",
            target: null,
            newAccountName: "assets:bank:ally:savings",
            newAccountType: "C",
            syncRules: [],
            manualRules: null,
        },
    };
}

/** Two linked accounts (one fine with held rows, one needing a sign-in), one unlinked. */
export function fixtureStatus(): WireSyncStatus {
    return {
        autoSync: true,
        available: true,
        unavailableReason: null,
        running: false,
        lastRun: FIXTURE_LAST_SYNC,
        nextAuto: FIXTURE_NEXT_AUTO,
        connections: [fixtureConnection()],
        accounts: [firstTechLinked(), chaseLinked()],
        unlinked: [allyRemote()],
        problemCount: {warning: 2, error: 0},
    };
}

/** Sync is there, nothing is linked yet. */
export function emptyStatus(): WireSyncStatus {
    return {
        autoSync: false,
        available: true,
        unavailableReason: null,
        running: false,
        lastRun: null,
        nextAuto: null,
        connections: [],
        accounts: [],
        unlinked: [],
        problemCount: {warning: 0, error: 0},
    };
}

export function fixturePreviews(): WireLinkPreview[] {
    return [
        {
            remoteId: "ACT-firsttech-1234",
            ok: true,
            error: null,
            new: 38,
            already: 0,
            held: 2,
            uncategorized: 6,
            droppedBeforeStart: 0,
            entries: "2026-11-02 Safeway\n    expenses:food:groceries        54.20\n    assets:bank:firsttech:checking\n",
        },
        {
            remoteId: "ACT-chase-9921",
            ok: true,
            error: null,
            new: 12,
            already: 3,
            held: 0,
            uncategorized: 1,
            droppedBeforeStart: 4,
            entries: "2026-11-03 Amazon\n    expenses:unknown        23.99\n    liabilities:cc:chase\n",
        },
    ];
}

export function fixtureSharePreview(): WireSharePreview {
    return {
        manualRulesId: "imports/rules/chase.csv.rules",
        categoriesId: "imports/rules/chase.categories.rules",
        moved: 38,
        kept: [
            {header: "if %trntype CHECK", reason: "Tests trntype, which sync doesn't have."},
            {header: "if ^PAYMENT,", reason: "The pattern depends on the CSV's column layout."},
            {header: "if table", reason: "if tables stay where they are."},
        ],
        diffs: {
            manual: "--- chase.csv.rules\n+++ chase.csv.rules\n-if AMAZON\n-  account2 expenses:shopping\n+include chase.categories.rules\n",
            categories: "--- /dev/null\n+++ chase.categories.rules\n+if AMAZON\n+  account2 expenses:shopping\n",
        },
        identical: true,
        differences: null,
    };
}

export function fixtureRun(): WireSyncRun {
    return {
        ok: true,
        requestsToday: 4,
        quotaWarning: false,
        accounts: [
            {key: "a1b2c3d4e5", label: "First Tech · Checking ••1234", added: 5, held: 0, already: 12, outcome: "added", problem: null},
            {key: "f6e5d4c3b2", label: "Chase · Sapphire ••9921", added: 4, held: 0, already: 8, outcome: "added", problem: null},
            {key: "0a9b8c7d6e", label: "Ally · Savings ••0077", added: 0, held: 0, already: 2, outcome: "nothing-new", problem: null},
        ],
        problems: [],
    };
}

export function fixtureManual(): WireManualAccount[] {
    return [
        {
            account: "assets:invest:vanguard",
            lastTransaction: "2026-09-30",
            lastImport: "2026-10-02",
            downloadUrl: "https://investor.vanguard.example/statements",
            nudge: true,
            stale: true,
        },
        {account: "assets:cash", lastTransaction: "2026-11-10", lastImport: null, downloadUrl: null, nudge: false, stale: false},
    ];
}
