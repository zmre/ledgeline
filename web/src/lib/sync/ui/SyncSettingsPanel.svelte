<!-- Settings → Sync (plan 43 §4.4), modelled on PricesSettingsPanel: the
     auto-sync toggle and interval, where synced statements are saved and for
     how long, "Use for this ledger only", and the connections with Disconnect.

     The direct-to-bank option is deliberately absent (plan 43 decision #3). -->
<script lang="ts">
    import {onMount} from "svelte";
    import {resolve} from "$app/paths";
    import {AUTO_SYNC_HOURS_RANGE, isRelDir, MANUAL_REMIND_DAYS_RANGE, SYNC_KEEP_DAYS_RANGE} from "#lib/settings/uiPrefs.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {syncStore} from "../syncStore.svelte";
    import DisconnectConfirm from "./DisconnectConfirm.svelte";
    import WhoSeesData from "./WhoSeesData.svelte";

    const KEYS = ["autoSync", "autoSyncEveryHours", "syncSourceDir", "syncKeepSourceDays", "manualRemindDays"] as const;

    let pinBusy = $state(false);
    async function pin(pinned: boolean): Promise<void> {
        pinBusy = true;
        try {
            // All of them together: one setting as far as a person is concerned.
            for (const key of KEYS) await settings.pinToLedger(key, pinned);
        } finally {
            pinBusy = false;
        }
    }
    const pinned = $derived(settings.isPinned("autoSync"));

    let folder = $state(settings.syncSourceDir);
    const folderOk = $derived(isRelDir(folder.trim()));

    let confirming = $state<string | null>(null);
    const connections = $derived(syncStore.status?.connections ?? []);

    onMount(() => void syncStore.refresh());
</script>

<section class="flex max-w-3xl flex-col gap-4" data-testid="settings-sync">
    <div class="flex flex-wrap items-baseline justify-between gap-2">
        <h2 class="text-base font-semibold">Bank sync</h2>
        {#if settings.canPinToLedger}
            <label class="flex cursor-pointer items-center gap-2 text-sm">
                <input
                    type="checkbox"
                    class="checkbox checkbox-sm"
                    checked={pinned}
                    disabled={pinBusy}
                    onchange={(e) => void pin(e.currentTarget.checked)}
                    data-testid="sync-pin"
                />
                Use for this ledger only
            </label>
        {/if}
    </div>

    <label class="flex cursor-pointer items-center gap-3">
        <input
            type="checkbox"
            class="toggle toggle-primary"
            checked={settings.autoSync}
            onchange={(e) => (settings.autoSync = e.currentTarget.checked)}
            data-testid="sync-auto"
        />
        <span>Keep linked accounts up to date in the background</span>
    </label>

    <label class="flex flex-wrap items-center gap-3">
        <span>Sync at most every</span>
        <span class="flex items-center gap-3">
            <input
                type="number"
                class="input w-20 input-sm"
                min={AUTO_SYNC_HOURS_RANGE[0]}
                max={AUTO_SYNC_HOURS_RANGE[1]}
                step="1"
                value={settings.autoSyncEveryHours}
                disabled={!settings.autoSync}
                onchange={(e) => {
                    settings.autoSyncEveryHours = e.currentTarget.valueAsNumber;
                    e.currentTarget.value = String(settings.autoSyncEveryHours);
                }}
                data-testid="sync-hours"
            />
            <span>hours</span>
        </span>
    </label>

    <label class="flex flex-col gap-1">
        <span>Save downloaded statements in</span>
        <input
            type="text"
            class="input w-80 max-w-full font-mono input-sm {folderOk ? '' : 'input-error'}"
            spellcheck="false"
            bind:value={folder}
            onchange={() => {
                if (folderOk) settings.syncSourceDir = folder.trim();
            }}
            data-testid="sync-folder"
        />
        <span class="text-xs {folderOk ? 'text-muted' : 'text-error'}">
            {folderOk ? "Relative to your journal's folder." : "Use a folder inside your journal's folder, like imports/sync."}
        </span>
    </label>

    <label class="flex flex-wrap items-center gap-3">
        <span>Keep them for</span>
        <span class="flex items-center gap-3">
            <input
                type="number"
                class="input w-24 input-sm"
                min={SYNC_KEEP_DAYS_RANGE[0]}
                max={SYNC_KEEP_DAYS_RANGE[1]}
                step="1"
                value={settings.syncKeepSourceDays}
                onchange={(e) => {
                    settings.syncKeepSourceDays = e.currentTarget.valueAsNumber;
                    e.currentTarget.value = String(settings.syncKeepSourceDays);
                }}
                data-testid="sync-keep"
            />
            <span>days</span>
            <span class="text-xs text-muted">{settings.syncKeepSourceDays === 0 ? "0 keeps every one" : "older ones are deleted"}</span>
        </span>
    </label>

    <label class="flex flex-wrap items-center gap-3">
        <span>Remind me about manual downloads after</span>
        <span class="flex items-center gap-3">
            <input
                type="number"
                class="input w-24 input-sm"
                min={MANUAL_REMIND_DAYS_RANGE[0]}
                max={MANUAL_REMIND_DAYS_RANGE[1]}
                step="1"
                value={settings.manualRemindDays}
                onchange={(e) => {
                    settings.manualRemindDays = e.currentTarget.valueAsNumber;
                    e.currentTarget.value = String(settings.manualRemindDays);
                }}
                data-testid="sync-remind"
            />
            <span>days with no new transactions</span>
        </span>
    </label>

    <div class="flex flex-col gap-2 text-sm text-muted">
        <p>
            With this on, Ledgeline syncs while it's open: once each morning after your bank's overnight update, and again when the time above has passed. New
            transactions go into your journal unreviewed. Possible duplicates are held for you to look at.
        </p>
        <p>
            With it off, Ledgeline only contacts SimpleFIN when you click <span class="font-medium">Sync now</span> on
            <a class="link" href="{resolve('/imports')}?tab=accounts">Imports → Accounts</a>.
        </p>
        {#if pinned}
            <p>This ledger has its own setting, kept in its <code>ledgeline.toml</code>.</p>
        {/if}
    </div>

    <section class="flex flex-col gap-2">
        <h3 class="font-semibold">Connections</h3>
        {#if connections.length === 0}
            <p class="text-sm text-muted">No connections on this computer.</p>
        {:else}
            <ul class="flex flex-col gap-2">
                {#each connections as connection (connection.id)}
                    <li class="flex flex-col gap-2 rounded-box border border-base-content/10 p-3" data-testid="sync-settings-connection">
                        <div class="flex flex-wrap items-center gap-3">
                            <span class="font-medium">{connection.name}</span>
                            {#if connection.host !== null}<span class="font-mono text-xs text-muted">{connection.host}</span>{/if}
                            {#if connection.connected && confirming !== connection.id}
                                <button type="button" class="btn ml-auto btn-outline btn-error btn-xs" onclick={() => (confirming = connection.id)}
                                    >Disconnect…</button
                                >
                            {:else if !connection.connected}
                                <span class="ml-auto text-xs text-muted"
                                    >{connection.state === "elsewhere" ? "Syncs on another computer" : "Not connected on this computer"}</span
                                >
                            {/if}
                        </div>
                        {#if confirming === connection.id}
                            <DisconnectConfirm {connection} onDone={() => (confirming = null)} />
                        {/if}
                    </li>
                {/each}
            </ul>
        {/if}
        <WhoSeesData />
    </section>
</section>
