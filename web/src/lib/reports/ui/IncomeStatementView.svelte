<!-- The grouped income statement (plans/13): ladder-ordered boxes instead of one
     long `hledger is` lookalike, mirroring BalanceSheetView beat for beat.

     The complaint this fixes: the old table printed a section roll-up, then each
     account under it, then a "Total Revenues" footer repeating the roll-up — the
     same number three times. A section's lines are GROUPS now, a group's total is
     summed over its members, and the accounts only exist inside an expanded
     disclosure. Nothing is printed twice.

     Boxes are ADAPTIVE: a section with no members is not sent, so an untagged
     personal journal gets exactly two boxes and a net income figure, with no
     empty headings and no GAAP jargon it never asked for. The ladder (Gross
     profit, EBITDA, Operating income, Income before taxes) materialises line by
     line as the journal's tags earn it.

     Ladder lines render RULED and BETWEEN the boxes, never inside one. A subtotal
     spans everything printed above it, so putting it in a box would claim it
     belonged to that box alone. They are also NOT cursorable — there is nothing
     to expand or drill into on a subtotal.

     Every figure is ONE number, because the engine values the whole report into a
     single base commodity; anything it could not convert is demoted to a small
     secondary line by the shared `amountCell`, rather than stacked as a second
     balance in the same cell.

     Two column layouts (plans/24). Interval = Total: Amount, Prior and % change
     ((current − prior) / |prior|, `—` when the prior is zero). Any other
     interval: one column per bucket and a Total that sums them, with no prior
     and no percentage. There every box, ladder line and the net income row
     share ONE column template (statementGrid.ts): the same colgroup under
     fixed layout, the same width, inside one horizontal scroller. Auto layout
     gave each table its own label width once the buckets overflowed, so the
     ladder lines' figures sat under no heading at all. Each ladder line also
     carries the period headings (and the Actual/Projected band) itself.

     Keyboard: j/k walk every VISIBLE row — group headings included, so `j` does
     something on first load rather than nothing — and Enter either opens the
     disclosure (on a group) or drills into the journal (on an account). -->
