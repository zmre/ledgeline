// Settings store (WP-02): Svelte 5 runes state persisted to localStorage
// under a versioned key. `setServerUrl` verifies GET /version before persisting.

import {HledgerApi} from "$lib/api/client";

/** Journal table column toggles (defaults per WP-03: Date, Status, Description, Accounts, Amount). */
export interface ColumnConfig {
    date: boolean;
    status: boolean;
    description: boolean;
    accounts: boolean;
    amount: boolean;
}

const STORAGE_KEY = "ledgeline.settings.v1";

const defaultColumns = (): ColumnConfig => ({date: true, status: true, description: true, accounts: true, amount: true});

interface PersistedSettings {
    serverUrl: string | null;
    columns: ColumnConfig;
    insightsOpen: boolean;
}

const defaults = (): PersistedSettings => ({serverUrl: null, columns: defaultColumns(), insightsOpen: true});

function load(): PersistedSettings {
    if (typeof localStorage === "undefined") return defaults();
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (raw === null) return defaults();
        const parsed = JSON.parse(raw) as Partial<PersistedSettings>;
        return {
            serverUrl: typeof parsed.serverUrl === "string" ? parsed.serverUrl : null,
            columns: {...defaultColumns(), ...(typeof parsed.columns === "object" && parsed.columns !== null ? parsed.columns : {})},
            insightsOpen: typeof parsed.insightsOpen === "boolean" ? parsed.insightsOpen : true,
        };
    } catch {
        return defaults();
    }
}

const state = $state<PersistedSettings>(load());

function persist(): void {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(STORAGE_KEY, JSON.stringify({serverUrl: state.serverUrl, columns: state.columns, insightsOpen: state.insightsOpen}));
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
    /** Verifies GET /version at `url`; persists only on success. Throws ApiUnreachableError/ApiShapeError. */
    async setServerUrl(url: string): Promise<void> {
        const normalized = url.trim().replace(/\/+$/, "");
        await new HledgerApi(normalized).version();
        state.serverUrl = normalized;
        persist();
    },
};
