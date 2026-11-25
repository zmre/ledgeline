<!-- The catalog (invoicer PLAN 4c, ARCHITECTURE §3.6): the catalog note's
     table as a grid. Add, edit, reorder and delete items; the account picker
     and deferral popover are the invoice editor's. Saving sends the grid with
     the note's content hash; the server writes only the table's bytes (the
     frontmatter and any prose around it stay as they are) and refuses an
     outside edit made meanwhile (409): reload theirs, or keep mine (save
     over theirs). Live reload: a change on disk shows at once when nothing is
     unsaved; otherwise the page says so and keeps the edits. The invoice
     editor's catalog picker re-reads the catalog after every change.

     New item (top right, H21) adds a row at the bottom and focuses it; with
     no catalog yet, the empty state's New item writes the catalog note
     first. `?new=1` (the sidebar's New menu, H20) does the same on arrival,
     then drops the parameter so a reload doesn't add another.

     Keyboard (PLAN 5k): Cmd/Ctrl+S saves; in the grid, Alt+↑/↓ moves the
     focused item. -->
<script lang="ts">
    import {onMount, tick, untrack} from "svelte";
    import {beforeNavigate, goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {changes} from "#lib/changes.svelte.js";
    import type {GridForm} from "#lib/editor.js";
    import {EditorApiError, fetchEditorOptions} from "#lib/editorApi.js";
    import {
        catalogProblems,
        cellProblemsOf,
        cloneGrid,
        createCatalog,
        fetchCatalog,
        gridsDiffer,
        saveCatalog,
        type CatalogList,
        type CatalogNote,
        type CellProblem,
    } from "#lib/library.js";
    import CatalogGrid from "#lib/components/CatalogGrid.svelte";
    import ConflictNotice from "#lib/components/ConflictNotice.svelte";
    import EmptyState from "#lib/components/EmptyState.svelte";
    import SaveDiscardDialog from "#lib/components/SaveDiscardDialog.svelte";
    import {errorText, friendlyMessage} from "#lib/labels.js";

    let list = $state<CatalogList | null>(null);
    let loadError = $state<string | null>(null);
    /** The note being edited (a books folder may have more than one catalog). */
    let chosen = $state<string | null>(null);
    const note = $derived<CatalogNote | null>(list?.catalogs.find((c) => c.path === chosen) ?? list?.catalogs[0] ?? null);

    /** The grid as read, and as edited; the hash it was read at. */
    let original = $state<GridForm | null>(null);
    let table = $state<GridForm | null>(null);
    let hash = $state("");
    const dirty = $derived(original !== null && table !== null && gridsDiffer(original, table));

    let accounts = $state<string[]>([]);
    let saving = $state(false);
    let saveError = $state<string | null>(null);
    let serverProblems = $state<CellProblem[]>([]);
    let savedNote = $state<string | null>(null);
    /** The note changed on disk under unsaved edits (a refused save, or live reload): its hash now. */
    let conflict = $state<string | null>(null);
    /** The hash this page last wrote: its own save coming back isn't a conflict. */
    let lastSaved = "";

    /** What waits on Save / Discard / Cancel over unsaved edits: leaving, or another catalog. */
    let pending = $state<{then: () => void} | null>(null);
    let pendingError = $state<string | null>(null);

    const ownProblems = $derived(table === null ? [] : catalogProblems(table));
    const problems = $derived([...ownProblems, ...serverProblems.filter((p) => !ownProblems.some((o) => o.row === p.row && o.column === p.column))]);

    function adopt(next: CatalogList): void {
        list = next;
        const current = next.catalogs.find((c) => c.path === chosen) ?? next.catalogs[0] ?? null;
        chosen = current?.path ?? null;
        original = current?.table ? cloneGrid(current.table) : null;
        table = current?.table ? cloneGrid(current.table) : null;
        hash = current?.hash ?? "";
        conflict = null;
        serverProblems = [];
        saveError = null;
    }

    async function load(): Promise<void> {
        loadError = null;
        try {
            adopt(await fetchCatalog());
        } catch (e) {
            loadError = errorText(e);
        }
    }

    /** After a change on disk: adopt it when nothing is unsaved; else say so. */
    async function refresh(): Promise<void> {
        // This page's own save comes back as a change: it adopts the result itself.
        if (saving) return;
        try {
            const next = await fetchCatalog();
            const theirs = next.catalogs.find((c) => c.path === chosen);
            if (!dirty) {
                if (theirs?.hash !== hash || next.catalogs.length !== (list?.catalogs.length ?? 0)) adopt(next);
                return;
            }
            // Keep the edits and the hash they were read at: a save is then
            // refused, and the notice offers the choice.
            if (theirs !== undefined && theirs.hash !== hash && theirs.hash !== lastSaved) conflict = theirs.hash;
        } catch (e) {
            console.warn("catalog:", e);
        }
    }

    async function save(over: string = hash): Promise<boolean> {
        if (note === null || table === null) return false;
        saving = true;
        saveError = null;
        savedNote = null;
        // What this save sends: edits made while it is in flight stay in
        // the grid (unsaved) instead of being replaced by the re-read note.
        const sent = JSON.stringify(table);
        try {
            const saved = await saveCatalog(note.path, over, table);
            lastSaved = saved.hash;
            savedNote = saved.written ? `Saved: ${saved.items} ${saved.items === 1 ? "item" : "items"}.` : "Nothing to save.";
            const next = await fetchCatalog();
            const typed = table === null || JSON.stringify(table) === sent ? null : cloneGrid(table);
            adopt(next);
            if (typed !== null && table !== null) table = typed;
            return true;
        } catch (e) {
            if (e instanceof EditorApiError && e.code === "changed") {
                conflict = e.revision ?? "";
            } else if (e instanceof EditorApiError && e.code === "invalid") {
                serverProblems = cellProblemsOf(e.body);
                saveError = serverProblems.length > 0 ? "Some cells don't read; they're marked below." : errorText(e);
            } else {
                saveError = errorText(e);
            }
            return false;
        } finally {
            saving = false;
        }
    }

    async function create(): Promise<boolean> {
        saving = true;
        saveError = null;
        try {
            const made = await createCatalog();
            chosen = made.path;
            adopt(await fetchCatalog());
            return true;
        } catch (e) {
            saveError = errorText(e);
            return false;
        } finally {
            saving = false;
        }
    }

    let grid = $state<{addItem: () => void} | null>(null);

    /** New item: a row at the bottom, focused; the catalog note first when there is none. */
    async function newItem(): Promise<void> {
        if (note === null && !(await create())) return;
        await tick();
        grid?.addItem();
    }

    // `?new=1` from the New menu: once the catalog has loaded, drop the
    // parameter (before the row makes the page dirty, so leaving isn't
    // asked about), then add the item.
    let handlingNew = false;
    $effect(() => {
        if (!page.url.searchParams.has("new") || list === null || handlingNew) return;
        handlingNew = true;
        untrack(() => {
            void goto(resolve("/catalog"), {replace: true, reset: false})
                .then(() => newItem())
                .finally(() => (handlingNew = false));
        });
    });

    function edit(next: GridForm): void {
        table = next;
        serverProblems = [];
        savedNote = null;
    }

    function choose(path: string): void {
        const open = (): void => {
            chosen = path;
            if (list !== null) adopt(list);
        };
        if (dirty) ask(open);
        else open();
    }

    function ask(then: () => void): void {
        pending = {then};
        pendingError = null;
    }

    function carryOn(): void {
        const then = pending?.then;
        pending = null;
        pendingError = null;
        then?.();
    }

    async function saveAndCarryOn(): Promise<void> {
        if (pending === null) return;
        if (await save()) carryOn();
        else pendingError = saveError ?? (conflict !== null ? "The catalog changed on disk; reload it or keep yours first." : null);
    }

    function discard(): void {
        if (original !== null) table = cloneGrid(original);
        serverProblems = [];
        saveError = null;
    }

    // Cmd/Ctrl+S saves, from a field too (PLAN 5k); with nothing to save
    // it still keeps the browser's Save Page dialog away.
    registerKeys({
        id: "catalog-page",
        bindings: [
            {
                keys: "mod+s",
                label: "Save the catalog",
                group: "Catalog",
                always: true,
                run: () => {
                    if (dirty && !saving && ownProblems.length === 0) void save();
                },
            },
        ],
    });

    beforeNavigate(({cancel, to, type}) => {
        // The same page with another query (the New menu's `?new=1`) isn't leaving.
        if (to === null || to.url.pathname === page.url.pathname || !dirty) return;
        cancel();
        // Closing the window or reloading: the browser's own prompt.
        if (type === "leave") return;
        const href = to.url.pathname + to.url.search + to.url.hash;
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- a URL SvelteKit itself was navigating to
        ask(() => void goto(href));
    });

    onMount(() => {
        void load();
        fetchEditorOptions()
            .then((o) => (accounts = o.accounts))
            .catch((e: unknown) => console.warn("accounts:", e));
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
</script>

<svelte:head>
    <title>Catalog · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-7xl flex-col gap-4" data-testid="catalog-page">
    <header class="flex flex-wrap items-center gap-x-3 gap-y-2">
        <h1 class="text-2xl font-semibold tracking-tight">Catalog</h1>
        {#if note !== null}
            <span class="text-xs text-base-content/60" title="The note in the books folder" data-testid="catalog-path">{note.path}</span>
        {/if}
        {#if list !== null && list.catalogs.length > 1}
            <label class="flex items-center gap-2 text-sm">
                <span class="text-base-content/70">Catalog</span>
                <select
                    class="select select-sm"
                    value={chosen}
                    onchange={(event) => {
                        const to = event.currentTarget.value;
                        // Stays on the open catalog until it is chosen (a Save / Discard prompt may come first).
                        event.currentTarget.value = chosen ?? "";
                        choose(to);
                    }}
                    data-testid="catalog-choice"
                >
                    {#each list.catalogs as c (c.path)}<option value={c.path}>{c.title} ({c.path})</option>{/each}
                </select>
            </label>
        {/if}
        {#if table !== null}
            <div class="ml-auto flex flex-wrap items-center gap-2">
                {#if dirty}<span class="text-sm text-base-content/70" data-testid="unsaved">Unsaved changes</span>{/if}
                <button type="button" class="btn btn-ghost btn-sm" disabled={!dirty || saving} onclick={discard}>Discard</button>
                <button
                    type="button"
                    class="btn btn-primary btn-sm"
                    disabled={!dirty || saving || ownProblems.length > 0}
                    title={ownProblems.length > 0 ? "Fix the marked cells first" : undefined}
                    onclick={() => void save()}
                    data-testid="save-catalog">{saving ? "Saving…" : "Save"}</button
                >
                <button type="button" class="btn btn-primary btn-sm" disabled={saving} onclick={() => void newItem()} data-testid="catalog-new-item"
                    >New item</button
                >
            </div>
        {/if}
    </header>

    {#if note !== null}
        <p class="text-sm text-base-content/70">
            Choosing an item in an invoice copies its code, description, unit, rate and account into the row; changing an item here never changes an invoice
            already written.
        </p>
    {/if}

    {#if loadError !== null}
        <div role="alert" class="alert alert-soft alert-error" data-testid="catalog-load-error">
            <span>{loadError}</span>
            <button type="button" class="btn btn-sm" onclick={() => void load()}>Try again</button>
        </div>
    {:else if list === null}
        <span class="loading loading-md loading-spinner" aria-label="Loading the catalog"></span>
    {:else if note === null}
        <EmptyState
            title="No catalog yet"
            text="Your products and services, with their rates, for invoices to pick from."
            action="New item"
            onAction={() => void newItem()}
            busy={saving}
            testid="catalog-empty"
            actionTestid="catalog-empty-new"
        />
        {#if saveError !== null}
            <div role="alert" class="alert alert-soft alert-error" data-testid="catalog-save-error">{saveError}</div>
        {/if}
    {:else}
        {#if conflict !== null}
            <ConflictNotice
                theirs={null}
                what="catalog"
                busy={saving}
                onReload={() => void load()}
                onKeepMine={() => {
                    const over = conflict ?? hash;
                    conflict = null;
                    void save(over);
                }}
            />
        {/if}
        {#if saveError !== null}
            <div role="alert" class="alert alert-soft alert-error" data-testid="catalog-save-error">{saveError}</div>
        {/if}
        {#if savedNote !== null && !dirty}
            <div role="status" class="text-sm text-success" data-testid="catalog-saved">{savedNote}</div>
        {/if}
        {#if note.problems.length > 0 && !dirty}
            <ul class="alert flex flex-col items-start alert-soft alert-warning" data-testid="catalog-note-problems">
                {#each note.problems as problem, at (at)}<li>
                        {problem.line === null ? "" : `Line ${problem.line}: `}{friendlyMessage(problem.message)}
                    </li>{/each}
            </ul>
        {/if}
        {#if table === null}
            <p class="text-sm">This catalog has no table with Code and Description columns yet. Add one in a text editor, then come back.</p>
        {:else}
            <CatalogGrid bind:this={grid} {table} {accounts} {problems} onChange={edit} />
        {/if}
    {/if}
</div>

{#if pending !== null}
    <SaveDiscardDialog
        message="The catalog has unsaved changes. Save them first?"
        {saving}
        error={pendingError}
        onSave={() => void saveAndCarryOn()}
        onDiscard={() => {
            discard();
            carryOn();
        }}
        onCancel={() => {
            pending = null;
            pendingError = null;
        }}
    />
{/if}
