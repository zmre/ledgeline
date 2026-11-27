<!-- Create bill… (plan 46 T18, B-ap): a timekeeper's unpaid time as a bill
     in the books (accounts payable), every line it covers tagged with the
     bill's reference. It opens on the preview: who is paid, the lines (one
     per account, or one in total), the rate and the accounts from their
     note. Whatever the note doesn't say is asked for: those fields start
     empty and the first takes focus. "Save to ‹name›'s note" (on by default)
     writes the answers into the note. With no note for the person, pick one
     or create one first: a bill always has a payee.

     Create bill books it and shows what was written, with Open in Ledgeline
     when this app can open it there. Keyboard: Tab walks the fields; ⌘↵ /
     Ctrl+↵ creates the bill; Enter in a field never does; Escape cancels. -->
<script lang="ts">
    import {onDestroy, onMount, tick, untrack} from "svelte";
    import AccountInput from "#ui/components/AccountInput.svelte";
    import Combobox from "#ui/components/Combobox.svelte";
    import DateInput from "#ui/components/DateInput.svelte";
    import {fetchDashboard, openInLedgeline} from "#lib/api.js";
    import {fetchEditorOptions} from "#lib/editorApi.js";
    import {errorText} from "#lib/labels.js";
    import DialogShell from "../DialogShell.svelte";
    import {createPay, isBooked, previewPay, type NeedField, type PayCreated, type PayPreview} from "../api/billing.js";
    import {needLabel, payFieldsOf, payRequest, periodLabel, savedWords, type PayFields} from "./words.js";

    let {
        person,
        name,
        from,
        to,
        entries,
        onBooked,
        onClose,
    }: {
        /** The `person:` value on the time cards. */
        person: string;
        /** Their name, for the title. */
        name: string;
        from?: string;
        to?: string;
        entries?: string[];
        /** The bill is booked (the page reads its lists again). */
        onBooked: (created: PayCreated) => void;
        /** Cancel, or Done after booking. */
        onClose: () => void;
    } = $props();

    const target = $derived({person, ...(from === undefined ? {} : {from}), ...(to === undefined ? {} : {to}), ...(entries === undefined ? {} : {entries})});

    let accounts = $state<string[]>([]);
    onMount(() => {
        fetchEditorOptions()
            .then((o) => (accounts = o.accounts))
            .catch((e: unknown) => console.warn("editor options:", e));
    });

    // ---- The first preview sets the fields; what it lacked is asked for
    let preview = $state<PayPreview | null>(null);
    let initial = $state<PayFields | null>(null);
    let fields = $state<PayFields | null>(null);
    /** What the first preview needed: those fields stay asked for, and are what Save to note saves. */
    let asked = $state<NeedField[]>([]);
    let loadError = $state<string | null>(null);
    let box = $state<HTMLDivElement | null>(null);

    onMount(async () => {
        try {
            const first = await previewPay(untrack(() => target));
            preview = first;
            initial = payFieldsOf(first);
            fields = payFieldsOf(first);
            asked = first.needs.map((n) => n.field);
            await tick();
            box?.querySelector<HTMLElement>("[data-need] input, input[data-need], [data-need] select")?.focus();
        } catch (e: unknown) {
            loadError = errorText(e);
        }
    });

    const needsNote = $derived(asked.includes("note"));
    const request = $derived(fields === null || initial === null ? null : payRequest(target, fields, initial, needsNote));

    // ---- Again after every change, the newest answer wins
    let pending = $state(false);
    let previewError = $state<string | null>(null);
    let latest = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let primed = false;
    $effect(() => {
        if (request === null) return;
        const body = $state.snapshot(request);
        untrack(() => {
            // The first preview is already here.
            if (!primed) {
                primed = true;
                return;
            }
            pending = true;
            clearTimeout(timer);
            const mine = ++latest;
            timer = setTimeout(() => {
                previewPay(body)
                    .then((p) => {
                        if (mine === latest) {
                            preview = p;
                            previewError = null;
                        }
                    })
                    .catch((e: unknown) => {
                        if (mine === latest) previewError = errorText(e);
                    })
                    .finally(() => {
                        if (mine === latest) pending = false;
                    });
            }, 200);
        });
    });
    onDestroy(() => clearTimeout(timer));

    const need = (field: NeedField) => preview?.needs.find((n) => n.field === field) ?? null;
    const isAsked = (field: NeedField): boolean => asked.includes(field);
    const blocking = $derived((preview?.needs ?? []).filter((n) => n.field === "time" || n.field === "reference"));
    const noteTitle = $derived(preview?.note?.title ?? (fields?.noteMode === "create" ? fields.newNote.title.trim() : fields?.pickedNote.trim()) ?? "");
    const offerSave = $derived(asked.some((f) => f === "rate" || f === "expense_account" || f === "payable_account"));

    // ---- Create bill
    let creating = $state(false);
    let createError = $state<string | null>(null);
    let booked = $state<PayCreated | null>(null);

    async function create(): Promise<void> {
        if (request === null || creating || preview === null || !preview.ready || pending) return;
        creating = true;
        createError = null;
        try {
            const answer = await createPay($state.snapshot(request));
            if (isBooked(answer)) {
                booked = answer;
                onBooked(answer);
            } else {
                preview = answer;
                createError = `Still needed: ${answer.needs.map((n) => needLabel(n.field).toLowerCase()).join(", ")}.`;
            }
        } catch (e: unknown) {
            createError = errorText(e);
        } finally {
            creating = false;
        }
    }

    // ---- After: Open in Ledgeline, when this app can
    let canOpen = $state(false);
    let openError = $state<string | null>(null);
    $effect(() => {
        if (booked === null) return;
        untrack(() => {
            fetchDashboard()
                .then((d) => (canOpen = d.open_ledgeline))
                .catch(() => (canOpen = false));
        });
    });
    function openBooks(): void {
        openError = null;
        openInLedgeline().catch((e: unknown) => (openError = errorText(e)));
    }

    const period = $derived(preview?.from != null && preview.to != null ? periodLabel(preview.from, preview.to) : null);
