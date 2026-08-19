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

     Keyboard: j/k walk every VISIBLE row — group headings included, so `j` does
     something on first load rather than nothing — and Enter either opens the
     disclosure (on a group) or drills into the journal (on an account).

     Under the boxes is the spreadsheet tie-out: the three section totals, then
     `Liabilities + equity` set against `Total assets`, which is where the ✓/✗
     hangs. Net worth follows as its own prominent figure rather than as the
     statement's bottom line — it is identically Total equity, so closing on it
     printed the same number twice under two names and proved nothing.

     The balance check is shown only when it is non-zero, and then as a warning:
     the engine computes it from exact `Dec` values, so a non-zero check is a
     real journal-integrity failure and never a rounding artefact. -->
<script lang="ts">
    import type {MixedAmount} from "$lib/domain/money";
    import type {AmountStyle} from "$lib/domain/types";
    import {openJournal} from "$lib/journal/openJournal";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import type {BalanceSheetReport, BsSectionKind} from "$lib/reports/types";
    import {listCursor} from "$lib/ui/listCursor.svelte";
    import {amountCell, bsSummary, sectionDisplayRows, type AmountCell, type BsDisplayRow} from "./balanceSheetRows";

    let {report, styles}: {report: BalanceSheetReport; styles: ReadonlyMap<string, AmountStyle>} = $props();

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
    // does: the cursor indexes into the FLATTENING of these and the template
    // iterates the very same arrays, so a row can never be reachable by `j` and
    // absent from the screen (or the reverse).
    const sectionRows = $derived(report.sections.map((section) => sectionDisplayRows(section, isExpanded)));
    const cursorable = $derived<BsDisplayRow[]>(sectionRows.flat());

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

    const cell = (row: {amount: BsDisplayRow["amount"]}): AmountCell => amountCell(row.amount, report.base, styles);

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
    const summary = $derived(bsSummary(report));
    const check = $derived(amountCell(report.check, report.base, styles));

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

<!-- One label/figure line of the tie-out. The third cell exists on every row so
     the verdict on the last one does not widen the column under the others. -->
{#snippet tieRow(label: string, ma: MixedAmount, emphasis: string)}
    <tr>
        <th class="w-full font-normal">{label}</th>
        <td class="text-right align-top font-mono whitespace-nowrap tabular-nums">{@render amount(amountCell(ma, report.base, styles), emphasis)}</td>
        <td></td>
    </tr>
{/snippet}

<div class="flex flex-col gap-4" data-testid="balance-sheet">
    <p class="text-base-content/50 -mb-1 text-xs">{valuationLabel} as of {report.asOf}</p>

    {#each report.sections as section, at (section.kind)}
        <section class="border-base-content/10 rounded-box overflow-hidden border" data-testid="bs-section-{section.kind}">
            <h3
                class="bg-base-200 {ACCENT[section.kind].rule} border-b-2 px-4 py-2.5 text-sm font-semibold tracking-wide uppercase {ACCENT[section.kind].text}"
            >
                {section.title}
            </h3>
            <table class="table-sm table">
                <tbody>
                    {#each sectionRows[at] ?? [] as row (row.key)}
                        <tr
                            class="scroll-mt-10 {cursor.key === row.key ? 'bg-primary/25' : ''} {row.kind === 'group' ? 'font-medium' : ''}"
                            aria-current={cursor.key === row.key ? "true" : undefined}
                            data-bs-row={row.key}
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
                                    <span class="text-base-content/70 whitespace-nowrap" style="padding-left: {row.indent}rem" title={row.account}
                                        >{row.label}</span
                                    >
                                {/if}
                            </th>
                            <td class="text-right align-top font-mono whitespace-nowrap tabular-nums">
                                {@render amount(cell(row), row.kind === "group" ? "font-medium" : "text-base-content/70")}
                            </td>
                        </tr>
                    {/each}
                    {#if section.groups.length === 0}
                        <tr>
                            <th class="text-base-content/50 w-full font-normal">No {section.title.toLowerCase()}</th>
                            <td></td>
                        </tr>
                    {/if}
                </tbody>
                <tfoot>
                    <tr class="border-base-content/20 bg-base-200 text-base-content border-t-2 text-sm font-bold">
                        <th class="w-full font-bold">Total {section.title}</th>
                        <td class="text-right align-top font-mono whitespace-nowrap tabular-nums">
                            {@render amount(amountCell(section.total, report.base, styles), "font-bold")}
                        </td>
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
    <div class="border-base-content/20 rounded-box overflow-hidden border" data-testid="bs-summary">
        <table class="table-sm table" aria-label="Balance sheet totals">
            <tbody>
                {@render tieRow("Total Assets", summary.assets, "")}
                {@render tieRow("Total Liabilities", summary.liabilities, "")}
                {@render tieRow("Total Equity", summary.equity, "")}
            </tbody>
            <tfoot class="text-base-content text-sm">
                <tr class="border-base-content/20 border-t-2">
                    <th class="w-full font-semibold">Liabilities + Equity</th>
                    <td class="text-right align-top font-mono whitespace-nowrap tabular-nums">
                        {@render amount(amountCell(summary.liabilitiesPlusEquity, report.base, styles), "font-semibold")}
                    </td>
                    <td></td>
                </tr>
                <tr data-testid="bs-tie-out">
                    <th class="w-full font-semibold">Total Assets</th>
                    <td class="text-right align-top font-mono whitespace-nowrap tabular-nums">
                        {@render amount(amountCell(summary.assets, report.base, styles), "font-semibold")}
                    </td>
                    <td class="align-top text-sm font-semibold whitespace-nowrap {summary.balanced ? 'text-success' : 'text-warning'}">
                        <span aria-hidden="true">{summary.balanced ? "✓" : "✗"}</span>
                        {summary.balanced ? "Balanced" : "Out of balance"}
                    </td>
                </tr>
            </tfoot>
        </table>
    </div>

    <!-- Net worth is what a personal-finance reader came for, so it stays big —
         but it is a DISTINCT figure below the statement, not its bottom line.
         `A − L` is identically Total equity, so closing the statement on it
         printed one number twice under two names. -->
    <div
        class="border-base-content/20 rounded-box bg-base-200 flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 border px-4 py-3"
        data-testid="bs-net-worth"
    >
        <span class="text-sm font-semibold tracking-wide uppercase">
            Net worth
            <span class="text-base-content/50 ml-1 text-xs font-normal tracking-normal normal-case">(assets − liabilities)</span>
        </span>
        <span class="text-right font-mono text-xl font-semibold tabular-nums">
            {@render amount(amountCell(summary.netWorth, report.base, styles), "")}
        </span>
    </div>

    {#if !summary.balanced}
        <div class="alert alert-warning rounded-box px-3 py-2 text-sm" role="alert" data-testid="bs-check">
            <span>
                This journal doesn't balance: assets − liabilities − equity should be zero, but it is
                <span class="font-mono tabular-nums">{check.text}{check.extras.length > 0 ? ` (${check.extras.join(", ")})` : ""}</span>. Look for a transaction
                whose postings don't sum to zero.
            </span>
        </div>
    {/if}
</div>