<script lang="ts">
    import type {AmountStyle} from "#lib/domain/types.js";
    import {openJournal} from "#lib/journal/openJournal.js";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import type {Amounts, IncomeStatementReport, IsSectionKind} from "#lib/reports/types.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {listCursor} from "#lib/ui/listCursor.svelte.js";
    import {amountCell, fmtPct, isCursorRows, isDisplayModel, type AmountCell, type IsDisplayRow} from "./incomeStatementRows";
    import FigureColgroup from "./FigureColgroup.svelte";
    import ProjectedHeadRow from "./ProjectedHeadRow.svelte";
    import {projectedEdge} from "./projectedColumns";
    import SankeyPanel from "./SankeyPanel.svelte";
    import {type FlowsPanel} from "./sankeyModel";
    import {statementMinWidth} from "./statementGrid";

    // `flows` is OPTIONAL with a null default deliberately: it keeps
    // IncomeStatementView.svelte.test.ts scoped exactly as it was written, and
    // the panels carry their own tests.
    // `projectedFrom` (plans/28): the first FORECAST bucket on the Projections
    // tab, drawn as a divider under "Projected". Null — every Reports-tab
    // statement — changes nothing.
    let {
        report,
        styles,
        flows = null,
        projectedFrom = null,
    }: {report: IncomeStatementReport; styles: ReadonlyMap<string, AmountStyle>; flows?: FlowsPanel | null; projectedFrom?: number | null} = $props();

    /**
     * Which groups are open, keyed by `IsDisplayRow.key`.
     *
     * A plain `$state` record rather than a flag per group, for one reason: the
     * keys are derived from the section kind and the group NAME, so they survive
     * a refetch. Changing the date range replaces the whole report object, and
     * anything anchored to object identity would silently slam every disclosure
     * shut under the user.
     */
    let open = $state<Record<string, true>>({});

    const isExpanded = (key: string): boolean => open[key] === true;

    function toggle(key: string): void {
        if (open[key] === true) delete open[key];
        else open[key] = true;
    }

    // ONE model, built once: the template iterates `model.boxes[i].rows` and the
    // cursor indexes into the flattening of those very arrays, so a row can never
    // be reachable by `j` and absent from the screen (or the reverse).
    const model = $derived(isDisplayModel(report, isExpanded));
    const cursorable = $derived<IsDisplayRow[]>(isCursorRows(model));

    const cursor = listCursor(
        () => cursorable,
        (row) => row.key
    );

    function move(delta: number): void {
        cursor.move(delta);
        // Every row is mounted (nothing here is virtualized), so `scrollIntoView`
        // is honest. `scroll-mt-10` keeps the row clear of the sticky chrome.
        document.querySelector(`[data-is-row="${CSS.escape(String(cursor.key ?? ""))}"]`)?.scrollIntoView({block: "nearest"});
    }

    /** Enter: open a group's disclosure, or drill a real account into the journal. */
    function activate(): void {
        const row = cursor.item;
        if (row === null) return;
        if (row.kind === "group") {
            if (row.expandable) toggle(row.key);
            return;
        }
        // The report's range, not `preset: "all"`: unlike the balance sheet's
        // as-of date, a P&L's window IS the report — every figure on this screen
        // is "what happened between these dates", so a drill-down that widened to
        // the whole journal would show postings that are not in the number clicked.
        if (row.account !== null) void openJournal({accounts: [row.account], from: report.from, to: report.to});
    }

    registerKeys({
        id: "income-statement",
        priority: PRIORITY.widget,
        bindings: [
            {keys: "j", label: "Next row", group: "Reports", run: () => move(1)},
            {keys: "ArrowDown", label: "Next row", group: "Reports", run: () => move(1)},
            {keys: "k", label: "Previous row", group: "Reports", run: () => move(-1)},
            {keys: "ArrowUp", label: "Previous row", group: "Reports", run: () => move(-1)},
            {keys: "g g", label: "First row", group: "Reports", run: () => (cursor.first(), move(0))},
            {keys: "G", label: "Last row", group: "Reports", run: () => (cursor.last(), move(0))},
            {keys: "Escape", label: "Clear the cursor", group: "Reports", run: () => cursor.clear()},
            {keys: "Enter", label: "Expand a group, or show an account in the journal", group: "Reports", run: activate},
        ],
    });

    const cell = (ma: Amounts["current"] | undefined): AmountCell => amountCell(ma ?? new Map(), report.base, styles);

    /** Per-box accent. Static literals so Tailwind's scanner sees every class. */
    const ACCENT: Record<IsSectionKind, {text: string; rule: string}> = {
        revenue: {text: "text-success", rule: "border-success/40"},
        cogs: {text: "text-warning", rule: "border-warning/40"},
        opex: {text: "text-error", rule: "border-error/40"},
        depreciation: {text: "text-secondary", rule: "border-secondary/40"},
        interest: {text: "text-accent", rule: "border-accent/40"},
        tax: {text: "text-warning", rule: "border-warning/40"},
        other: {text: "text-info", rule: "border-info/40"},
    };

    // The xlsx export reads the same `isSummary` (through `isDisplayModel`), so
    // the workbook cannot disagree with the page it came from.
    const summary = $derived(model.summary);
    const comparing = $derived(model.comparing);
    const buckets = $derived(model.buckets);
    /**
     * Interval mode: every table is on the shared column template, so the
     * line-name column takes its width from the colgroup (and truncates) rather
     * than claiming the whole row with `w-full`.
     */
    const grid = $derived(buckets !== null);
    const label = $derived(grid ? "truncate" : "w-full");
    /** How many figure cells a line has — for the empty-box row's colspan. */
    const figureColumns = $derived(buckets !== null ? buckets.length + 1 : comparing ? 3 : 1);

    const valuationLabel = $derived(
        report.value === "market" ? `Market value${report.base === null ? "" : ` in ${report.base}`}` : report.value === "cost" ? "At cost" : "Unvalued"
    );

    /**
     * The kind of the first COST box this report actually prints, or null when
     * it prints none. "Money out" hangs off it rather than off `opex`, because
     * an adaptive statement may open its cost side with `cogs` instead, and a
     * diagram of the spending has to sit above the first box of it.
     */
    const COST_KINDS: readonly IsSectionKind[] = ["cogs", "opex", "depreciation", "interest", "tax"];
    const firstCost = $derived(model.boxes.find((box) => COST_KINDS.includes(box.kind))?.kind ?? null);
</script>

