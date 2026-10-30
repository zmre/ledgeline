<!-- A recurring rule (one `~` block), read like the text it is (plan 31 C3).

         ~ monthly to 2027-02-01   payroll          ⛓ step 1 of 2
         ; A STEP: payroll jumps in February.
             expenses:payroll                 $5,000.00
             → assets:checking

     Amounts are signed as the file holds them and right-aligned in one column
     per card; growth is a chip; an asset posting says where its balance comes
     from (the journal's figure, or an override) and where its gains go. The
     counter-account is the last line, greyed when it is the engine's default.
     A single-date rule (`~ 2027-03-01`) reads the same way and badges as a
     one-off. -->
<script lang="ts">
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {amountText, growthText, lineAmountText} from "../../entryText";
    import {attachedText, type Entry} from "../../entries";
    import {journalBalanceFor} from "../../projectionView";
    import {noteIsStale} from "../../scenarioModel";
    import type {AssetRow, ForecastDefaults} from "../../types";
    import EntryFrame from "./EntryFrame.svelte";
    import KindBadge from "./KindBadge.svelte";
    import {POSTING_GRID, POSTING_NOTES, type FrameProps} from "./frame";

    let {
        entry,
        frame,
        declared,
        styles,
        assetBalances,
        defaults,
        onJump,
    }: {
        entry: Extract<Entry, {kind: "rule"}>;
        frame: FrameProps;
        declared: AccountClasses;
        styles: ReadonlyMap<string, AmountStyle>;
        assetBalances: ReadonlyMap<string, AssetRow>;
        defaults: ForecastDefaults | null;
        /** Move the cursor to another segment of this step change. */
        onJump: (group: string) => void;
    } = $props();

    const first = $derived(entry.lines[0]);
    const attached = $derived(attachedText(entry.item));
    const estimated = $derived(entry.lines.some((line) => line.source === "unbudgeted"));
    const stale = $derived(entry.lines.some(noteIsStale));
    const counter = $derived(first.counter);
    const counterDefault = $derived(defaults?.lines.get(entry.key) ?? null);
    const nextStep = $derived(entry.step === null ? null : entry.step.groups[entry.step.index % entry.step.of]);
</script>

<EntryFrame {entry} {...frame}>
    {#snippet header(id)}
        <h3 {id} class={["flex min-w-0 flex-wrap items-baseline gap-x-3 text-sm", entry.disabled && "line-through decoration-base-content/50"]}>
            <span class="font-mono text-[0.8125rem] whitespace-nowrap text-base-content/75"><span class="text-base-content/35">~</span> {first.period.raw}</span
            >
            {#if first.note !== ""}
                <span class="min-w-0 font-medium break-words text-base-content">{first.note}</span>
            {/if}
            {#if stale}
                <span
                    class="size-1.5 self-center rounded-full bg-warning"
                    title="This description was written for the seeded average; the amount has changed since"
                    data-testid="note-stale"><span class="sr-only">description out of date</span></span
                >
            {/if}
            {#if first.ruleComment !== ""}
                <span class="min-w-0 truncate font-mono text-xs text-base-content/45 italic">; {first.ruleComment.split("\n").join(" · ")}</span>
            {/if}
        </h3>
        {#if estimated}
            <span title="An average of your recent history, not something written in a budget">
                <KindBadge kind="estimated" />
            </span>
        {/if}
        {#if entry.step !== null && nextStep !== null}
            <button
                type="button"
                class="ml-auto inline-flex items-center gap-1 rounded-full border border-primary/30 px-2 font-mono text-[0.6875rem] leading-[1.125rem] text-primary/90 hover:bg-primary/10"
                title="Step {entry.step.index} of {entry.step.of}: go to the other segment"
                aria-label="Step {entry.step.index} of {entry.step.of} of {entry.name}: go to step {(entry.step.index % entry.step.of) + 1}"
                data-testid="step-chip"
                data-no-activate
                onclick={() => onJump(nextStep)}
            >
                <span aria-hidden="true">⛓</span> step {entry.step.index} of {entry.step.of}
            </button>
        {/if}
    {/snippet}
    {#snippet body()}
        {#if attached !== ""}
            <p class="mt-0.5 pl-3 font-mono text-xs whitespace-pre-wrap text-base-content/50 italic" data-testid="entry-attached">
                {attached
                    .split("\n")
                    .map((line) => `; ${line}`)
                    .join("\n")}
            </p>
        {/if}
        <div class={POSTING_GRID} data-testid="entry-postings">
            {#each entry.lines as line (line.id)}
                {@const asset = line.role === "asset"}
                {@const journal = asset ? journalBalanceFor(assetBalances, entry.key, line.account, line.amount.commodity) : null}
                {@const amount = lineAmountText(line, declared, styles)}
                <span class="min-w-0 truncate text-base-content/90" title={line.account}>{line.account === "" ? "(no account)" : line.account}</span>
                <span class={["text-right whitespace-nowrap", amount.startsWith("−") && "text-error"]}>{amount}</span>
                <span class="flex min-w-0 flex-wrap items-baseline gap-x-2 font-sans text-xs text-base-content/55 {POSTING_NOTES}">
                    {#if line.growth !== null}
                        <span class="rounded bg-base-content/6 px-1 font-mono whitespace-nowrap text-base-content/70">{growthText(line.growth)}</span>
                    {/if}
                    {#if asset}
                        <span class="font-mono whitespace-nowrap" data-testid="entry-balance">
                            {#if line.opening !== null}
                                <span class="text-warning">bal {amountText(line.opening.commodity, line.opening.quantity, styles)} (override)</span>
                            {:else if journal !== null}
                                bal {amountText(line.amount.commodity, journal, styles)} (journal)
                            {:else}
                                bal from the journal
                            {/if}
                        </span>
                        {#if line.gains !== null}
                            <span class="font-mono whitespace-nowrap">gains → {line.gains}</span>
                        {/if}
                    {/if}
                    {#if line.comment !== ""}
                        <span class="min-w-0 truncate font-mono italic" title={line.comment}>; {line.comment.split("\n").join(" · ")}</span>
                    {/if}
                </span>
            {/each}
            <span class="col-span-full min-w-0 truncate text-base-content/50" data-testid="entry-counter">
                <span aria-hidden="true">→</span><span class="sr-only">balanced against</span>
                {#if counter !== null}
                    {counter}
                {:else}
                    <span class="italic">{counterDefault ?? "the default cash account"}</span>
                    <span class="font-sans text-xs text-base-content/35">(default)</span>
                {/if}
            </span>
        </div>
    {/snippet}
</EntryFrame>
