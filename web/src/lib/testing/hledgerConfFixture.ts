// A `GET /api/hledger-conf` body, as the engine sends it (conf_api.rs `WireConf`).
// `overrides` replaces top-level keys.

const NO_DEPTH = {all: null, perRegex: []};

export function hledgerConfBody(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        handle: "hledger.conf",
        origin: "ledger",
        createHandle: null,
        editable: true,
        readOnlyReason: null,
        hijacked: null,
        revision: "rev-1",
        auto: false,
        depth: {
            general: {all: 3, perRegex: []},
            balancesheet: {all: null, perRegex: [{regex: "^assets:inv", depth: 3}]},
            incomestatement: NO_DEPTH,
            cashflow: NO_DEPTH,
            balance: NO_DEPTH,
        },
        effectiveDepth: {
            balancesheet: {all: 3, perRegex: [{regex: "^assets:inv", depth: 3}]},
            incomestatement: {all: 3, perRegex: []},
            cashflow: {all: 3, perRegex: []},
            balance: {all: 3, perRegex: []},
        },
        warnings: [],
        ...overrides,
    };
}
