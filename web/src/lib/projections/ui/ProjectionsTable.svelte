<!-- The what-if table: four sections over one scenario.

     Income and Expenses hold the recurring FLOWS; Assets and balances holds the
     recurring STOCKS; One-off events holds the dated ones. A flow's section
     comes from the account's resolved TYPE, never from its name — but not while
     it is being edited. See the rule below.

     # Assets and balances

     An asset row states a balance rather than a per-period amount: what you
     hold, what rate it compounds at, and what you add to it. Three things about
     it are not obvious from the markup.

     **Its section is its `role`, and only its `role`.** `resolvedSection` tests
     that first, ahead of the period and ahead of the account, so an asset row
     cannot leave this section by being typed into — nothing in the row edits
     `role`. That is why the section-stability rule below needs no hold here:
     the property the hold exists to provide, this section has for free.

     **The Balance column is the JOURNAL's figure, and it does not come from the
     scenario.** An opening balance is not part of a what-if. It comes from the
     projection, which seeds exactly that balance for every asset row it walks
     (`Projection.assets`), so the number under the user's cursor is the number
     the curve below was drawn from. Reading a balance-sheet report beside it
     instead would give a second source that can disagree with the first.

     **An override never hides it.** Typing a balance sets `opening`, which the
     file writes as an `opening:` tag; the box then reads in the warning colour,
     gains a clear button, and the ledger's own figure appears under it. The
     failure mode this column exists to prevent is a user looking at a number
     they typed a month ago and believing it is what they own.

     The row menu deliberately offers NO "Add a step" here: two bounded segments
     of one asset row would each seed the full journal balance and each compound
     it over the whole span, which is the same money twice. A rate change is a
     second row with a `from`, not a step.

     # THE SECTION-STABILITY RULE

     **A row never moves, and never loses focus, while it is being edited.**

     Section membership used to be re-derived from the account text on every
     render, so it changed on every keystroke: `` is not revenue and neither is
     `i`, so a row added under Income was born in Expenses, and finishing
     `revenues:…` threw it back. Income and Expenses are two different `{#each}`
     blocks, so each flip DESTROYED the row's DOM node and rebuilt it elsewhere,
     taking the caret with it. One letter was enough.

     So the section is a stored property (`ScenarioLine.section`, UI-only) and
     the rule is:

       - A row added by a section's "Add a line" button is HELD in that section.
       - Any row is held in whatever section it is already in the moment focus
         enters it (`holdSection`), so editing an existing account cannot move
         it mid-word either.
       - The hold is released, and the account's type gets to decide again, only
         once focus has left the whole row (`scheduleRelease`). Never on input.
       - A row whose account is BLANK is never reconciled at all. That row
         cannot be classified, and dropping it into Expenses because of it is
         the original bug wearing a hat.
       - If releasing a hold MOVES a row, the live region below the heading says
         so by name. A row that relocates in silence is indistinguishable from
         one that was lost.

     `signedQuantity` keys off the account's resolved TYPE and not off the
     section, so none of this touches the sign convention.

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

     `RowMenu.svelte`, which is PORTALLED to <body> and positioned. It has to be:
     both tables live in `overflow-x-auto`, which clips on both axes, so an
     in-flow menu was invisible for the rows that most needed it. That component
     also owns Escape, click-outside and the trigger's toggle — a native
     `<details>` gives none of those, whatever this comment used to claim. Its
     items stay real `<button>`s, so a test finds them by role and name and
     never by geometry (jsdom has no layout engine — web/README.md).

     Every row's menu carries a UNIQUE accessible name, falling back to its
     position when the account is blank (`rowName`), because "Delete row" you
     cannot address is not a delete. -->
