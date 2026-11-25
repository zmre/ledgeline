<!-- The template editor (invoicer PLAN 4d): a template's name and kinds as
     fields, its HTML and Tera as code, the base's blocks it overrides with
     "Override block", and a live preview of the unsaved text against an
     invoice of the books (the latest by default).

     The preview posts the text 400 ms after typing stops
     (`POST /api/templates/preview`); the server renders it, keeps the page
     under a random id and answers its URL, which the frame loads behind
     MBR's gate (never `srcdoc`: F1, E8). A text that doesn't render keeps
     the last good page and shows Tera's error in plain words with its line
     (Go to line puts the caret there). Stale answers are dropped.

     Save sends only what changed with the content hash read; an outside
     edit meanwhile is refused (409): reload theirs, or keep mine. Save and
     Discard are top right; leaving with unsaved changes asks Save / Discard
     / Cancel. A starter not copied in is read-only: Customize copies it into
     the books, then it is edited here. Duplicate and Delete work on the saved
     file (Delete is refused while invoices or customers use it).

     Keyboard: Cmd/Ctrl+S saves; in the code, Tab indents and Esc then Tab
     moves on. -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {beforeNavigate, goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import {ensureSession, fetchInvoices} from "#lib/api.js";
    import {changes} from "#lib/changes.svelte.js";
    import {EditorApiError} from "#lib/editorApi.js";
    import type {InvoiceEntry} from "#lib/invoices.js";
    import {friendlyMessage} from "#lib/labels.js";
    import {
        TEMPLATE_KINDS,
        baseText,
        createTemplate,
        customizeTemplate,
        definedBlocks,
        deleteTemplate,
        fetchTemplate,
        insertBlock,
        invoiceLabel,
        latestInvoice,
        previewTemplate,
        saveTemplate,
        templateChanges,
        templateDirty,
        templateForm,
        templateOfParam,
        templateParam,
        usageText,
        type TemplateDetail,
        type TemplateForm,
        type TemplateProblem,
    } from "#lib/templates.js";
    import BlockPanel from "#lib/components/BlockPanel.svelte";
    import CodeEditor from "#lib/components/CodeEditor.svelte";
    import ConfirmDialog from "#lib/components/ConfirmDialog.svelte";
    import ConflictNotice from "#lib/components/ConflictNotice.svelte";
    import InvoiceFrame from "#lib/components/InvoiceFrame.svelte";
    import SaveDiscardDialog from "#lib/components/SaveDiscardDialog.svelte";

    const asMessage = (e: unknown): string => friendlyMessage(e instanceof Error ? e.message : String(e));

    const template = $derived(templateOfParam(page.params.path ?? ""));

    let detail = $state<TemplateDetail | null>(null);
    let loadError = $state<string | null>(null);
    const empty: TemplateForm = {name: "", kinds: [], body: ""};
    let original = $state<TemplateForm>({...empty});
    let form = $state<TemplateForm>({...empty});
    let hash = $state("");
    const readOnly = $derived(detail?.read_only ?? true);
    const dirty = $derived(detail !== null && !readOnly && templateDirty(original, form));

    let saving = $state(false);
    let saveError = $state<string | null>(null);
    let savedNote = $state<string | null>(null);
    /** The file changed on disk under unsaved edits: its hash now. */
    let conflict = $state<string | null>(null);
    /** The hash this page last wrote: its own save coming back isn't a conflict. */
    let lastSaved = "";

    let leaving = $state<{to: string} | null>(null);
    let leaveError = $state<string | null>(null);

    let duplicating = $state(false);
    let copyName = $state("");
    let copyError = $state<string | null>(null);
    let deleting = $state(false);
    let deleteError = $state<string | null>(null);
    let customizing = $state(false);

    let editor = $state<CodeEditor | null>(null);
    const defined = $derived(definedBlocks(form.body));
    const extendsBase = $derived(/\{%-?\s*extends\s/.test(form.body));

    // The preview.
    let invoices = $state<InvoiceEntry[]>([]);
    let previewNumber = $state("");
    let previewUrl = $state<string | undefined>(undefined);
    let problem = $state<TemplateProblem | null>(null);
    let previewError = $state<string | null>(null);
    let previewing = $state(false);
    let previewSeq = 0;
    let previewTimer: ReturnType<typeof setTimeout> | undefined;

    function adopt(next: TemplateDetail): void {
        detail = next;
        original = templateForm(next);
        form = templateForm(next);
        hash = next.hash ?? "";
        conflict = null;
        saveError = null;
    }

    async function load(target: string = template): Promise<void> {
        loadError = null;
        try {
            adopt(await fetchTemplate(target));
            schedulePreview(0);
        } catch (e) {
            loadError = asMessage(e);
        }
    }

    async function loadInvoices(): Promise<void> {
        try {
            invoices = await fetchInvoices();
            if (previewNumber === "" || !invoices.some((i) => i.number === previewNumber)) {
                previewNumber = latestInvoice(invoices)?.number ?? "";
            }
            schedulePreview(0);
        } catch (e) {
            previewError = asMessage(e);
        }
    }

    /** Render the form's body against the chosen invoice, `delay` ms from now. */
    function schedulePreview(delay = 400): void {
        clearTimeout(previewTimer);
        previewTimer = setTimeout(() => void renderPreview(), delay);
    }

    async function renderPreview(): Promise<void> {
        if (detail === null || previewNumber === "") return;
        const seq = ++previewSeq;
        previewing = true;
        try {
            const answer = await previewTemplate(previewNumber, template, form.body);
            if (seq !== previewSeq) return;
            previewError = null;
            problem = answer.problem;
            if (answer.url !== null) previewUrl = answer.url;
        } catch (e) {
            if (seq === previewSeq) previewError = asMessage(e);
        } finally {
            if (seq === previewSeq) previewing = false;
        }
    }

    // Typing: preview after a pause.
    let seenBody: string | null = null;
    $effect(() => {
        const body = form.body;
        if (seenBody === null || detail === null) {
            seenBody = body;
            return;
        }
        if (body === seenBody) return;
        seenBody = body;
        untrack(() => schedulePreview());
    });

    /** After a change on disk: adopt it when nothing is unsaved; else say so. */
    async function refresh(): Promise<void> {
        if (saving || detail === null) return;
        void loadInvoices();
        try {
            const next = await fetchTemplate(template);
            if (!dirty) {
                adopt(next);
                schedulePreview(0);
                return;
            }
            if (next.hash !== hash && next.hash !== lastSaved) conflict = next.hash ?? "";
        } catch (e) {
            // Deleted or renamed on disk while open.
            if (!dirty) loadError = asMessage(e);
        }
    }

    async function save(over: string = hash): Promise<boolean> {
        if (detail === null || readOnly) return false;
        saving = true;
        saveError = null;
        savedNote = null;
        try {
            const saved = await saveTemplate(template, over, templateChanges(original, form));
            lastSaved = saved.hash;
            savedNote = saved.written ? "Saved." : "Nothing to save.";
            adopt(await fetchTemplate(template));
            return true;
        } catch (e) {
            if (e instanceof EditorApiError && e.code === "changed") conflict = e.revision ?? "";
            else saveError = asMessage(e);
            return false;
        } finally {
            saving = false;
        }
    }

    function discard(): void {
        form = {...original, kinds: [...original.kinds]};
        schedulePreview(0);
    }

    async function customize(): Promise<void> {
        customizing = true;
        saveError = null;
        try {
            await customizeTemplate(template);
            await load();
        } catch (e) {
            saveError = asMessage(e);
        } finally {
            customizing = false;
        }
    }

    function startDuplicate(): void {
        copyName = `${detail?.name ?? "Template"} copy`;
        copyError = null;
        duplicating = true;
    }

    async function duplicate(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        copyError = null;
        try {
            const made = await createTemplate({name: copyName.trim(), from: template});
            duplicating = false;
            await goto(resolve("/templates/[...path]", {path: templateParam(made.template)}));
        } catch (e) {
            copyError = asMessage(e);
        }
    }

    async function remove(): Promise<void> {
        if (detail === null) return;
        saving = true;
        deleteError = null;
        try {
            await deleteTemplate(template, hash);
            deleting = false;
            await goto(resolve("/templates"));
        } catch (e) {
            deleteError = asMessage(e);
        } finally {
            saving = false;
        }
    }

    function overrideBlock(name: string): void {
        const {body, caret} = insertBlock(form.body, name, editor?.caret() ?? null);
        form.body = body;
        // After the textarea has the new text.
        setTimeout(() => editor?.focusAt(caret), 0);
    }

    function showBlock(name: string): void {
        const at = new RegExp(`\\{%-?\\s*block\\s+${name}\\b`).exec(form.body);
        if (at !== null) editor?.focusAt(at.index);
    }

    function onKeydown(event: KeyboardEvent): void {
        if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
            event.preventDefault();
            if (dirty && !saving) void save();
        }
    }

    function finishLeaving(to: string): void {
        leaving = null;
        leaveError = null;
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- a URL SvelteKit itself was navigating to
        void goto(to);
    }

    async function saveAndLeave(): Promise<void> {
        if (leaving === null) return;
        const to = leaving.to;
        if (await save()) finishLeaving(to);
        else leaveError = saveError ?? (conflict !== null ? "The template changed on disk; reload it or keep yours first." : null);
    }

    function discardAndLeave(): void {
        if (leaving === null) return;
        const to = leaving.to;
        discard();
        finishLeaving(to);
    }

    beforeNavigate((navigation) => {
        if (!dirty || navigation.to === null) return;
        navigation.cancel();
        // Closing the window or reloading: the browser's own prompt.
        if (navigation.type === "leave") return;
        leaving = {to: navigation.to.url.pathname + navigation.to.url.search + navigation.to.url.hash};
    });

    // Another template opened in this page (Duplicate, a link): read it.
    $effect(() => {
        const target = template;
        untrack(() => {
            detail = null;
            previewUrl = undefined;
            problem = null;
            savedNote = null;
            seenBody = null;
            void load(target);
        });
    });

    onMount(() => {
        // The preview frame needs MBR's session cookie.
        ensureSession().catch((e: unknown) => console.warn("session:", e));
        void loadInvoices();
        const warn = (event: BeforeUnloadEvent): void => {
            if (dirty) event.preventDefault();
        };
        window.addEventListener("beforeunload", warn);
        return () => {
            window.removeEventListener("beforeunload", warn);
            clearTimeout(previewTimer);
        };
    });

    let seenSeq = changes.seq;
    $effect(() => {
        const seq = changes.seq;
        if (seq === seenSeq) return;
        seenSeq = seq;
        untrack(() => void refresh());
    });

    const usedInvoices = $derived(detail?.used_by.filter((u) => u.how === "invoice") ?? []);
    const usedCustomers = $derived(detail?.used_by.filter((u) => u.how === "customer") ?? []);
</script>

<svelte:head>
    <title>{detail?.name ?? "Template"} · Ledgeline Invoicer</title>
</svelte:head>

<svelte:window onkeydown={onKeydown} />

<div class="@container mx-auto flex max-w-[110rem] flex-col gap-4" data-testid="template-editor">
    <header class="flex flex-wrap items-center gap-x-3 gap-y-2">
        <a class="link text-sm link-hover" href={resolve("/templates")}>Templates</a>
        <span class="text-base-content/50" aria-hidden="true">/</span>
        <h1 class="min-w-0 truncate text-2xl font-semibold tracking-tight" data-testid="template-heading">{detail?.name ?? "Template"}</h1>
        {#if detail !== null && readOnly}<span class="badge badge-ghost" data-testid="template-read-only">Starter, read only</span>{/if}
        {#if detail !== null}
            <div class="ml-auto flex flex-wrap items-center gap-2">
                {#if readOnly}
                    <button type="button" class="btn btn-ghost btn-sm" onclick={startDuplicate} data-testid="duplicate-template">Duplicate…</button>
                    <button
                        type="button"
                        class="btn btn-primary btn-sm"
                        disabled={customizing}
                        onclick={() => void customize()}
                        data-testid="customize-template"
                    >
                        {#if customizing}<span class="loading loading-xs loading-spinner"></span>{/if}
                        Customize
                    </button>
                {:else}
                    {#if dirty}<span class="text-sm text-base-content/70" data-testid="unsaved">Unsaved changes</span>{/if}
                    <button
                        type="button"
                        class="btn btn-ghost btn-sm"
                        disabled={dirty || saving}
                        title={dirty ? "Save first: it copies the saved template" : undefined}
                        onclick={startDuplicate}
                        data-testid="duplicate-template">Duplicate…</button
                    >
                    <button
                        type="button"
                        class="btn btn-ghost btn-sm"
                        disabled={saving || detail.used_by.length > 0}
                        title={detail.used_by.length > 0 ? "In use: choose another template for its invoices and customers first" : undefined}
                        onclick={() => (deleting = true)}
                        data-testid="delete-template">Delete…</button
                    >
                    <button type="button" class="btn btn-ghost btn-sm" disabled={!dirty || saving} onclick={discard} data-testid="discard-template"
                        >Discard changes</button
                    >
                    <button type="button" class="btn btn-primary btn-sm" disabled={!dirty || saving} onclick={() => void save()} data-testid="save-template">
                        {#if saving}<span class="loading loading-xs loading-spinner"></span>{/if}
                        Save
                    </button>
                {/if}
            </div>
        {/if}
    </header>

    {#if loadError !== null}
        <div role="alert" class="alert alert-soft alert-error" data-testid="template-load-error">
            <span>{loadError}</span>
            <button type="button" class="btn btn-sm" onclick={() => void load()}>Try again</button>
        </div>
    {:else if detail === null}
        <span class="loading loading-md loading-spinner" aria-label="Loading the template"></span>
    {:else}
        {#if duplicating}
            <form class="flex flex-wrap items-end gap-2 rounded-box border border-base-300 p-3" onsubmit={duplicate} data-testid="duplicate-form">
                <label class="flex min-w-0 flex-col gap-1 text-sm">
                    <span class="font-medium">Name of the copy</span>
                    <!-- svelte-ignore a11y_autofocus -->
                    <input class="input w-72 max-w-full input-sm" bind:value={copyName} required autofocus data-testid="duplicate-name" />
                </label>
                <button type="submit" class="btn btn-primary btn-sm" disabled={copyName.trim() === ""} data-testid="duplicate-create">Duplicate</button>
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => (duplicating = false)}>Cancel</button>
                {#if copyError !== null}<p role="alert" class="w-full text-sm text-error" data-testid="duplicate-error">{copyError}</p>{/if}
            </form>
        {/if}
        {#if conflict !== null}
            <ConflictNotice
                theirs={null}
                what="template"
                busy={saving}
                onReload={() => void load()}
                onKeepMine={() => {
                    const over = conflict ?? hash;
                    conflict = null;
                    void save(over);
                }}
            />
        {/if}
        {#if saveError !== null}<div role="alert" class="alert alert-soft alert-error" data-testid="template-save-error">{saveError}</div>{/if}
        {#if savedNote !== null && !dirty}<div role="status" class="text-sm text-success" data-testid="template-saved">{savedNote}</div>{/if}
        {#if readOnly}
            <p class="text-sm text-base-content/70" data-testid="starter-note">
                This starter ships with the app. Customize copies it into your books, where it is yours to change; or Duplicate it under a new name.
            </p>
        {/if}

        <div class="grid gap-4 @5xl:grid-cols-2">
            <div class="flex min-w-0 flex-col gap-3">
                <div class="grid gap-3 @lg:grid-cols-[minmax(0,1fr)_auto]">
                    <label class="flex min-w-0 flex-col gap-1 text-sm">
                        <span class="font-medium">Name</span>
                        <input class="input w-full input-sm" bind:value={form.name} readonly={readOnly} required data-testid="template-name" />
                        <span class="text-xs whitespace-normal text-base-content/60">How pickers show it. The file keeps its name.</span>
                    </label>
                    <fieldset class="flex min-w-0 flex-col gap-1 text-sm">
                        <legend class="mb-1 font-medium">For</legend>
                        <div class="flex flex-wrap gap-x-3 gap-y-1">
                            {#each TEMPLATE_KINDS as kind (kind.id)}
                                <label class="flex items-center gap-1.5">
                                    <input
                                        type="checkbox"
                                        class="checkbox checkbox-sm"
                                        value={kind.id}
                                        bind:group={form.kinds}
                                        disabled={readOnly}
                                        data-testid="template-kind-{kind.id}"
                                    />
                                    {kind.label}
                                </label>
                            {/each}
                        </div>
                    </fieldset>
                </div>
                <p class="text-xs text-base-content/70" data-testid="template-facts">
                    {baseText(detail)} · Used by {usageText(detail).toLowerCase()}{#if usedInvoices.length > 0}: invoices {usedInvoices
                            .map((u) => u.number ?? u.title)
                            .join(", ")}{/if}{#if usedCustomers.length > 0}{usedInvoices.length > 0 ? "; " : ": "}new invoices for {usedCustomers
                            .map((u) => u.title)
                            .join(", ")}{/if}
                </p>
                <BlockPanel blocks={detail.base_blocks} {defined} {extendsBase} readonly={readOnly} onOverride={overrideBlock} onShow={showBlock} />
                {#if problem !== null}
                    <div role="alert" class="alert flex-wrap alert-soft text-sm alert-warning" data-testid="template-problem">
                        <span>
                            {#if problem.line !== null}<strong>Line {problem.line}:</strong>{/if}
                            {problem.message}
                        </span>
                        {#if problem.line !== null}
                            {@const line = problem.line}
                            <button type="button" class="btn btn-ghost btn-xs" onclick={() => editor?.goToLine(line)} data-testid="template-problem-go"
                                >Go to line {line}</button
                            >
                        {/if}
                    </div>
                {/if}
                <CodeEditor
                    bind:this={editor}
                    bind:value={form.body}
                    label="Template code"
                    readonly={readOnly}
                    problemLine={problem?.line ?? null}
                    testid="template-code"
                />
            </div>

            <section class="flex min-w-0 flex-col gap-2 @5xl:sticky @5xl:top-0 @5xl:self-start" aria-label="Preview" data-testid="template-preview">
                <div class="flex flex-wrap items-center gap-2">
                    <label class="text-sm font-medium" for="template-preview-invoice">Preview with</label>
                    <select
                        id="template-preview-invoice"
                        class="select max-w-full select-sm"
                        bind:value={previewNumber}
                        onchange={() => schedulePreview(0)}
                        disabled={invoices.length === 0}
                        data-testid="template-preview-invoice"
                    >
                        {#each invoices.filter((i) => i.number !== null) as invoice (invoice.path)}
                            <option value={invoice.number}>{invoiceLabel(invoice)}</option>
                        {/each}
                    </select>
                    {#if previewing}<span class="loading loading-xs loading-spinner" aria-label="Updating the preview"></span>{/if}
                </div>
                {#if previewError !== null}
                    <div role="alert" class="alert alert-soft text-sm alert-error" data-testid="template-preview-error">{previewError}</div>
                {/if}
                {#if invoices.length === 0}
                    <p class="text-sm text-base-content/70" data-testid="template-preview-none">There's no invoice yet to preview it with.</p>
                {:else if previewUrl !== undefined}
                    <InvoiceFrame src={previewUrl} title="Preview of {detail.name}" mode="preview" />
                {/if}
            </section>
        </div>
    {/if}
</div>

{#if deleting && detail !== null}
    <ConfirmDialog
        title="Delete {detail.name}?"
        confirmLabel="Delete"
        tone="error"
        busy={saving}
        error={deleteError}
        onConfirm={() => void remove()}
        onCancel={() => {
            deleting = false;
            deleteError = null;
        }}
        testid="delete-template-dialog"
    >
        <p>This removes the template from your books. No invoice or customer uses it. The invoice base it builds on stays.</p>
    </ConfirmDialog>
{/if}

{#if leaving !== null}
    <SaveDiscardDialog
        message="This template has unsaved changes. Save them before you leave?"
        {saving}
        error={leaveError}
        onSave={() => void saveAndLeave()}
        onDiscard={discardAndLeave}
        onCancel={() => {
            leaving = null;
            leaveError = null;
        }}
    />
{/if}
