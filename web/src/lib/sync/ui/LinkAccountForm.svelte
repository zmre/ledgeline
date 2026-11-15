<!-- One account in the wizard's link step (plan 43 §4.3 step 3): where it
     goes (an existing account, a new one with its opening balance, or not at
     all), the balance check and sign question, the start date, the file it
     writes into, its rules file, and — when a manual rules file already
     categorizes it — whether to share those rules. -->
<script lang="ts">
    import DateInput from "#lib/components/DateInput.svelte";
    import type {JournalTarget} from "#lib/imports/importTypes.js";
    import AccountInput from "#lib/journal/edit/AccountInput.svelte";
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {formatMoney} from "../format";
    import type {NewAccountType, WireRemoteAccount} from "../types";
    import {balanceCompare, chunkNote, draftIssues, historyNote, newRulesName, signQuestion, type LinkDraft} from "../wizardModel";

    let {
        remote,
        draft = $bindable(),
        journals,
        accountNames,
        onShare,
    }: {
        remote: WireRemoteAccount;
        draft: LinkDraft;
        journals: readonly JournalTarget[];
        accountNames: string[];
        /** Open the share preview; the wizard sets `share` on its answer. */
        onShare: () => void;
    } = $props();

    const id = $props.id();
    const today = localToday();
    const TYPES: {value: NewAccountType; label: string}[] = [
        {value: "C", label: "Cash (bank account)"},
        {value: "A", label: "Asset"},
        {value: "L", label: "Liability (card or loan)"},
    ];

    type Mode = "existing" | "new" | "skip";
    const mode = $derived<Mode>(!draft.include ? "skip" : draft.createAccount ? "new" : "existing");
    function setMode(next: Mode): void {
        draft.include = next !== "skip";
        draft.createAccount = next === "new";
        if (draft.rulesChoice === "new")
            draft.newRulesName = newRulesName(next === "new" ? draft.newAccount.name : draft.journalAccount || draft.newAccount.name);
    }

    const compare = $derived(mode === "existing" ? balanceCompare(remote, draft.balanceSign) : null);
    const question = $derived(mode === "existing" ? signQuestion(remote) : null);
    const notes = $derived([historyNote(remote, draft.startDate, today), chunkNote(draft.startDate, today)].filter((n): n is string => n !== null));
    const fitting = $derived(remote.suggestion.syncRules.filter((r) => r.fits));
    const manual = $derived(remote.suggestion.manualRules);
    const writable = $derived(journals.filter((j) => j.writable));
    const issues = $derived(draftIssues(draft).filter((i) => !i.startsWith("Say whether")));
    const fileName = (path: string): string => path.split("/").at(-1) ?? path;
</script>

