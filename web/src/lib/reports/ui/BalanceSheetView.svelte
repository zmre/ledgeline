<!-- The grouped balance sheet (plans/12): three visually separate boxes —
     Assets, Liabilities, Equity — instead of one long `hledger bs` lookalike.

     Every line is ONE number, because the engine values the whole report into a
     single base commodity. Anything it could not convert (a holding with no `P`
     directive) is demoted to a small secondary line via the `fmtBase` + `extras`
     pattern the insights dashboard uses, rather than stacked as a second
     balance in the same cell — which is what the old `formatTotals` `<div>`s did
     and what made this table unreadable.

     Groups are COLLAPSED by default: the useful reading of a balance sheet is
     "cash, investments, credit cards", and the account detail is a drill-down.
     Expanding one shows its depth-clamped accounts, single-child chains
     compressed exactly as every other report table compresses them.

     Assets and Liabilities may be banded CURRENT / NON-CURRENT, each band with
     its own subheading and subtotal. It is adaptive, like the P&L's GAAP ladder:
     a journal that tags nothing gets no bands at all rather than two headings
     and an empty half. Equity is never banded. The prose in a subheading and in
     a band's subtotal label is the ENGINE's — deriving "Total non-current
     assets" from a term and a title here would put that mapping in this file and
     in the xlsx builder both.

     Keyboard: j/k walk every VISIBLE GROUP AND ACCOUNT row — group headings
     included, so `j` does something on first load rather than nothing — and Enter
     either opens the disclosure (on a group) or drills into the journal (on an
     account). Band rows are skipped: there is nothing to expand or drill on a
     heading or a subtotal, so stopping there would be a stop that does nothing.

     Under the boxes is the spreadsheet tie-out: the three section totals, then
     `Liabilities + equity` set against `Total assets`, which is where the ✓/✗
     hangs. Nothing follows it: net worth is identically Total equity, so a
     net-worth line under the statement printed the same number twice under two
     names, and the Net Worth tab is where that figure is the subject.

     The balance check is shown only when it is non-zero, and then as a warning:
     the engine computes it from exact `Dec` values, so a non-zero check is a
     real journal-integrity failure and never a rounding artefact.

     COLUMNS (plans/28). The same view renders a multi-column sheet — the
     Projections tab's balance sheet at the end of every period, and the Reports
     tab's multi-period mode — from the engine's pivoted `BalanceSheetColumns`. A
     single-date report is turned into a ONE-column one (`asColumns`), so there
     is one template, and the Reports tab's single date reads exactly as before:
     one "Amount" column. `projectedFrom` marks where actuals end.

     A multi-column sheet puts every box and the tie-out on ONE column template
     (statementGrid.ts) inside one horizontal scroller, so their period columns
     line up and scroll together. That is also why the tie-out's verdict moves
     into its label cell there: an extra verdict column on the tie-out alone
     would shift its periods away from the boxes above it. -->
