<!-- The what-if table: three sections over one scenario.

     Income and Expenses hold the RECURRING lines; One-off events holds the
     dated ones. Which section a line lands in is decided by
     `scenarioModel.sectionOfLine` — by the account's resolved TYPE, never by
     its name.

     # Two amount conventions, and why they differ

     A RECURRING line takes a MAGNITUDE, exactly as a budget goal does: an
     income line is what you earn, not the negation of it, and
     `scenarioModel.signedQuantity` puts the journal's sign back on in one
     place. Changing a row's account re-signs its amount, which is why nothing
     here writes `line.account` without going through `withAccount`.

     An EVENT POSTING takes a SIGNED amount, and deliberately. An event is where
     a balance-sheet movement is written — the raise that puts $2M into
     `assets:cash`, the purchase that takes it back out — and the direction is
     the user's to choose, not something an account's type can decide. There is
     no type rule that makes both legs of "buy a building with cash" come out
     right. So the column says so, and accepts a leading minus.

     # The row menu

     A native <details class="dropdown">, not a positioned popup: it needs no
     measurement, it closes on Escape for free, and its items are real buttons
     that a test can find without asserting on geometry (jsdom has no layout
     engine — web/README.md). -->
<script lang="ts">
    import AccountInput from "$lib/journal/edit/AccountInput.svelte";
    import {decToInput, parseAmountInput} from "$lib/api/editMapping";
    import type {AccountType} from "$lib/domain/accountTypes";
    import type {ISODate} from "$lib/domain/types";
    import {
        amountFor,
        blankEvent,
        blankLine,
        duplicateRow,
        freshId,
        GROWTH_UNIT_ABBREV,
        isIsoDate,
        logicalRows,
        magnitudeOf,
        mergeStep,
        percentFromRate,
        rateFromPercent,
        removeRow,
        removeSegment,
        sectionPrefix,
        splitStep,
        takenIds,
        withAccount,
        withBounds,
        withInterval,
        type LineSection,
    } from "../scenarioModel";
    import {GROWTH_UNITS, SCENARIO_INTERVALS, type GrowthUnit, type Scenario, type ScenarioAmount, type ScenarioEvent, type ScenarioLine} from "../types";

    let {
        scenario,
        accountNames,
        declared,
        commodity,
        stepDate,
        onChange,
    }: {
        /** The live scenario. Mutated in place; `onChange` is what tells the store. */
        scenario: Scenario;
        accountNames: string[];
        declared: ReadonlyMap<string, AccountType>;
        /** The commodity a brand-new row opens in — the journal's dominant one. */
        commodity: string;
        /** The date "Add a step" splits at, and a new event opens on. The page picks it from the projected window. */
        stepDate: ISODate;
        /** Called after every edit. The store bumps its revision and debounces a recompute. */
        onChange: () => void;
    } = $props();

    /** The two recurring sections, in the order the ask names them. */
    const SECTIONS: {id: Exclude<LineSection, "oneoff">; title: string; blurb: string}[] = [
        {id: "income", title: "Income", blurb: "What comes in, as a positive figure."},
        // Not just "Expenses": a recurring transfer to savings is neither
        // revenue nor an expense, and it belongs somewhere visible.
        {id: "expense", title: "Expenses and other outflows", blurb: "What goes out, as a positive figure."},
    ];

    const rowsFor = (section: Exclude<LineSection, "oneoff">) => logicalRows(scenario.lines, declared, section);
    /** Single-date `~` rules. The engine keeps them LINES; a reader reads them as one-offs, so they show here. */
    const datedLines = $derived(logicalRows(scenario.lines, declared, "oneoff").flatMap((row) => row.segments));

    // --- Editing one line ----------------------------------------------------

    function commitAccount(line: ScenarioLine): void {
        // `bind:value` has already written the text; this is the re-sign, which
        // is the half a direct binding cannot do.
        line.amount = withAccount(line, line.account, declared).amount;
        onChange();
    }

    function setMagnitude(line: ScenarioLine, raw: string): void {
        const parsed = parseAmountInput(raw);
        if (parsed === null) return;
        line.amount = amountFor(line.amount, parsed, line.account, declared);
        onChange();
    }

    function setInterval(line: ScenarioLine, value: string): void {
        const found = SCENARIO_INTERVALS.find((interval) => interval === value);
        if (found === undefined) return;
        line.period = withInterval(line.period, found);
        onChange();
    }

    function setBound(line: ScenarioLine, key: "from" | "to", value: string): void {
        // A `type=date` input emits "" while it is being cleared, which is how a
        // bound is removed; anything else has to be a whole ISO date before it
        // reaches a period expression the engine will re-parse.
        const next = value === "" ? null : value;
        if (next !== null && !isIsoDate(next)) return;
        line.period = withBounds(line.period, key === "from" ? next : line.period.from, key === "to" ? next : line.period.to);
        onChange();
    }

    function setGrowthRate(line: ScenarioLine, raw: string): void {
        if (raw.trim() === "") {
            line.growth = null;
            onChange();
            return;
        }
        const parsed = parseAmountInput(raw);
        if (parsed === null) return;
        // The box holds a PERCENT; the wire carries a fraction. One conversion,
        // here and in `percentFromRate` below, and nowhere else.
        line.growth = {rate: rateFromPercent(parsed), unit: line.growth?.unit ?? "year"};
        onChange();
    }

    function setGrowthUnit(line: ScenarioLine, value: string): void {
        const found = GROWTH_UNITS.find((unit) => unit === value);
        if (found === undefined || line.growth === null) return;
        line.growth = {rate: line.growth.rate, unit: found};
        onChange();
    }

    const growthPercent = (line: ScenarioLine): string => (line.growth === null ? "" : decToInput(percentFromRate(line.growth.rate)));
    const growthUnitOf = (line: ScenarioLine): GrowthUnit => line.growth?.unit ?? "year";

    // --- Rows ----------------------------------------------------------------

    function addLine(section: Exclude<LineSection, "oneoff">): void {
        const id = freshId("line", takenIds(scenario));
        // Seeded with a prefix of the right TYPE, so the row appears in the
        // section the button was in rather than wherever an accountless row
        // classifies to — see `sectionPrefix`.
        const line = {...blankLine(id, commodity, 2), account: sectionPrefix(section, accountNames, declared)};
        scenario.lines = [...scenario.lines, line];
        onChange();
    }

    function addEvent(): void {
        scenario.events = [...scenario.events, blankEvent(freshId("event", takenIds(scenario)), stepDate, commodity, 2)];
        onChange();
    }

    function addStep(id: string): void {
        scenario.lines = splitStep(scenario.lines, id, stepDate, takenIds(scenario));
        onChange();
    }

    function dropStep(id: string): void {
        scenario.lines = mergeStep(scenario.lines, id);
        onChange();
    }

    function duplicate(id: string): void {
        scenario.lines = duplicateRow(scenario.lines, id, takenIds(scenario));
        onChange();
    }

    function dropRow(id: string): void {
        scenario.lines = removeRow(scenario.lines, id);
        onChange();
    }

    function dropSegment(group: string): void {
        scenario.lines = removeSegment(scenario.lines, group);
        onChange();
    }

    function dropEvent(id: string): void {
        scenario.events = scenario.events.filter((event) => event.id !== id);
        onChange();
    }

    function setEventDate(event: ScenarioEvent, value: string): void {
        if (!isIsoDate(value)) return;
        event.date = value;
        onChange();
    }

    /** An event posting's amount is SIGNED — see the header comment. */
    function setSignedAmount(holder: {amount: ScenarioAmount}, raw: string): void {
        const parsed = parseAmountInput(raw);
        if (parsed === null) return;
        holder.amount = {...holder.amount, quantity: parsed, precision: Math.max(holder.amount.precision, parsed.p)};
        onChange();
    }

    function addPosting(event: ScenarioEvent): void {
        event.postings = [...event.postings, {account: "", amount: {commodity, quantity: {m: 0n, p: 2}, precision: 2}}];
        onChange();
    }

    function dropPosting(event: ScenarioEvent, at: number): void {
        event.postings = event.postings.filter((_, i) => i !== at);
        onChange();
    }

    /** The date a single-date line fires on lives in its `raw`, which IS the date. */
    function setDatedLine(line: ScenarioLine, value: string): void {
        if (!isIsoDate(value)) return;
        line.period = {raw: value, simple: null, from: null, to: null};
        onChange();
    }