{#snippet amount(c: AmountCell, size: string)}
    <span class="{size} {c.negative ? 'text-error' : ''}">{c.text}</span>
    {#if c.extras.length > 0}
        <span class="block text-xs font-normal text-base-content/50">{c.extras.join(" · ")}</span>
    {/if}
{/snippet}

<!-- The figure columns of one line. Widths are PINNED rather than left to the
     table's auto layout: each box is its own `<table>`, so without fixed numeric
     columns the amounts would land in a different place in every box and the
     ruled subtotals between them would line up with nothing. -->
{#snippet figures(amounts: Amounts, pct: number | null, emphasis: string)}
    {#if buckets !== null}
        {#each amounts.values ?? [] as value, i (i)}
            <td class="w-28 min-w-28 {projectedEdge(i, projectedFrom)} text-right align-top font-mono whitespace-nowrap tabular-nums"
                >{@render amount(cell(value), "")}</td
            >
        {/each}
        <td class="w-32 min-w-32 text-right align-top font-mono whitespace-nowrap tabular-nums" data-column="total">
            {@render amount(cell(amounts.current), emphasis)}
        </td>
    {:else}
        <td class="w-32 text-right align-top font-mono whitespace-nowrap tabular-nums">{@render amount(cell(amounts.current), emphasis)}</td>
        {#if comparing}
            <td class="w-32 text-right align-top font-mono whitespace-nowrap text-base-content/60 tabular-nums">{@render amount(cell(amounts.prior), "")}</td>
            <td class="w-20 text-right align-top font-mono whitespace-nowrap text-base-content/60 tabular-nums" data-column="pct">{fmtPct(pct)}</td>
        {/if}
    {/if}
{/snippet}

<!-- The column headings for those figures, in the same order. -->
{#snippet template()}
    {#if buckets !== null}<FigureColgroup periods={buckets.length} total={true} />{/if}
{/snippet}

{#snippet figureHeads()}
    {#if buckets !== null}
        {#each buckets as bucket, i (bucket.key)}
            <th class="w-28 min-w-28 {projectedEdge(i, projectedFrom)} text-right font-normal" title="{bucket.from} to {bucket.to}">{bucket.label}</th>
        {/each}
        <th class="w-32 min-w-32 text-right font-normal">Total</th>
    {:else}
        <th class="w-32 text-right font-normal">Amount</th>
        {#if comparing}
            <th class="w-32 text-right font-normal">Prior</th>
            <th class="w-20 text-right font-normal">% change</th>
        {/if}
    {/if}
{/snippet}

<!-- One Sankey, outside the statement's boxes the way the ladder subtotals are.

     The whole async slice goes down, not a resolved graph: `SankeyPanel` owns
     its own <AsyncSection> so its header and collapse arrow exist before the
     data does. Each panel has its OWN persisted flag; one flag made both arrows
     move together. -->
{#snippet flowPanel(panel: FlowsPanel, title: string, caption: string, inbound: boolean)}
    {#snippet sankey()}
        <SankeyPanel
            {title}
            {caption}
            {panel}
            {inbound}
            {styles}
            open={inbound ? settings.flowsInOpen : settings.flowsOutOpen}
            onToggle={(next) => (inbound ? (settings.flowsInOpen = next) : (settings.flowsOutOpen = next))}
        />
    {/snippet}
    {#if grid}
        <!-- Inside the interval-mode scroller, so pinned to its left edge at the
             scroller's own width (`100cqw`): the diagram stays on screen while
             the tables under it scroll sideways. -->
        <div class="sticky left-0 w-[100cqw]">{@render sankey()}</div>
    {:else}
        {@render sankey()}
    {/if}
{/snippet}

<div class="flex flex-col gap-4" data-testid="income-statement">
    <p class="-mb-1 text-xs text-base-content/50">
        {valuationLabel} for {report.from} to {report.to}{#if report.prior !== null}, against {report.prior.from} to {report.prior.to}{/if}
    </p>

    <!-- Interval mode: ONE sideways scroller around the whole stack, held to one
         width, so every block keeps the same column edges and they all scroll
         together. Total mode: both wrappers are `contents`, so the boxes stay
         direct flex children and the page is exactly as it was. -->
    <div class={grid ? "@container overflow-x-auto" : "contents"} data-testid="is-scroller">
        <div class={grid ? "flex flex-col gap-4" : "contents"} style:min-width={buckets !== null ? statementMinWidth(buckets.length, true) : undefined}>
            {#each model.boxes as box (box.kind)}
                {#if flows !== null && box.kind === "revenue"}
                    {@render flowPanel(flows, "Money in", "Where the period's income came from, and which accounts it landed in.", true)}
                {/if}
                {#if flows !== null && box.kind === firstCost}
                    {@render flowPanel(flows, "Money out", "Which accounts paid, and what the period spent it on.", false)}
                {/if}
                <!-- Total mode scrolls sideways rather than clipping; in interval mode the
             shared scroller around the stack does that. -->
                <section class="{grid ? 'overflow-hidden' : 'overflow-x-auto'} rounded-box border border-base-content/10" data-testid="is-section-{box.kind}">
                    <h3
                        class="bg-base-200 {ACCENT[box.kind].rule} border-b-2 px-4 py-2.5 text-sm font-semibold tracking-wide uppercase {ACCENT[box.kind].text}"
                    >
                        {box.title}
                    </h3>
                    <table class="table table-sm {grid ? 'table-fixed' : ''}">
                        {@render template()}
                        <!-- Repeated per box rather than printed once above them: each box
                     is a separate table, so a single heading strip could not stay
                     aligned with the columns under it, and a table with no column
                     headers is a table a screen reader cannot narrate. -->
                        <thead>
                            <ProjectedHeadRow columns={buckets?.length ?? 0} {projectedFrom} trailing={1} fixed={grid} />
                            <tr class="text-[0.65rem] text-base-content/40">
                                <th class="{label} font-normal"><span class="sr-only">{box.title} line</span></th>
                                {@render figureHeads()}
                            </tr>
                        </thead>
                        <tbody>
                            {#each box.rows as row (row.key)}
                                <tr
                                    class="scroll-mt-10 {cursor.key === row.key ? 'bg-primary/25' : ''} {row.kind === 'group' ? 'font-medium' : ''}"
                                    aria-current={cursor.key === row.key ? "true" : undefined}
                                    data-is-row={row.key}
                                    data-account={row.account ?? undefined}
                                >
                                    <th class="{label} font-normal">
                                        {#if row.kind === "group"}
                                            {#if row.expandable}
                                                <button
                                                    type="button"
                                                    class="flex cursor-pointer items-center gap-1.5 text-left font-medium hover:text-primary"
                                                    aria-expanded={row.expanded}
                                                    onclick={() => toggle(row.key)}
                                                >
                                                    <span
                                                        class="inline-block w-3 shrink-0 text-[0.65rem] text-base-content/40 transition-transform {row.expanded
                                                            ? 'rotate-90'
                                                            : ''}"
                                                        aria-hidden="true">▶</span
                                                    >
                                                    {row.label}
                                                </button>
                                            {:else}
                                                <!-- A group the engine sent no accounts for. The spacer keeps its
                                             label on the same left edge as the groups that do. -->
                                                <span class="flex items-center gap-1.5 font-medium">
                                                    <span class="inline-block w-3 shrink-0" aria-hidden="true"></span>{row.label}
                                                </span>
                                            {/if}
                                        {:else}
                                            <!-- Inline padding, not a class: the depth is data, and `ReportTable`
                                         indents its own rows the same way (1rem per level). -->
                                            <span class="whitespace-nowrap text-base-content/70" style="padding-left: {row.indent}rem" title={row.account}
                                                >{row.label}</span
                                            >
                                        {/if}
                                    </th>
                                    {@render figures(row.amounts, row.pct, row.kind === "group" ? "font-medium" : "text-base-content/70")}
                                </tr>
                            {/each}
                            {#if box.rows.length === 0}
                                <tr>
                                    <th class="{label} font-normal text-base-content/50">No {box.title.toLowerCase()}</th>
                                    <td colspan={figureColumns}></td>
                                </tr>
                            {/if}
                        </tbody>
                        <tfoot>
                            <tr class="border-t-2 border-base-content/20 bg-base-200 text-sm font-bold text-base-content">
                                <th class="{label} font-bold">Total {box.title}</th>
                                {@render figures(box.total, box.totalPct, "font-bold")}
                            </tr>
                        </tfoot>
                    </table>
                </section>

                <!-- The ladder. Between the boxes, ruled top and bottom, and outside every
             box's border precisely so it cannot be read as part of one. In
             interval mode it is its own block on the shared template, so it
             names its columns (and the projected band) like every box does:
             a row of bare figures is not readable on its own. -->
                {#each box.trailing as subtotal (subtotal.kind)}
                    <div
                        class="-my-1 {grid ? 'border-x border-x-transparent' : 'overflow-x-auto'} border-y-2 border-base-content/30"
                        data-testid="is-subtotal-{subtotal.kind}"
                    >
                        <table class="table table-sm {grid ? 'table-fixed' : ''}">
                            {@render template()}
                            {#if grid}
                                <thead>
                                    <ProjectedHeadRow columns={buckets?.length ?? 0} {projectedFrom} trailing={1} fixed />
                                    <tr class="text-[0.65rem] text-base-content/40">
                                        <th class="{label} font-normal"><span class="sr-only">{subtotal.label} line</span></th>
                                        {@render figureHeads()}
                                    </tr>
                                </thead>
                            {/if}
                            <tbody>
                                <tr>
                                    <th class="{label} text-sm font-semibold">{subtotal.label}</th>
                                    {@render figures(subtotal.amounts, subtotal.pct, "font-semibold")}
                                </tr>
                            </tbody>
                        </table>
                    </div>
                {/each}
            {/each}

            <!-- The summary: net income, and nothing else.

         A condensed "Total Revenue / Less: Cost of revenue / …" table stood here
         and has been removed. It was modelled on the balance sheet's tie-out,
         but the two are not analogous: that tie-out PROVES `A = L + E`, which a
         reader cannot otherwise check, while this one restated seven section
         totals that are each already in a box footer directly above it, with
         every intermediate figure already a rung of the ladder. Seven duplicated
         totals is the exact complaint this redesign exists to fix.

         What is left is the one figure that appears nowhere else on the page.

         In interval mode it is a ruled ROW instead, aligned under the bucket
         columns like the ladder lines: net income per month is the one figure a
         reader most wants there, and a single Total would hide it. -->
            {#if buckets !== null}
                <div class="overflow-hidden rounded-box border border-base-content/20 bg-base-200" data-testid="is-net-income">
                    <table class="table table-fixed table-sm">
                        {@render template()}
                        <thead>
                            <ProjectedHeadRow columns={buckets.length} {projectedFrom} trailing={1} fixed />
                            <tr class="text-[0.65rem] text-base-content/40">
                                <th class="{label} font-normal"><span class="sr-only">Net income</span></th>
                                {@render figureHeads()}
                            </tr>
                        </thead>
                        <tbody>
                            <tr>
                                <th class="{label} text-sm font-semibold tracking-wide uppercase">
                                    Net income
                                    <span class="ml-1 text-xs font-normal tracking-normal text-base-content/50 normal-case">(revenue − expenses)</span>
                                </th>
                                {@render figures(summary.netIncome, null, "font-semibold")}
                            </tr>
                        </tbody>
                    </table>
                </div>
            {:else}
                <div
                    class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 rounded-box border border-base-content/20 bg-base-200 px-4 py-3"
                    data-testid="is-net-income"
                >
                    <span class="text-sm font-semibold tracking-wide uppercase">
                        Net income
                        <span class="ml-1 text-xs font-normal tracking-normal text-base-content/50 normal-case">(revenue − expenses)</span>
                    </span>
                    <!-- Current, THEN prior, then the percentage — the same left-to-right order
             as the Amount / Prior / % change columns in every box above, and
             the order `setIsAmounts` writes to the workbook. This panel is the one
             place on the page where the figures carry no column header, so it is
             the one place a reader has nothing but the established order to go on:
             rendered prior-first it read as though net income were $14,880.79 when
             the period actually earned $8,883.52. -->
                    <span class="flex flex-wrap items-baseline justify-end gap-x-4">
                        <span class="text-right font-mono text-xl font-semibold tabular-nums">
                            {@render amount(cell(summary.netIncome.current), "")}
                        </span>
                        {#if comparing}
                            <span class="text-right font-mono text-base-content/60 tabular-nums" data-testid="is-net-income-prior">
                                {@render amount(cell(summary.netIncome.prior), "")}
                            </span>
                            <span class="w-20 text-right font-mono text-base-content/60 tabular-nums" data-testid="is-net-income-pct"
                                >{fmtPct(summary.netPct)}</span
                            >
                        {/if}
                    </span>
                </div>
            {/if}
        </div>
    </div>
</div>
