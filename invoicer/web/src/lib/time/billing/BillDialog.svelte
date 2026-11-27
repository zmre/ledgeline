<!-- Bill… (plan 46 B-invoice): one customer's unbilled time for a period (or
     the lines Entries picked) as an invoice draft. Choose how the rows group
     the time; per row, the catalog item (the suggestion and where it came
     from, in words), its rate, books account and description, whether to
     leave it off, and whether to remember the item. The rounding preview
     shows the hours as tracked, as billed and the difference, per row and in
     total; the timesheet choice shows the table that goes under the bill.

     Every change asks the server for a fresh preview (the newest answer
     wins); Create draft writes the draft and tags its time in one step, then
     hands the result to `onCreated` (the page opens the draft).

     Keyboard: focus starts on the grouping; Tab walks every field; ⌘↵ /
     Ctrl+↵ creates the draft from anywhere; Enter in a field never does;
     Escape cancels (DialogShell). -->
<script lang="ts">
    import {onDestroy, onMount, untrack} from "svelte";
    import AccountInput from "#ui/components/AccountInput.svelte";
    import Combobox from "#ui/components/Combobox.svelte";
    import DateInput from "#ui/components/DateInput.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import {fetchEditorOptions} from "#lib/editorApi.js";
    import type {CatalogItem} from "#lib/editor.js";
    import {errorText} from "#lib/labels.js";
    import DialogShell from "../DialogShell.svelte";
    import {createBill, previewBill, type BillCreated, type BillGroup, type BillPreview, type Detail, type GroupChoice, type Grouping} from "../api/billing.js";
    import {
        billRequest,
        DETAILS,
        differenceText,
        groupLabel,
        GROUPINGS,
        parseTable,
        periodLabel,
        rememberLabel,
        roundingWords,
        rowNeeds,
        sourceWords,
        withChoice,
        type BillOptions,
    } from "./words.js";

    let {
        customer,
        name,
        from,
        to,
        entries,
        step,
        onCreated,
        onCancel,
        onSkip,
    }: {
        /** The `customer:` value (or the note's title). */
        customer: string;
        /** The customer's name, for the title. */
        name: string;
        from?: string;
        to?: string;
        /** The picked lines (Entries), instead of a period. */
        entries?: string[];
        /** Bill all…: this dialog's place in the run. */
        step?: {at: number; of: number};
        onCreated: (created: BillCreated) => void;
        onCancel: () => void;
        /** Bill all…: go on to the next customer without billing this one. */
        onSkip?: () => void;
    } = $props();

    // ---- What is edited
    let options = $state<BillOptions>({group: "account", detail: null, date: "", number: "", terms: "", reference: ""});
    let choices = $state<GroupChoice[]>([]);
    const target = $derived({customer, ...(from === undefined ? {} : {from}), ...(to === undefined ? {} : {to}), ...(entries === undefined ? {} : {entries})});
    const request = $derived(billRequest(target, options, choices));

    // ---- The catalog and the books' accounts, for the pickers
    let catalog = $state<CatalogItem[]>([]);
    let accounts = $state<string[]>([]);
    onMount(() => {
        fetchEditorOptions()
            .then((o) => {
                catalog = o.catalog;
                accounts = o.accounts;
            })
            .catch((e: unknown) => console.warn("editor options:", e));
    });
    const itemOptions = $derived<ComboOption[]>(
        catalog.map((c) => ({value: c.code, label: c.code, detail: `${c.description}${c.rate === null ? "" : ` · ${c.rate}`}`}))
    );

    // ---- The preview: again after every change, the newest answer wins
    let preview = $state<BillPreview | null>(null);
    let previewError = $state<string | null>(null);
    let pending = $state(true);
    let latest = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    function ask(body: typeof request): void {
        const mine = ++latest;
        previewBill(body)
            .then((p) => {
                if (mine !== latest) return;
                preview = p;
                previewError = null;
            })
            .catch((e: unknown) => {
                if (mine === latest) previewError = errorText(e);
            })
            .finally(() => {
                if (mine === latest) pending = false;
            });
    }

    let first = true;
    $effect(() => {
        const body = $state.snapshot(request);
        untrack(() => {
            pending = true;
            clearTimeout(timer);
            if (first) {
                first = false;
                ask(body);
            } else timer = setTimeout(() => ask(body), 200);
        });
    });
    onDestroy(() => clearTimeout(timer));

    // ---- Rows
    const choiceOf = (key: string): GroupChoice | undefined => choices.find((c) => c.key === key);
    const set = (key: string, patch: Omit<GroupChoice, "key">): void => {
        choices = withChoice(choices, key, patch);
    };

    function setGroup(group: Grouping): void {
        // The rows' keys change with the grouping, so the per-row choices go.
        choices = [];
        options = {...options, group};
    }

    function chooseItem(key: string, code: string): void {
        // A chosen item brings its own rate and account.
        set(key, {item: code, rate: undefined, account: undefined});
    }

    const field = (g: BillGroup, name: "item" | "rate" | "account" | "description"): string => {
        const own = choiceOf(g.key)?.[name];
        if (own !== undefined) return own;
        if (name === "item") return g.item?.code ?? "";
        if (name === "account") return g.books ?? "";
        if (name === "rate") return g.rate ?? "";
        return g.description;
    };

    // ---- Create
    let creating = $state(false);
    let createError = $state<string | null>(null);

    async function create(): Promise<void> {
        if (creating || preview === null || !preview.ready || pending) return;
        creating = true;
        createError = null;
        try {
            onCreated(await createBill($state.snapshot(request)));
        } catch (e: unknown) {
            createError = errorText(e);
        } finally {
            creating = false;
        }
    }

    const heading = $derived(
        entries !== undefined
            ? `${entries.length === 1 ? "1 picked line" : `${entries.length} picked lines`}`
            : from !== undefined && to !== undefined
              ? periodLabel(from, to)
              : preview?.from != null && preview.to != null
                ? periodLabel(preview.from, preview.to)
                : "Unbilled time"
    );
    const table = $derived(parseTable(preview?.timesheet ?? null));
    const detail = $derived<Detail>(options.detail ?? preview?.detail ?? "daily");
    const globalNeeds = $derived.by((): string[] => {
        if (preview === null) return [];
        if (preview.groups.length === 0) return preview.needs;
        if (preview.groups.every((g) => g.excluded)) return ["Every row is left off, so there is nothing to bill."];
        return [];
    });
    const uid = $props.id();
