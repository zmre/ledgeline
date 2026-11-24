<!-- The catalog (invoicer PLAN 4c, ARCHITECTURE §3.6): the catalog note's
     table as a grid. Add, edit, reorder and delete items; the account picker
     and deferral popover are the invoice editor's. Saving sends the grid with
     the note's content hash; the server writes only the table's bytes (the
     frontmatter and any prose around it stay as they are) and refuses an
     outside edit made meanwhile (409): reload theirs, or keep mine (save
     over theirs). Live reload: a change on disk shows at once when nothing is
     unsaved; otherwise the page says so and keeps the edits. The invoice
     editor's catalog picker re-reads the catalog after every change.

     Keyboard: Cmd/Ctrl+S saves. -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {beforeNavigate} from "$app/navigation";
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
    import {friendlyMessage} from "#lib/labels.js";

    const asMessage = (e: unknown): string => (e instanceof Error ? e.message : String(e));

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
            loadError = asMessage(e);
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

    async function save(over: string = hash): Promise<void> {
        if (note === null || table === null) return;
        saving = true;
        saveError = null;
        savedNote = null;
        try {
            const saved = await saveCatalog(note.path, over, table);
            lastSaved = saved.hash;
            savedNote = saved.written ? `Saved: ${saved.items} ${saved.items === 1 ? "item" : "items"}.` : "Nothing to save.";
            adopt(await fetchCatalog());
        } catch (e) {
            if (e instanceof EditorApiError && e.code === "changed") {
                conflict = e.revision ?? "";
            } else if (e instanceof EditorApiError && e.code === "invalid") {
                serverProblems = cellProblemsOf(e.body);
                saveError = serverProblems.length > 0 ? "Some cells don't read; they're marked below." : e.message;
            } else {
                saveError = asMessage(e);
            }
        } finally {
            saving = false;
        }
    }

    async function create(): Promise<void> {
        saving = true;
        saveError = null;
        try {
            const made = await createCatalog();
            chosen = made.path;
            adopt(await fetchCatalog());
        } catch (e) {
            saveError = asMessage(e);
        } finally {
            saving = false;
        }
    }

    function edit(next: GridForm): void {
        table = next;
        serverProblems = [];
        savedNote = null;
    }

    function choose(path: string): void {
        if (dirty && !confirm("The catalog has unsaved changes. Switch without saving?")) return;
        chosen = path;
        if (list !== null) adopt(list);
    }

    function discard(): void {
        if (original !== null) table = cloneGrid(original);
        serverProblems = [];
        saveError = null;
    }

    function onKeydown(event: KeyboardEvent): void {
        if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
            event.preventDefault();
            if (dirty && !saving && ownProblems.length === 0) void save();
        }
    }

    beforeNavigate(({cancel, to}) => {
        if (dirty && to !== null && !confirm("The catalog has unsaved changes. Leave without saving?")) cancel();
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

<svelte:window onkeydown={onKeydown} />

<div class="mx-auto flex max-w-7xl flex-col gap-4" data-testid="catalog-page">
    <header class="flex flex-wrap items-center gap-x-3 gap-y-2">
        <h1 class="text-2xl font-semibold tracking-tight">Catalog</h1>
        {#if note !== null}
            <span class="text-xs text-base-content/60" title="The note in the books folder" data-testid="catalog-path">{note.path}</span>
        {/if}
        {#if list !== null && list.catalogs.length > 1}
            <label class="flex items-center gap-2 text-sm">
                <span class="text-base-content/70">Catalog</span>
                <select class="select select-sm" value={chosen} onchange={(event) => choose(event.currentTarget.value)} data-testid="catalog-choice">
                    {#each list.catalogs as c (c.path)}<option value={c.path}>{c.title} ({c.path})</option>{/each}
                </select>
            </label>
        {/if}
        {#if table !== null}
            <div class="ml-auto flex items-center gap-2">
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
            </div>
        {/if}
    </header>

    <p class="text-sm text-base-content/70">
        Choosing an item in an invoice copies its code, description, unit, rate and account into the row; changing an item here never changes an invoice already
        written.
    </p>

    {#if loadError !== null}
        <div role="alert" class="alert alert-soft alert-error" data-testid="catalog-load-error">
            <span>{loadError}</span>
            <button type="button" class="btn btn-sm" onclick={() => void load()}>Try again</button>
        </div>
    {:else if list === null}
        <span class="loading loading-md loading-spinner" aria-label="Loading the catalog"></span>
    {:else if note === null}
        <div class="rounded-box border border-dashed border-base-300 p-6 text-sm" data-testid="catalog-empty">
            <p>There is no catalog yet: a list of your products and services, with their rates, for invoices to pick from.</p>
            <button type="button" class="btn mt-3 btn-primary btn-sm" disabled={saving} onclick={() => void create()} data-testid="create-catalog"
                >Create the catalog</button
            >
        </div>
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
            <CatalogGrid {table} {accounts} {problems} onChange={edit} />
        {/if}
    {/if}
</div>
