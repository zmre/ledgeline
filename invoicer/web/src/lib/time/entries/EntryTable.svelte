<!-- Entries as a table, grouped by day (the timesheet: each day's heading
     carries its total) or by account (T-entries). Columns: Date · Person ·
     Account · Hours · Note · Billing · File; Person and File drop out below
     `md` (container queries follow the UI scale, plan 39).

     A line on a sent invoice or a booked bill is read-only: its Billing cell
     says so with a lock, and editing it explains why in plain words, with a
     link to the invoice. The edited line is replaced in place by the form
     the page hands in (`editor`).

     Keyboard: `j`/`k` move the row cursor, Enter edits the cursored line, `x`
     picks it for a bulk action (Shift+click on the boxes picks a range),
     Escape drops the cursor. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import {resolve} from "$app/paths";
    import type {SvelteSet} from "svelte/reactivity";
    import type {ListCursor} from "#ui/ui/listCursor.svelte.js";
    import {CURSOR_ROW, rowKeys} from "#lib/keys/rowKeys.svelte.js";
    import type {EntryView} from "../api/entries.js";
    import {billingText, flatten, type EntryGroup} from "./group.js";
    import {dayHeading} from "../dates.js";
    import {formatDuration} from "../duration.js";

    let {
        groups,
        grouping,
        selected,
        editing,
        lockedNotice,
        editor,
        onEdit,
        onDelete,
        onToggle,
        onToggleRange,
    }: {
        groups: EntryGroup[];
        grouping: "day" | "account";
        selected: SvelteSet<string>;
        /** The line the form replaces, by id. */
        editing: string | null;
        /** The locked line whose explanation shows, by id. */
        lockedNotice: string | null;
        editor: Snippet<[EntryView]>;
        /** Edit a line (or, when locked, say why it can't be). */
        onEdit: (entry: EntryView) => void;
        onDelete: (entry: EntryView) => void;
        onToggle: (entry: EntryView) => void;
        onToggleRange: (ids: string[]) => void;
    } = $props();

    const rows = $derived(flatten(groups));
    const COLUMNS = 9;

    let tableEl = $state<HTMLTableElement | null>(null);
    const cursor: ListCursor<EntryView> = rowKeys<EntryView>({
        id: "time-entries",
        group: "Time entries",
        noun: "entry",
        items: () => rows,
        keyOf: (e) => e.id,
        root: () => tableEl,
        open: () => onEdit,
        extra: [
            {
                keys: "x",
                label: "Pick the entry for a bulk action",
                group: "Time entries",
                enabled: () => cursor.item !== null,
                run: () => {
                    if (cursor.item !== null) onToggle(cursor.item);
                },
            },
        ],
    });

    /** The last box clicked: Shift+click picks from it to the one clicked now. */
    let anchor: string | null = null;

    function boxClick(event: MouseEvent, entry: EntryView): void {
        if (event.shiftKey && anchor !== null) {
            const from = rows.findIndex((e) => e.id === anchor);
            const to = rows.findIndex((e) => e.id === entry.id);
            if (from >= 0 && to >= 0) {
                event.preventDefault();
                const [lo, hi] = from < to ? [from, to] : [to, from];
                onToggleRange(rows.slice(lo, hi + 1).map((e) => e.id));
                anchor = entry.id;
                return;
            }
        }
        anchor = entry.id;
        onToggle(entry);
    }

    const heading = (group: EntryGroup): string => (grouping === "day" ? dayHeading(group.key) : group.key);
    const indexOf = (entry: EntryView): number => rows.indexOf(entry);
</script>

