<!-- Spreadsheet-style report table (WP-07): sticky header row + sticky account
     column (daisyUI table-pin-rows/pin-cols), zebra rows, depth-indented
     accounts with single-child chain compression, right-aligned exact amounts
     (negatives in text-error), emphasized subtotal/total rows. Handles both
     SectionedReport (bs/is — engine rows arrive pre-sign-flipped, rendered
     as-is) and PeriodReport (cf/nw — one column per bucket, horizontal scroll
     on mobile).

     Keyboard: j/k move a cursor over the ACCOUNT rows (section headings, "Total
     X" and "Net" are not in the flattened list, so they are skipped for free)
     and Enter drills into the journal filtered to that account. -->
<script lang="ts">
    import type {MixedAmount} from "$lib/domain/money";
    import type {AmountStyle} from "$lib/domain/types";
    import {formatTotals} from "$lib/journal/rowModel";
    import {openJournal} from "$lib/journal/openJournal";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {bucketLabel} from "$lib/reports/periods";
    import type {PeriodReport, SectionedReport} from "$lib/reports/types";
    import {listCursor} from "$lib/ui/listCursor.svelte";
    import {compressPeriodRows, compressSectionRows} from "./displayRows";

    let {report, styles}: {report: SectionedReport | PeriodReport; styles: ReadonlyMap<string, AmountStyle>} = $props();

    const sectioned = $derived("sections" in report ? report : null);
    const period = $derived("sections" in report ? null : report);

    // Compression hoisted out of the `{#each}`, where it recomputed on every
    // render, into one flat list. The cursor indexes into THIS, and the template
    // iterates it too, so the two cannot drift apart.
    const sectionRows = $derived(sectioned === null ? [] : sectioned.sections.map((section) => compressSectionRows(section.rows)));
    const periodRows = $derived(period === null ? [] : compressPeriodRows(period.rows));
    /**
     * Every cursorable row, in visual order. Totals and headings are deliberately
     * absent, so `j` skips them for free rather than by a filter that could drift.
     *
     * Narrowed to the account, because that is all the cursor needs and it is the
     * only field the sectioned and period row types share.
     */
    const cursorable = $derived<{account: string}[]>(
        sectioned === null ? periodRows.map((r) => ({account: r.row.account})) : sectionRows.flat().map((r) => ({account: r.row.account}))
    );

    const cursor = listCursor(
        () => cursorable,
        (row) => row.account
    );

    function move(delta: number): void {
        cursor.move(delta);
        // Always mounted here (this table is not virtualized), so
        // `scrollIntoView` is honest — unlike in the journal's virtual list.
        // `scroll-mt-10` on the row keeps it clear of the pinned header.
        document.querySelector(`[data-account="${CSS.escape(String(cursor.key ?? ""))}"]`)?.scrollIntoView({block: "nearest"});
    }

    registerKeys({
        id: "report-table",
        priority: PRIORITY.widget,
        bindings: [
            {keys: "j", label: "Next account", group: "Reports", run: () => move(1)},
            {keys: "ArrowDown", label: "Next account", group: "Reports", run: () => move(1)},
            {keys: "k", label: "Previous account", group: "Reports", run: () => move(-1)},
            {keys: "ArrowUp", label: "Previous account", group: "Reports", run: () => move(-1)},
            {keys: "g g", label: "First account", group: "Reports", run: () => (cursor.first(), move(0))},
            {keys: "G", label: "Last account", group: "Reports", run: () => (cursor.last(), move(0))},
            {keys: "Escape", label: "Clear the cursor", group: "Reports", run: () => cursor.clear()},
            {
                keys: "Enter",
                label: "Show this account in the journal",
                group: "Reports",
                run: () => {
                    const row = cursor.item;
                    // `preset: "all"` because a report's own date range lives in
                    // its controls, not in the row — narrowing the journal to
                    // dates the user cannot see here would look like data loss.
                    if (row !== null) void openJournal({accounts: [row.account], preset: "all"});
                },
            },
        ],
    });
</script>

{#snippet amount(ma: MixedAmount)}
    {@const lines = formatTotals(ma, styles)}
    {#if lines.length === 0}
        <span class="text-base-content/40">0</span>
    {:else}
        {#each lines as line (line.text)}
            <div class={line.negative ? "text-error" : ""}>{line.text}</div>
        {/each}
    {/if}
{/snippet}

<div class="border-base-content/10 rounded-box max-h-[70vh] overflow-auto border">
    <table class="table-zebra table-pin-rows table-pin-cols table-sm table">
        {#if sectioned !== null}
            <thead>
                <tr>
                    <th class="w-full">Account</th>
                    <td class="text-right">Amount</td>
                </tr>
            </thead>
            {#each sectioned.sections as section, at (section.title)}
                <tbody>
                    <tr>
                        <th class="text-base-content/60 pt-3 text-xs font-semibold tracking-wide uppercase">{section.title}</th>
                        <td></td>
                    </tr>
                    {#each sectionRows[at] ?? [] as display (display.row.account)}
                        <!-- `scroll-mt-10`: table-pin-rows makes the header
                             sticky, and `block: "nearest"` would otherwise park
                             the row underneath it. -->
                        <tr
                            class="scroll-mt-10 {cursor.key === display.row.account ? 'bg-primary/25' : ''}"
                            aria-current={cursor.key === display.row.account ? "true" : undefined}
                            data-account={display.row.account}
                        >
                            <th class="font-normal whitespace-nowrap">
                                <span style="padding-left: {display.indent}rem">{display.label}</span>
                            </th>
                            <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(display.row.inclusive)}</td>
                        </tr>
                    {/each}
                    <tr class="border-base-content/20 border-t font-semibold">
                        <th>Total {section.title}</th>
                        <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(section.total)}</td>
                    </tr>
                </tbody>
            {/each}
            <tbody>
                <tr class="border-base-content/40 border-t-2 text-base font-bold">
                    <th>Net</th>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(sectioned.grandTotal)}</td>
                </tr>
            </tbody>
        {:else if period !== null}
            <thead>
                <tr>
                    <th>Account</th>
                    {#each period.buckets as bucket (bucket)}
                        <td class="min-w-24 text-right whitespace-nowrap">{bucketLabel(bucket)}</td>
                    {/each}
                </tr>
            </thead>
            <tbody>
                {#each periodRows as display (display.row.account)}
                    <tr
                        class="scroll-mt-10 {cursor.key === display.row.account ? 'bg-primary/25' : ''}"
                        aria-current={cursor.key === display.row.account ? "true" : undefined}
                        data-account={display.row.account}
                    >
                        <th class="font-normal whitespace-nowrap">
                            <span style="padding-left: {display.indent}rem">{display.label}</span>
                        </th>
                        {#each display.row.values as value, i (period.buckets[i])}
                            <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(value)}</td>
                        {/each}
                    </tr>
                {/each}
                {#if period.rows.length === 0}
                    <tr>
                        <th class="text-base-content/50 font-normal">No matching accounts</th>
                        {#each period.buckets as bucket (bucket)}
                            <td></td>
                        {/each}
                    </tr>
                {/if}
                <tr class="border-base-content/40 border-t-2 font-bold">
                    <th>Net</th>
                    {#each period.totals as total, i (period.buckets[i])}
                        <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render amount(total)}</td>
                    {/each}
                </tr>
            </tbody>
        {/if}
    </table>
</div>
