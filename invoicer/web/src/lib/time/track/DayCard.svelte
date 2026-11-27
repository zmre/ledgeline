<!-- Today's card (plan 46 T-slots `c`): the printed tag of the original
     device. Each account's total today with its notes, compact, to copy as
     text, print or share. Printing shows the card alone (the page hides the
     rest under `print:`). -->
<script lang="ts">
    import {dayCardText, dayHeading, durationText, type CardRow} from "./format.js";

    let {
        date,
        rows,
        person,
        loading = false,
        onClose,
    }: {date: string; rows: CardRow[]; person: string | null; loading?: boolean; onClose: () => void} = $props();

    const total = $derived(rows.reduce((sum, r) => sum + r.seconds, 0));
    let copied = $state<"yes" | "no" | null>(null);

    async function copy(): Promise<void> {
        try {
            await navigator.clipboard.writeText(dayCardText(date, rows, person));
            copied = "yes";
        } catch {
            copied = "no";
        }
    }
</script>

<section class="rounded-box border border-base-300 bg-base-100 p-4 print:border-0 print:p-0" aria-labelledby="time-card-title" data-testid="time-card">
    <header class="flex flex-wrap items-center gap-2">
        <h2 id="time-card-title" class="text-base font-semibold">
            {dayHeading(date)}{person === null ? "" : ` · ${person}`}
        </h2>
        <span class="text-sm tabular-nums" data-testid="time-card-total">{durationText(total)}</span>
        <div class="ml-auto flex gap-2 print:hidden">
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => void copy()} data-testid="time-card-copy"
                >{copied === "yes" ? "Copied" : "Copy"}</button
            >
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => window.print()}>Print</button>
            <button type="button" class="btn btn-ghost btn-sm" onclick={onClose}>Close</button>
        </div>
    </header>
    {#if copied === "no"}<p class="mt-1 text-xs print:hidden" role="status">The card couldn't be copied; select it and copy instead.</p>{/if}
    {#if loading && rows.length === 0}
        <p class="mt-3 text-sm text-base-content/60">Loading…</p>
    {:else if rows.length === 0}
        <p class="mt-3 text-sm text-base-content/60">Nothing tracked today yet.</p>
    {:else}
        <table class="table mt-2 table-sm">
            <tbody>
                {#each rows as row (row.account)}
                    <tr data-testid="time-card-row">
                        <td class="font-medium">{row.label}</td>
                        <td class="text-right whitespace-nowrap tabular-nums">{durationText(row.seconds)}</td>
                        <td class="text-base-content/70">{row.notes.join("; ")}</td>
                    </tr>
                {/each}
            </tbody>
        </table>
    {/if}
</section>
