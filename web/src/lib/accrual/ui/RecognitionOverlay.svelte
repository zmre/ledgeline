<script lang="ts">
    // The Recognition overlay (plan 45 D6, Q8): opened from the Journal
    // toolbar's Recognition button or `a`, on the review overlay's frame.
    //
    // Three tabs:
    //  - **Due**: the catch-up table. Months, then customers/vendors, every
    //    open row checked; each month's ties (the deferred/prepaid account's
    //    balance before and after posting against the schedules' remainder);
    //    the closed periods apart and unchecked, posted only as one adjustment
    //    dated today. **Post through ‹date›** writes exactly the checked rows
    //    in one request (the engine drops anything already posted).
    //  - **Needs a schedule**: deferrals with no `recognition:` tag inside the
    //    ▾ lookback, each opening the editor (`onEdit`).
    //  - **Invoices**: the settlement proposals (`SettlementsPanel`), as on the
    //    Invoices page.
    //
    // Rows are not edited here: the engine's post takes no per-entry edits
    // yet, so a changed entry is posted, then edited in the journal.
    import {untrack} from "svelte";
    import {resolve} from "$app/paths";
    import {dismissible} from "#ui/keys/dismissible.js";
    import {hint} from "#ui/keys/hint.svelte.js";
    import {keymap, registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY} from "#ui/keys/types.js";
    import {sub, isZero} from "#lib/domain/money.js";
    import {fmt} from "#lib/format/amounts.js";
    import {backdrop, barIn, layerOut, prefersReducedMotion} from "#lib/review/ui/motion.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {openRecognitionMonth} from "#lib/journal/openJournal.js";
    import {checkState, dayShort, entryKey, monthShort, NEEDS_LOOKBACK_CHOICES, postThroughDate} from "../model";
    import {accrual, type RecognitionTab} from "../store.svelte";
    import type {DueEntry, DueMonth, Money, SettleSide} from "../types";
    import {lookbackLabel} from "./format";
    import SettlementsPanel from "./SettlementsPanel.svelte";
    import TxnDetails from "./TxnDetails.svelte";
    import {txnDetails} from "../details.svelte";

    interface Props {
        /** Open transaction `index` in the editor (Needs a schedule). The overlay closes first. */
        onEdit?: (index: number) => void;
    }

    const {onEdit}: Props = $props();

    const reduced = prefersReducedMotion();
    const TABS: readonly {id: RecognitionTab; label: string}[] = [
        {id: "due", label: "Due"},
        {id: "needs", label: "Needs a schedule"},
        {id: "invoices", label: "Invoices"},
    ];

    let root = $state<HTMLDivElement | null>(null);
    const due = $derived(accrual.due);
    const needs = $derived(accrual.needs);
    const status = $derived(accrual.status);
    const styles = $derived(journal.reportStyles);
    const busy = $derived(accrual.busy);

    /** The checked open entries, by `entryKey`. */
    let checked = $state.raw<ReadonlySet<string>>(new Set());
    let includeClosed = $state(false);
    /** Collapsed months (all open to begin with). */
    let collapsed = $state.raw<ReadonlySet<string>>(new Set());
    /** Entries shown with their postings. */
    let expanded = $state.raw<ReadonlySet<string>>(new Set());
    let failure = $state<string | null>(null);
    let seededFor: string | null = null;

    // Every open row checked whenever a fresh list arrives (a new generation).
    $effect(() => {
        const listed = due;
        if (listed === null || listed.generation === seededFor) return;
        seededFor = listed.generation;
        untrack(() => {
            checked = new Set(listed.months.flatMap((month) => monthKeys(month)));
            includeClosed = false;
        });
    });

    function monthEntries(month: DueMonth): DueEntry[] {
        return month.groups.flatMap((group) => [...group.entries]);
    }
    function monthKeys(month: DueMonth): string[] {
        return monthEntries(month).map(entryKey);
    }

    const checkedEntries = $derived((due?.months ?? []).flatMap(monthEntries).filter((entry) => checked.has(entryKey(entry))));
    const checkedMonths = $derived(new Set(checkedEntries.map((entry) => entry.recognized.slice(0, 7))));
    const through = $derived(due === null ? null : (postThroughDate(checkedMonths, due.today) ?? (includeClosed ? due.today : null)));
    const canPost = $derived(due !== null && !busy && (checkedEntries.length > 0 || (includeClosed && due.closed.count > 0)));
    const empty = $derived(due !== null && due.months.length === 0 && due.closed.count === 0);

    const money = (amount: Money): string => fmt(amount.commodity, amount.quantity, styles);
    const totalsText = (totals: readonly Money[]): string => totals.map(money).join(" + ") || "—";

    function toggleKeys(keys: readonly string[]): void {
        // eslint-disable-next-line svelte/prefer-svelte-reactivity -- a scratch copy, assigned whole to `$state.raw`
        const next = new Set(checked);
        const all = keys.every((key) => next.has(key));
        for (const key of keys) {
            if (all) next.delete(key);
            else next.add(key);
        }
        checked = next;
    }

    function toggleIn(set: ReadonlySet<string>, key: string): ReadonlySet<string> {
        // eslint-disable-next-line svelte/prefer-svelte-reactivity -- a scratch copy, assigned whole to `$state.raw`
        const next = new Set(set);
        if (next.has(key)) next.delete(key);
        else next.add(key);
        return next;
    }

    /** `indeterminate` is a DOM property with no attribute form. */
    function triState(node: HTMLInputElement, state: "none" | "some" | "all") {
        const apply = (value: "none" | "some" | "all") => {
            node.indeterminate = value === "some";
            node.checked = value === "all";
        };
        apply(state);
        return {update: apply};
    }

    async function post(): Promise<void> {
        if (!canPost) return;
        failure = null;
        const outcome = await accrual.post(checkedEntries, includeClosed);
        if (!outcome.ok) failure = outcome.failure.message;
    }

    function close(): void {
        root?.setAttribute("data-closing", "");
        accrual.close();
    }

    function edit(index: number): void {
        close();
        onEdit?.(index);
    }

    /** Needs a schedule rows expanded to their whole transaction (Phase 3b), by `index:posting`; absent: "Show details". */
    let needsOpen = $state<Record<string, boolean>>({});
    const needsIsOpen = (key: string): boolean => needsOpen[key] ?? txnDetails.showAll;

    function setLookback(event: Event): void {
        void accrual.setNeedsLookback(Number((event.currentTarget as HTMLSelectElement).value));
    }

    function sideAmount(item: SettleSide): string {
        return money(item.amount);
    }

    // Focus the layer on open, so its keys (and Escape) reach it.
    $effect(() => {
        root?.focus();
    });

    registerKeys({
        id: "recognition-overlay",
        priority: PRIORITY.overlay,
        modal: true,
        bindings: [
            {keys: "1", label: "Due", group: "Journal", run: () => accrual.setTab("due")},
            {keys: "2", label: "Needs a schedule", group: "Journal", run: () => accrual.setTab("needs")},
            {keys: "3", label: "Invoices", group: "Journal", run: () => accrual.setTab("invoices")},
            {keys: "mod+Enter", label: "Post the checked entries", group: "Journal", enabled: () => accrual.tab === "due" && canPost, run: () => void post()},
            {keys: "Escape", label: "Close", group: "Journal", run: close},
            {keys: "?", label: "Show keyboard shortcuts", group: "Journal", run: () => keymap.toggleHelp()},
        ],
    });
