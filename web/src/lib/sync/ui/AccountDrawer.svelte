<!-- A linked account's drawer (plan 43 §4.2): its mapping with Change… and
     Edit, the sync start date and balance sign, held rows to look at, the last
     ten sync results, and Stop syncing (history stays, the link goes).

     A daisyUI modal placed at the end, with the app's `dismissible` (Escape
     closes, focus is trapped and returns to the row's button). Every change is
     one PATCH and the screen shows the status it answers with. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {goto} from "$app/navigation";
    import DateInput from "#ui/components/DateInput.svelte";
    import {importStore} from "#lib/imports/importStore.svelte.js";
    import AccountInput from "#lib/journal/edit/AccountInput.svelte";
    import {dismissible} from "#lib/keys/dismissible.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {when} from "../format";
    import type {ProblemEffect} from "../problemAction";
    import {syncStore} from "../syncStore.svelte";
    import type {RunOutcome, WireAccountPatch, WireLinkedAccount} from "../types";
    import {importsHref} from "./effects";
    import HeldList from "./HeldList.svelte";
    import ProblemNotice from "./ProblemNotice.svelte";

    let {
        account,
        focus = null,
        now,
        onClose,
        onEffect,
    }: {
        account: WireLinkedAccount;
        /** Open with this part in view: the held list, or the mapping editor (a "Choose another file" fix). */
        focus?: "held" | "mapping" | null;
        now: number;
        onClose: () => void;
        onEffect: (effect: ProblemEffect) => void;
    } = $props();

    const headingId = $props.id();
    const OUTCOME: Record<RunOutcome, string> = {added: "Added", "nothing-new": "Nothing new", failed: "Failed", skipped: "Skipped"};

    const connected = $derived(syncStore.status?.connections.some((c) => c.id === account.connectionId && c.connected) ?? false);
    const problems = $derived(account.problems.filter((p) => p.code !== "held"));
    const journals = $derived((importStore.capabilities?.journals ?? []).filter((j) => j.writable));

    let changing = $state(untrack(() => focus === "mapping"));
    let draftAccount = $state("");
    let draftTarget = $state("");
    let draftRules = $state("");
    let busy = $state(false);
    let error = $state<string | null>(null);
    let confirmStop = $state(false);

    function startChange(): void {
        draftAccount = account.journalAccount;
        draftTarget = account.target;
        draftRules = account.rules;
        error = null;
        changing = true;
    }
    // Opened by a "Choose another file" fix: start in the editor, seeded.
    untrack(() => {
        if (changing) startChange();
    });

    async function patch(body: WireAccountPatch): Promise<boolean> {
        if (Object.keys(body).length === 0) return true;
        busy = true;
        error = null;
        const outcome = await syncStore.patchAccount(account.key, body);
        busy = false;
        if (!outcome.ok) error = outcome.failure.message;
        return outcome.ok;
    }

    async function saveMapping(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        const body: WireAccountPatch = {};
        if (draftAccount.trim() !== account.journalAccount) body.journalAccount = draftAccount.trim();
        if (draftTarget.trim() !== account.target) body.target = draftTarget.trim();
        if (draftRules.trim() !== account.rules) body.rules = draftRules.trim();
        if (await patch(body)) changing = false;
    }

    async function stop(): Promise<void> {
        busy = true;
        const outcome = await syncStore.stopAccount(account.key);
        busy = false;
        if (outcome.ok) onClose();
        else error = outcome.failure.message;
    }

    let heldSection: HTMLElement | undefined = $state();
    $effect(() => {
        if (focus === "held") heldSection?.scrollIntoView({block: "nearest"});
    });
</script>

<div
    class="modal modal-open modal-end"
    role="dialog"
    aria-modal="true"
    aria-labelledby={headingId}
    tabindex="-1"
    data-testid="sync-account-drawer"
    use:dismissible={{trap: true, onDismiss: onClose}}
