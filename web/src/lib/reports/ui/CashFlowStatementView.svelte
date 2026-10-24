<!-- The statement of cash flows (plans/27): net income, then the change in
     every non-cash balance-sheet account sorted into operating, investing and
     financing, then the net change in cash and the cash balances either side of
     it.

     Laid out like IncomeStatementView: each section is its own table with the
     figure columns pinned to one width, so the ruled lines between sections line
     up with the columns above and below them. One column per period plus a
     Total, or a single Amount column when Interval is Total.

     Every line reads as its effect on cash: an asset that grew is negative (cash
     went into it), a liability that grew is positive. The engine guarantees the
     sections add up to the change in the cash accounts; the footnote at the
     bottom appears only if a posting date splits a transaction across two
     columns.

     Cash lines start OPEN (the per-account cash detail this tab used to show);
     section lines start closed. Keyboard: j/k walk every visible row, Enter opens
     a line or shows an account's postings for the report's dates. -->
<script lang="ts">
    import type {MixedAmount} from "#lib/domain/money.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {openJournal} from "#lib/journal/openJournal.js";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import type {CashFlowStatement, CfColumns, CfSectionKind} from "#lib/reports/types.js";
    import {listCursor} from "#lib/ui/listCursor.svelte.js";
    import {amountCell, type AmountCell} from "./amountCell";
    import {cfCursorRows, cfDisplayModel, cfFoldKey, type CfDisplayRow} from "./cashFlowRows";
    import ProjectedHeadRow from "./ProjectedHeadRow.svelte";
    import {projectedEdge} from "./projectedColumns";

    // `projectedFrom` (plans/28): the first FORECAST column on the Projections
    // tab, drawn as a divider under "Projected". Null changes nothing.
    let {
        report,
        styles,
        base,
        projectedFrom = null,
    }: {report: CashFlowStatement; styles: ReadonlyMap<string, AmountStyle>; base: string | null; projectedFrom?: number | null} = $props();

    /** Folds the user has flipped from their starting state, keyed by `cfFoldKey`, so they survive a refetch. */
    let flipped = $state<Record<string, boolean>>({});

    const isOpen = (key: string, startsOpen: boolean): boolean => flipped[key] ?? startsOpen;

    function toggle(row: CfDisplayRow): void {
        if (!row.expandable) return;
        flipped[cfFoldKey(row)] = !row.expanded;
    }

    const model = $derived(cfDisplayModel(report, isOpen));
    const cursorable = $derived(cfCursorRows(model));
    const cursor = listCursor(
        () => cursorable,
        (row) => row.key
    );

    function move(delta: number): void {
        cursor.move(delta);
        document.querySelector(`[data-cf-row="${CSS.escape(String(cursor.key ?? ""))}"]`)?.scrollIntoView({block: "nearest"});
    }

    /** Enter: open a line, or show an account's postings over the report's dates. */
    function activate(): void {
        const row = cursor.item;
        if (row === null) return;
        if (row.expandable) {
            toggle(row);
            return;
        }
        if (row.account !== null) void openJournal({accounts: [row.account], from: report.from, to: report.to});
    }

    registerKeys({
        id: "cash-flow",
        priority: PRIORITY.widget,
        bindings: [
            {keys: "j", label: "Next row", group: "Reports", run: () => move(1)},
            {keys: "ArrowDown", label: "Next row", group: "Reports", run: () => move(1)},
            {keys: "k", label: "Previous row", group: "Reports", run: () => move(-1)},
            {keys: "ArrowUp", label: "Previous row", group: "Reports", run: () => move(-1)},
            {keys: "g g", label: "First row", group: "Reports", run: () => (cursor.first(), move(0))},
            {keys: "G", label: "Last row", group: "Reports", run: () => (cursor.last(), move(0))},
            {keys: "Escape", label: "Clear the cursor", group: "Reports", run: () => cursor.clear()},
            {keys: "Enter", label: "Expand a line, or show an account in the journal", group: "Reports", run: activate},
        ],
    });

    const cell = (ma: MixedAmount): AmountCell => amountCell(ma, base, styles);
    const buckets = $derived(model.buckets);

    /** Static literals so Tailwind's scanner sees every class. */
    const ACCENT: Record<CfSectionKind, {text: string; rule: string}> = {
        operating: {text: "text-info", rule: "border-info/40"},
        investing: {text: "text-secondary", rule: "border-secondary/40"},
        financing: {text: "text-accent", rule: "border-accent/40"},
    };
</script>