</script>

{#snippet entryRow(entry: DueEntry, party: string | null)}
    {@const key = entryKey(entry)}
    {@const open = expanded.has(key)}
    <li class="flex flex-col" data-testid="due-entry">
        <div class="flex items-center gap-2 py-1 pl-6 text-sm">
            <button
                type="button"
                class="btn btn-square btn-ghost btn-xs"
                aria-label={open ? "Hide postings" : "Show postings"}
                aria-expanded={open}
                onclick={() => (expanded = toggleIn(expanded, key))}
            >
                <ReviewIcon name={open ? "chevron-down" : "chevron-right"} class="h-3 w-3" />
            </button>
            <input
                type="checkbox"
                class="checkbox checkbox-xs"
                checked={checked.has(key)}
                onchange={() => (checked = toggleIn(checked, key))}
                aria-label="Post {entry.description} {entry.recognized}"
                data-testid="due-entry-check"
            />
            <span class="w-14 shrink-0 font-mono text-base-content/60">{dayShort(entry.recognized)}</span>
            <span class="min-w-0 grow truncate">{party ?? entry.description}</span>
            <span class="hidden truncate text-base-content/50 sm:inline">{entry.postings.map((p) => p.account).join(" → ")}</span>
            <span class="shrink-0 font-mono tabular-nums">
                {totalsText(entry.postings.flatMap((p) => p.amounts.filter((a) => a.quantity.m > 0n)))}
            </span>
        </div>
        {#if open}
            <ul class="mb-1 ml-14 rounded-box bg-base-200/60 px-3 py-1 font-mono text-xs" data-testid="due-entry-postings">
                <li class="text-base-content/60">{entry.recognized} {entry.description}</li>
                {#each entry.postings as posting, at (at)}
                    <li class="flex gap-4 pl-4">
                        <span class="grow">{posting.account}</span>
                        <span class="tabular-nums">{posting.amounts.map(money).join(", ")}</span>
                    </li>
                {/each}
            </ul>
        {/if}
    </li>
{/snippet}

{#snippet monthBlock(month: DueMonth)}
    {@const keys = monthKeys(month)}
    {@const state = checkState(keys, checked)}
    {@const open = !collapsed.has(month.month)}
    <section class="border-b border-base-content/10 py-1 last:border-b-0" data-testid="due-month" data-month={month.month}>
        <div class="flex items-center gap-2 py-1">
            <button
                type="button"
                class="btn btn-square btn-ghost btn-xs"
                aria-label={open ? `Collapse ${monthShort(month.month)}` : `Expand ${monthShort(month.month)}`}
                aria-expanded={open}
                onclick={() => (collapsed = toggleIn(collapsed, month.month))}
            >
                <ReviewIcon name={open ? "chevron-down" : "chevron-right"} class="h-3.5 w-3.5" />
            </button>
            <input
                type="checkbox"
                class="checkbox checkbox-sm"
                use:triState={state}
                onchange={() => toggleKeys(keys)}
                aria-label="Post every entry in {monthShort(month.month)}"
                data-testid="due-month-check"
                data-state={state}
            />
            <span class="font-semibold">{monthShort(month.month)}</span>
            <span class="text-sm text-base-content/60">· {month.count} {month.count === 1 ? "entry" : "entries"} ·</span>
            <span class="font-mono text-sm tabular-nums">{totalsText(month.totals)}</span>
            {#if !open && month.ties.length > 0}
                <span class={["ml-auto text-xs", month.ties.every((tie) => tie.ties) ? "text-success" : "text-warning"]}>
                    {month.ties.every((tie) => tie.ties) ? "ties ✓" : "doesn't tie"}
                </span>
            {/if}
        </div>
        {#if open}
            {#each month.ties as tie (`${tie.account}|${tie.commodity}`)}
                {@const off = sub(tie.after, tie.scheduled)}
                <p class="flex flex-wrap items-center gap-2 pl-14 text-xs text-base-content/70" data-testid="due-tie">
                    <span class="font-mono">{tie.account}</span>
                    <span class="font-mono tabular-nums">{fmt(tie.commodity, tie.before, styles)} → {fmt(tie.commodity, tie.after, styles)}</span>
                    {#if tie.ties || isZero(off)}
                        <span class="text-success">ties ✓</span>
                    {:else}
                        <span class="text-warning">off by {fmt(tie.commodity, off, styles)} against the schedules</span>
                        <button type="button" class="link text-xs link-hover" onclick={() => accrual.setTab("needs")} data-testid="due-tie-needs">
                            Needs a schedule?
                        </button>
                    {/if}
                </p>
            {/each}
            <ul>
                {#each month.groups as group, at (`${group.party?.name ?? ""}|${at}`)}
                    {#if group.party !== null && month.groups.length > 1}
                        <li class="pt-1 pl-6 text-xs font-semibold text-base-content/60">{group.party.name}</li>
                    {/if}
                    {#each group.entries as entry (entryKey(entry))}
                        {@render entryRow(entry, group.party?.name ?? null)}
                    {/each}
                {/each}
            </ul>
        {/if}
    </section>
{/snippet}

<div
    bind:this={root}
    class="review-layer fixed inset-0 z-[999] overflow-hidden"
    role="dialog"
    aria-modal="true"
    aria-label="Recognition"
    data-testid="recognition-overlay"
    tabindex="-1"
    use:dismissible={{trap: true, onDismiss: close}}
>
    <div class="pointer-events-none relative z-10 flex h-full flex-col items-center gap-3 px-3 pt-3 sm:gap-4 sm:px-6 sm:pt-5" out:layerOut|global={{reduced}}>
        <div
            class="pointer-events-auto relative z-20 flex w-full max-w-4xl shrink-0 flex-wrap items-center gap-x-3 gap-y-1 rounded-box border border-base-content/10 bg-base-300/95 py-1.5 pr-1.5 pl-4 text-base-content shadow-lg backdrop-blur"
            in:barIn|global={{reduced}}
        >
            <h2 class="flex min-w-0 items-center gap-2 text-sm font-semibold tracking-wide whitespace-nowrap"><ReviewIcon name="calendar" /> Recognition</h2>
            <div role="tablist" class="tabs tabs-box tabs-sm" aria-label="Recognition">
                {#each TABS as item, at (item.id)}
                    {@const count =
                        item.id === "due"
                            ? (status?.counts.due ?? 0)
                            : item.id === "needs"
                              ? (status?.counts.needsSchedule ?? 0)
                              : (status?.counts.invoices ?? 0)}
                    <button
                        type="button"
                        role="tab"
                        class={["tab gap-1.5", accrual.tab === item.id && "tab-active"]}
                        aria-selected={accrual.tab === item.id}
                        title={hint(item.label, String(at + 1))}
                        onclick={() => accrual.setTab(item.id)}
                        data-testid="recognition-tab-{item.id}"
                    >
                        {item.label}
                        <span class="badge badge-ghost badge-xs tabular-nums">{count}</span>
                    </button>
                {/each}
            </div>
            {#if busy}
                <span class="flex items-center gap-1.5 text-xs text-base-content/60" role="status">
                    <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-success"></span> saving…
                </span>
            {/if}
            <button
                type="button"
                class="btn ml-auto btn-square btn-ghost text-base-content/80 btn-sm hover:bg-base-content/10"
                aria-label="Close Recognition"
                title={hint("Close", "Esc")}
                onclick={close}
            >
                <ReviewIcon name="x" />
            </button>
        </div>

        {#if accrual.notice !== null}
            {@const notice = accrual.notice}
            <div class="pointer-events-auto alert w-full max-w-4xl alert-success shadow-lg" role="status" data-testid="recognition-notice">
                <span class="grow">{notice.message}</span>
                {#if notice.month !== null}
                    <!-- A button, not a link: the journal page is already mounted and reads its URL once, so the month is set through
                         the filter store and the expand seed (openRecognitionMonth), the way Insights' Top Transactions opens one. -->
                    <button
                        type="button"
                        class="btn btn-ghost btn-sm"
                        onclick={() => {
                            const month = notice.month;
                            close();
                            if (month !== null) void openRecognitionMonth(month);
                        }}
                        data-testid="recognition-notice-link">Show {monthShort(notice.month)}</button
                    >
                {/if}
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => accrual.clearNotice()}>Dismiss</button>
            </div>
        {/if}
        {#if failure !== null}
            <div class="pointer-events-auto alert w-full max-w-4xl alert-error shadow-lg" role="alert" data-testid="recognition-failure">
                <span class="grow break-words">Not saved: {failure}</span>
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => (failure = null)}>Dismiss</button>
            </div>
        {/if}

        <div
            class="pointer-events-auto flex max-h-[calc(100dvh-7.5rem-var(--ll-top-inset))] w-full max-w-4xl flex-col overflow-hidden rounded-box border border-base-content/10 bg-base-100 shadow-2xl"
        >
            {#if accrual.tab === "due"}
                <div class="flex flex-wrap items-center gap-2 border-b border-base-content/10 px-4 py-2">
                    <p class="text-xs text-base-content/60">To change an entry, post it, then edit it in the journal.</p>
                    <button type="button" class="btn ml-auto btn-primary btn-sm" disabled={!canPost} onclick={() => void post()} data-testid="recognition-post">
                        {through === null ? "Post" : `Post through ${dayShort(through)}`}
                        <ReviewIcon name="arrow-right" class="h-3.5 w-3.5" />
                    </button>
                </div>
                <div class="min-h-0 overflow-y-auto overscroll-contain px-3 py-2">
                    {#if due === null}
                        <div class="flex items-center justify-center gap-2 py-12 text-sm text-base-content/60" role="status">
                            {#if accrual.error !== null}
                                <span class="text-error">Couldn't read the due entries: {accrual.error}</span>
                            {:else}
                                <span class="loading loading-sm loading-spinner"></span> Reading due entries…
                            {/if}
                        </div>
                    {:else if empty}
                        <p class="py-12 text-center text-sm text-base-content/60" data-testid="recognition-empty">Nothing is due.</p>
                    {:else}
                        {#each due.months as month (month.month)}
                            {@render monthBlock(month)}
                        {/each}
                        {#if due.closed.count > 0}
                            <label class="mt-2 flex flex-wrap items-center gap-2 rounded-box bg-base-200/60 px-3 py-2 text-sm" data-testid="due-closed">
                                <input type="checkbox" class="checkbox checkbox-sm" bind:checked={includeClosed} data-testid="due-closed-check" />
                                <span class="font-semibold">Closed periods{due.closedThrough === null ? "" : ` (through ${due.closedThrough})`}</span>
                                <span class="text-base-content/60">· {due.closed.count} {due.closed.count === 1 ? "entry" : "entries"} ·</span>
                                <span class="font-mono tabular-nums">{totalsText(due.closed.totals)}</span>
                                <span class="ml-auto text-xs text-base-content/60">post as one adjustment today</span>
                            </label>
                        {/if}
                    {/if}
                </div>
            {:else if accrual.tab === "needs"}
                <div class="flex flex-wrap items-center gap-2 border-b border-base-content/10 px-4 py-2">
                    <p class="text-xs text-base-content/60">Deferred-revenue credits and prepaid debits with no schedule.</p>
                    <label class="ml-auto flex items-center gap-1.5 text-sm">
                        Look back
                        <select class="select w-28 select-sm" value={String(accrual.needsLookback)} onchange={setLookback} data-testid="needs-lookback">
                            {#each NEEDS_LOOKBACK_CHOICES as days (days)}
                                <option value={String(days)}>{lookbackLabel(days)}</option>
                            {/each}
                        </select>
                    </label>
                </div>
                <div class="min-h-0 overflow-y-auto overscroll-contain px-3 py-2">
                    {#if needs === null}
                        <div class="flex items-center justify-center gap-2 py-12 text-sm text-base-content/60" role="status">
                            <span class="loading loading-sm loading-spinner"></span> Reading…
                        </div>
                    {:else if needs.items.length === 0}
                        <p class="py-12 text-center text-sm text-base-content/60" data-testid="needs-empty">Every deferral in the lookback has a schedule.</p>
                    {:else}
                        <ul class="divide-y divide-base-content/10">
                            {#each needs.items as item (`${item.index}:${item.posting}`)}
                                {@const detailKey = `${item.index}:${item.posting}`}
                                <li class="flex flex-wrap items-center gap-3 py-2 text-sm" data-testid="needs-item">
                                    <span class="w-16 shrink-0 font-mono text-base-content/60">{dayShort(item.date)}</span>
                                    <span class="min-w-0 grow truncate font-medium">{item.description || "(no description)"}</span>
                                    {#if item.party !== null}<span class="badge badge-ghost badge-sm">{item.party.name}</span>{/if}
                                    {#if item.invoice !== null}<span class="badge badge-ghost badge-sm">INV {item.invoice}</span>{/if}
                                    <span class="truncate text-base-content/60">{item.account}</span>
                                    <span class="font-mono tabular-nums">{sideAmount(item)}</span>
                                    <button
                                        type="button"
                                        class="btn btn-ghost btn-sm"
                                        aria-expanded={needsIsOpen(detailKey)}
                                        onclick={() => (needsOpen = {...needsOpen, [detailKey]: !needsIsOpen(detailKey)})}
                                        data-testid="details-toggle">Details {needsIsOpen(detailKey) ? "▾" : "▸"}</button
                                    >
                                    <button type="button" class="btn btn-sm" onclick={() => edit(item.index)} data-testid="needs-edit">
                                        <ReviewIcon name="pencil" class="h-3.5 w-3.5" /> Open in editor
                                    </button>
                                    {#if needsIsOpen(detailKey)}
                                        <div class="basis-full">
                                            <TxnDetails index={item.index} highlight={item.posting} />
                                        </div>
                                    {/if}
                                </li>
                            {/each}
                        </ul>
                    {/if}
                </div>
            {:else}
                <div class="min-h-0 overflow-y-auto overscroll-contain p-4">
                    <SettlementsPanel onEdit={edit} />
                    <p class="mt-4 text-right text-sm">
                        <a class="link link-hover" href={resolve("/settlements")} onclick={close} data-testid="recognition-invoices-page"
                            >Open the Invoices page</a
                        >
                    </p>
                </div>
            {/if}
        </div>
    </div>
    <button
        type="button"
        class="absolute inset-0 z-0 h-full w-full cursor-default bg-black/70 backdrop-blur-sm"
        aria-label="Close Recognition"
        onclick={close}
        in:backdrop|global={{reduced}}
        out:backdrop|global={{reduced, closing: true}}
    ></button>
</div>
