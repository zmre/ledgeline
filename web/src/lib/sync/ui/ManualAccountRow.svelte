<!-- One manually imported account (plan 43 §4.2, "Manual downloads"): when it
     was last imported, a link to the bank's download page if one was saved,
     and a jump to New Transactions. Amber after 35 days with nothing new,
     unless that nudge is off for it. -->
<script lang="ts">
    import {goto} from "$app/navigation";
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {manualStale} from "../manualStale";
    import {safeUrl} from "../problemAction";
    import {syncStore} from "../syncStore.svelte";
    import type {WireManualAccount} from "../types";
    import {importsHref} from "./effects";

    let {account, highlight = false}: {account: WireManualAccount; highlight?: boolean} = $props();

    const view = $derived(manualStale(account, localToday(), undefined, settings.manualRemindDays));
    const link = $derived(safeUrl(account.downloadUrl ?? undefined));

    let editing = $state(false);
    let draftUrl = $state("");
    let draftNudge = $state(true);
    let saving = $state(false);
    let error = $state<string | null>(null);

    function edit(): void {
        draftUrl = account.downloadUrl ?? "";
        draftNudge = account.nudge;
        error = null;
        editing = true;
    }

    async function save(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        const url = draftUrl.trim();
        if (url !== "" && safeUrl(url) === null) {
            error = "The download page needs a full https:// address.";
            return;
        }
        saving = true;
        const outcome = await syncStore.putManual({account: account.account, downloadUrl: url === "" ? null : url, nudge: draftNudge});
        saving = false;
        if (outcome.ok) editing = false;
        else error = outcome.failure.message;
    }

    /** No download page and no reminders: the same as saving an empty form. */
    async function skip(): Promise<void> {
        saving = true;
        error = null;
        const outcome = await syncStore.putManual({account: account.account, downloadUrl: null, nudge: false});
        saving = false;
        if (!outcome.ok) error = outcome.failure.message;
    }
</script>

<li
    class="flex flex-col gap-1 border-b border-base-content/5 px-3 py-2 last:border-b-0 {highlight ? 'bg-primary/10' : ''}"
    data-testid="sync-manual-row"
    data-stale={view.stale}
>
    <span class="font-mono text-sm">{account.account}</span>
    <span class="text-xs {view.stale ? 'text-warning' : 'text-muted'}">{view.stale ? "⚠ " : ""}{view.line}{account.nudge ? "" : " · No reminders"}</span>
    {#if editing}
        <form class="flex flex-wrap items-center gap-2" onsubmit={(e) => void save(e)}>
            <input
                type="url"
                class="input w-72 input-xs"
                placeholder="https://your-bank.example/statements"
                aria-label="Download page"
                bind:value={draftUrl}
                disabled={saving}
            />
            <label class="flex cursor-pointer items-center gap-1 text-xs">
                <input type="checkbox" class="checkbox checkbox-xs" bind:checked={draftNudge} disabled={saving} />
                Remind me after {settings.manualRemindDays} days
            </label>
            <button type="submit" class="btn btn-primary btn-xs" disabled={saving}>Save</button>
            <button type="button" class="btn btn-ghost btn-xs" disabled={saving} onclick={() => (editing = false)}>Cancel</button>
            {#if error !== null}<span class="text-xs text-error" role="alert">{error}</span>{/if}
        </form>
    {:else}
        <div class="flex flex-wrap justify-end gap-2">
            {#if link !== null}
                <a class="btn btn-xs" href={link} target="_blank" rel="noopener noreferrer external">Open download page ↗</a>
            {/if}
            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- importsHref resolves the route; the query string is appended -->
            <button type="button" class="btn btn-xs" onclick={() => void goto(importsHref("new"))}>Import a file…</button>
            {#if link === null && account.nudge}
                <button type="button" class="btn btn-ghost btn-xs" onclick={edit}>Add download page</button>
                <button
                    type="button"
                    class="btn btn-ghost btn-xs"
                    title="No download page and no reminders, for an account you don't download, like a car or a loan"
                    disabled={saving}
                    onclick={() => void skip()}
                    data-testid="sync-manual-skip">Skip</button
                >
            {:else}
                <button type="button" class="btn btn-ghost btn-xs" onclick={edit}>Edit</button>
            {/if}
        </div>
        {#if error !== null}<span class="self-end text-xs text-error" role="alert">{error}</span>{/if}
    {/if}
</li>
