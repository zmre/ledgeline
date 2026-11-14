<!-- One posting's account cell: its type badge in a FIXED-width slot, then
     the account. The slot is the same width on every card, so account names
     line up down the whole list the way the amounts do (`POSTING_GRID`).

     The counter-account line uses it too (`children` carries the arrow and
     the "(default)" note); `dim` greys the badge of an inferred default. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import TypeBadge from "./TypeBadge.svelte";
    import {TYPE_SLOT} from "./frame";

    let {
        account,
        classes,
        dim = false,
        class: extra = "",
        testid,
        children,
    }: {
        account: string;
        classes: AccountClasses;
        dim?: boolean;
        class?: string;
        testid?: string;
        /** The cell's text; the account name when absent. */
        children?: Snippet;
    } = $props();

    const segments = $derived(account.split(":"));
</script>

<span class="flex min-w-0 items-baseline {extra}" data-testid={testid}>
    <span class="{TYPE_SLOT} flex self-center">
        <TypeBadge {account} {classes} {dim} />
    </span>
    <!-- A phone has no hover to read a `title`, so there the name wraps (at
         its colons first) instead of being cut short. -->
    <span class="min-w-0 [overflow-wrap:anywhere] sm:truncate" title={account}>
        {#if children}{@render children()}{:else if account === ""}(no account){:else}{#each segments as segment, at (at)}{#if at > 0}:<wbr
                    />{/if}{segment}{/each}{/if}
    </span>
</span>