{#snippet amount(c: AmountCell, size: string)}
    <span class="{size} {c.negative ? 'text-error' : ''}">{c.text}</span>
    {#if c.extras.length > 0}
        <span class="block text-xs font-normal text-base-content/50">{c.extras.join(" · ")}</span>
    {/if}
{/snippet}

{#snippet figures(columns: CfColumns, emphasis: string)}
    {#if buckets !== null}
        {#each columns.values as value, i (i)}
            <td class="w-28 min-w-28 {projectedEdge(i, projectedFrom)} text-right align-top font-mono whitespace-nowrap tabular-nums"
                >{@render amount(cell(value), "")}</td
            >
        {/each}
        <td class="w-32 min-w-32 text-right align-top font-mono whitespace-nowrap tabular-nums" data-column="total">
            {@render amount(cell(columns.total), emphasis)}
        </td>
    {:else}
        <td class="w-32 text-right align-top font-mono whitespace-nowrap tabular-nums">{@render amount(cell(columns.total), emphasis)}</td>
    {/if}
{/snippet}

{#snippet figureHeads(label: string)}
    <th class="w-full font-normal"><span class="sr-only">{label}</span></th>
    {#if buckets !== null}
        {#each buckets as bucket, i (bucket.key)}
            <th class="w-28 min-w-28 {projectedEdge(i, projectedFrom)} text-right font-normal" title="{bucket.from} to {bucket.to}">{bucket.label}</th>
        {/each}
        <th class="w-32 min-w-32 text-right font-normal">Total</th>
    {:else}
        <th class="w-32 text-right font-normal">Amount</th>
    {/if}
{/snippet}

{#snippet rowCells(row: CfDisplayRow)}
    <tr
        class="scroll-mt-10 {cursor.key === row.key ? 'bg-primary/25' : ''} {row.kind === 'account' ? '' : 'font-medium'}"
        aria-current={cursor.key === row.key ? "true" : undefined}
        data-cf-row={row.key}
        data-account={row.account ?? undefined}
    >
        <th class="w-full font-normal">
            {#if row.expandable}
                <button
                    type="button"
                    class="flex cursor-pointer items-center gap-1.5 text-left font-medium hover:text-primary"
                    aria-expanded={row.expanded}
                    onclick={() => toggle(row)}
                >
                    <span
                        class="inline-block w-3 shrink-0 text-[0.65rem] text-base-content/40 transition-transform {row.expanded ? 'rotate-90' : ''}"
                        aria-hidden="true">▶</span
                    >
                    {row.label}
                </button>
            {:else if row.kind === "account"}
                <span class="whitespace-nowrap text-base-content/70" style="padding-left: {row.indent * 1.25}rem" title={row.account}>{row.label}</span>
            {:else}
                <span class="flex items-center gap-1.5 font-medium" title={row.account ?? undefined}>
                    <span class="inline-block w-3 shrink-0" aria-hidden="true"></span>{row.label}
                </span>
            {/if}
        </th>
        {@render figures(row.amounts, row.kind === "account" ? "text-base-content/70" : "font-medium")}
    </tr>
{/snippet}

{#snippet ruled(label: string, columns: CfColumns, testid: string)}
    <div class="-my-1 overflow-x-auto border-y-2 border-base-content/30" data-testid={testid}>
        <table class="table table-sm">
            <tbody>
                <tr>
                    <th class="w-full text-sm font-semibold">{label}</th>
                    {@render figures(columns, "font-semibold")}
                </tr>
            </tbody>
        </table>
    </div>
{/snippet}

<div class="flex flex-col gap-4" data-testid="cash-flow-statement">
    <p class="-mb-1 text-xs text-base-content/50">At cost, for {report.from} to {report.to}</p>

    {#each model.boxes as box (box.kind)}
        <section class="overflow-x-auto rounded-box border border-base-content/10" data-testid="cf-section-{box.kind}">
            <h3 class="bg-base-200 {ACCENT[box.kind].rule} border-b-2 px-4 py-2.5 text-sm font-semibold tracking-wide uppercase {ACCENT[box.kind].text}">
                {box.title}
            </h3>
            <table class="table table-sm">
                <thead>
                    <ProjectedHeadRow columns={buckets?.length ?? 0} {projectedFrom} trailing={1} />
                    <tr class="text-[0.65rem] text-base-content/40">{@render figureHeads(`${box.title} line`)}</tr>
                </thead>
                <tbody>
                    {#each box.rows as row (row.key)}
                        {@render rowCells(row)}
                    {/each}
                </tbody>
                <tfoot>
                    <tr class="border-t-2 border-base-content/20 bg-base-200 text-sm font-bold text-base-content">
                        <th class="w-full font-bold">{box.totalLabel}</th>
                        {@render figures(box.total, "font-bold")}
                    </tr>
                </tfoot>
            </table>
        </section>
    {/each}

    {#if model.exchange !== null}
        {@render ruled("Effect of exchange rates on cash", model.exchange, "cf-exchange")}
    {/if}
    {@render ruled("Net change in cash", model.netChange, "cf-net-change")}

    {#each [model.beginning, model.ending] as block (block.kind)}
        <section class="overflow-x-auto rounded-box border border-base-content/20 bg-base-200/40" data-testid="cf-cash-{block.kind}">
            <table class="table table-sm">
                <thead>
                    <ProjectedHeadRow columns={buckets?.length ?? 0} {projectedFrom} trailing={1} />
                    <tr class="text-[0.65rem] text-base-content/40">{@render figureHeads(block.title)}</tr>
                </thead>
                <tbody>
                    <tr class="text-sm">
                        <th class="w-full font-semibold tracking-wide uppercase">{block.title}</th>
                        {@render figures(block.total, "font-semibold")}
                    </tr>
                    {#each block.rows as row (row.key)}
                        {@render rowCells(row)}
                    {/each}
                </tbody>
            </table>
        </section>
    {/each}

    <!-- A footnote, not a warning: it is almost always a posting date putting
         one side of a transaction in the next period, the Total column still
         agrees, and nothing needs fixing. -->
    {#if !report.reconciles}
        <p class="text-xs text-base-content/50" role="note" data-testid="cf-unreconciled">
            In some periods the net change in cash differs from the change in cash balances. A posting date can do this by placing one side of a transaction in
            a different period from the other. The Total column still agrees.
        </p>
    {/if}
</div>