>
    <div class="modal-box flex h-full max-h-none w-full max-w-lg flex-col gap-4 overflow-y-auto rounded-none">
        <header class="flex items-start gap-2">
            <div class="flex flex-col">
                <h2 id={headingId} class="text-base font-semibold">{account.label}</h2>
                {#if !account.enabled}<span class="text-xs text-base-content/60">Paused</span>{/if}
            </div>
            <button type="button" class="btn ml-auto btn-square btn-ghost btn-sm" aria-label="Close" onclick={onClose}>✕</button>
        </header>

        {#each problems as problem, i (i)}
            <ProblemNotice
                {problem}
                scope={{connectionId: connected ? account.connectionId : null, accountKey: account.key}}
                {onEffect}
                busy={syncStore.running}
            />
        {/each}

        <section class="flex flex-col gap-2" aria-label="Where it goes">
            <div class="flex items-center gap-2">
                <h3 class="text-sm font-semibold">Where it goes</h3>
                {#if !changing}
                    <button type="button" class="btn ml-auto btn-ghost btn-xs" onclick={startChange}>Change…</button>
                {/if}
            </div>
            {#if changing}
                <form class="flex flex-col gap-2" onsubmit={(e) => void saveMapping(e)}>
                    <label class="flex flex-col gap-1 text-xs">
                        <span>Journal account</span>
                        <AccountInput bind:value={draftAccount} accountNames={journal.accountNames} label="Journal account" disabled={busy} />
                    </label>
                    <label class="flex flex-col gap-1 text-xs">
                        <span>Write into</span>
                        {#if journals.length > 0}
                            <select class="select w-full select-sm" bind:value={draftTarget} disabled={busy}>
                                {#if !journals.some((j) => j.id === draftTarget)}<option value={draftTarget}>{draftTarget}</option>{/if}
                                {#each journals as j (j.id)}<option value={j.id}>{j.label}</option>{/each}
                            </select>
                        {:else}
                            <input type="text" class="input w-full input-sm" bind:value={draftTarget} disabled={busy} />
                        {/if}
                    </label>
                    <label class="flex flex-col gap-1 text-xs">
                        <span>Rules file</span>
                        {#if account.rulesChoices.length > 0}
                            <select class="select w-full font-mono select-sm" bind:value={draftRules} disabled={busy} data-testid="sync-rules-choice">
                                {#if !account.rulesChoices.some((c) => c.id === draftRules)}<option value={draftRules}>{draftRules}</option>{/if}
                                {#each account.rulesChoices as choice (choice.id)}
                                    <option value={choice.id}>{choice.id}{choice.fits ? "" : " (doesn't read the sync CSV)"}</option>
                                {/each}
                            </select>
                            <span class="text-base-content/60">Rules files beside your journal that read a sync CSV.</span>
                        {:else}
                            <input type="text" class="input w-full font-mono input-sm" bind:value={draftRules} disabled={busy} spellcheck="false" />
                            <span class="text-base-content/60">A sync rules file, relative to your journal's folder.</span>
                        {/if}
                    </label>
                    <div class="flex gap-2">
                        <button type="submit" class="btn btn-primary btn-sm" disabled={busy}>Save</button>
                        <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (changing = false)}>Cancel</button>
                    </div>
                </form>
            {:else}
                <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
                    <dt class="text-base-content/60">Account</dt>
                    <dd class="truncate font-mono">{account.journalAccount}</dd>
                    <dt class="text-base-content/60">Writes into</dt>
                    <dd class="truncate font-mono">{account.target}</dd>
                    <dt class="text-base-content/60">Rules</dt>
                    <dd class="flex items-center gap-2">
                        <span class="truncate font-mono">{account.rules}</span>
                        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- importsHref resolves the route; the query string is appended -->
                        <button type="button" class="btn btn-ghost btn-xs" onclick={() => void goto(importsHref("rules"))}>Edit</button>
                    </dd>
                    {#if account.categories !== null}
                        <dt class="text-base-content/60">Categories</dt>
                        <dd class="truncate font-mono" title="Shared with this account's manual imports">{account.categories}</dd>
                    {/if}
                </dl>
            {/if}
        </section>

        <section class="flex flex-col gap-2" aria-label="Sync settings">
            <h3 class="text-sm font-semibold">Sync</h3>
            <label class="flex flex-wrap items-center gap-2 text-sm">
                <span class="w-32">Start syncing from</span>
                <DateInput
                    value={account.startDate}
                    disabled={busy}
                    onchange={(e) => void patch({startDate: e.currentTarget.value})}
                    aria-label="Start syncing from"
                />
            </label>
            <label class="flex flex-wrap items-center gap-2 text-sm">
                <span class="w-32">Balance sign</span>
                <select
                    class="select w-auto select-sm"
                    value={String(account.balanceSign)}
                    disabled={busy}
                    onchange={(e) => void patch({balanceSign: e.currentTarget.value === "-1" ? -1 : 1})}
                >
                    <option value="1">As the bank reports it</option>
                    <option value="-1">Reversed</option>
                </select>
            </label>
            <label class="flex cursor-pointer items-center gap-2 text-sm">
                <input
                    type="checkbox"
                    class="toggle toggle-sm"
                    checked={account.enabled}
                    disabled={busy}
                    onchange={(e) => void patch({enabled: e.currentTarget.checked})}
                />
                Sync this account
            </label>
        </section>

        <div bind:this={heldSection}><HeldList {account} /></div>

        <section class="flex flex-col gap-2" aria-label="Recent syncs">
            <h3 class="text-sm font-semibold">Recent syncs</h3>
            {#if account.history.length === 0}
                <p class="text-sm text-base-content/60">Not synced yet.</p>
            {:else}
                <table class="table table-xs">
                    <thead><tr><th>When</th><th>Result</th><th class="text-right">Added</th><th class="text-right">Held</th></tr></thead>
                    <tbody>
                        {#each account.history.slice(0, 10) as entry (entry.at)}
                            <tr>
                                <td>{when(entry.at, now)}</td>
                                <td class={entry.outcome === "failed" ? "text-error" : ""}>{OUTCOME[entry.outcome]}</td>
                                <td class="text-right">{entry.added}</td>
                                <td class="text-right">{entry.held}</td>
                            </tr>
                        {/each}
                    </tbody>
                </table>
            {/if}
        </section>

        {#if error !== null}
            <p class="text-sm text-error" role="alert">{error}</p>
        {/if}

        <section class="mt-auto flex flex-col gap-2 border-t border-base-content/10 pt-3">
            {#if confirmStop}
                <p class="text-sm">Stop syncing this account? Its transactions and rules file stay. Only the link goes.</p>
                <div class="flex gap-2">
                    <button type="button" class="btn btn-error btn-sm" disabled={busy} onclick={() => void stop()}>Stop syncing</button>
                    <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (confirmStop = false)}>Keep it</button>
                </div>
            {:else}
                <button type="button" class="btn self-start btn-outline btn-error btn-sm" onclick={() => (confirmStop = true)}>Stop syncing…</button>
            {/if}
        </section>
    </div>
    <button type="button" class="modal-backdrop" aria-label="Close" onclick={onClose}></button>
</div>