<script lang="ts">
    import {onDestroy} from "svelte";
    import AccountInput from "$lib/journal/edit/AccountInput.svelte";
    import {decToInput, parseAmountInput} from "$lib/api/editMapping";
    import type {AccountType} from "$lib/domain/accountTypes";
    import type {Dec} from "$lib/domain/money";
    import type {AmountStyle, ISODate} from "$lib/domain/types";
    import {EM_DASH, fmt} from "$lib/format/amounts";
    import {journalBalanceFor} from "../projectionView";
    import {
        amountFor,
        blankAssetLine,
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
        resolvedSection,
        rowNames,
        sectionOfLine,
        sectionPrefix,
        splitStep,
        takenIds,
        withAccount,
        withBounds,
        withInterval,
        type LogicalRow,
    } from "../scenarioModel";
    import {
        GROWTH_UNITS,
        SCENARIO_INTERVALS,
        type AssetRow,
        type GrowthUnit,
        type HeldSection,
        type Scenario,
        type ScenarioAmount,
        type ScenarioEvent,
        type ScenarioLine,
    } from "../types";
    import RowMenu from "./RowMenu.svelte";

    let {
        scenario,
        accountNames,
        declared,
        commodity,
        stepDate,
        assetBalances,
        styles,
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
        /**
         * What the last projection made of each asset row, keyed by the row's
         * `group` — the only source of the journal's own balance for an account.
         *
         * Empty before the first projection lands, and the Balance column says
         * so with a dash rather than a zero. It is read from the LAST GOOD
         * projection, not a matching one: the journal does not change between
         * two keystrokes, so blanking the column on every edit would be
         * flicker with no information in it.
         */
        assetBalances: ReadonlyMap<string, AssetRow>;
        /** Commodity display styles, for the ledger figure under an overridden balance. */
        styles: ReadonlyMap<string, AmountStyle>;
        /** Called after every edit. The store bumps its revision and debounces a recompute. */
        onChange: () => void;
    } = $props();

    /** The two recurring sections, in the order the ask names them. */
    const SECTIONS: {id: HeldSection; title: string; blurb: string}[] = [
        {id: "income", title: "Income", blurb: "What comes in, as a positive figure."},
        // Not just "Expenses": a recurring transfer to savings is neither
        // revenue nor an expense, and it belongs somewhere visible.
        {id: "expense", title: "Expenses and other outflows", blurb: "What goes out, as a positive figure."},
    ];

    /** The Assets section's heading, and the word its rows are named after. */
    const ASSET_TITLE = "Assets and balances";

    const titleOf = (section: HeldSection): string => (section === "income" ? "Income" : "Expenses");

    const rowsFor = (section: HeldSection) => logicalRows(scenario.lines, declared, section);
    /** Every `role: "asset"` row, whatever its account and whatever its period. */
    const assetRows = $derived(logicalRows(scenario.lines, declared, "asset"));
    const assetNames = $derived(rowNames(assetRows, ASSET_TITLE));

    /**
     * An asset row's name, for the controls a FLOW row also has.
     *
     * `rowNames` makes a name unique within its section and cannot do more than
     * that — but one account can legitimately be both a recurring transfer (a
     * flow row, under Expenses) and a compounding balance (a row here). A
     * monthly move into `assets:checking` beside an `assets:checking` row
     * earning interest is an ordinary thing to write, and it would otherwise
     * put two "Growth rate for assets:checking" boxes on the page with no way —
     * for a screen reader or for a test — to say which was which.
     *
     * Not applied to Balance or Contribution: those two words appear in no
     * other table, so those labels are already unambiguous and qualifying them
     * would only read as "Balance for the … balance".
     */
    const balanceOf = (name: string): string => `the ${name} balance`;
    /** Single-date `~` rules. The engine keeps them LINES; a reader reads them as one-offs, so they show here. */
    const datedLines = $derived(logicalRows(scenario.lines, declared, "oneoff").flatMap((row) => row.segments));

    /** One accessible name per row of a section, each unique within it — see `rowNames`. */
    const namesFor = (rows: LogicalRow[], section: HeldSection) => rowNames(rows, titleOf(section));

    // --- The section hold (see THE SECTION-STABILITY RULE in the header) ------

    /** How long the "this row moved" notice stays up, in ms. */
    const MOVE_NOTICE_MS = 6000;

    let moveNotice = $state("");
    let noticeTimer: ReturnType<typeof setTimeout> | null = null;

    function announceMove(account: string, to: HeldSection): void {
        moveNotice = `Moved ${account} to ${titleOf(to)} — that is what its account type makes it.`;
        if (noticeTimer !== null) clearTimeout(noticeTimer);
        noticeTimer = setTimeout(() => (moveNotice = ""), MOVE_NOTICE_MS);
    }

    onDestroy(() => {
        if (noticeTimer !== null) clearTimeout(noticeTimer);
    });

    /**
     * Hold every segment of `line`'s logical row where it is now.
     *
     * Both segments of a step change, not just the focused one: they are two
     * lines sharing an id, and `logicalRows` gathers them per section, so a
     * half-held row would tear in two.
     */
    function holdSection(line: ScenarioLine): void {
        const at = sectionOfLine(line, declared);
        // Only the two FLOW sections can be held. `oneoff` is the period
        // talking and `asset` is the role talking, and no box on a row argues
        // with either — so neither can be held and neither needs to be.
        if (at === "oneoff" || at === "asset") return;
        for (const segment of scenario.lines) {
            if (segment.id === line.id) segment.section = at;
        }
    }

    /**
     * Focus has left the row: hand the section back to the account's type, and
     * say so if that moves anything.
     *
     * Deferred by one task, and the check is deliberate rather than defensive.
     * Choosing from the account combobox blurs the field — its popup is
     * portalled to <body>, so the option clicked is not inside this row — and
     * the combobox then puts focus straight back. Releasing on the raw
     * `focusout` would move the row out from under the very click that was
     * choosing its account.
     */
    function scheduleRelease(line: ScenarioLine, row: HTMLElement): void {
        setTimeout(() => {
            if (row.contains(document.activeElement)) return;
            releaseSection(line);
        });
    }

    function releaseSection(line: ScenarioLine): void {
        const held = line.section;
        if (held === undefined) return;
        // A row with no account cannot be classified. Reconciling it would send
        // every blank Income row to Expenses, which is the bug the hold exists
        // to fix, arriving one event later.
        if (line.account.trim() === "") return;

        const to = resolvedSection(line, declared);
        for (const segment of scenario.lines) {
            if (segment.id === line.id) segment.section = undefined;
        }
        // Only a move between the two FLOW sections is announceable: those are
        // the only two a release can produce, and they are the only two whose
        // name `titleOf` knows.
        if (to !== held && to !== "oneoff" && to !== "asset") announceMove(line.account, to);
    }

    // --- Editing one line ----------------------------------------------------

    function commitAccount(line: ScenarioLine): void {
        // `bind:value` has already written the text; this is the re-sign, which
        // is the half a direct binding cannot do. It deliberately does NOT
        // reconcile the section: a commit is also what Enter does, and Enter
        // leaves the caret in the field.
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

    // --- An asset row's balance ----------------------------------------------

    /**
     * What the LEDGER says this row's account holds, in the row's own
     * commodity — or null when that is not known.
     *
     * Null is not zero. Before the first projection lands there is no answer;
     * for a row the engine declined to model there is none either (the warnings
     * above say why); and for a row whose account was just retyped the held
     * answer belongs to the previous account, which `journalBalanceFor` checks.
     */
    const journalBalance = (line: ScenarioLine): Dec | null => journalBalanceFor(assetBalances, line.group, line.account, line.amount.commodity);

    /** The ledger's figure spelled the way every other figure on the page is. */
    const ledgerText = (line: ScenarioLine): string => {
        const journal = journalBalance(line);
        return journal === null ? EM_DASH : fmt(line.amount.commodity, journal, styles);
    };

    /**
     * Type a balance over the journal's, or clear back to it.
     *
     * An EMPTY box means "use the journal's" — the same thing the clear button
     * does, and the thing a user who selects-all-and-deletes means. Anything
     * else that is not a number is ignored rather than dropping the override,
     * because a half-typed `1,2` is not an instruction to go back to the ledger.
     *
     * The override takes the ROW's commodity, which is the only one it could
     * take: the file writes `opening:` as a bare number and reads its commodity
     * off the row's own amount (plan 23, amendment 3).
     */
    function setOpening(line: ScenarioLine, raw: string): void {
        const parsed = parseAmountInput(raw);
        if (parsed === null) {
            if (raw.trim() === "") clearOpening(line);
            return;
        }
        line.opening = {commodity: line.amount.commodity, quantity: parsed, precision: Math.max(line.amount.precision, parsed.p)};
        onChange();
    }

    function clearOpening(line: ScenarioLine): void {
        if (line.opening === null) return;
        line.opening = null;
        onChange();
    }

    // --- Rows ----------------------------------------------------------------

    function addLine(section: HeldSection): void {
        const id = freshId("line", takenIds(scenario));
        // HELD in the section whose button was pressed — that is what keeps it
        // there no matter what the user types next, including nothing. The
        // seeded prefix is only a head start on the typing (`sectionPrefix`).
        const line: ScenarioLine = {
            ...blankLine(id, commodity, 2),
            account: sectionPrefix(section, accountNames, declared),
            section,
        };
        scenario.lines = [...scenario.lines, line];
        onChange();
    }

    /**
     * A new ASSET row: a balance that compounds, born with `role: "asset"`.
     *
     * No section hold, unlike `addLine`. This row is held here by its role, and
     * a hold is a thing only the two flow sections can need.
     */
    function addAssetLine(): void {
        const id = freshId("line", takenIds(scenario));
        const line: ScenarioLine = {
            ...blankAssetLine(id, commodity, 2),
            account: sectionPrefix("asset", accountNames, declared),
        };
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

{#snippet rowMenu(row: LogicalRow, name: string)}
    <!-- Delete is here for EVERY row, whatever is in its account field — and
         `name` is what makes a blank row's copy of it addressable. -->
    <RowMenu
        label="Row menu for {name}"
        items={[
            row.segments.length === 1
                ? {label: `Add a step on ${stepDate}`, onSelect: () => addStep(row.id)}
                : {label: "Remove the step", onSelect: () => dropStep(row.id)},
            {label: "Duplicate", onSelect: () => duplicate(row.id)},
            {label: "Delete row", onSelect: () => dropRow(row.id), danger: true},
        ]}
    />
{/snippet}

<div class="flex flex-col gap-4">
    <!-- Mounted always, not conditionally: a live region has to exist BEFORE
         the text lands in it to be announced reliably. `sr-only` while empty,
         so an empty region costs no layout. -->
    <p role="status" aria-live="polite" data-testid="section-move-notice" class={moveNotice === "" ? "sr-only" : "text-xs text-base-content/70"}>
        {moveNotice}
    </p>

    {#each SECTIONS as section (section.id)}
        {@const rows = rowsFor(section.id)}
        {@const names = namesFor(rows, section.id)}
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
                            {#each rows as row, rowAt (row.id)}
                                {@const name = names[rowAt]}
                                {#each row.segments as line, at (line.group)}
                                    <!-- The hold is taken and released on the WHOLE ROW, not on
                                         the account cell: leaving the account box for the amount
                                         box beside it is still editing this row, and a row that
                                         jumped sections then would take that box with it. -->
                                    <tr
                                        data-testid="projection-line"
                                        data-line-id={line.id}
                                        data-line-group={line.group}
                                        onfocusin={() => holdSection(line)}
                                        onfocusout={(e) => scheduleRelease(line, e.currentTarget)}
                                    >
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
                                                    aria-label="Amount for {name}"
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
                                                    aria-label="Period for {name}"
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
                                                    aria-label="Growth rate for {name}, in percent"
                                                    onchange={(e) => setGrowthRate(line, e.currentTarget.value)}
                                                />
                                                <span class="text-base-content/50">%</span>
                                                <select
                                                    class="select w-16 select-xs"
                                                    value={growthUnitOf(line)}
                                                    disabled={line.growth === null}
                                                    aria-label="Growth period for {name}"
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
                                                aria-label="From date for {name}"
                                                onchange={(e) => setBound(line, "from", e.currentTarget.value)}
                                            />
                                        </td>
                                        <td>
                                            <input
                                                type="date"
                                                class="input w-full input-xs"
                                                value={line.period.to ?? ""}
                                                disabled={line.period.simple === null}
                                                aria-label="To date for {name} (exclusive)"
                                                onchange={(e) => setBound(line, "to", e.currentTarget.value)}
                                            />
                                        </td>
                                        <td class="text-right">
                                            {#if at === 0}
                                                {@render rowMenu(row, name)}
                                            {:else}
                                                <button
                                                    type="button"
                                                    class="btn btn-ghost text-error btn-xs"
                                                    aria-label="Delete this segment of {name}"
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

    <!-- The recurring STOCKS. After the two flow sections and before the
         one-offs, because it is still a recurring row — and its own table,
         because Balance and Contribution are columns the flow rows have no
         meaning for. -->
    <section class="flex flex-col gap-1" data-testid="projection-section-asset">
        <div class="flex flex-wrap items-baseline justify-between gap-2">
            <h2 class="text-sm font-semibold">
                {ASSET_TITLE}
                <span class="ml-1 font-normal text-base-content/50">What you hold and what it grows at. Only the growth moves net worth.</span>
            </h2>
            <button type="button" class="btn btn-ghost btn-xs" onclick={addAssetLine}>+ Add an asset</button>
        </div>
        {#if assetRows.length === 0}
            <p class="py-2 text-sm text-base-content/60">Nothing here yet. A brokerage account, a 401k, a house.</p>
        {:else}
            <div class="overflow-x-auto">
                <table class="table table-xs">
                    <thead>
                        <tr>
                            <th class="w-64">Account</th>
                            <th class="w-40">Balance</th>
                            <th class="w-36">Growth</th>
                            <th class="w-36">Contribution</th>
                            <th class="w-28">Per</th>
                            <th class="w-36">From</th>
                            <th class="w-36">To</th>
                            <th class="w-10"><span class="sr-only">Row menu</span></th>
                        </tr>
                    </thead>
                    <tbody>
                        {#each assetRows as row, rowAt (row.id)}
                            {@const name = assetNames[rowAt]}
                            {#each row.segments as line, at (line.group)}
                                {@const overridden = line.opening !== null}
                                {@const journal = journalBalance(line)}
                                <!-- No `onfocusin`/`onfocusout` hold: this row cannot change
                                     section, because only `role` decides it and no box here
                                     edits `role`. -->
                                <tr data-testid="projection-asset-line" data-line-id={line.id} data-line-group={line.group}>
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
                                        </div>
                                    </td>
                                    <td>
                                        <div class="flex flex-col gap-0.5">
                                            <!-- GREYED until typed over: the box is empty and the
                                                 journal's figure is its placeholder. An override
                                                 fills the box, recolours it, and brings the ledger's
                                                 own number back below — never instead of it. -->
                                            <label class="input flex items-center gap-1 input-xs {overridden ? 'input-warning' : ''}">
                                                <span class="text-base-content/50">{line.amount.commodity}</span>
                                                <input
                                                    type="text"
                                                    inputmode="decimal"
                                                    class="w-full grow {overridden ? 'font-semibold' : ''}"
                                                    value={line.opening === null ? "" : decToInput(line.opening.quantity)}
                                                    placeholder={journal === null ? EM_DASH : decToInput(journal)}
                                                    aria-label="Balance for {name}"
                                                    title={journal === null
                                                        ? "The journal's balance for this account is not known yet"
                                                        : `Your journal says ${ledgerText(line)}`}
                                                    onchange={(e) => setOpening(line, e.currentTarget.value)}
                                                />
                                                {#if overridden}
                                                    <button
                                                        type="button"
                                                        class="btn btn-ghost px-1 btn-xs"
                                                        aria-label="Use the journal's balance for {name}"
                                                        onclick={() => clearOpening(line)}
                                                    >
                                                        ✕
                                                    </button>
                                                {/if}
                                            </label>
                                            {#if overridden}
                                                <span class="text-xs text-warning" data-testid="ledger-balance">ledger {ledgerText(line)}</span>
                                            {/if}
                                        </div>
                                    </td>
                                    <td>
                                        <div class="flex items-center gap-1">
                                            <input
                                                type="text"
                                                inputmode="decimal"
                                                class="input w-14 input-xs"
                                                placeholder="—"
                                                value={growthPercent(line)}
                                                aria-label="Growth rate for {balanceOf(name)}, in percent"
                                                onchange={(e) => setGrowthRate(line, e.currentTarget.value)}
                                            />
                                            <span class="text-base-content/50">%</span>
                                            <select
                                                class="select w-16 select-xs"
                                                value={growthUnitOf(line)}
                                                disabled={line.growth === null}
                                                aria-label="Growth period for {balanceOf(name)}"
                                                onchange={(e) => setGrowthUnit(line, e.currentTarget.value)}
                                            >
                                                {#each GROWTH_UNITS as unit (unit)}
                                                    <option value={unit}>{GROWTH_UNIT_ABBREV[unit]}</option>
                                                {/each}
                                            </select>
                                        </div>
                                    </td>
                                    <td>
                                        <!-- The row's `amount`. Zero is the normal case and is
                                             what the file writes as `$0` — a balance that only
                                             compounds. -->
                                        <label class="input flex items-center gap-1 input-xs">
                                            <span class="text-base-content/50">{line.amount.commodity}</span>
                                            <input
                                                type="text"
                                                inputmode="decimal"
                                                class="w-full grow"
                                                value={decToInput(magnitudeOf(line.amount))}
                                                aria-label="Contribution for {name}"
                                                onchange={(e) => setMagnitude(line, e.currentTarget.value)}
                                            />
                                        </label>
                                    </td>
                                    <td>
                                        {#if line.period.simple === null}
                                            <span class="font-mono text-xs text-base-content/60" title="This recurrence is shown as written"
                                                >{line.period.raw}</span
                                            >
                                        {:else}
                                            <select
                                                class="select w-full select-xs"
                                                value={line.period.simple}
                                                aria-label="Contribution period for {name}"
                                                onchange={(e) => setInterval(line, e.currentTarget.value)}
                                            >
                                                {#each SCENARIO_INTERVALS as interval (interval)}
                                                    <option value={interval}>{interval}</option>
                                                {/each}
                                            </select>
                                        {/if}
                                    </td>
                                    <td>
                                        <input
                                            type="date"
                                            class="input w-full input-xs"
                                            value={line.period.from ?? ""}
                                            disabled={line.period.simple === null}
                                            aria-label="From date for {balanceOf(name)}"
                                            onchange={(e) => setBound(line, "from", e.currentTarget.value)}
                                        />
                                    </td>
                                    <td>
                                        <input
                                            type="date"
                                            class="input w-full input-xs"
                                            value={line.period.to ?? ""}
                                            disabled={line.period.simple === null}
                                            aria-label="To date for {balanceOf(name)} (exclusive)"
                                            onchange={(e) => setBound(line, "to", e.currentTarget.value)}
                                        />
                                    </td>
                                    <td class="text-right">
                                        {#if at === 0}
                                            <!-- No "Add a step": see the header. A second bounded
                                                 segment would compound the same journal balance
                                                 twice over the same span. -->
                                            <RowMenu
                                                label="Row menu for {balanceOf(name)}"
                                                items={[
                                                    ...(overridden ? [{label: "Use the journal's balance", onSelect: () => clearOpening(line)}] : []),
                                                    {label: "Duplicate", onSelect: () => duplicate(row.id)},
                                                    {label: "Delete row", onSelect: () => dropRow(row.id), danger: true},
                                                ]}
                                            />
                                        {:else}
                                            <button
                                                type="button"
                                                class="btn btn-ghost text-error btn-xs"
                                                aria-label="Delete this segment of {balanceOf(name)}"
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
                        {#each scenario.events as event, eventAt (event.id)}
                            {@const eventName = event.description.trim() === "" ? `the new event ${eventAt + 1}` : event.description}
                            {#each event.postings as posting, at (at)}
                                <tr data-testid="projection-event" data-event-id={event.id}>
                                    <td>
                                        {#if at === 0}
                                            <input
                                                type="date"
                                                class="input w-full input-xs"
                                                value={event.date}
                                                aria-label="Date of {eventName}"
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
                                            <RowMenu
                                                label="Row menu for {eventName}"
                                                items={[
                                                    {label: "Add a posting", onSelect: () => addPosting(event)},
                                                    {label: "Delete event", onSelect: () => dropEvent(event.id), danger: true},
                                                ]}
                                            />
                                        {:else}
                                            <button
                                                type="button"
                                                class="btn btn-ghost text-error btn-xs"
                                                aria-label="Delete posting {at + 1} of {eventName}"
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