<script lang="ts">
    import type {MixedAmount} from "#lib/domain/money.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {openJournal} from "#lib/journal/openJournal.js";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import type {BalanceSheetColumns, BalanceSheetReport, BsSectionKind} from "#lib/reports/types.js";
    import {listCursor} from "#lib/ui/listCursor.svelte.js";
    import {
        amountCell,
        asColumns,
        bsColumnSummaries,
        bsCursorRows,
        columnSectionRows,
        type AmountCell,
        type BsDisplayRow,
        type BsSummary,
    } from "./balanceSheetRows";
    import FigureColgroup from "./FigureColgroup.svelte";
    import ProjectedHeadRow from "./ProjectedHeadRow.svelte";
    import {projectedEdge} from "./projectedColumns";
    import {statementMinWidth} from "./statementGrid";

    let {
        report,
        styles,
        projectedFrom = null,
    }: {
        /** A single-date sheet (the Reports tab) or a multi-column one (Projections). */
        report: BalanceSheetReport | BalanceSheetColumns;
        styles: ReadonlyMap<string, AmountStyle>;
        /** The first FORECAST column, drawn as a divider under "Projected"; null for none. */
        projectedFrom?: number | null;
    } = $props();

    /** Every sheet as columns: one template, whichever shape arrived. */
    const sheet = $derived<BalanceSheetColumns>(report.kind === "balanceSheet" ? asColumns(report) : report);
    const multi = $derived(report.kind === "balanceSheetColumns");

    /**
     * Which groups are open, keyed by `BsDisplayRow.key`.
     *
     * A plain `$state` record rather than a component-level `expanded` flag per
     * group, for one reason: the keys are derived from the section kind and the
     * group NAME, so they survive a refetch. Changing the depth or the as-of
     * date replaces the whole report object, and anything anchored to object
     * identity would silently slam every disclosure shut under the user.
     */
    let open = $state<Record<string, true>>({});

    const isExpanded = (key: string): boolean => open[key] === true;

    function toggle(key: string): void {
        if (open[key] === true) delete open[key];
        else open[key] = true;
    }

    // Hoisted out of the `{#each}` into one list per section, as ReportTable
    // does: the cursor indexes into a FILTERING of these and the template
    // iterates the very same arrays, so a row can never be reachable by `j` and
    // absent from the screen. The filter drops the band rows only — see
    // `bsCursorRows`.
    const sectionRows = $derived(sheet.sections.map((section) => columnSectionRows(section, isExpanded)));
    const cursorable = $derived<BsDisplayRow<MixedAmount[]>[]>(bsCursorRows(sectionRows.flat()));

    const cursor = listCursor(
        () => cursorable,
        (row) => row.key
    );

    function move(delta: number): void {
        cursor.move(delta);
        // Every row is mounted (nothing here is virtualized), so `scrollIntoView`
        // is honest. `scroll-mt-10` keeps the row clear of the sticky chrome.
        document.querySelector(`[data-bs-row="${CSS.escape(String(cursor.key ?? ""))}"]`)?.scrollIntoView({block: "nearest"});
    }

    /** Enter: open a group's disclosure, or drill a real account into the journal. */
    function activate(): void {
        const row = cursor.item;
        if (row === null) return;
        if (row.kind === "group") {
            if (row.expandable) toggle(row.key);
            return;
        }
        // `preset: "all"`: the balance sheet's as-of date lives in its controls,
        // not in the row, so narrowing the journal to dates the user cannot see
        // here would read as data loss.
        if (row.account !== null) void openJournal({accounts: [row.account], preset: "all"});
    }

    registerKeys({
        id: "balance-sheet",
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

    const cell = (ma: MixedAmount | undefined): AmountCell => amountCell(ma ?? new Map(), sheet.base, styles);
    /** The widths every box pins its figure columns to, so the boxes line up. */
    const figureWidth = $derived(multi ? "w-28 min-w-28" : "");
    /** Multi-column: the line-name column takes its width from the shared colgroup. */
    const label = $derived(multi ? "truncate" : "w-full");
    /** The verdict gets its own column only on a single-date sheet (see the header comment). */
    const verdictColumn = $derived(!multi);

    /** Per-box accent. Static literals so Tailwind's scanner sees every class. */
    const ACCENT: Record<BsSectionKind, {text: string; rule: string}> = {
        assets: {text: "text-success", rule: "border-success/40"},
        liabilities: {text: "text-warning", rule: "border-warning/40"},
        equity: {text: "text-info", rule: "border-info/40"},
    };

    // `liabilitiesPlusEquity` is summed from the exact Decs, never from the
    // rendered strings, and `balanced` is the engine's own verdict rather than
    // anything decided here — see `bsSummary`. The xlsx export reads the same
    // function, so the workbook cannot disagree with the page it came from.
    const summaries = $derived(bsColumnSummaries(sheet));
    const allBalanced = $derived(summaries.every((summary) => summary.balanced));
    /** The first column that does not balance — the one the warning names. */
    const unbalanced = $derived(summaries.findIndex((summary) => !summary.balanced));
    const check = $derived(cell(sheet.check[Math.max(unbalanced, 0)]));

    const valuationLabel = $derived(
        sheet.value === "market" ? `Market value${sheet.base === null ? "" : ` in ${sheet.base}`}` : sheet.value === "cost" ? "At cost" : "Unvalued"
    );
    const asOfLabel = $derived(report.kind === "balanceSheet" ? `as of ${report.asOf}` : "at the end of each period");
</script>

<!-- Side by side, each unconverted commodity takes its own line rather than
     one `·`-joined run: a run of three is wider than a period column, and the
     boxes' columns then stop lining up with each other. -->
{#snippet amount(c: AmountCell, size: string)}
    <span class="{size} {c.negative ? 'text-error' : ''}">{c.text}</span>
    {#if c.extras.length > 0}
        {#if multi}
            {#each c.extras as extra (extra)}
                <span class="block text-xs font-normal text-base-content/50">{extra}</span>
            {/each}
        {:else}
            <span class="block text-xs font-normal text-base-content/50">{c.extras.join(" · ")}</span>
        {/if}
    {/if}
{/snippet}

<!-- One figure per column, divider and all. -->
{#snippet figures(values: readonly MixedAmount[], emphasis: string)}
    {#each sheet.columns as column, at (column.key)}
        <td
            class="{figureWidth} {projectedEdge(at, projectedFrom)} text-right align-top font-mono whitespace-nowrap tabular-nums"
            data-column={multi ? column.key : undefined}
        >
            {@render amount(cell(values[at]), emphasis)}
        </td>
    {/each}
{/snippet}

<!-- The column headings: "Amount" on a single-date sheet, each period on a
     multi-column one, under the actual/projected band when there is one. -->
{#snippet template()}
    {#if multi}<FigureColgroup periods={sheet.columns.length} total={false} />{/if}
{/snippet}

{#snippet figureHeads(name: string, verdict: boolean)}
    <ProjectedHeadRow columns={sheet.columns.length} {projectedFrom} trailing={verdict ? 1 : 0} fixed={multi} />
    <tr class="text-[0.65rem] text-base-content/40">
        <th class="{label} font-normal"><span class="sr-only">{name}</span></th>
        {#each sheet.columns as column, at (column.key)}
            <th class="{figureWidth} {projectedEdge(at, projectedFrom)} text-right font-normal" title={multi ? `as of ${column.to}` : undefined}
                >{column.label}</th
            >
        {/each}
        {#if verdict}
            <th class="font-normal"><span class="sr-only">Balance check</span></th>
        {/if}
    </tr>
{/snippet}

<!-- One label/figure line of the tie-out. The last cell exists on every row so
     the verdict on the last one does not widen the column under the others. -->
{#snippet tieRow(name: string, pick: (summary: BsSummary) => MixedAmount, emphasis: string)}
    <tr>
        <th class="{label} font-normal">{name}</th>
        {@render figures(
            summaries.map((summary) => pick(summary)),
            emphasis
        )}
        {#if verdictColumn}<td></td>{/if}
    </tr>
{/snippet}

{#snippet verdict(size: string)}
    <span class="{size} font-semibold whitespace-nowrap {allBalanced ? 'text-success' : 'text-warning'}" data-testid="bs-verdict">
        <span aria-hidden="true">{allBalanced ? "✓" : "✗"}</span>
        {allBalanced ? "Balanced" : "Out of balance"}
    </span>
{/snippet}

<div class="flex flex-col gap-4" data-testid="balance-sheet">
    <p class="-mb-1 text-xs text-base-content/50">{valuationLabel} {asOfLabel}</p>

    <!-- Multi-column: one sideways scroller, one width, so every block keeps the
         same column edges. Single-date: `contents`, exactly as before. -->
    <div class={multi ? "overflow-x-auto" : "contents"} data-testid="bs-scroller">
        <div class={multi ? "flex flex-col gap-4" : "contents"} style:min-width={multi ? statementMinWidth(sheet.columns.length, false) : undefined}>
            {#each sheet.sections as section, at (section.kind)}
                <section class="overflow-hidden rounded-box border border-base-content/10" data-testid="bs-section-{section.kind}">
                    <h3
                        class="bg-base-200 {ACCENT[section.kind].rule} border-b-2 px-4 py-2.5 text-sm font-semibold tracking-wide uppercase {ACCENT[
                            section.kind
                        ].text}"
                    >
                        {section.title}
                    </h3>
                    <table class="table table-sm {multi ? 'table-fixed' : ''}">
                        {@render template()}
                        <!-- Repeated per box rather than printed once above them, exactly
                     as the income statement's boxes do it: each box is a separate
                     table, so a single heading strip could not stay aligned with
                     the columns under it, and a table with no column headers is a
                     table a screen reader cannot narrate. One "Amount" column
                     always — the report is valued into one base commodity, and
                     whatever could not convert is a footnote INSIDE that cell,
                     never a column of its own. -->
                        <thead>
                            {@render figureHeads(`${section.title} line`, false)}
                        </thead>
                        <tbody>
                            {#each sectionRows[at] ?? [] as row (row.key)}
                                {#if row.kind === "subsection"}
                                    <!-- A band's subheading. Deliberately between the two
                                 registers around it: uppercase and letter-spaced like
                                 the box header so it reads as structure rather than as
                                 a line item, but smaller, unaccented and only tinted —
                                 the box header is the louder of the two, and a group
                                 line is plainly quieter than this.

                                 It also sits a step to the LEFT of the groups under it,
                                 without any padding of its own: a group line opens with
                                 the disclosure column, and this one does not. That is
                                 why nothing here indents the groups — the offset the
                                 hierarchy needs is already on the screen, and adding to
                                 it would push every account row further right for it.

                                 No figure: the band's total belongs to the subtotal
                                 that closes it, and printing it twice would invite the
                                 reader to look for a difference between them. -->
                                    <tr class="bg-base-200/40" data-bs-row={row.key} data-testid="bs-subsection-{section.kind}-{row.term}">
                                        <th class="{label} pt-3 text-[0.7rem] font-semibold tracking-widest text-base-content/60 uppercase">{row.label}</th>
                                        <td colspan={sheet.columns.length}></td>
                                    </tr>
                                {:else if row.kind === "subtotal"}
                                    <!-- A band's subtotal. A thin rule and semibold, against the
                                 section total's double rule, fill and bold weight below
                                 — the two must not be mistakable, because this one is a
                                 part and that one is the whole. -->
                                    <tr
                                        class="border-t border-base-content/20 text-sm"
                                        data-bs-row={row.key}
                                        data-testid="bs-subtotal-{section.kind}-{row.term}"
                                    >
                                        <th class="{label} font-semibold text-base-content/80">{row.label}</th>
                                        {@render figures(row.amount, "font-semibold")}
                                    </tr>
                                {:else}
                                    <tr
                                        class="scroll-mt-10 {cursor.key === row.key ? 'bg-primary/25' : ''} {row.kind === 'group' ? 'font-medium' : ''}"
                                        aria-current={cursor.key === row.key ? "true" : undefined}
                                        data-bs-row={row.key}
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
                                                    <!-- A computed group (Retained earnings, Valuation adjustment) has no
                                                 accounts to open. The spacer keeps its label on the same
                                                 left edge as the groups that do. -->
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
                                        {@render figures(row.amount, row.kind === "group" ? "font-medium" : "text-base-content/70")}
                                    </tr>
                                {/if}
                            {/each}
                            {#if section.groups.length === 0}
                                <tr>
                                    <th class="{label} font-normal text-base-content/50">No {section.title.toLowerCase()}</th>
                                    <td colspan={sheet.columns.length}></td>
                                </tr>
                            {/if}
                        </tbody>
                        <tfoot>
                            <tr class="border-t-2 border-base-content/20 bg-base-200 text-sm font-bold text-base-content">
                                <th class="{label} font-bold">Total {section.title}</th>
                                {@render figures(section.total, "font-bold")}
                            </tr>
                        </tfoot>
                    </table>
                </section>
            {/each}

            <!-- The tie-out. `Liabilities + equity` against `Total assets` is the check a
         reader of a balance sheet actually performs, so that pair — not net
         worth — is what carries the verdict. -->
            <!-- A `div`, not a `section`: the three boxes above are named landmarks by
         their `h3`, and an unnamed one here would just be noise to a screen
         reader. The table carries the name instead. -->
            <div class="overflow-hidden rounded-box border border-base-content/20" data-testid="bs-summary">
                <table class="table table-sm {multi ? 'table-fixed' : ''}" aria-label="Balance sheet totals">
                    {@render template()}
                    <!-- The boxes' header treatment again, plus a name for the last
                 column: the verdict cell exists on EVERY row (see `tieRow`), so
                 to a screen reader it is a real column, and a real column with
                 no name is the omission this thead exists to prevent. -->
                    <thead>
                        {@render figureHeads("Total", verdictColumn)}
                    </thead>
                    <tbody>
                        {@render tieRow("Total Assets", (summary) => summary.assets, "")}
                        {@render tieRow("Total Liabilities", (summary) => summary.liabilities, "")}
                        {@render tieRow("Total Equity", (summary) => summary.equity, "")}
                    </tbody>
                    <tfoot class="text-sm text-base-content">
                        <tr class="border-t-2 border-base-content/20">
                            <th class="{label} font-semibold">Liabilities + Equity</th>
                            {@render figures(
                                summaries.map((summary) => summary.liabilitiesPlusEquity),
                                "font-semibold"
                            )}
                            {#if verdictColumn}<td></td>{/if}
                        </tr>
                        <tr data-testid="bs-tie-out">
                            <th class="{label} font-semibold">
                                Total Assets
                                {#if !verdictColumn}{@render verdict("ml-2 text-xs")}{/if}
                            </th>
                            {@render figures(
                                summaries.map((summary) => summary.assets),
                                "font-semibold"
                            )}
                            <!-- One verdict for the whole row: every column balances, or the
                         warning below names the first that does not. -->
                            {#if verdictColumn}
                                <td class="align-top text-sm">{@render verdict("")}</td>
                            {/if}
                        </tr>
                    </tfoot>
                </table>
            </div>
        </div>
    </div>

    {#if !allBalanced}
        <div class="alert rounded-box px-3 py-2 text-sm alert-warning" role="alert" data-testid="bs-check">
            <span>
                {#if multi}At {sheet.columns[unbalanced]?.label ?? "one column"}, this{:else}This{/if} journal doesn't balance: assets − liabilities − equity should
                be zero, but it is
                <span class="font-mono tabular-nums">{check.text}{check.extras.length > 0 ? ` (${check.extras.join(", ")})` : ""}</span>. Look for a transaction
                whose postings don't sum to zero.
            </span>
        </div>
    {/if}
</div>
