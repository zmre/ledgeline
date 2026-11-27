<!-- Inside Void…'s confirmation (plan 46 B-invoice lifecycle): what happens
     to the invoice's time. Release it (the default: back to To bill, to bill
     again) or mark it not billed (written off). Nothing shows for an invoice
     no time is on. -->
<script lang="ts">
    import {hoursWords} from "#lib/time/billing/words.js";
    import type {Qty} from "#lib/time/types.js";

    let {time, how = $bindable("release")}: {time: Qty | null; how?: "release" | "nobill"} = $props();
</script>

{#if time !== null}
    <fieldset class="flex flex-col gap-1" data-testid="void-time">
        <legend class="mb-1">Its {hoursWords(time)} of time:</legend>
        <label class="flex items-center gap-2"
            ><input type="radio" class="radio radio-sm" bind:group={how} value="release" data-testid="void-time-release" /> Release it: back to To bill, to bill again</label
        >
        <label class="flex items-center gap-2"
            ><input type="radio" class="radio radio-sm" bind:group={how} value="nobill" data-testid="void-time-nobill" /> Mark it not billed (written off)</label
        >
    </fieldset>
{/if}
