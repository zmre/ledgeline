// Settings store (WP-02): Svelte 5 runes state persisted to localStorage
// under a versioned key. `setServerUrl` verifies GET /version before persisting.

import {HledgerApi, SETTINGS_STORAGE_KEY} from "$lib/api/client";

/** Journal table column toggles (defaults per WP-03: Date, Status, Description, Accounts, Amount). */
export interface ColumnConfig {
    date: boolean;
    status: boolean;
    description: boolean;
    accounts: boolean;
    amount: boolean;
}

const defaultColumns = (): ColumnConfig => ({date: true, status: true, description: true, accounts: true, amount: true});

interface PersistedSettings {
    serverUrl: string | null;
    /**
     * Access token for a CROSS-ORIGIN engine (vite dev, the e2e harness), which
     * the engine prints at startup and takes from $LEDGELINE_TOKEN. Embedded
     * (same-origin) pages ignore this: the binary injects the token straight
     * into the page, and `apiToken()` in api/client.ts prefers that. Null when
     * talking to a plain hledger-web, which has no token.
     */
    serverToken: string | null;
    columns: ColumnConfig;
    insightsOpen: boolean;
}

const defaults = (): PersistedSettings => ({serverUrl: null, serverToken: null, columns: defaultColumns(), insightsOpen: true});

/**
 * When the SPA is served in-process by the `ledgeline` binary, that binary
 * injects `window.__LEDGELINE_EMBEDDED__ = true` into the served index.html. In
 * that case the API lives at the SAME ORIGIN as the page, so we force the server
 * URL to the current origin — no setup modal, and immune to a stale/ephemeral
 * port left in localStorage from a previous run. Standalone dev (vite) has no
 * such marker and keeps the null-→-modal flow.
 */
function embeddedServerUrl(): string | null {
    if (typeof window === "undefined") return null;
    return (window as {__LEDGELINE_EMBEDDED__?: boolean}).__LEDGELINE_EMBEDDED__ === true ? window.location.origin : null;
}

function load(): PersistedSettings {
    const embedded = embeddedServerUrl();
    if (typeof localStorage === "undefined") return {...defaults(), serverUrl: embedded ?? null};
    try {
        const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
        const parsed = raw === null ? ({} as Partial<PersistedSettings>) : (JSON.parse(raw) as Partial<PersistedSettings>);
        return {
            // Embedded mode always wins over any persisted URL.
            serverUrl: embedded ?? (typeof parsed.serverUrl === "string" ? parsed.serverUrl : null),
            serverToken: typeof parsed.serverToken === "string" ? parsed.serverToken : null,
            columns: {...defaultColumns(), ...(typeof parsed.columns === "object" && parsed.columns !== null ? parsed.columns : {})},
            insightsOpen: typeof parsed.insightsOpen === "boolean" ? parsed.insightsOpen : true,
        };
    } catch {
        return {...defaults(), serverUrl: embedded ?? null};
    }
}

const state = $state<PersistedSettings>(load());

function persist(): void {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(
        SETTINGS_STORAGE_KEY,
        JSON.stringify({serverUrl: state.serverUrl, serverToken: state.serverToken, columns: state.columns, insightsOpen: state.insightsOpen})
    );
}

export const settings = {
    /** null until a server URL has been verified — the layout shows the setup modal. */
    get serverUrl(): string | null {
        return state.serverUrl;
    },
    get columns(): ColumnConfig {
        return state.columns;
    },
    set columns(columns: ColumnConfig) {
        state.columns = columns;
        persist();
    },
    get insightsOpen(): boolean {
        return state.insightsOpen;
    },
    set insightsOpen(open: boolean) {
        state.insightsOpen = open;
        persist();
    },
    /** The cross-origin engine token, if one was entered. Null in embedded mode. */
    get serverToken(): string | null {
        return state.serverToken;
    },
    /**
     * Verifies GET /version at `url`; persists only on success. Throws
     * ApiUnreachableError/ApiShapeError.
     *
     * `token` (omit to keep the current one) is the Ledgeline engine's access
     * token, needed only when the engine is at a DIFFERENT origin than this
     * page. It has to be persisted before the probe goes out, because
     * `apiToken()` reads the stored blob rather than this reactive state — so a
     * failed verification rolls it back.
     */
    async setServerUrl(url: string, token?: string): Promise<void> {
        const normalized = url.trim().replace(/\/+$/, "");
        const candidate = token === undefined || token.trim() === "" ? null : token.trim();
        // Verify with the candidate token passed explicitly, so an unreachable
        // server leaves storage completely untouched (a failed attempt must not
        // persist a half-configured server, and must not clobber a working one).
        await new HledgerApi(normalized, candidate ?? undefined).version();
        if (token !== undefined) state.serverToken = candidate;
        state.serverUrl = normalized;
        persist();
    },
};