</script>

{#if booked !== null}
    <DialogShell
        title="Bill booked"
        submitLabel="Done"
        showCancel={false}
        onSubmit={onClose}
        onCancel={onClose}
        testid="pay-dialog"
        initial="[data-testid=pay-dialog-submit]"
    >
        <div class="flex flex-col gap-2 text-sm" data-testid="pay-booked">
            <p>
                <span class="font-semibold">{booked.amount}</span> owed to {booked.vendor}, dated {booked.date}. Reference
                <span class="font-medium" data-testid="pay-booked-reference">{booked.reference}</span>; its {booked.tagged.length === 1
                    ? "line is"
                    : `${booked.tagged.length} lines are`}
                tagged with it.
            </p>
            {#if booked.saved_to_note.length > 0}
                <p data-testid="pay-booked-saved">Saved {savedWords(booked.saved_to_note)} to {noteTitle === "" ? "the note" : `${noteTitle}'s note`}.</p>
            {/if}
            {#if booked.note_created !== null}<p>Created the note {booked.note_created}.</p>{/if}
            <pre class="overflow-x-auto rounded-box bg-base-200 p-2 text-xs" data-testid="pay-booked-text">{booked.text}</pre>
            {#if canOpen}
                <button type="button" class="btn self-start btn-outline btn-sm" onclick={openBooks} data-testid="pay-open-ledgeline">Open in Ledgeline</button>
            {:else}
                <p class="text-base-content/70">It is in the books now.</p>
            {/if}
            {#if openError !== null}<p class="text-xs text-error" role="alert">{openError}</p>{/if}
        </div>
    </DialogShell>
{:else}
    <DialogShell
        title="Pay {name}"
        submitLabel="Create bill"
        busy={creating}
        submitDisabled={preview === null || !preview.ready || pending}
        error={createError ?? previewError ?? loadError}
        onSubmit={() => void create()}
        onCancel={onClose}
        testid="pay-dialog"
        wide
        enterSubmits={false}
        initial="[data-testid=pay-dialog-cancel]"
    >
        <div bind:this={box} class="flex flex-col gap-3">
            {#if preview === null || fields === null}
                {#if loadError === null}<span class="loading loading-sm loading-spinner" aria-label="Working out the bill"></span>{/if}
            {:else}
                <p class="text-sm text-base-content/70" data-testid="pay-summary">
                    {period ?? "Unpaid time"} · {preview.quantity.text}{#if preview.payee !== null}
                        · paid to <span class="font-medium" data-testid="pay-payee">{preview.payee}</span>{/if}
                </p>

                {#if needsNote}
                    <fieldset class="flex flex-col gap-2 rounded-box border border-warning/50 p-3" data-need="note" data-testid="pay-note">
                        <legend class="px-1 text-sm font-medium">Whose note</legend>
                        <p class="text-xs text-base-content/70">No note says whose time “{person}” is. A bill needs a payee: pick their note, or create one.</p>
                        <div class="flex flex-wrap gap-4 text-sm">
                            <label class="flex items-center gap-2"
                                ><input type="radio" class="radio radio-sm" bind:group={fields.noteMode} value="pick" data-testid="pay-note-pick" /> Pick a note</label
                            >
                            <label class="flex items-center gap-2"
                                ><input type="radio" class="radio radio-sm" bind:group={fields.noteMode} value="create" data-testid="pay-note-create" /> Create note…</label
                            >
                        </div>
                        {#if fields.noteMode === "pick"}
                            <Combobox
                                bind:value={fields.pickedNote}
                                options={(need("note")?.candidates ?? []).map((c) => ({value: c, label: c}))}
                                label="Note"
                                placeholder="A person or organization"
                                testid="pay-note-choice"
                            />
                        {:else}
                            <div class="flex flex-wrap items-end gap-2">
                                <label class="flex min-w-0 grow basis-48 flex-col gap-1 text-sm"
                                    >Name
                                    <input class="input w-full input-sm" bind:value={fields.newNote.title} autocomplete="off" data-testid="pay-note-title" />
                                </label>
                                <label class="flex flex-col gap-1 text-sm"
                                    >Kind
                                    <select class="select w-40 select-sm" bind:value={fields.newNote.kind} data-testid="pay-note-kind">
                                        <option value="person">Person</option>
                                        <option value="organization">Organization</option>
                                    </select>
                                </label>
                            </div>
                        {/if}
                    </fieldset>
                {/if}

                <div class="grid gap-2 @2xl:grid-cols-4">
                    <label class="flex min-w-0 flex-col gap-1 text-sm" data-need={isAsked("rate") ? "rate" : undefined}
                        >{needLabel("rate")}
                        <input
                            class={["input w-full tabular-nums input-sm", need("rate") !== null && "input-warning"]}
                            bind:value={fields.rate}
                            placeholder="$85"
                            autocomplete="off"
                            data-testid="pay-rate"
                        />
                        {#if need("rate") !== null && fields.rate.trim() !== ""}<span class="text-xs text-warning">{need("rate")?.message}</span>{/if}
                    </label>
                    <div class="flex min-w-0 flex-col gap-1 text-sm @2xl:col-span-2" data-need={isAsked("expense_account") ? "expense_account" : undefined}>
                        <span aria-hidden="true">{needLabel("expense_account")}</span>
                        <AccountInput
                            bind:value={fields.expense}
                            accountNames={accounts.filter((a) => a.startsWith("expenses"))}
                            label={needLabel("expense_account")}
                            placeholder="expenses:contractors"
                        />
                    </div>
                    <label class="flex min-w-0 flex-col gap-1 text-sm"
                        >Terms (days)
                        <input
                            class="input w-full input-sm"
                            bind:value={fields.terms}
                            inputmode="numeric"
                            placeholder="None"
                            autocomplete="off"
                            data-testid="pay-terms"
                        />
                    </label>
                    <div class="flex min-w-0 flex-col gap-1 text-sm @2xl:col-span-2" data-need={isAsked("payable_account") ? "payable_account" : undefined}>
                        <span aria-hidden="true">{needLabel("payable_account")}</span>
                        <AccountInput
                            bind:value={fields.payable}
                            accountNames={need("payable_account")?.candidates.length
                                ? (need("payable_account")?.candidates ?? [])
                                : accounts.filter((a) => a.startsWith("liabilities"))}
                            label={needLabel("payable_account")}
                            placeholder="liabilities:accounts payable"
                        />
                        {#if need("payable_account") !== null && (need("payable_account")?.candidates.length ?? 0) === 0}
                            <span class="text-xs text-warning">{need("payable_account")?.message}</span>
                        {/if}
                    </div>
                    <label class="flex flex-col gap-1 text-sm"
                        >Date
                        <DateInput bind:value={fields.date} aria-label="Bill date" data-testid="pay-date" />
                    </label>
                    <label class="flex min-w-0 flex-col gap-1 text-sm"
                        >Reference
                        <input class="input w-full input-sm" bind:value={fields.reference} autocomplete="off" data-testid="pay-reference" />
                    </label>
                </div>

                <div class="flex flex-wrap items-center gap-3">
                    <div class="join" role="group" aria-label="Lines">
                        <button
                            type="button"
                            class={["btn join-item btn-sm", fields.lines === "account" ? "btn-active" : "btn-ghost"]}
                            aria-pressed={fields.lines === "account"}
                            onclick={() => fields !== null && (fields.lines = "account")}
                            data-testid="pay-lines-account">A line per account</button
                        >
                        <button
                            type="button"
                            class={["btn join-item btn-sm", fields.lines === "total" ? "btn-active" : "btn-ghost"]}
                            aria-pressed={fields.lines === "total"}
                            onclick={() => fields !== null && (fields.lines = "total")}
                            data-testid="pay-lines-total">One line in total</button
                        >
                    </div>
                    {#if offerSave}
                        <label class="flex items-center gap-2 text-sm"
                            ><input type="checkbox" class="checkbox checkbox-sm" bind:checked={fields.saveToNote} data-testid="pay-save-to-note" />
                            Save to {noteTitle === "" ? "the note" : `${noteTitle}'s note`}</label
                        >
                    {/if}
                </div>

                <table class="table table-sm" data-testid="pay-lines">
                    <thead><tr><th>Time</th><th class="text-right">Hours</th><th class="text-right">Amount</th></tr></thead>
                    <tbody>
                        {#each preview.lines as line, i (i)}
                            <tr data-testid="pay-line"
                                ><td class="break-all">{line.account ?? `${name}'s time`}</td><td class="text-right tabular-nums">{line.hours}</td><td
                                    class="text-right tabular-nums">{line.amount ?? "—"}</td
                                ></tr
                            >
                        {/each}
                    </tbody>
                    <tfoot>
                        <tr
                            ><th
                                >Total{#if preview.due !== null}<span class="ml-2 font-normal text-base-content/60">due {preview.due}</span>{/if}</th
                            ><th class="text-right tabular-nums">{preview.quantity.text}</th><th class="text-right tabular-nums" data-testid="pay-total"
                                >{preview.amount ?? "—"}</th
                            ></tr
                        >
                    </tfoot>
                </table>
                {#each blocking as n (n.field)}<p class="text-sm text-warning" data-testid="pay-need">{n.message}</p>{/each}
            {/if}
        </div>
    </DialogShell>
{/if}
