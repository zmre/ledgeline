<!-- The weekly grid (T-entries, as Harvest and Clockify draw it): accounts ×
     the seven days from Monday, with each row's, each day's and the week's
     totals. A cell is a field: type a new total (`2h`, `1h30`, `1.5`, `0:45`)
     and Enter or leaving it writes the difference (`planCellEdit`): more time
     on the day's open line or a new one, less off the open line. Time that is
     billed, locked or clocked is never changed from here; a cell holding any
     is marked, and a cut that would need it is refused in words.

     Add an account's row with the picker below the grid; it stays until the
     week changes. Narrow widths scroll sideways, the account column pinned. -->
<script lang="ts">
    import AccountInput from "#ui/components/AccountInput.svelte";
    import {isOpenLine, type WeekCell, type WeekGrid} from "./group.js";
    import {dayMonth, weekdayName} from "../dates.js";
    import {formatDuration, parseDuration} from "../duration.js";

    let {
        grid,
        today,
        accounts,
        onCell,
        onAddRow,
    }: {
        grid: WeekGrid;
        today: string;
        accounts: string[];
        /** Set `cell` on `account` to `seconds`: null when written, else why not. */
        onCell: (account: string, cell: WeekCell, seconds: number) => Promise<string | null>;
        onAddRow: (account: string) => void;
    } = $props();

    let problem = $state<{key: string; message: string} | null>(null);
    let busy = $state<string | null>(null);
    let adding = $state("");

    const cellKey = (account: string, date: string): string => `${account}\u0000${date}`;
    const shown = (seconds: number): string => (seconds === 0 ? "" : formatDuration(seconds).replace(" ", ""));
    const partlyFixed = (cell: WeekCell): boolean => cell.entries.some((e) => !isOpenLine(e));

    async function commit(event: Event, account: string, cell: WeekCell): Promise<void> {
        // Enter and the change event that follows it are one edit.
        if (busy !== null) return;
        const input = event.currentTarget as HTMLInputElement;
        const key = cellKey(account, cell.date);
        const text = input.value.trim();
        const seconds = text === "" ? 0 : parseDuration(text);
        if (seconds === null) {
            problem = {key, message: `“${text}” isn't a duration: write 45m, 1h30, 1.5 or 0:45.`};
            return;
        }
        if (Math.round(seconds / 60) === Math.round(cell.seconds / 60)) {
            if (problem?.key === key) problem = null;
            input.value = shown(cell.seconds);
            return;
        }
        busy = key;
        const refused = await onCell(account, cell, seconds);
        busy = null;
        if (refused === null) {
            if (problem?.key === key) problem = null;
        } else {
            problem = {key, message: refused};
            input.value = shown(cell.seconds);
        }
    }

    function cellKeydown(event: KeyboardEvent, account: string, cell: WeekCell): void {
        if (event.key === "Enter") {
            event.preventDefault();
            void commit(event, account, cell);
        } else if (event.key === "Escape") {
            const input = event.currentTarget as HTMLInputElement;
            if (input.value !== shown(cell.seconds)) {
                event.preventDefault();
                input.value = shown(cell.seconds);
            }
        }
    }

    function addRow(): void {
        const account = adding.trim();
        if (account === "") return;
        onAddRow(account);
        adding = "";
    }
</script>

<div class="flex flex-col gap-2">
    <div class="overflow-x-auto rounded-box border border-base-300">
        <table class="table table-sm" data-testid="week-grid">
            <thead>
                <tr>
                    <th class="sticky left-0 z-10 bg-base-100">Account</th>
                    {#each grid.days as day (day)}
                        <th class={["text-right whitespace-nowrap", day === today && "text-primary"]} data-testid="week-day">
                            {weekdayName(day)}<br /><span class="font-normal">{dayMonth(day)}</span>
                        </th>
                    {/each}
                    <th class="text-right">Total</th>
                </tr>
            </thead>
            <tbody>
                {#each grid.rows as row (row.account)}
                    <tr data-testid="week-row" data-account={row.account}>
                        <th class="sticky left-0 z-10 max-w-56 truncate bg-base-100 font-normal" title={row.account} scope="row">{row.account}</th>
                        {#each row.cells as cell (cell.date)}
                            {@const key = cellKey(row.account, cell.date)}
                            <td class="p-1 text-right">
                                <input
                                    class={[
                                        "input w-16 text-right tabular-nums input-xs",
                                        partlyFixed(cell) && "border-dashed",
                                        problem?.key === key && "input-error",
                                    ]}
                                    value={shown(cell.seconds)}
                                    readonly={busy === key}
                                    title={partlyFixed(cell) ? "Some of this time is billed, locked or clocked; only the rest changes here" : undefined}
                                    aria-label="{row.account}, {weekdayName(cell.date)} {cell.date}"
                                    autocomplete="off"
                                    onchange={(event) => void commit(event, row.account, cell)}
                                    onkeydown={(event) => cellKeydown(event, row.account, cell)}
                                    data-testid="week-cell"
                                    data-date={cell.date}
                                />
                            </td>
                        {/each}
                        <td class="text-right font-medium tabular-nums">{formatDuration(row.seconds)}</td>
                    </tr>
                {:else}
                    <tr><td colspan="9" class="text-center text-base-content/70">No time this week. Add an account's row below, then type its hours.</td></tr>
                {/each}
            </tbody>
            <tfoot>
                <tr>
                    <th class="sticky left-0 z-10 bg-base-100">Total</th>
                    {#each grid.dayTotals as total, i (grid.days[i])}
                        <td class="text-right tabular-nums">{total === 0 ? "" : formatDuration(total)}</td>
                    {/each}
                    <td class="text-right font-semibold tabular-nums" data-testid="week-total">{formatDuration(grid.seconds)}</td>
                </tr>
            </tfoot>
        </table>
    </div>
    {#if problem !== null}
        <p class="text-sm text-error" role="alert" data-testid="week-problem">{problem.message}</p>
    {/if}
    <form
        class="flex flex-wrap items-end gap-2"
        onsubmit={(event) => {
            event.preventDefault();
            addRow();
        }}
        aria-label="Add a row to the week"
    >
        <div class="flex min-w-0 grow basis-56 flex-col gap-1 text-sm sm:max-w-sm">
            <span aria-hidden="true">Add a row</span>
            <AccountInput bind:value={adding} accountNames={accounts} placeholder="clients:acme" label="Account to add a row for" />
        </div>
        <button type="submit" class="btn btn-outline btn-sm" disabled={adding.trim() === ""} data-testid="week-add-row">Add row</button>
    </form>
</div>
