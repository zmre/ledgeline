<!-- The invoice editor (invoicer PLAN 4a, ARCHITECTURE §6.3): the header, the
     prose above the bill table, the bill grid, the prose below; the live
     preview and the Books panel beside them when the page is wide, below
     them when it is narrow (a container query on the page). Save, Discard and
     the save state are top right, as on the customer page; Send…, Void…,
     Re-book… and the Send panel (PDF) show only while nothing is unsaved
     (Phase 4 feedback H14): save, then send. State and the save and conflict
     rules are lib/invoiceEditor.svelte.ts; every write goes through the
     server, which writes through invoicer-core like the CLI.

     Keyboard (PLAN 5k): Cmd/Ctrl+S saves; Shift+D discards (after asking),
     Shift+S sends, Shift+R re-books and Shift+V voids, each opening its
     confirmation; `c` and `t` go to the customer and template fields; the
     bill grid adds and moves rows (BillGrid). Leaving with unsaved changes (a link, Back)
     asks Save / Discard / Cancel (H13). Steps that touch the books ask first
     (ARCHITECTURE §4): Send books it; Void reverses it; Re-book replaces its
     transaction; saving a sent invoice re-books it. -->
<script lang="ts">
    import {onDestroy, onMount, untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import DateInput from "#ui/components/DateInput.svelte";
    import {hint} from "#ui/keys/hint.svelte.js";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {fetchDelivery, fetchTemplates, sendInvoice} from "#lib/api.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {unsavedGuard} from "#lib/unsavedGuard.svelte.js";
    import {defaultVia, sendBlocker, VIA_CHOICES, type Delivery} from "#lib/delivery.js";
    import {DEFAULT_WORDS, isDepositTable, totalWords, type EditorForm} from "#lib/editor.js";
    import {fetchDeposits, fetchEditorOptions, fetchFieldValues, rebookInvoice, voidInvoice} from "#lib/editorApi.js";
    import {InvoiceEditor} from "#lib/invoiceEditor.svelte.js";
    import {localToday, pageUrl, statusLabel, type TemplateEntry} from "#lib/invoices.js";
    import {errorText, friendlyMessage} from "#lib/labels.js";
    import AddCustomerDialog from "#lib/components/AddCustomerDialog.svelte";
    import BillGrid from "#lib/components/BillGrid.svelte";
    import ConfirmDialog from "#lib/components/ConfirmDialog.svelte";
    import ConflictNotice from "#lib/components/ConflictNotice.svelte";
    import HeaderForm from "#lib/components/HeaderForm.svelte";
    import InvoiceFrame from "#lib/components/InvoiceFrame.svelte";
    import ProseField from "#lib/components/ProseField.svelte";
    import SendSection from "#lib/components/SendSection.svelte";
    import SaveDiscardDialog from "#lib/components/SaveDiscardDialog.svelte";
    import WouldBookPanel from "#lib/components/WouldBookPanel.svelte";

    const editor = new InvoiceEditor();
    const number = $derived(page.params.number ?? "");

    let templates = $state<TemplateEntry[]>([]);
    let addingCustomer = $state<string | null>(null);
    let sendReload = $state(0);

    $effect(() => {
        const target = number;
        untrack(() => {
            if (target !== editor.number || editor.form === null) void editor.load(target);
        });
    });

    onMount(() => {
        fetchEditorOptions()
            .then((o) => (editor.options = o))
            .catch((e: unknown) => console.warn("editor options:", e));
        fetchTemplates()
            .then((t) => (templates = t))
            .catch((e: unknown) => console.warn("templates:", e));
    });
    onDestroy(() => editor.dispose());

    // Leaving with unsaved edits: Save / Discard / Cancel. A sent invoice's
    // save asks to re-book first; that confirmation takes over from there.
    // A save that wrote the file but didn't re-book it stays put with why.
    const guard = unsavedGuard({
        dirty: () => editor.dirty && !readonly,
        save: async () => {
            const after = await save();
            if (editor.confirmRebook) return "asked";
            return after !== null && !editor.dirty ? "saved" : "failed";
        },
        discard: () => editor.discard(),
        saveError: () => editor.saveError,
        conflict: () => editor.conflict !== null,
        noun: "invoice",
    });

    // A change on disk (lib/events.ts): this file, or anything when `paths` is [].
    onFilesChanged((paths) => {
        void editor.diskChanged(paths);
        sendReload += 1;
        // The pickers follow the books: a catalog item or customer added elsewhere is offered at once.
        fetchEditorOptions()
            .then((o) => (editor.options = o))
            .catch((e: unknown) => console.warn("editor options:", e));
    });

    const edit = (form: EditorForm): void => editor.edit(form);
    const words = $derived(editor.options?.words ?? DEFAULT_WORDS);
    const labels = $derived(totalWords(editor.options, words));
    const deposit = $derived(isDepositTable(editor.form?.table ?? null, editor.options?.deposit_account));
    const commodity = $derived(editor.computed?.commodity ?? "$");
    const readonly = $derived(editor.status === "void");

    /** Save (a sent invoice: and re-book it); the number afterwards, or null when it didn't save. */
    async function save(rebook = false): Promise<string | null> {
        const before = editor.number;
        const after = await editor.save({rebook});
        if (after !== null && after !== before) await goto(resolve("/invoices/[number]/edit", {number: after}), {replaceState: true});
        if (after !== null) sendReload += 1;
        return after;
    }

    // Send / Void / Re-book, each after a confirmation.
    let dialog = $state<"send" | "void" | "rebook" | null>(null);
    let stepBusy = $state(false);
    let stepError = $state<string | null>(null);
    let notice = $state<string | null>(null);
    let delivery = $state<Delivery | null>(null);
    let via = $state("email");
    let voidDate = $state(localToday());

    async function ask(step: "send" | "void" | "rebook"): Promise<void> {
        stepError = null;
        notice = null;
        dialog = step;
        if (step === "void") voidDate = localToday();
        if (step === "send") {
            try {
                delivery = await fetchDelivery(editor.number);
                via = defaultVia(delivery);
            } catch (e) {
                stepError = errorText(e);
            }
        }
    }

    async function run(step: "send" | "void" | "rebook"): Promise<void> {
        stepBusy = true;
        stepError = null;
        try {
            if (step === "send") await sendInvoice(editor.number, via);
            if (step === "void") await voidInvoice(editor.number, voidDate);
            if (step === "rebook") await rebookInvoice(editor.number);
            dialog = null;
            notice =
                step === "send"
                    ? `Invoice ${editor.number} is sent.`
                    : step === "void"
                      ? `Invoice ${editor.number} is void.`
                      : `Invoice ${editor.number} is re-booked.`;
            await editor.load(editor.number);
            sendReload += 1;
        } catch (e) {
            stepError = errorText(e);
        } finally {
            stepBusy = false;
        }
    }

    // Discard from the keyboard asks first: a stray Shift+D shouldn't lose
    // an afternoon's edits (the button, aimed, doesn't ask).
    let confirmDiscard = $state(false);

    const loaded = (): boolean => editor.form !== null;
    const saved = (): boolean => loaded() && !editor.dirty && !readonly;
    registerKeys({
        id: "invoice-editor",
        bindings: [
            // `always`: from a field too, and with nothing to save it still
            // keeps the browser's Save Page dialog away (as before PLAN 5k).
            {
                keys: "mod+s",
                label: "Save the invoice",
                group: "Invoice editor",
                always: true,
                run: () => {
                    if (editor.dirty && !readonly && !editor.saving) void save();
                },
            },
            {
                keys: "D",
                label: "Discard the changes…",
                group: "Invoice editor",
                enabled: () => loaded() && editor.dirty && !readonly && !editor.saving,
                run: () => (confirmDiscard = true),
            },
            {keys: "S", label: "Send…", group: "Invoice editor", enabled: () => saved() && editor.status === "draft", run: () => void ask("send")},
            {keys: "R", label: "Re-book…", group: "Invoice editor", enabled: () => saved() && editor.status === "sent", run: () => void ask("rebook")},
            {keys: "V", label: "Void…", group: "Invoice editor", enabled: saved, run: () => void ask("void")},
        ],
    });

    const accrual = $derived(editor.books?.basis !== "cash");
    const blocker = $derived(delivery === null ? null : sendBlocker(delivery));
    const linkBase = $derived(editor.path === "" ? "/" : pageUrl(editor.path).replace(/[^/]+\/$/, ""));
</script>

<svelte:head>
    <title>Edit {editor.title || `invoice ${number}`} · Ledgeline Invoicer</title>
</svelte:head>

<div class="@container mx-auto flex max-w-[110rem] flex-col gap-4">
    <a class="link text-sm link-hover" href={resolve("/invoices/[number]", {number})}>← Invoice {number}</a>

    {#if editor.loadError !== null}
        <div class="alert alert-error" role="alert" data-testid="editor-error">
            Couldn't open invoice {number} for editing: {errorText(editor.loadError)}
            <button type="button" class="btn btn-sm" onclick={() => void editor.load(number)}>Try again</button>
        </div>
    {:else if editor.form === null}
        <span class="loading loading-md loading-spinner" aria-label="Loading the invoice"></span>
    {:else}
        {@const form = editor.form}
        <header class="flex flex-wrap items-center gap-x-3 gap-y-2">
            <h1 class="text-2xl font-semibold tracking-tight" data-testid="editor-heading">Invoice {editor.number}</h1>
            <span class="badge badge-sm" data-testid="editor-status">{statusLabel(editor.status)}</span>
            {#if editor.computed?.total}
                <span class="text-lg font-semibold tabular-nums {editor.pending ? 'text-base-content/60 italic' : ''}" data-testid="editor-total"
                    >{editor.computed.total}</span
                >
            {/if}
            <div class="ml-auto flex items-center gap-2">
                <span class="text-sm text-base-content/70" aria-live="polite" data-testid="editor-saved">
                    {#if editor.saving}Saving…{:else if editor.dirty}Unsaved changes{:else if editor.savedAt !== null}Saved{/if}
                </span>
                {#if !readonly}
                    <button
                        type="button"
                        class="btn btn-ghost btn-sm"
                        disabled={!editor.dirty || editor.saving}
                        onclick={() => editor.discard()}
                        data-testid="editor-discard">Discard changes</button
                    >
                    <button
                        type="button"
                        class="btn btn-primary btn-sm"
                        disabled={!editor.dirty || editor.saving}
                        onclick={() => void save()}
                        title={hint("Save", "⌘/Ctrl+S")}
                        data-testid="editor-save">Save</button
                    >
                {/if}
            </div>
        </header>

        {#if notice !== null}<div class="alert text-sm alert-success" role="status" data-testid="editor-notice">{notice}</div>{/if}
        <!-- What went wrong once New wrote the draft (its starter template not copied in): the draft is here either way. -->
        {#each page.state.createWarnings ?? [] as warning, i (i)}
            <div class="alert text-sm alert-warning" role="status" data-testid="editor-create-warning">{errorText(warning)}</div>
        {/each}
        {#if editor.saveError !== null}<div class="alert text-sm alert-error" role="alert" data-testid="editor-save-error">{editor.saveError}</div>{/if}
        {#if editor.conflict !== null}
            <ConflictNotice
                theirs={editor.conflict.computed.text}
                busy={editor.saving}
                onReload={() => editor.reloadTheirs()}
                onKeepMine={() => void editor.keepMine()}
            />
        {/if}
        {#if readonly}<div class="alert text-sm" role="status">This invoice is void, so it can't be edited.</div>{/if}

        {#if editor.dirty}
            <p class="text-sm text-base-content/60" data-testid="editor-save-to-send">Save your changes to send it, export its PDF or void it.</p>
        {:else}
            {#if !readonly}
                <div class="flex flex-wrap gap-2" role="toolbar" aria-label="Invoice actions">
                    {#if editor.status === "draft"}
                        <button type="button" class="btn btn-sm" onclick={() => void ask("send")} data-testid="editor-send">Send…</button>
                    {/if}
                    {#if editor.status === "sent"}
                        <button type="button" class="btn btn-sm" onclick={() => void ask("rebook")} data-testid="editor-rebook">Re-book…</button>
                    {/if}
                    <button type="button" class="btn btn-outline btn-error btn-sm" onclick={() => void ask("void")} data-testid="editor-void">Void…</button>
                </div>
            {/if}
            <SendSection number={editor.number} reload={sendReload} onChanged={() => void editor.load(editor.number)} />
        {/if}

        <div class="grid gap-4 @7xl:grid-cols-2">
            <div class="flex min-w-0 flex-col gap-4">
                <HeaderForm
                    {form}
                    options={editor.options}
                    {templates}
                    status={editor.status}
                    {deposit}
                    onChange={edit}
                    onAddCustomer={(name) => (addingCustomer = name)}
                    fieldValues={fetchFieldValues}
                />
                <div class="flex flex-col gap-1 text-sm">
                    <span aria-hidden="true">Above the table</span>
                    <ProseField
                        value={form.prose_before}
                        label="Prose above the table"
                        placeholder="A note to the customer"
                        {readonly}
                        {linkBase}
                        onChange={(v) => edit({...form, prose_before: v})}
                        testid="prose-before"
                    />
                </div>
                {#if form.table !== null}
                    <BillGrid
                        table={form.table}
                        view={editor.computed?.table ?? null}
                        local={editor.local}
                        pending={editor.pending}
                        accounts={editor.options?.accounts ?? []}
                        catalog={editor.options?.catalog ?? []}
                        {words}
                        {labels}
                        {commodity}
                        {readonly}
                        loadDeposits={() => fetchDeposits(editor.number)}
                        onChange={(table) => edit({...form, table})}
                    />
                {:else}
                    <p class="alert text-sm alert-warning">This invoice has no bill table (a table with an {words.amount} column). Add one in the file.</p>
                {/if}
                {#if editor.computed !== null && (editor.computed.table_error !== null || editor.computed.problems.length > 0)}
                    <ul class="flex flex-col gap-1 text-sm" aria-label="Problems" data-testid="editor-problems">
                        {#if editor.computed.table_error !== null}<li class="text-error-text">{friendlyMessage(editor.computed.table_error)}</li>{/if}
                        {#each editor.computed.problems as problem, p (p)}
                            <li class={problem.severity === "error" ? "text-error-text" : "text-warning"}>{friendlyMessage(problem.message)}</li>
                        {/each}
                    </ul>
                {/if}
                <div class="flex flex-col gap-1 text-sm">
                    <span aria-hidden="true">Below the table</span>
                    <ProseField
                        value={form.prose_after}
                        label="Prose below the table"
                        placeholder="Payment details, thanks"
                        {readonly}
                        {linkBase}
                        onChange={(v) => edit({...form, prose_after: v})}
                        testid="prose-after"
                    />
                </div>
            </div>
            <div class="flex min-w-0 flex-col gap-4 @7xl:sticky @7xl:top-0 @7xl:self-start">
                <section aria-label="Preview" class="flex flex-col gap-1">
                    <h2 class="text-sm font-medium text-base-content/70">Preview{editor.dirty ? " (unsaved)" : ""}</h2>
                    {#if editor.previewUrl !== null}
                        {#key editor.previewUrl}
                            <InvoiceFrame src={editor.previewUrl} title="{editor.title}, previewed" mode="preview" />
                        {/key}
                    {/if}
                </section>
                <WouldBookPanel books={editor.books} status={editor.status} dirty={editor.dirty} onRebook={() => void ask("rebook")} />
            </div>
        </div>
    {/if}
</div>

{#if addingCustomer !== null && editor.form !== null}
    <AddCustomerDialog
        name={addingCustomer}
        onCancel={() => (addingCustomer = null)}
        onCreated={(customer) => {
            addingCustomer = null;
            if (editor.form !== null) edit({...editor.form, customer: customer.link});
            fetchEditorOptions()
                .then((o) => (editor.options = o))
                .catch(() => undefined);
        }}
    />
{/if}

{#if guard.pending !== null}
    <SaveDiscardDialog
        title="Leave invoice {editor.number}?"
        message="Your changes aren't saved. Save them before leaving, or discard them?"
        saving={editor.saving}
        error={guard.error}
        onSave={() => void guard.save()}
        onDiscard={() => void guard.discard()}
        onCancel={() => guard.cancel()}
    />
{/if}

{#if editor.confirmRebook}
    <ConfirmDialog
        title="Save and re-book invoice {editor.number}?"
        confirmLabel="Save and re-book"
        tone="warning"
        busy={editor.saving}
        error={editor.saveError}
        onConfirm={() => void save(true)}
        onCancel={() => (editor.confirmRebook = false)}
        testid="confirm-save-rebook"
    >
        <p>
            Invoice {editor.number} is sent, so its amount is booked. Saving these edits replaces its transaction in the books and restates any recognition schedule.
        </p>
        <p>An invoice dated on or before the books' closed-through date can't be re-booked; void it and issue a new one instead.</p>
    </ConfirmDialog>
{/if}

{#if confirmDiscard}
    <ConfirmDialog
        title="Discard the changes to invoice {editor.number}?"
        confirmLabel="Discard changes"
        tone="error"
        onConfirm={() => {
            confirmDiscard = false;
            editor.discard();
        }}
        onCancel={() => (confirmDiscard = false)}
        testid="confirm-discard"
    >
        <p>The invoice goes back to how it was last saved.</p>
    </ConfirmDialog>
{/if}

{#if dialog === "send"}
    <ConfirmDialog
        title="Send invoice {editor.number}?"
        confirmLabel="Mark as sent"
        busy={stepBusy}
        error={stepError ?? blocker}
        onConfirm={() => void run("send")}
        onCancel={() => (dialog = null)}
        testid="confirm-send"
    >
        {#if accrual}
            <p>
                It books {editor.computed?.total ?? "its total"} to {editor.books?.receivable ?? "accounts receivable"}, dated {editor.form?.date}, and marks it
                sent. Drafts never touch the books; this does.
            </p>
            {#if editor.books?.would_book}<pre class="overflow-x-auto rounded-box bg-base-200 p-2 text-xs">{editor.books.would_book}</pre>{/if}
        {:else}
            <p>It marks the invoice sent. Cash books take it when its payment is applied.</p>
        {/if}
        <label class="flex items-center gap-2"
            >Sent by
            <select class="select w-40 select-sm" bind:value={via} aria-label="Sent by">
                {#each VIA_CHOICES as choice (choice)}<option value={choice}>{choice}</option>{/each}
            </select>
        </label>
        <p class="text-base-content/70">Export its PDF from the Send panel afterwards.</p>
    </ConfirmDialog>
{/if}

{#if dialog === "void"}
    <ConfirmDialog
        title="Void invoice {editor.number}?"
        confirmLabel="Void it"
        tone="error"
        busy={stepBusy}
        error={stepError}
        onConfirm={() => void run("void")}
        onCancel={() => (dialog = null)}
        testid="confirm-void"
    >
        {#if editor.status === "draft"}
            <p>A draft is in no books: this only marks the file void.</p>
        {:else if accrual}
            <p>It writes a reversing entry dated the void date, linked to the original, so the receivable nets to zero, and marks the file void.</p>
        {:else}
            <p>Cash books: it marks the file void. An invoice with a payment applied can't be voided until that payment is undone.</p>
        {/if}
        <label class="flex items-center gap-2">Void date <DateInput bind:value={voidDate} aria-label="Void date" /></label>
    </ConfirmDialog>
{/if}

{#if dialog === "rebook"}
    <ConfirmDialog
        title="Re-book invoice {editor.number}?"
        confirmLabel="Re-book"
        tone="warning"
        busy={stepBusy}
        error={stepError}
        onConfirm={() => void run("rebook")}
        onCancel={() => (dialog = null)}
        testid="confirm-rebook"
    >
        <p>
            It replaces the invoice's transaction with what the file says now, and restates any recognition schedule. A payment linked to it is checked again.
        </p>
        <p>Refused when the invoice is dated on or before the books' closed-through date.</p>
    </ConfirmDialog>
{/if}
