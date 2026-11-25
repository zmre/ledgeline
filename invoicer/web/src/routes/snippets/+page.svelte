<!-- Snippets (invoicer PLAN 4c, ARCHITECTURE §3.7): payment instructions,
     terms, late-fee wording, thank-yous. The list on the left (with how each
     is used), the chosen one's form on the right; New snippet asks a title
     and a kind and writes the note at once in `<templates-dir>/snippets/`.

     Saving sends only what changed with the note's content hash: the title
     and kind are edited by key (a rename keeps the old title as an alias, so
     customers' links still resolve), the body replaced, and an outside edit
     made meanwhile is refused (409): reload theirs, or keep mine. Delete asks
     first, and says which customers name it as a default.

     New snippet is top right once there is one (H21), else the empty
     state's button; `?new=1` (the sidebar's New menu, H20) opens the form
     on arrival and drops the parameter.

     Keyboard (PLAN 5k): Cmd/Ctrl+S saves; in the list, `j`/`k` move a
     cursor and Enter or `o` opens the snippet. -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {beforeNavigate, goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import {changes} from "#lib/changes.svelte.js";
    import {routeParam} from "#lib/customers.js";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {CURSOR_ROW, rowKeys} from "#lib/keys/rowKeys.svelte.js";
    import {EditorApiError} from "#lib/editorApi.js";
    import {errorText, snippetFieldList} from "#lib/labels.js";
    import {
        SNIPPET_KINDS,
        createSnippet,
        deleteSnippet,
        fetchSnippets,
        namingCustomers,
        saveSnippet,
        snippetChanges,
        snippetDirty,
        snippetForm,
        usageText,
        type SnippetEntry,
        type SnippetForm,
        type SnippetUse,
    } from "#lib/library.js";
    import ConfirmDialog from "#lib/components/ConfirmDialog.svelte";
    import ConflictNotice from "#lib/components/ConflictNotice.svelte";
    import EmptyState from "#lib/components/EmptyState.svelte";
    import SnippetEditor from "#lib/components/SnippetEditor.svelte";

    let list = $state<SnippetEntry[] | null>(null);
    let loadError = $state<string | null>(null);
    let chosen = $state<string | null>(null);
    const entry = $derived<SnippetEntry | null>(list?.find((s) => s.path === chosen) ?? null);

    /** The form as read and as edited, and the hash it was read at. */
    let original = $state<SnippetForm>({title: "", kind: "", body: ""});
    let form = $state<SnippetForm>({title: "", kind: "", body: ""});
    let hash = $state("");
    const dirty = $derived(entry !== null && snippetDirty(original, form));

    let saving = $state(false);
    let saveError = $state<string | null>(null);
    let savedNote = $state<string | null>(null);
    /** The note changed on disk under unsaved edits: its hash now. */
    let conflict = $state<string | null>(null);
    /** The hash this page last wrote: its own save coming back isn't a conflict. */
    let lastSaved = "";

    let creating = $state(false);
    let newTitle = $state("");
    let newKind = $state<string>(SNIPPET_KINDS[0]);
    let createError = $state<string | null>(null);

    let deleting = $state(false);
    let deleteError = $state<string | null>(null);

    function select(path: string | null, from: SnippetEntry[] | null = list): void {
        chosen = path;
        const found = from?.find((s) => s.path === path) ?? null;
        original = found === null ? {title: "", kind: "", body: ""} : snippetForm(found);
        form = {...original};
        hash = found?.hash ?? "";
        conflict = null;
        saveError = null;
    }

    /**
     * Read the list and select `keep`. After a save, `sent` is the form it
     * sent: anything typed since stays in the form (unsaved).
     */
    async function load(keep: string | null = chosen, sent?: string): Promise<void> {
        loadError = null;
        try {
            const next = await fetchSnippets();
            const typed = sent === undefined || JSON.stringify(form) === sent ? null : {...form};
            list = next;
            select(next.some((s) => s.path === keep) ? keep : (next[0]?.path ?? null), next);
            if (typed !== null && chosen === keep) form = typed;
        } catch (e) {
            loadError = errorText(e);
        }
    }

    function choose(path: string): void {
        if (path === chosen) return;
        if (dirty && !confirm("This snippet has unsaved changes. Switch without saving?")) return;
        savedNote = null;
        select(path);
    }

    /** After a change on disk: adopt it when nothing is unsaved; else say so. */
    async function refresh(): Promise<void> {
        if (saving) return;
        try {
            const next = await fetchSnippets();
            const theirs = next.find((s) => s.path === chosen);
            if (!dirty || theirs === undefined) {
                list = next;
                select(theirs === undefined ? (next[0]?.path ?? null) : chosen, next);
                return;
            }
            // Keep the edits and the hash they were read at; the uses refresh.
            list = next;
            if (theirs.hash !== hash && theirs.hash !== lastSaved) conflict = theirs.hash;
        } catch (e) {
            console.warn("snippets:", e);
        }
    }

    async function save(over: string = hash): Promise<void> {
        if (entry === null) return;
        saving = true;
        saveError = null;
        savedNote = null;
        const sent = JSON.stringify(form);
        try {
            const saved = await saveSnippet(entry.path, over, snippetChanges(original, form));
            lastSaved = saved.hash;
            savedNote = saved.written ? `Saved: ${snippetFieldList(saved.changed)}.` : "Nothing to save.";
            await load(saved.path, sent);
        } catch (e) {
            if (e instanceof EditorApiError && e.code === "changed") conflict = e.revision ?? "";
            else saveError = errorText(e);
        } finally {
            saving = false;
        }
    }

    async function create(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        createError = null;
        try {
            const made = await createSnippet({title: newTitle, kind: newKind});
            creating = false;
            newTitle = "";
            savedNote = null;
            await load(made.path);
        } catch (e) {
            createError = errorText(e);
        }
    }

    function startCreate(): void {
        if (dirty && !confirm("This snippet has unsaved changes. Start a new one without saving?")) return;
        creating = true;
        createError = null;
    }

    // `?new=1` from the New menu: drop the parameter, then open the form.
    let handlingNew = false;
    $effect(() => {
        if (!page.url.searchParams.has("new") || handlingNew) return;
        handlingNew = true;
        untrack(() => {
            void goto(resolve("/snippets"), {replace: true, reset: false})
                .then(() => startCreate())
                .finally(() => (handlingNew = false));
        });
    });

    async function remove(force: boolean): Promise<void> {
        if (entry === null) return;
        saving = true;
        deleteError = null;
        try {
            await deleteSnippet(entry.path, hash, force);
            deleting = false;
            savedNote = null;
            await load(null);
        } catch (e) {
            deleteError = errorText(e);
        } finally {
            saving = false;
        }
    }

    // Cmd/Ctrl+S saves, from a field too; with nothing to save it still
    // keeps the browser's Save Page dialog away.
    registerKeys({
        id: "snippet-page",
        bindings: [
            {
                keys: "mod+s",
                label: "Save the snippet",
                group: "Snippets",
                always: true,
                run: () => {
                    if (dirty && !saving) void save();
                },
            },
        ],
    });

    let listEl = $state<HTMLUListElement | null>(null);
    const cursor = rowKeys<SnippetEntry>({
        id: "snippet-list",
        group: "Snippets",
        noun: "snippet",
        items: () => list ?? [],
        keyOf: (snippet) => snippet.path,
        root: () => listEl,
        open: () => (snippet) => choose(snippet.path),
    });

    beforeNavigate(({cancel, to}) => {
        // The same page with another query (the New menu's `?new=1`) isn't leaving.
        if (to?.url.pathname === page.url.pathname) return;
        if (dirty && to !== null && !confirm("This snippet has unsaved changes. Leave without saving?")) cancel();
    });

    onMount(() => {
        void load();
        const warn = (event: BeforeUnloadEvent): void => {
            if (dirty) event.preventDefault();
        };
        window.addEventListener("beforeunload", warn);
        return () => window.removeEventListener("beforeunload", warn);
    });

    let seenSeq = changes.seq;
    $effect(() => {
        const seq = changes.seq;
        if (seq === seenSeq) return;
        seenSeq = seq;
        untrack(() => void refresh());
    });

    const customerHref = (use: SnippetUse): string => resolve("/customers/[...path]", {path: routeParam(use.path)});
    const invoiceHref = (use: SnippetUse): string => resolve("/invoices/[number]", {number: use.number ?? use.path});
</script>

<svelte:head>
    <title>Snippets · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-7xl flex-col gap-4" data-testid="snippets-page">
    <header class="flex flex-wrap items-center gap-x-3 gap-y-2">
        <h1 class="text-2xl font-semibold tracking-tight">Snippets</h1>
        {#if (list?.length ?? 0) > 0}
            <button type="button" class="btn ml-auto btn-primary btn-sm" onclick={startCreate} data-testid="new-snippet">New snippet</button>
        {/if}
    </header>
    {#if (list?.length ?? 0) > 0}
        <p class="text-sm text-base-content/70">
            Payment instructions, terms and notes an invoice can copy. A customer's default snippets start each of its new invoices.
        </p>
    {/if}

    {#if creating}
        <form class="flex flex-wrap items-end gap-2 rounded-box border border-base-300 p-3" onsubmit={create} data-testid="new-snippet-form">
            <label class="flex flex-col gap-1 text-sm">
                <span class="font-medium">Title</span>
                <!-- svelte-ignore a11y_autofocus -->
                <input class="input w-72 input-sm" bind:value={newTitle} required autofocus data-testid="new-snippet-title" />
            </label>
            <label class="flex flex-col gap-1 text-sm">
                <span class="font-medium">Kind</span>
                <select class="select select-sm" bind:value={newKind} data-testid="new-snippet-kind">
                    {#each SNIPPET_KINDS as kind (kind)}<option value={kind}>{kind}</option>{/each}
                </select>
            </label>
            <button type="submit" class="btn btn-primary btn-sm" disabled={newTitle.trim() === ""} data-testid="create-snippet">Create</button>
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => (creating = false)}>Cancel</button>
            {#if createError !== null}<p role="alert" class="w-full text-sm text-error" data-testid="new-snippet-error">{createError}</p>{/if}
        </form>
    {/if}

    {#if loadError !== null}
        <div role="alert" class="alert alert-soft alert-error" data-testid="snippets-load-error">
            <span>{loadError}</span>
            <button type="button" class="btn btn-sm" onclick={() => void load()}>Try again</button>
        </div>
    {:else if list === null}
        <span class="loading loading-md loading-spinner" aria-label="Loading the snippets"></span>
    {:else if list.length === 0}
        {#if !creating}
            <EmptyState
                title="No snippets yet"
                text="Payment instructions, terms or a thank-you, written once for invoices to copy."
                action="New snippet"
                onAction={startCreate}
                testid="snippets-empty"
                actionTestid="snippets-empty-new"
            />
        {/if}
    {:else}
        <div class="grid gap-4 md:grid-cols-[minmax(0,1fr)_minmax(0,3fr)]">
            <nav aria-label="Snippets">
                <ul class="menu w-full gap-0.5 rounded-box border border-base-300 p-1" bind:this={listEl} data-testid="snippet-list">
                    {#each list as snippet, at (snippet.path)}
                        <li data-cursor={cursor.index === at ? "" : undefined}>
                            <button
                                type="button"
                                class="{snippet.path === chosen ? 'menu-active' : ''} {cursor.index === at ? CURSOR_ROW : ''}"
                                aria-current={snippet.path === chosen ? "true" : undefined}
                                onclick={() => choose(snippet.path)}
                                data-testid="snippet-item"
                                data-path={snippet.path}
                            >
                                <span class="flex min-w-0 flex-col items-start">
                                    <span class="truncate font-medium">{snippet.title}</span>
                                    <span class="text-xs text-base-content/60">{snippet.kind ?? "—"} · {usageText(snippet)}</span>
                                </span>
                            </button>
                        </li>
                    {/each}
                </ul>
            </nav>

            {#if entry !== null}
                {#key entry.path}
                    <section class="flex flex-col gap-3" aria-label={entry.title}>
                        <div class="flex flex-wrap items-center gap-2">
                            <h2 class="text-lg font-semibold" data-testid="snippet-heading">{entry.title}</h2>
                            <span class="text-xs text-base-content/60" title="The note in the books folder">{entry.path}</span>
                            <div class="ml-auto flex items-center gap-2">
                                {#if dirty}<span class="text-sm text-base-content/70" data-testid="unsaved">Unsaved changes</span>{/if}
                                <button
                                    type="button"
                                    class="btn btn-ghost btn-sm"
                                    disabled={saving}
                                    onclick={() => (deleting = true)}
                                    data-testid="delete-snippet">Delete…</button
                                >
                                <button type="button" class="btn btn-ghost btn-sm" disabled={!dirty || saving} onclick={() => (form = {...original})}
                                    >Discard</button
                                >
                                <button
                                    type="button"
                                    class="btn btn-primary btn-sm"
                                    disabled={!dirty || saving}
                                    onclick={() => void save()}
                                    data-testid="save-snippet">{saving ? "Saving…" : "Save"}</button
                                >
                            </div>
                        </div>
                        {#if conflict !== null}
                            <ConflictNotice
                                theirs={null}
                                what="snippet"
                                busy={saving}
                                onReload={() => void load()}
                                onKeepMine={() => {
                                    const over = conflict ?? hash;
                                    conflict = null;
                                    void save(over);
                                }}
                            />
                        {/if}
                        {#if saveError !== null}<div role="alert" class="alert alert-soft alert-error" data-testid="snippet-save-error">{saveError}</div>{/if}
                        {#if savedNote !== null && !dirty}<div role="status" class="text-sm text-success" data-testid="snippet-saved">{savedNote}</div>{/if}
                        <SnippetEditor bind:form {entry} {customerHref} {invoiceHref} />
                    </section>
                {/key}
            {/if}
        </div>
    {/if}
</div>

{#if deleting && entry !== null}
    {@const naming = namingCustomers(entry)}
    <ConfirmDialog
        title="Delete {entry.title}?"
        confirmLabel={naming.length > 0 ? "Delete anyway" : "Delete"}
        tone="error"
        busy={saving}
        error={deleteError}
        onConfirm={() => void remove(naming.length > 0)}
        onCancel={() => {
            deleting = false;
            deleteError = null;
        }}
        testid="delete-snippet-dialog"
    >
        <p>This removes {entry.path}. Invoices keep their copies of its text.</p>
        {#if naming.length > 0}
            <p class="mt-2">
                {naming.map((u) => u.title).join(", ")}
                {naming.length === 1 ? "names" : "name"} it as a default snippet: their new invoices will no longer get it, and the link in their note will point
                nowhere.
            </p>
        {/if}
    </ConfirmDialog>
{/if}