<div class="overflow-x-auto rounded-box border border-base-300">
    <table class="table table-sm" bind:this={tableEl} data-testid="entry-table">
        <thead>
            <tr>
                <th class="w-8"><span class="sr-only">Pick</span></th>
                <th>Date</th>
                <th class="hidden md:table-cell">Person</th>
                <th>Account</th>
                <th class="text-right">Hours</th>
                <th>Note</th>
                <th>Billing</th>
                <th class="hidden md:table-cell">File</th>
                <th><span class="sr-only">Actions</span></th>
            </tr>
        </thead>
        {#each groups as group (group.key)}
            <tbody data-testid="entry-group" data-key={group.key}>
                <tr class="bg-base-200/70">
                    <th colspan={COLUMNS - 5} class="font-medium" scope="colgroup">{heading(group)}</th>
                    <td class="text-right font-medium tabular-nums" data-testid="entry-group-total">{formatDuration(group.seconds)}</td>
                    <td colspan="4"></td>
                </tr>
                {#each group.entries as entry (entry.id)}
                    {@const at = indexOf(entry)}
                    {#if editing === entry.id}
                        <tr data-cursor={cursor.index === at ? "" : undefined}>
                            <td colspan={COLUMNS}>{@render editor(entry)}</td>
                        </tr>
                    {:else}
                        <tr
                            class={["hover:bg-base-200", cursor.index === at && CURSOR_ROW, selected.has(entry.id) && "bg-primary/10"]}
                            data-cursor={cursor.index === at ? "" : undefined}
                            data-testid="entry-row"
                            data-id={entry.id}
                            data-locked={entry.locked === null ? undefined : "true"}
                        >
                            <td>
                                <input
                                    type="checkbox"
                                    class="checkbox checkbox-xs"
                                    checked={selected.has(entry.id)}
                                    onclick={(event) => boxClick(event, entry)}
                                    aria-label="Pick {entry.account} on {entry.date}"
                                    data-testid="entry-pick"
                                />
                            </td>
                            <td class="whitespace-nowrap tabular-nums">{entry.date}</td>
                            <td class="hidden md:table-cell">{entry.person ?? "—"}</td>
                            <td class="max-w-56 truncate" title={entry.account}>{entry.account}</td>
                            <td class="text-right whitespace-nowrap tabular-nums" title="{entry.quantity.hours} h">{entry.quantity.text}</td>
                            <td class="max-w-72 truncate" title={entry.note}>{entry.note}</td>
                            <td class="whitespace-nowrap" data-testid="entry-billing">
                                {#if entry.locked !== null}
                                    <span class="mr-1 badge badge-ghost badge-xs" title={entry.locked} data-testid="entry-locked">read-only</span>
                                {/if}
                                {#if entry.invoice !== null}
                                    <a class="link link-hover" href={resolve("/invoices/[number]", {number: entry.invoice})}>Invoice {entry.invoice}</a>
                                    {#if entry.bill !== null}<span class="text-base-content/70"> · Bill {entry.bill}</span>{/if}
                                {:else}
                                    <span class={entry.billed === "unbilled" ? "font-medium" : "text-base-content/70"}>{billingText(entry)}</span>
                                {/if}
                            </td>
                            <td class="hidden text-xs whitespace-nowrap text-base-content/50 md:table-cell" title="{entry.file}, line {entry.line}">
                                {entry.file}:{entry.line}
                            </td>
                            <td class="whitespace-nowrap">
                                <button type="button" class="btn btn-ghost btn-xs" onclick={() => onEdit(entry)} data-testid="entry-edit">Edit</button>
                                {#if entry.locked === null}
                                    <button type="button" class="btn btn-ghost text-error btn-xs" onclick={() => onDelete(entry)} data-testid="entry-delete"
                                        >Delete</button
                                    >
                                {/if}
                            </td>
                        </tr>
                        {#if lockedNotice === entry.id && entry.locked !== null}
                            <tr>
                                <td colspan={COLUMNS}>
                                    <div class="alert py-2 text-sm alert-info" role="status" data-testid="entry-locked-notice">
                                        <span>This line can't be changed: {entry.locked}.</span>
                                        {#if entry.invoice !== null}
                                            <a class="btn btn-outline btn-xs" href={resolve("/invoices/[number]", {number: entry.invoice})}
                                                >Open invoice {entry.invoice}</a
                                            >
                                        {/if}
                                    </div>
                                </td>
                            </tr>
                        {/if}
                    {/if}
                {/each}
            </tbody>
        {/each}
    </table>
</div>