</script>

<DialogShell
    title="Bill {preview?.customer.title ?? name}"
    submitLabel="Create draft"
    busy={creating}
    submitDisabled={preview === null || !preview.ready || pending}
    error={createError ?? previewError}
    onSubmit={() => void create()}
    {onCancel}
    testid="bill-dialog"
    wide
    enterSubmits={false}
>
    {#snippet header()}
        {#if step !== undefined}
            <span class="badge badge-ghost badge-sm" data-testid="bill-dialog-step">{step.at} of {step.of}</span>
        {/if}
        {#if onSkip !== undefined}
            <button type="button" class="btn btn-ghost btn-sm" onclick={onSkip} disabled={creating} data-testid="bill-dialog-skip">Skip</button>
        {/if}
    {/snippet}

    <p class="text-sm text-base-content/70" data-testid="bill-dialog-period">
        {heading}{#if preview !== null}
            · {preview.actual.text} tracked{preview.entries.length === 0
                ? ""
                : `, ${preview.entries.length === 1 ? "1 entry" : `${preview.entries.length} entries`}`}{/if}
    </p>

    <div class="flex flex-wrap items-end gap-3">
        <label class="flex flex-col gap-1 text-sm"
            >Rows
            <select
                class="select w-56 select-sm"
                value={options.group}
                onchange={(e) => setGroup(e.currentTarget.value as Grouping)}
                data-initial
                data-testid="bill-grouping"
            >
                {#each GROUPINGS as g (g.value)}<option value={g.value}>{g.label}</option>{/each}
            </select>
        </label>
        <p class="text-xs text-base-content/60" data-testid="bill-rounding">{roundingWords(preview?.customer.rounding)}</p>
    </div>

    <section class="flex flex-col gap-2" aria-label="Rows" data-testid="bill-rows">
        {#if preview === null && pending}
            <span class="loading loading-sm loading-spinner" aria-label="Working out the bill"></span>
        {/if}
        {#each preview?.groups ?? [] as g (g.key)}
            {@const choice = choiceOf(g.key)}
            {@const needs = rowNeeds(g, choice)}
            <div
                class={["flex flex-col gap-2 rounded-box border border-base-300 p-3", g.excluded && "opacity-60"]}
                data-testid="bill-row"
                data-key={g.key}
                role="group"
                aria-label={groupLabel(g)}
            >
                <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                    <span class="font-medium break-all">{groupLabel(g)}</span>
                    <span class="text-sm tabular-nums" data-testid="bill-row-hours">
                        <span title="As tracked">{g.actual.text}</span>
                        → <span class="font-medium" title="As billed">{g.hours} h</span>
                        <span class="text-base-content/60" title="Rounding">({differenceText(g.actual, g.rounded)})</span>
                        {#if g.amount !== null && !g.excluded}<span class="ml-2 font-semibold" data-testid="bill-row-amount">{g.amount}</span>{/if}
                    </span>
                </div>
                {#if !g.excluded}
                    <div class="grid gap-2 @2xl:grid-cols-[minmax(0,9rem)_minmax(0,7rem)_minmax(0,1fr)_minmax(0,1.5fr)]">
                        <div class="flex min-w-0 flex-col gap-1 text-sm">
                            <span aria-hidden="true">Item</span>
                            <Combobox
                                bind:value={() => field(g, "item"), (v) => set(g.key, {item: v})}
                                options={itemOptions}
                                label="Item for {groupLabel(g)}"
                                onChoose={(o) => chooseItem(g.key, o.value)}
                                testid="bill-row-item"
                            />
                            {#if g.item !== null && choice?.item === undefined}
                                <span class="text-xs text-base-content/60" data-testid="bill-row-source">{sourceWords(g.item.source)}</span>
                            {/if}
                        </div>
                        <label class="flex min-w-0 flex-col gap-1 text-sm"
                            >Rate
                            <input
                                class="input w-full tabular-nums input-sm"
                                value={field(g, "rate")}
                                oninput={(e) => set(g.key, {rate: e.currentTarget.value})}
                                autocomplete="off"
                                data-testid="bill-row-rate"
                            />
                        </label>
                        <div class="flex min-w-0 flex-col gap-1 text-sm" data-testid="bill-row-books">
                            <span aria-hidden="true">Books account</span>
                            <AccountInput
                                bind:value={() => field(g, "account"), (v) => set(g.key, {account: v})}
                                accountNames={accounts}
                                label="Books account for {groupLabel(g)}"
                                placeholder="The item's account"
                            />
                        </div>
                        <label class="flex min-w-0 flex-col gap-1 text-sm"
                            >Description
                            <input
                                class="input w-full input-sm"
                                value={field(g, "description")}
                                oninput={(e) => set(g.key, {description: e.currentTarget.value})}
                                autocomplete="off"
                                data-testid="bill-row-description"
                            />
                        </label>
                    </div>
                {/if}
                <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
                    <label class="flex items-center gap-2"
                        ><input
                            type="checkbox"
                            class="checkbox checkbox-sm"
                            checked={g.excluded}
                            onchange={(e) => set(g.key, {exclude: e.currentTarget.checked || undefined})}
                            data-testid="bill-row-exclude"
                        /> Leave off this bill</label
                    >
                    {#if !g.excluded}
                        <label class="flex items-center gap-2"
                            ><input
                                type="checkbox"
                                class="checkbox checkbox-sm"
                                checked={choice?.remember ?? false}
                                onchange={(e) => set(g.key, {remember: e.currentTarget.checked || undefined})}
                                data-testid="bill-row-remember"
                            />
                            {rememberLabel(options.group)}</label
                        >
                    {/if}
                </div>
                {#each needs as need (need)}<p class="text-xs text-warning" data-testid="bill-row-need">{need}</p>{/each}
            </div>
        {/each}
        {#each globalNeeds as need (need)}<p class="text-sm text-warning" data-testid="bill-need">{need}</p>{/each}
    </section>

    {#if preview !== null && preview.groups.length > 0}
        <div class="flex flex-wrap items-baseline justify-end gap-x-4 gap-y-1 border-t border-base-300 pt-2 text-sm tabular-nums" data-testid="bill-total">
            <span>Tracked <span class="font-medium">{preview.actual.text}</span></span>
            <span>Billed <span class="font-medium">{preview.rounded.text}</span></span>
            <span class="text-base-content/60">Rounding {differenceText(preview.actual, preview.rounded)}</span>
            {#if preview.amount !== null}<span class="text-base font-semibold" data-testid="bill-total-amount">{preview.amount}</span>{/if}
        </div>
    {/if}

    <div class="flex flex-col gap-2">
        <label class="flex flex-wrap items-center gap-2 text-sm"
            >Timesheet under the bill
            <select
                class="select w-64 select-sm"
                value={detail}
                onchange={(e) => (options = {...options, detail: e.currentTarget.value as Detail})}
                data-testid="bill-detail"
            >
                {#each DETAILS as d (d.value)}<option value={d.value}>{d.label}</option>{/each}
            </select>
        </label>
        {#if table !== null}
            <div class="max-h-48 overflow-auto rounded-box border border-base-300" data-testid="bill-timesheet">
                <table class="table table-xs" aria-labelledby="{uid}-sheet">
                    <caption id="{uid}-sheet" class="px-2 py-1 text-left text-xs text-base-content/60"
                        >{table.title ?? "Timesheet"}, as the invoice shows it</caption
                    >
                    <thead
                        ><tr
                            >{#each table.headers as h, c (c)}<th class={table.right[c] ? "text-right" : ""}>{h}</th>{/each}</tr
                        ></thead
                    >
                    <tbody>
                        {#each table.rows as row, r (r)}
                            <tr data-testid="bill-timesheet-row"
                                >{#each row as cell, c (c)}<td class={table.right[c] ? "text-right tabular-nums" : ""}>{cell}</td>{/each}</tr
                            >
                        {/each}
                    </tbody>
                </table>
            </div>
        {/if}
    </div>

    <fieldset class="grid gap-2 @xl:grid-cols-4">
        <legend class="mb-1 text-sm font-medium">The invoice</legend>
        <label class="flex flex-col gap-1 text-sm"
            >Date
            <DateInput bind:value={options.date} aria-label="Invoice date" data-testid="bill-date" />
        </label>
        <label class="flex flex-col gap-1 text-sm"
            >Number
            <input class="input w-full input-sm" bind:value={options.number} placeholder="The next number" autocomplete="off" data-testid="bill-number" />
        </label>
        <label class="flex flex-col gap-1 text-sm"
            >Terms (days)
            <input
                class="input w-full input-sm"
                bind:value={options.terms}
                inputmode="numeric"
                placeholder="The customer's"
                autocomplete="off"
                data-testid="bill-terms"
            />
        </label>
        <label class="flex flex-col gap-1 text-sm"
            >Reference
            <input class="input w-full input-sm" bind:value={options.reference} placeholder="Optional" autocomplete="off" data-testid="bill-reference" />
        </label>
    </fieldset>
</DialogShell>
