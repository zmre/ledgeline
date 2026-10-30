<!-- The dialog's fields for a recurring rule — one `~` block (plan 31 C3).

     Block-level: the period (an interval and From/To, or the expression as
     written when the simple controls cannot state it), the description, the
     counter-account (the block's amountless posting) and the header comment.
     Every line of the block carries these, so each setter writes them all.

     Per posting: the account, the amount as a MAGNITUDE (the engine signs it
     by the account's type, plan 26 B6 — the preview below shows the sign the
     file will hold), growth, the asset switch, and for an asset its balance
     override and gains account, and its own comment. Postings reorder with
     ↑/↓. A posting that needs a different counter-account belongs in a rule
     of its own (+ Add, or Duplicate this one).

     Edits write the DRAFT only. `onEdit` fires on each committed change (a
     change event, an account chosen), never per keystroke, and is what asks
     the server for the "Writes:" preview. -->
<script lang="ts">
    import {decToInput, parseAmountInput} from "#lib/api/editMapping.js";
    import DateInput from "#lib/components/DateInput.svelte";
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {EM_DASH} from "#lib/format/amounts.js";
    import AccountInput from "#lib/journal/edit/AccountInput.svelte";
    import {amountText} from "../../../entryText";
    import type {StepInfo} from "../../../entries";
    import {journalBalanceFor} from "../../../projectionView";
    import {
        accountOrNull,
        boundsLockedReason,
        GROWTH_UNIT_ABBREV,
        isIsoDate,
        magnitudeOf,
        noteProblem,
        percentFromRate,
        periodWords,
        rateFromPercent,
        signedAmount,
        withAccount,
        withBounds,
        withInterval,
        withMagnitude,
    } from "../../../scenarioModel";
    import {GROWTH_UNITS, SCENARIO_INTERVALS, type AssetRow, type ForecastDefaults, type ScenarioLine} from "../../../types";

    let {
        lines,
        group,
        accountNames,
        declared,
        styles,
        defaults,
        assetBalances,
        cash,
        step,
        onEdit,
        onJump,
    }: {
        /** The draft's lines. Mutated in place. */
        lines: ScenarioLine[];
        /** The rule's key, for the engine's greyed defaults. */
        group: string;
        accountNames: string[];
        declared: AccountClasses;
        styles: ReadonlyMap<string, AmountStyle>;
        defaults: ForecastDefaults | null;
        assetBalances: ReadonlyMap<string, AssetRow>;
        /** The scenario's `cash:`, the counter-account a new rule falls back to. */
        cash: string | null;
        step: (StepInfo & {others: {group: string; period: string}[]}) | null;
        onEdit: () => void;
        onJump: ((group: string) => void) | null;
    } = $props();

    const first = $derived(lines[0]);
    const period = $derived(first.period);
    const locked = $derived(boundsLockedReason(period));
    const noteError = $derived(noteProblem(first.note));

    /** One name per posting, unique in the dialog: its account, or its position. */
    const names = $derived.by(() => {
        const accounts = lines.map((line) => line.account.trim());
        return accounts.map((account, at) => {
            if (account === "") return `posting ${at + 1}`;
            return accounts.filter((other) => other === account).length > 1 ? `${account} (posting ${at + 1})` : account;
        });
    });

    /** Set a block-level field on every line, then say so. */
    function setAll(patch: Partial<ScenarioLine>): void {
        for (const line of lines) Object.assign(line, patch);
        onEdit();
    }

    function setInterval(value: string): void {
        const found = SCENARIO_INTERVALS.find((interval) => interval === value);
        if (found !== undefined) setAll({period: withInterval(period, found)});
    }

    function setBound(key: "from" | "to", value: string): void {
        // A `type=date` emits "" while it is being cleared, which is how a
        // bound is removed; anything else must be a whole ISO date.
        const next = value === "" ? null : value;
        if (next !== null && !isIsoDate(next)) return;
        setAll({period: withBounds(period, key === "from" ? next : period.from, key === "to" ? next : period.to)});
    }

    function setRaw(value: string): void {
        const raw = value.trim();
        if (raw === "" || raw === period.raw) return;
        setAll({period: {raw, simple: null, base: null, from: null, to: null}});
    }

    function setMagnitude(line: ScenarioLine, raw: string): void {
        const parsed = parseAmountInput(raw);
        if (parsed === null) return;
        const next = withMagnitude(line, parsed);
        line.entry = next.entry;
        line.amount = next.amount;
        onEdit();
    }

    function commitAccount(line: ScenarioLine): void {
        // The re-sign: the magnitude re-entered, for the engine to sign by the new account's type.
        const next = withAccount(line, line.account);
        line.entry = next.entry;
        line.amount = next.amount;
        onEdit();
    }

    function setGrowthRate(line: ScenarioLine, raw: string): void {
        if (raw.trim() === "") {
            line.growth = null;
            onEdit();
            return;
        }
        const parsed = parseAmountInput(raw);
        if (parsed === null) return;
        // The box holds a PERCENT; the wire carries a fraction.
        line.growth = {rate: rateFromPercent(parsed), unit: line.growth?.unit ?? "year"};
        onEdit();
    }

    function setGrowthUnit(line: ScenarioLine, value: string): void {
        const found = GROWTH_UNITS.find((unit) => unit === value);
        if (found === undefined || line.growth === null) return;
        line.growth = {rate: line.growth.rate, unit: found};
        onEdit();
    }

    function setRole(line: ScenarioLine, asset: boolean): void {
        line.role = asset ? "asset" : "flow";
        if (!asset) {
            line.opening = null;
            line.gains = null;
        }
        onEdit();
    }

    /** An empty box is "use the journal's" — the same as the clear button. */
    function setOpening(line: ScenarioLine, raw: string): void {
        const parsed = parseAmountInput(raw);
        if (parsed === null) {
            if (raw.trim() === "") clearOpening(line);
            return;
        }
        line.opening = {commodity: line.amount.commodity, quantity: parsed, precision: Math.max(line.amount.precision, parsed.p)};
        onEdit();
    }

    function clearOpening(line: ScenarioLine): void {
        if (line.opening === null) return;
        line.opening = null;
        onEdit();
    }

    function move(at: number, delta: -1 | 1): void {
        const to = at + delta;
        if (to < 0 || to >= lines.length) return;
        const [line] = lines.splice(at, 1);
        lines.splice(to, 0, line);
        onEdit();
    }

    function addPosting(): void {
        const template = lines[0];
        const taken = new Set(lines.map((line) => line.id));
        let n = lines.length + 1;
        while (taken.has(`${template.group}#${n}`)) n += 1;
        lines.push({
            ...template,
            id: `${template.group}#${n}`,
            role: "flow",
            account: "",
            amount: {commodity: template.amount.commodity, quantity: {m: 0n, p: 2}, precision: 2},
            entry: {m: 0n, p: 2},
            growth: null,
            opening: null,
            gains: null,
            source: "journal",
            comment: "",
            resolved: undefined,
        });
        onEdit();
    }

    function dropPosting(at: number): void {
        if (lines.length <= 1) return;
        lines.splice(at, 1);
        onEdit();
    }

    const growthPercent = (line: ScenarioLine): string => (line.growth === null ? "" : decToInput(percentFromRate(line.growth.rate)));
    const counterPlaceholder = $derived(defaults?.lines.get(group) ?? cash ?? defaults?.cash ?? "the default cash account");
</script>

<div class="flex flex-col gap-4" data-testid="rule-editor">
    <!-- When it runs -->
    <fieldset class="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,10rem)_minmax(0,1fr)_minmax(0,1fr)]">
        <legend class="sr-only">When it runs</legend>
        {#if periodWords(period) !== null}
            <label class="flex flex-col gap-1">
                <span class="text-xs text-base-content/60">Repeats</span>
                {#if period.simple !== null}
                    <select class="select w-full select-sm" value={period.simple} aria-label="Repeats" onchange={(e) => setInterval(e.currentTarget.value)}>
                        {#each SCENARIO_INTERVALS as interval (interval)}
                            <option value={interval}>{interval}</option>
                        {/each}
                    </select>
                {:else}
                    <span
                        class="input flex items-center font-mono text-xs text-base-content/70 input-sm"
                        title="Shown as written: not one of the intervals offered here">{period.base}</span
                    >
                {/if}
            </label>
            <label class="flex flex-col gap-1">
                <span class="text-xs text-base-content/60">From</span>
                <DateInput class="w-full" value={period.from} aria-label="From" onchange={(e) => setBound("from", e.currentTarget.value)} />
            </label>
            <label class="flex flex-col gap-1">
                <span class="text-xs text-base-content/60">To <span class="text-base-content/40">(exclusive)</span></span>
                <DateInput class="w-full" value={period.to} aria-label="To (exclusive)" onchange={(e) => setBound("to", e.currentTarget.value)} />
            </label>
        {:else}
            <label class="flex flex-col gap-1 sm:col-span-3">
                <span class="text-xs text-base-content/60">Period, as hledger writes it</span>
                <input
                    type="text"
                    class="input w-full font-mono input-sm"
                    value={period.raw}
                    aria-label="Period expression"
                    title={locked ?? undefined}
                    spellcheck="false"
                    onchange={(e) => setRaw(e.currentTarget.value)}
                />
                <span class="text-xs text-base-content/45"
                    >This recurrence isn't one the simple controls can state; it is checked when the projection runs.</span
                >
            </label>
        {/if}
    </fieldset>

    <label class="flex flex-col gap-1">
        <span class="text-xs text-base-content/60">Description</span>
        <input
            type="text"
            class={["input w-full input-sm", noteError !== null && "input-error"]}
            value={first.note}
            placeholder="what this is"
            aria-label="Description"
            aria-invalid={noteError !== null}
            onchange={(e) => setAll({note: e.currentTarget.value})}
        />
        {#if noteError !== null}<span class="text-xs text-error">{noteError}</span>{/if}
    </label>

    {#if step !== null}
        <p class="flex flex-wrap items-center gap-x-2 text-xs text-base-content/60" data-testid="step-info">
            <span aria-hidden="true" class="text-primary">⛓</span>
            Step {step.index} of {step.of} of a step change.
            {#each step.others as other (other.group)}
                <span>The other segment runs <span class="font-mono">{other.period}</span>.</span>
                {#if onJump !== null}
                    <button type="button" class="link text-xs link-primary" onclick={() => onJump?.(other.group)}>Go to it</button>
                {/if}
            {/each}
        </p>
    {/if}

    <!-- The postings -->
    <div class="flex flex-col gap-1.5">
        <div class="flex items-baseline justify-between">
            <span class="text-xs font-medium text-base-content/70">Postings</span>
            <span class="text-xs text-base-content/45">Amounts are per period, as a positive figure; the file's sign follows the account.</span>
        </div>
        <ol class="flex flex-col gap-2">
            {#each lines as line, at (line.id)}
                {@const name = names[at]}
                {@const asset = line.role === "asset"}
                {@const journal = asset ? journalBalanceFor(assetBalances, group, line.account, line.amount.commodity) : null}
                <li class="rounded-box border border-base-content/8 bg-base-100/40 p-2" data-testid="rule-posting">
                    <div class="flex flex-wrap items-center gap-2">
                        <div class="join">
                            <button
                                type="button"
                                class="btn join-item btn-ghost btn-xs"
                                disabled={at === 0}
                                aria-label="Move {name} up"
                                onclick={() => move(at, -1)}>↑</button
                            >
                            <button
                                type="button"
                                class="btn join-item btn-ghost btn-xs"
                                disabled={at === lines.length - 1}
                                aria-label="Move {name} down"
                                onclick={() => move(at, 1)}>↓</button
                            >
                        </div>
                        <div class="min-w-40 grow basis-48">
                            <AccountInput
                                bind:value={line.account}
                                {accountNames}
                                size="sm"
                                label="Account for {name}"
                                placeholder="account"
                                onCommit={() => commitAccount(line)}
                            />
                        </div>
                        <label class="input w-32 input-sm" title="The file will hold {amountText(line.amount.commodity, signedAmount(line, declared), styles)}">
                            <span class="text-base-content/50">{line.amount.commodity}</span>
                            <input
                                type="text"
                                inputmode="decimal"
                                class="text-right font-mono"
                                value={decToInput(magnitudeOf(line))}
                                aria-label="{asset ? 'Contribution' : 'Amount'} for {name}"
                                onchange={(e) => setMagnitude(line, e.currentTarget.value)}
                            />
                        </label>
                        <div class="flex items-center gap-1">
                            <input
                                type="text"
                                inputmode="decimal"
                                class="input w-16 text-right font-mono input-sm"
                                placeholder="—"
                                value={growthPercent(line)}
                                aria-label="Growth rate for {name}, in percent"
                                onchange={(e) => setGrowthRate(line, e.currentTarget.value)}
                            />
                            <span class="text-xs text-base-content/50">%/</span>
                            <select
                                class="select w-18 select-sm"
                                value={line.growth?.unit ?? "year"}
                                disabled={line.growth === null}
                                aria-label="Growth period for {name}"
                                onchange={(e) => setGrowthUnit(line, e.currentTarget.value)}
                            >
                                {#each GROWTH_UNITS as unit (unit)}
                                    <option value={unit}>{GROWTH_UNIT_ABBREV[unit]}</option>
                                {/each}
                            </select>
                        </div>
                        <label class="flex items-center gap-1.5 text-xs text-base-content/70" title="A balance that compounds, rather than a per-period flow">
                            <input
                                type="checkbox"
                                class="toggle toggle-xs"
                                checked={asset}
                                aria-label="{name} is an asset balance"
                                onchange={(e) => setRole(line, e.currentTarget.checked)}
                            />
                            asset
                        </label>
                        <button
                            type="button"
                            class="btn btn-square btn-ghost text-error btn-xs"
                            disabled={lines.length <= 1}
                            aria-label="Remove {name}"
                            onclick={() => dropPosting(at)}>✕</button
                        >
                    </div>
                    {#if asset}
                        <div class="mt-2 flex flex-wrap items-center gap-2 pl-14 text-xs">
                            <label class={["input w-44 input-sm", line.opening !== null && "input-warning"]}>
                                <span class="text-base-content/50">Balance</span>
                                <input
                                    type="text"
                                    inputmode="decimal"
                                    class="text-right font-mono"
                                    value={line.opening === null ? "" : decToInput(line.opening.quantity)}
                                    placeholder={journal === null ? EM_DASH : decToInput(journal)}
                                    aria-label="Balance for {name}"
                                    onchange={(e) => setOpening(line, e.currentTarget.value)}
                                />
                            </label>
                            {#if line.opening !== null}
                                <button
                                    type="button"
                                    class="btn btn-ghost btn-xs"
                                    aria-label="Use the journal's balance for {name}"
                                    onclick={() => clearOpening(line)}
                                    >✕ use the journal's{journal === null ? "" : ` (${amountText(line.amount.commodity, journal, styles)})`}</button
                                >
                            {:else}
                                <span class="text-base-content/45" data-testid="ledger-balance">
                                    {journal === null
                                        ? "the journal's balance, once projected"
                                        : `the journal's: ${amountText(line.amount.commodity, journal, styles)}`}
                                </span>
                            {/if}
                            <div class="ml-auto w-64">
                                <AccountInput
                                    bind:value={() => line.gains ?? "", (next) => (line.gains = accountOrNull(next) === null ? null : next)}
                                    {accountNames}
                                    size="sm"
                                    label="Gains account for {name}"
                                    placeholder={defaults?.gains ?? "equity:unrealized gains"}
                                    onCommit={onEdit}
                                />
                            </div>
                        </div>
                    {/if}
                    <div class="mt-1.5 pl-14">
                        <input
                            type="text"
                            class="input w-full input-ghost font-mono text-base-content/70 input-xs"
                            value={line.comment}
                            placeholder="; comment on this posting (optional)"
                            aria-label="Comment for {name}"
                            onchange={(e) => {
                                line.comment = e.currentTarget.value;
                                onEdit();
                            }}
                        />
                    </div>
                </li>
            {/each}
        </ol>
        <button type="button" class="btn self-start btn-ghost btn-xs" onclick={addPosting}>+ Add a posting</button>
    </div>

    <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Counter-account <span class="text-base-content/40">(the posting with no amount)</span></span>
            <AccountInput
                bind:value={() => first.counter ?? "", (next) => lines.forEach((line) => (line.counter = next === "" ? null : next))}
                {accountNames}
                size="sm"
                label="Counter-account"
                placeholder={counterPlaceholder}
                onCommit={() => setAll({counter: accountOrNull(first.counter)})}
            />
        </label>
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Header comment</span>
            <input
                type="text"
                class="input w-full font-mono input-sm"
                value={first.ruleComment.split("\n").join(" ")}
                placeholder="; on the ~ line (optional)"
                aria-label="Header comment"
                onchange={(e) => setAll({ruleComment: e.currentTarget.value})}
            />
        </label>
    </div>
</div>