</script>

{#snippet estimated(line: ScenarioLine)}
    {#if line.source === "unbudgeted"}
        <!-- The flag the seed sets. It says where the number came from, which is
             the difference between a figure the user wrote and an average of
             their last twelve months. -->
        <span class="badge badge-ghost badge-xs" data-testid="estimated-badge" title={line.note === "" ? "Estimated from your history" : line.note}>
            estimated
        </span>
    {/if}
{/snippet}

{#snippet rowMenu(row: {id: string; segments: ScenarioLine[]})}
    <details class="dropdown dropdown-end">
        <summary class="btn btn-ghost btn-xs" aria-label="Row menu for {row.segments[0].account || 'a new row'}">⋯</summary>
        <ul class="menu dropdown-content z-10 w-52 rounded-box bg-base-100 p-2 shadow">
            {#if row.segments.length === 1}
                <li>
                    <button type="button" onclick={() => addStep(row.id)}>Add a step on {stepDate}</button>
                </li>
            {:else}
                <li>
                    <button type="button" onclick={() => dropStep(row.id)}>Remove the step</button>
                </li>
            {/if}
            <li><button type="button" onclick={() => duplicate(row.id)}>Duplicate</button></li>
            <li><button type="button" class="text-error" onclick={() => dropRow(row.id)}>Delete row</button></li>
        </ul>
    </details>
{/snippet}

<div class="flex flex-col gap-4">
    {#each SECTIONS as section (section.id)}
        {@const rows = rowsFor(section.id)}
        <section class="flex flex-col gap-1" data-testid="projection-section-{section.id}">
            <div class="flex flex-wrap items-baseline justify-between gap-2">
                <h2 class="text-sm font-semibold">
                    {section.title}
                    <span class="ml-1 font-normal text-base-content/50">{section.blurb}</span>
                </h2>
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => addLine(section.id)}>+ Add a line</button>
            </div>
            {#if rows.length === 0}
                <p class="py-2 text-sm text-base-content/60">Nothing here yet.</p>
            {:else}
                <div class="overflow-x-auto">
                    <table class="table table-xs">
                        <thead>
                            <tr>
                                <th class="w-64">Account</th>
                                <th class="w-36">Amount</th>
                                <th class="w-28">Per</th>
                                <th class="w-36">Growth</th>
                                <th class="w-36">From</th>
                                <th class="w-36">To</th>
                                <th class="w-10"><span class="sr-only">Row menu</span></th>
                            </tr>
                        </thead>
                        <tbody>
                            {#each rows as row (row.id)}
                                {#each row.segments as line, at (line.group)}
                                    <tr data-testid="projection-line" data-line-id={line.id} data-line-group={line.group}>
                                        <td>
                                            <div class="flex items-center gap-1">
                                                {#if at > 0}
                                                    <span class="text-base-content/40" title="A later segment of the row above">↳</span>
                                                {/if}
                                                <AccountInput
                                                    bind:value={line.account}
                                                    {accountNames}
                                                    size="xs"
                                                    placeholder="account"
                                                    onCommit={() => commitAccount(line)}
                                                />
                                                {@render estimated(line)}
                                            </div>
                                        </td>
                                        <td>
                                            <label class="input flex items-center gap-1 input-xs">
                                                <span class="text-base-content/50">{line.amount.commodity}</span>
                                                <input
                                                    type="text"
                                                    inputmode="decimal"
                                                    class="w-full grow"
                                                    value={decToInput(magnitudeOf(line.amount))}
                                                    aria-label="Amount for {line.account || 'a new row'}"
                                                    onchange={(e) => setMagnitude(line, e.currentTarget.value)}
                                                />
                                            </label>
                                        </td>
                                        <td>
                                            {#if line.period.simple === null}
                                                <!-- A period this editor cannot rebuild — its `raw` is the only
                                                     reading of it, and the engine re-parses exactly that. -->
                                                <span class="font-mono text-xs text-base-content/60" title="This recurrence is shown as written"
                                                    >{line.period.raw}</span
                                                >
                                            {:else}
                                                <select
                                                    class="select w-full select-xs"
                                                    value={line.period.simple}
                                                    aria-label="Period for {line.account || 'a new row'}"
                                                    onchange={(e) => setInterval(line, e.currentTarget.value)}
                                                >
                                                    {#each SCENARIO_INTERVALS as interval (interval)}
                                                        <option value={interval}>{interval}</option>
                                                    {/each}
                                                </select>
                                            {/if}
                                        </td>
                                        <td>
                                            <div class="flex items-center gap-1">
                                                <input
                                                    type="text"
                                                    inputmode="decimal"
                                                    class="input w-14 input-xs"
                                                    placeholder="—"
                                                    value={growthPercent(line)}
                                                    aria-label="Growth rate for {line.account || 'a new row'}, in percent"
                                                    onchange={(e) => setGrowthRate(line, e.currentTarget.value)}
                                                />
                                                <span class="text-base-content/50">%</span>
                                                <select
                                                    class="select w-16 select-xs"
                                                    value={growthUnitOf(line)}
                                                    disabled={line.growth === null}
                                                    aria-label="Growth period for {line.account || 'a new row'}"
                                                    onchange={(e) => setGrowthUnit(line, e.currentTarget.value)}
                                                >
                                                    {#each GROWTH_UNITS as unit (unit)}
                                                        <option value={unit}>{GROWTH_UNIT_ABBREV[unit]}</option>
                                                    {/each}
                                                </select>
                                            </div>
                                        </td>
                                        <td>
                                            <input
                                                type="date"
                                                class="input w-full input-xs"
                                                value={line.period.from ?? ""}
                                                disabled={line.period.simple === null}
                                                aria-label="From date for {line.account || 'a new row'}"
                                                onchange={(e) => setBound(line, "from", e.currentTarget.value)}
                                            />
                                        </td>
                                        <td>
                                            <input
                                                type="date"
                                                class="input w-full input-xs"
                                                value={line.period.to ?? ""}
                                                disabled={line.period.simple === null}
                                                aria-label="To date for {line.account || 'a new row'} (exclusive)"
                                                onchange={(e) => setBound(line, "to", e.currentTarget.value)}
                                            />
                                        </td>
                                        <td class="text-right">
                                            {#if at === 0}
                                                {@render rowMenu(row)}
                                            {:else}
                                                <button
                                                    type="button"
                                                    class="btn btn-ghost text-error btn-xs"
                                                    aria-label="Delete this segment of {line.account || 'a new row'}"
                                                    onclick={() => dropSegment(line.group)}
                                                >
                                                    ✕
                                                </button>
                                            {/if}
                                        </td>
                                    </tr>
                                {/each}
                            {/each}
                        </tbody>
                    </table>
                </div>
            {/if}
        </section>
    {/each}

    <section class="flex flex-col gap-1" data-testid="projection-section-oneoff">
        <div class="flex flex-wrap items-baseline justify-between gap-2">
            <h2 class="text-sm font-semibold">
                One-off events
                <span class="ml-1 font-normal text-base-content/50">Dated, and SIGNED — a minus is money leaving that account.</span>
            </h2>
            <button type="button" class="btn btn-ghost btn-xs" onclick={addEvent}>+ Add an event</button>
        </div>
        {#if scenario.events.length === 0 && datedLines.length === 0}
            <p class="py-2 text-sm text-base-content/60">Nothing here yet. An investment, a bonus, a big purchase.</p>
        {:else}
            <div class="overflow-x-auto">
                <table class="table table-xs">
                    <thead>
                        <tr>
                            <th class="w-36">Date</th>
                            <th class="w-48">Description</th>
                            <th class="w-64">Account</th>
                            <th class="w-36">Amount</th>
                            <th class="w-10"><span class="sr-only">Row menu</span></th>
                        </tr>
                    </thead>
                    <tbody>
                        {#each scenario.events as event (event.id)}
                            {#each event.postings as posting, at (at)}
                                <tr data-testid="projection-event" data-event-id={event.id}>
                                    <td>
                                        {#if at === 0}
                                            <input
                                                type="date"
                                                class="input w-full input-xs"
                                                value={event.date}
                                                aria-label="Date of {event.description || 'a new event'}"
                                                onchange={(e) => setEventDate(event, e.currentTarget.value)}
                                            />
                                        {/if}
                                    </td>
                                    <td>
                                        {#if at === 0}
                                            <input
                                                type="text"
                                                class="input w-full input-xs"
                                                placeholder="what it is"
                                                bind:value={event.description}
                                                aria-label="Description of the event on {event.date}"
                                                onchange={onChange}
                                            />
                                        {/if}
                                    </td>
                                    <td>
                                        <AccountInput bind:value={posting.account} {accountNames} size="xs" placeholder="account" onCommit={onChange} />
                                    </td>
                                    <td>
                                        <label class="input flex items-center gap-1 input-xs">
                                            <span class="text-base-content/50">{posting.amount.commodity}</span>
                                            <input
                                                type="text"
                                                inputmode="decimal"
                                                class="w-full grow"
                                                value={decToInput(posting.amount.quantity)}
                                                aria-label="Amount for {posting.account || 'a new posting'} on {event.date}"
                                                onchange={(e) => setSignedAmount(posting, e.currentTarget.value)}
                                            />
                                        </label>
                                    </td>
                                    <td class="text-right">
                                        {#if at === 0}
                                            <details class="dropdown dropdown-end">
                                                <summary class="btn btn-ghost btn-xs" aria-label="Row menu for {event.description || 'a new event'}">⋯</summary>
                                                <ul class="menu dropdown-content z-10 w-52 rounded-box bg-base-100 p-2 shadow">
                                                    <li><button type="button" onclick={() => addPosting(event)}>Add a posting</button></li>
                                                    <li><button type="button" class="text-error" onclick={() => dropEvent(event.id)}>Delete event</button></li>
                                                </ul>
                                            </details>
                                        {:else}
                                            <button
                                                type="button"
                                                class="btn btn-ghost text-error btn-xs"
                                                aria-label="Delete posting {at + 1} of {event.description || 'a new event'}"
                                                onclick={() => dropPosting(event, at)}
                                            >
                                                ✕
                                            </button>
                                        {/if}
                                    </td>
                                </tr>
                            {/each}
                        {/each}

                        <!-- Single-date `~` rules from a journal. The engine keeps them
                             LINES so a file round trip says so; a reader reads them as
                             one-offs, so they are shown here. -->
                        {#each datedLines as line (line.group)}
                            <tr data-testid="projection-dated-line" data-line-group={line.group}>
                                <td>
                                    <input
                                        type="date"
                                        class="input w-full input-xs"
                                        value={line.period.raw}
                                        aria-label="Date of the one-off {line.account}"
                                        onchange={(e) => setDatedLine(line, e.currentTarget.value)}
                                    />
                                </td>
                                <td class="text-base-content/60">{line.note === "" ? "from the journal" : line.note}</td>
                                <td>
                                    <div class="flex items-center gap-1">
                                        <AccountInput
                                            bind:value={line.account}
                                            {accountNames}
                                            size="xs"
                                            placeholder="account"
                                            onCommit={() => commitAccount(line)}
                                        />
                                        {@render estimated(line)}
                                    </div>
                                </td>
                                <td>
                                    <label class="input flex items-center gap-1 input-xs">
                                        <span class="text-base-content/50">{line.amount.commodity}</span>
                                        <input
                                            type="text"
                                            inputmode="decimal"
                                            class="w-full grow"
                                            value={decToInput(magnitudeOf(line.amount))}
                                            aria-label="Amount for the one-off {line.account}"
                                            onchange={(e) => setMagnitude(line, e.currentTarget.value)}
                                        />
                                    </label>
                                </td>
                                <td class="text-right">
                                    <button
                                        type="button"
                                        class="btn btn-ghost text-error btn-xs"
                                        aria-label="Delete the one-off {line.account}"
                                        onclick={() => dropSegment(line.group)}
                                    >
                                        ✕
                                    </button>
                                </td>
                            </tr>
                        {/each}
                    </tbody>
                </table>
            </div>
        {/if}
    </section>
</div>
