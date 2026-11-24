<!-- A labelled list (MBR's contact schema: `emails`, `phones`, `addresses`):
     one row per value, each with its label. Invoices pick the value labelled
     `billing`, then `work`, then the first (ARCHITECTURE §3.4), and MBR's
     contact card shows them in this order, so a row can move up. Addresses
     are several lines each. -->
<script lang="ts">
    import {LABELS, type Labelled} from "#lib/customers.js";

    let {
        items = $bindable(),
        legend,
        noun,
        name,
        multiline = false,
        type = "text",
        placeholder = "",
    }: {
        items: Labelled[];
        /** The section's heading ("Emails"). */
        legend: string;
        /** One of them ("email"), for the Add button. */
        noun: string;
        /** A stable id prefix and test id (`emails`). */
        name: string;
        multiline?: boolean;
        type?: "text" | "email" | "tel";
        placeholder?: string;
    } = $props();

    function add(): void {
        items.push({label: items.some((i) => i.label === "billing") ? "work" : "billing", value: ""});
    }

    function remove(at: number): void {
        items.splice(at, 1);
    }

    function up(at: number): void {
        if (at === 0) return;
        const [moved] = items.splice(at, 1);
        items.splice(at - 1, 0, moved);
    }

    const setLabel = (item: Labelled, value: string): void => {
        item.label = value.trim() === "" ? null : value;
    };
</script>

<fieldset class="fieldset" data-testid="labelled-{name}">
    <legend class="fieldset-legend">{legend}</legend>
    <datalist id="{name}-labels">
        {#each LABELS as label (label)}<option value={label}></option>{/each}
    </datalist>
    {#each items as item, at (at)}
        <div class="flex items-start gap-2" data-testid="{name}-row">
            <input
                class="input w-28 shrink-0 input-sm"
                aria-label="{legend} {at + 1}: label"
                list="{name}-labels"
                placeholder="label"
                value={item.label ?? ""}
                oninput={(e) => setLabel(item, e.currentTarget.value)}
            />
            {#if multiline}
                <textarea class="textarea min-h-16 flex-1 textarea-sm" aria-label="{legend} {at + 1}" {placeholder} rows="3" bind:value={item.value}></textarea>
            {:else}
                <input class="input flex-1 input-sm" aria-label="{legend} {at + 1}" {type} {placeholder} bind:value={item.value} />
            {/if}
            <button type="button" class="btn btn-ghost btn-sm" aria-label="Move {legend} {at + 1} up" disabled={at === 0} onclick={() => up(at)}>↑</button>
            <button type="button" class="btn btn-ghost btn-sm" aria-label="Remove {legend} {at + 1}" onclick={() => remove(at)}>✕</button>
        </div>
    {/each}
    <div>
        <button type="button" class="btn btn-ghost btn-xs" onclick={add}>Add {noun}</button>
    </div>
</fieldset>