<fieldset class="flex flex-col gap-2 rounded-box border border-base-content/10 p-3" data-testid="sync-link-form" data-remote={remote.remoteId}>
    <legend class="flex w-full flex-wrap items-baseline gap-x-3 px-1">
        <span class="font-medium">{remote.label}</span>
        <span class="font-mono text-sm">{formatMoney(remote.balance, remote.currency)}</span>
    </legend>

    <div class="flex flex-col gap-2 text-sm" role="radiogroup" aria-label="Where {remote.label} goes">
        <div class="flex flex-wrap items-center gap-2">
            <input
                type="radio"
                class="radio radio-sm"
                name="{id}-mode"
                aria-label="Sync into an existing journal account"
                checked={mode === "existing"}
                onchange={() => setMode("existing")}
            />
            <span class="w-28">Journal account</span>
            <span class="min-w-56 grow">
                <AccountInput
                    bind:value={draft.journalAccount}
                    {accountNames}
                    label="Journal account for {remote.label}"
                    placeholder="assets:bank:checking"
                    disabled={mode !== "existing"}
                    onCommit={() => {
                        if (draft.rulesChoice === "new") draft.newRulesName = newRulesName(draft.journalAccount);
                    }}
                />
            </span>
            {#if compare !== null}
                <span class="text-xs {compare.matches ? 'text-success' : 'text-base-content/70'}">{compare.text}</span>
            {/if}
        </div>
        <div class="flex flex-wrap items-center gap-2">
            <input
                type="radio"
                class="radio radio-sm"
                name="{id}-mode"
                aria-label="Create a new journal account"
                checked={mode === "new"}
                onchange={() => setMode("new")}
            />
            <span class="w-28">New account</span>
            {#if mode === "new"}
                <input type="text" class="input min-w-56 grow font-mono input-sm" aria-label="New account name" bind:value={draft.newAccount.name} />
                <select class="select w-auto select-sm" aria-label="Account type" bind:value={draft.newAccount.type}>
                    {#each TYPES as type (type.value)}<option value={type.value}>{type.label}</option>{/each}
                </select>
                <span class="flex flex-wrap items-center gap-2">
                    opening balance
                    <input
                        type="text"
                        inputmode="decimal"
                        class="input w-32 font-mono input-sm"
                        aria-label="Opening balance"
                        bind:value={draft.newAccount.openingBalance}
                    />
                    on
                    <DateInput bind:value={draft.newAccount.openingDate} aria-label="Opening balance date" />
                </span>
            {:else}
                <span class="text-xs text-base-content/60">{draft.newAccount.name}</span>
            {/if}
        </div>
        <label class="flex items-center gap-2">
            <input type="radio" class="radio radio-sm" name="{id}-mode" checked={mode === "skip"} onchange={() => setMode("skip")} />
            <span>Don't sync</span>
        </label>
    </div>

    {#if mode !== "skip"}
        {#if question !== null}
            <label class="flex cursor-pointer items-start gap-2 text-sm" data-testid="sync-sign-question">
                <input
                    type="checkbox"
                    class="checkbox mt-0.5 checkbox-sm"
                    checked={draft.balanceSign === -1}
                    onchange={(e) => (draft.balanceSign = e.currentTarget.checked ? -1 : 1)}
                />
                <span>{question}</span>
            </label>
        {/if}

        <label class="flex flex-wrap items-center gap-2 text-sm">
            <span class="w-28">Start syncing</span>
            <DateInput bind:value={draft.startDate} aria-label="Start syncing {remote.label} from" />
            {#if mode === "existing" && remote.suggestion.journalAccount !== null && draft.startDate === remote.suggestion.startDate}
                <span class="text-xs text-base-content/60">the day after your journal's last transaction in it</span>
            {/if}
        </label>
        {#each notes as note (note)}
            <p class="text-xs text-warning">{note}</p>
        {/each}

        <label class="flex flex-wrap items-center gap-2 text-sm">
            <span class="w-28">Write into</span>
            {#if writable.length > 0}
                <select class="select w-auto select-sm" bind:value={draft.target}>
                    {#if !writable.some((j) => j.id === draft.target)}<option value={draft.target}>{draft.target || "Choose a file"}</option>{/if}
                    {#each writable as j (j.id)}<option value={j.id}>{j.label}</option>{/each}
                </select>
            {:else}
                <input type="text" class="input w-56 font-mono input-sm" aria-label="Write into" bind:value={draft.target} />
            {/if}
        </label>

        <label class="flex flex-wrap items-center gap-2 text-sm">
            <span class="w-28">Rules file</span>
            <select class="select w-auto select-sm" bind:value={draft.rulesChoice}>
                <option value="new">New: {draft.newRulesName}</option>
                {#each fitting as rules (rules.id)}<option value={rules.id}>{fileName(rules.id)}</option>{/each}
            </select>
        </label>

        {#if manual !== null}
            <div class="flex flex-col gap-1 rounded-box bg-base-200 p-2 text-sm" data-testid="sync-share-prompt">
                <p>
                    ⓘ <code>{fileName(manual.id)}</code> already categorizes this account ({manual.ruleCount} rule{manual.ruleCount === 1 ? "" : "s"}). Share
                    them with sync, so a rule you add on either side works for both?
                </p>
                {#if draft.share === "ask"}
                    <div class="flex flex-wrap gap-2">
                        <button type="button" class="btn btn-primary btn-xs" onclick={onShare}>Share them</button>
                        <button type="button" class="btn btn-ghost btn-xs" onclick={() => (draft.share = "separate")}>Keep separate</button>
                    </div>
                {:else}
                    <p class="text-xs text-base-content/70">
                        {draft.share === "share" ? "They'll be shared when you press Finish." : "Kept separate: the sync rules start empty."}
                        <button type="button" class="link" onclick={() => (draft.share = "ask")}>Change</button>
                    </p>
                {/if}
            </div>
        {/if}

        {#each issues as issue (issue)}
            <p class="text-xs text-error">{issue}</p>
        {/each}
    {/if}
</fieldset>
