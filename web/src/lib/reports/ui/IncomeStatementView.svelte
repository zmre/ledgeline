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

     Keyboard: j/k walk every VISIBLE row — group headings included, so `j` does
     something on first load rather than nothing — and Enter either opens the
     disclosure (on a group) or drills into the journal (on an account). -->
<script lang="ts">
    import type {AmountStyle} from "$lib/domain/types";
    import {openJournal} from "$lib/journal/openJournal";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import type {Amounts, IncomeStatementReport, IsSectionKind} from "$lib/reports/types";
    import {listCursor} from "$lib/ui/listCursor.svelte";
    import {amountCell, fmtPct, isCursorRows, isDisplayModel, type AmountCell, type IsDisplayRow} from "./incomeStatementRows";

    let {report, styles}: {report: IncomeStatementReport; styles: ReadonlyMap<string, AmountStyle>} = $props();

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

    const valuationLabel = $derived(
        report.value === "market" ? `Market value${report.base === null ? "" : ` in ${report.base}`}` : report.value === "cost" ? "At cost" : "Unvalued"
    );
</script>

{#snippet amount(c: AmountCell, size: string)}
    <span class="{size} {c.negative ? 'text-error' : ''}">{c.text}</span>
    {#if c.extras.length > 0}
        <span class="text-base-content/50 block text-xs font-normal">{c.extras.join(" · ")}</span>
    {/if}
{/snippet}

<!-- The three figure columns of one line. Widths are PINNED rather than left to
     the table's auto layout: each box is its own `<table>`, so without fixed
     numeric columns the amounts would land in a different place in every box and
     the ruled subtotals between them would line up with nothing. -->
{#snippet figures(amounts: Amounts, pct: number | null, emphasis: string)}
    <td class="w-32 text-right align-top font-mono whitespace-nowrap tabular-nums">{@render amount(cell(amounts.current), emphasis)}</td>
    {#if comparing}
        <td class="text-base-content/60 w-32 text-right align-top font-mono whitespace-nowrap tabular-nums">{@render amount(cell(amounts.prior), "")}</td>
    {/if}
    <td class="text-base-content/60 w-20 text-right align-top font-mono whitespace-nowrap tabular-nums">{fmtPct(pct)}</td>
{/snippet}

<div class="flex flex-col gap-4" data-testid="income-statement">
    <p class="text-base-content/50 -mb-1 text-xs">
        {valuationLabel} for {report.from} to {report.to}{#if report.prior !== null}, against {report.prior.from} to {report.prior.to}{/if}
    </p>

    {#each model.boxes as box (box.kind)}
        <section class="border-base-content/10 rounded-box overflow-hidden border" data-testid="is-section-{box.kind}">
            <h3 class="bg-base-200 {ACCENT[box.kind].rule} border-b-2 px-4 py-2.5 text-sm font-semibold tracking-wide uppercase {ACCENT[box.kind].text}">
                {box.title}
            </h3>
            <table class="table-sm table">
                <!-- Repeated per box rather than printed once above them: each box
                     is a separate table, so a single heading strip could not stay
                     aligned with the columns under it, and a table with no column
                     headers is a table a screen reader cannot narrate. -->
                <thead>
                    <tr class="text-base-content/40 text-[0.65rem]">
                        <th class="w-full font-normal"><span class="sr-only">{box.title} line</span></th>
                        <th class="w-32 text-right font-normal">Amount</th>
                        {#if comparing}<th class="w-32 text-right font-normal">Prior</th>{/if}
                        <th class="w-20 text-right font-normal">% of revenue</th>
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
                            <th class="w-full font-normal">
                                {#if row.kind === "group"}
                                    {#if row.expandable}
                                        <button
                                            type="button"
                                            class="hover:text-primary flex cursor-pointer items-center gap-1.5 text-left font-medium"
                                            aria-expanded={row.expanded}
                                            onclick={() => toggle(row.key)}
                                        >
                                            <span
                                                class="text-base-content/40 inline-block w-3 shrink-0 text-[0.65rem] transition-transform {row.expanded
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
                                    <span class="text-base-content/70 whitespace-nowrap" style="padding-left: {row.indent}rem" title={row.account}
                                        >{row.label}</span
                                    >
                                {/if}
                            </th>
                            {@render figures(row.amounts, row.pct, row.kind === "group" ? "font-medium" : "text-base-content/70")}
                        </tr>
                    {/each}
                    {#if box.rows.length === 0}
                        <tr>
                            <th class="text-base-content/50 w-full font-normal">No {box.title.toLowerCase()}</th>
                            <td colspan={comparing ? 3 : 2}></td>
                        </tr>
                    {/if}
                </tbody>
                <tfoot>
                    <tr class="border-base-content/20 bg-base-200 text-base-content border-t-2 text-sm font-bold">
                        <th class="w-full font-bold">Total {box.title}</th>
                        {@render figures(box.total, box.totalPct, "font-bold")}
                    </tr>
                </tfoot>
            </table>
        </section>

        <!-- The ladder. Between the boxes, ruled top and bottom, and outside every
             box's border precisely so it cannot be read as part of one. -->
        {#each box.trailing as subtotal (subtotal.kind)}
            <div class="border-base-content/30 -my-1 border-y-2" data-testid="is-subtotal-{subtotal.kind}">
                <table class="table-sm table">
                    <tbody>
                        <tr>
                            <th class="w-full text-sm font-semibold">{subtotal.label}</th>
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

         What is left is the one figure that appears nowhere else on the page. -->
    <div
        class="border-base-content/20 rounded-box bg-base-200 flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 border px-4 py-3"
        data-testid="is-net-income"
    >
        <span class="text-sm font-semibold tracking-wide uppercase">
            Net income
            <span class="text-base-content/50 ml-1 text-xs font-normal tracking-normal normal-case">(revenue − expenses)</span>
        </span>
        <!-- Current, THEN prior, then the percentage — the same left-to-right order
             as the Amount / Prior / % of revenue columns in every box above, and
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
                <span class="text-base-content/60 text-right font-mono tabular-nums" data-testid="is-net-income-prior">
                    {@render amount(cell(summary.netIncome.prior), "")}
                </span>
            {/if}
            <span class="text-base-content/60 w-20 text-right font-mono tabular-nums">{fmtPct(summary.netPct)}</span>
        </span>
    </div>
</div>
