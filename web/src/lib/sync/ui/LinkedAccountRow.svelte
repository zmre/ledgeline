<!-- One linked account on the Accounts tab (plan 43 §4.2): its name and where
     it goes, one line of how the last sync went, and any problem with its fix.
     The name opens the account's drawer. -->
<script lang="ts">
    import type {ProblemEffect} from "../problemAction";
    import {accountLine} from "../statusLine";
    import {syncStore} from "../syncStore.svelte";
    import type {WireLinkedAccount} from "../types";
    import ProblemNotice from "./ProblemNotice.svelte";

    let {
        account,
        now,
        connected,
        elsewhere = false,
        onOpen,
        onEffect,
    }: {
        account: WireLinkedAccount;
        now: number;
        /** Its connection exists on this computer — a problem's re-sync can only target one that does. */
        connected: boolean;
        /** Its connection syncs on another computer: this one has no history of its own to show. */
        elsewhere?: boolean;
        onOpen: (key: string) => void;
        onEffect: (effect: ProblemEffect) => void;
    } = $props();

    // Held rows included: "2 transactions need a look" with its Review is the
    // warning that makes the header's sync icon amber, so it shows where it is.
    const problems = $derived(account.problems.filter((p) => p.severity !== "info"));
</script>

<li class="flex flex-col gap-1 border-b border-base-content/5 px-3 py-2 last:border-b-0" data-testid="sync-linked-row" data-key={account.key}>
    <div class="flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
        <button type="button" class="link font-medium link-hover" onclick={() => onOpen(account.key)}>{account.label}</button>
        <span class="truncate font-mono text-xs text-base-content/70">→ {account.journalAccount}</span>
    </div>
    <span class="text-xs text-base-content/70" data-testid="sync-linked-line">{elsewhere ? "Synced on another computer" : accountLine(account, now)}</span>
    {#each problems as problem, i (i)}
        <ProblemNotice {problem} scope={{connectionId: connected ? account.connectionId : null, accountKey: account.key}} {onEffect} busy={syncStore.running} />
    {/each}
</li>
