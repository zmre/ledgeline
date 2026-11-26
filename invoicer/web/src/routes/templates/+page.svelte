<!-- Templates (invoicer PLAN 4d): the books' templates and the starters not
     copied in, by name, with what each is for, what it builds on, the blocks
     it changes and who uses it. A name opens the editor.

     New template asks a name and what to start from (the invoice base as it
     comes, a starter, or a copy of one of the books' templates), writes the
     file at once and opens it. It is top right once the books have a
     template of their own (H21), else the empty state's button; `?new=1`
     (the sidebar's New menu, H20) opens the form on arrival and drops the
     parameter. -->
<script lang="ts">
    import {onMount, untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {errorText} from "#lib/labels.js";
    import {createTemplate, fetchTemplateList, templateParam, type TemplateListEntry} from "#lib/templates.js";
    import EmptyState from "#lib/components/EmptyState.svelte";
    import TemplateList from "#lib/components/TemplateList.svelte";

    let list = $state<TemplateListEntry[] | null>(null);
    let loadError = $state<string | null>(null);
    const own = $derived((list ?? []).filter((t) => t.installed));

    let creating = $state(false);
    let busy = $state(false);
    let newName = $state("");
    /** What it starts from: "" the base as it comes, else a template's path. */
    let newFrom = $state("");
    let createError = $state<string | null>(null);

    async function load(): Promise<void> {
        loadError = null;
        try {
            list = await fetchTemplateList();
        } catch (e) {
            loadError = errorText(e);
        }
    }

    function startCreate(): void {
        creating = true;
        createError = null;
    }

    const editorHref = (entry: Pick<TemplateListEntry, "path">): string => resolve("/templates/[...path]", {path: templateParam(entry.path)});

    async function create(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        createError = null;
        busy = true;
        try {
            const made = await createTemplate({name: newName.trim(), from: newFrom === "" ? undefined : newFrom});
            creating = false;
            newName = "";
            newFrom = "";
            // eslint-disable-next-line svelte/no-navigation-without-resolve -- editorHref resolves
            await goto(editorHref({path: made.template}));
        } catch (e) {
            createError = errorText(e);
        } finally {
            busy = false;
        }
    }

    // `?new=1` from the New menu: drop the parameter, then open the form.
    let handlingNew = false;
    $effect(() => {
        if (!page.url.searchParams.has("new") || handlingNew) return;
        handlingNew = true;
        untrack(() => {
            void goto(resolve("/templates"), {replace: true, reset: false})
                .then(() => startCreate())
                .finally(() => (handlingNew = false));
        });
    });

    onMount(() => void load());

    onFilesChanged(() => void load());
</script>

<svelte:head>
    <title>Templates · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-7xl flex-col gap-4" data-testid="templates-page">
    <header class="flex flex-wrap items-center gap-x-3 gap-y-2">
        <h1 class="text-2xl font-semibold tracking-tight">Templates</h1>
        {#if own.length > 0}
            <button type="button" class="btn ml-auto btn-primary btn-sm" onclick={startCreate} data-testid="new-template">New template</button>
        {/if}
    </header>
    <p class="text-sm text-base-content/70">
        How an invoice looks on screen and on paper. Most templates build on the invoice base and change only what they need: colours and fonts, or one part of
        the page.
    </p>

    {#if creating}
        <form class="flex flex-wrap items-end gap-2 rounded-box border border-base-300 p-3" onsubmit={create} data-testid="new-template-form">
            <label class="flex min-w-0 flex-col gap-1 text-sm">
                <span class="font-medium">Name</span>
                <!-- svelte-ignore a11y_autofocus -->
                <input class="input w-72 max-w-full input-sm" bind:value={newName} required autofocus data-testid="new-template-name" />
            </label>
            <label class="flex min-w-0 flex-col gap-1 text-sm">
                <span class="font-medium">Start from</span>
                <select class="select max-w-full select-sm" bind:value={newFrom} data-testid="new-template-from">
                    <option value="">The invoice base, as it comes</option>
                    {#each list ?? [] as template (template.path)}
                        <option value={template.path}>{template.installed ? `A copy of ${template.name}` : `The ${template.name} starter`}</option>
                    {/each}
                </select>
            </label>
            <button type="submit" class="btn btn-primary btn-sm" disabled={busy || newName.trim() === ""} data-testid="create-template">Create</button>
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => (creating = false)}>Cancel</button>
            {#if createError !== null}<p role="alert" class="w-full text-sm text-error" data-testid="new-template-error">{createError}</p>{/if}
        </form>
    {/if}

    {#if loadError !== null}
        <div role="alert" class="alert alert-soft alert-error" data-testid="templates-load-error">
            <span>{loadError}</span>
            <button type="button" class="btn btn-sm" onclick={() => void load()}>Try again</button>
        </div>
    {:else if list === null}
        <span class="loading loading-md loading-spinner" aria-label="Loading the templates"></span>
    {:else}
        {#if own.length === 0 && !creating}
            <EmptyState
                title="No templates of your own yet"
                text="Start from the invoice base or a starter, then change the colours, the fonts or any part of the page."
                action="New template"
                onAction={startCreate}
                testid="templates-empty"
                actionTestid="templates-empty-new"
            />
        {/if}
        {#if list.length > 0}
            {#if own.length === 0}<h2 class="text-lg font-semibold">Starters</h2>{/if}
            <TemplateList entries={list} href={editorHref} onOpen={(entry) => void goto(resolve("/templates/[...path]", {path: templateParam(entry.path)}))} />
        {/if}
    {/if}
</div>
