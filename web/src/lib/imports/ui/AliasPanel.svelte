<script lang="ts">
    // The Account Aliases tab: the mapping table an import hands to hledger.
    //
    // # Why this lives under Imports
    //
    // An `alias` is journal-wide storage, but in Ledgeline it has exactly one
    // consumer: the import pipeline. The engine deliberately does NOT apply
    // aliases when it reads a journal (see `parse.rs` — reproducing hledger's
    // regex dialect over every account name is the kind of near-miss that
    // produces silent wrong answers), so putting this editor anywhere else would
    // advertise an effect the application does not have. Imports already owns a
    // file-backed, revision-guarded, format-preserving editor next door in Edit
    // Rules, and this is the same shape of thing for the same audience.
    //
    // # What the screen refuses to do
    //
    // - It will not rewrite a line the engine flagged read-only. Such a row
    //   renders as text with the engine's own explanation; only Delete is
    //   offered, because removing a whole top-level line cannot inject anything.
    // - It will not insert anywhere but the end of the file. An alias is
    //   positional, so an appended one is in force exactly where an import
    //   appends and provably changes the meaning of nothing above it.
    // - It will not reorder. Same reason: a reorder of positional directives is
    //   a semantic change wearing a cosmetic's clothes.
    //
    // Every decision here is a call into `aliasModel.ts`. This file is markup.
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {dataView} from "$lib/stores/loadState";
    import {onServerReady} from "$lib/stores/serverWatch.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import {ALIAS_EXPLAINER, aliasBadges, aliasText, blankRow, isDirty, toForm, toSaveRequest, validateForm} from "../aliasModel";
    import type {AliasDraft, AliasForm} from "../aliasModel";
    import {aliasListing, aliasStore} from "../aliasStore.svelte";
    import type {AliasFile} from "../importTypes";

    let selectedId = $state<string | null>(null);
    let baseFile = $state<AliasFile | null>(null);
    let form = $state<AliasForm | null>(null);
    let clientErrors = $state<string[]>([]);
    let serverError = $state<string | null>(null);
    let savedAt = $state<number | null>(null);

    onServerReady((url) => {
        void aliasStore.ensureListing(url, settings.serverNonce);
    });

    const listing = $derived(aliasListing.value);
    const view = $derived(dataView(aliasListing.status, listing !== null));

    // Seeding is an explicit call after a load, latched on the file's identity —
    // never a `$effect` that re-runs on every keystroke and eats the user's
    // typing. Same discipline as `EditRulesPanel`.
    $effect(() => {
        const files = listing?.files ?? [];
        if (files.length === 0) return;
        const chosen = files.find((file) => file.journalId === selectedId) ?? files[0];
        if (baseFile === chosen) return;
        selectedId = chosen.journalId;
        baseFile = chosen;
        form = toForm(chosen);
        clientErrors = [];
        serverError = null;
        savedAt = null;
    });

    const baseline = $derived(baseFile === null ? null : toForm(baseFile));
    const dirty = $derived(isDirty(baseline, form));
    const canEdit = $derived((listing?.editable ?? false) && (form?.writable ?? false));
    const disabled = $derived(!canEdit || aliasStore.saving);

    function select(id: string): void {
        selectedId = id;
        baseFile = null;
    }

    function update(at: number, patch: Partial<AliasDraft>): void {
        if (form === null) return;
        form = {...form, rows: form.rows.map((row, i) => (i === at ? {...row, ...patch} : row))};
        savedAt = null;
    }

    function addRow(): void {
        if (form === null) return;
        form = {...form, rows: [...form.rows, blankRow()]};
        savedAt = null;
    }

    async function save(): Promise<void> {
        if (form === null || baseline === null) return;
        clientErrors = validateForm(form);
        serverError = null;
        if (clientErrors.length > 0) return;
        const result = await aliasStore.save(form.journalId, toSaveRequest(baseline, form));
        if (!result.ok) {
            serverError = result.failure.message;
            return;
        }
        // Re-seed from what the engine WROTE: an alias index is a parse ordinal,
        // and a delete renumbers every line below it.
        baseFile = result.file;
        form = toForm(result.file);
        savedAt = Date.now();
    }

    function reloadDiscarding(): void {
        const url = settings.serverUrl;
        if (url === null) return;
        aliasStore.clearConflict();
        baseFile = null;
        void aliasStore.reload(url);
    }

    function retry(): void {
        const url = settings.serverUrl;
        if (url !== null) void aliasStore.reload(url);
    }
</script>

<AsyncSection
    {view}
    value={listing}
    error={aliasListing.error}
    testid="imports-aliases-error"
    label="your aliases"
    loadingLabel="Reading your aliases"
    onRetry={retry}
>
    {#snippet children(loaded)}
        <div class="flex flex-col gap-4" data-testid="imports-aliases">
            <p class="text-base-content/70 max-w-3xl text-sm">{ALIAS_EXPLAINER}</p>

            {#if !loaded.editable}
                <div class="alert alert-info rounded-box items-start py-2 text-sm" role="status" data-testid="imports-aliases-read-only">
                    <span>This server has no journal open for editing, so these are shown but cannot be changed.</span>
                </div>
            {/if}

            {#if loaded.files.length > 1}
                <div role="tablist" class="tabs tabs-box tabs-sm w-fit">
                    {#each loaded.files as file (file.journalId)}
                        <button type="button" role="tab" class="tab {file.journalId === selectedId ? 'tab-active' : ''}" onclick={() => select(file.journalId)}>
                            {file.label}
                            {#if file.aliases.length > 0}<span class="badge badge-ghost badge-xs ml-1">{file.aliases.length}</span>{/if}
                        </button>
                    {/each}
                </div>
            {/if}

            {#if aliasStore.conflict}
                <div class="alert alert-warning rounded-box flex-col items-start gap-2 py-2 text-sm" role="alert" data-testid="imports-aliases-conflict">
                    <span>
                        {form?.label ?? "This journal"} changed on disk since you opened it, so nothing was written. Reload it and re-apply your edit — saving over
                        it would discard whatever the other change was.
                    </span>
                    <button type="button" class="btn btn-sm" onclick={reloadDiscarding}>Reload and discard my changes</button>
                </div>
            {/if}

            {#if form !== null}
                <div class="flex flex-col gap-2">
                    {#each form.rows as row, at (at)}
                        {@const entry = baseFile?.aliases.find((alias) => alias.index === row.index) ?? null}
                        <div class="border-base-content/10 rounded-box flex flex-col gap-2 border p-3" class:opacity-60={row.deleted}>
                            <div class="flex flex-wrap items-center gap-2">
                                {#if entry !== null}
                                    {#each aliasBadges(entry) as badge (badge.text)}
                                        <span class="badge badge-sm {badge.tone === 'warning' ? 'badge-warning' : 'badge-ghost'}">{badge.text}</span>
                                    {/each}
                                {/if}
                                <span class="text-base-content/50 ml-auto text-xs">
                                    {#if row.index === null}new{:else}line {entry?.line ?? "?"}{/if}
                                </span>
                            </div>

                            {#if row.locked}
                                <!-- Read-only: shown exactly as written, with the engine's own reason. -->
                                <code class="text-xs break-all" data-testid="imports-alias-locked">{entry === null ? "" : aliasText(entry)}</code>
                                <p class="text-base-content/60 text-xs">
                                    Ledgeline will not rewrite this line because {entry?.lockMessage ?? "it is not modelled"}.
                                </p>
                            {:else}
                                <div class="flex flex-wrap items-end gap-2">
                                    <label class="form-control grow">
                                        <span class="label-text text-xs">What the bank calls it</span>
                                        <input
                                            type="text"
                                            class="input input-sm input-bordered w-full font-mono"
                                            value={row.pattern}
                                            disabled={disabled || row.deleted}
                                            oninput={(event) => update(at, {pattern: event.currentTarget.value})}
                                        />
                                    </label>
                                    <label class="form-control grow">
                                        <span class="label-text text-xs">Your account</span>
                                        <input
                                            type="text"
                                            class="input input-sm input-bordered w-full font-mono"
                                            value={row.replacement}
                                            disabled={disabled || row.deleted}
                                            oninput={(event) => update(at, {replacement: event.currentTarget.value})}
                                        />
                                    </label>
                                    <label class="label cursor-pointer gap-2 text-xs">
                                        <input
                                            type="checkbox"
                                            class="checkbox checkbox-sm"
                                            checked={row.regex}
                                            disabled={disabled || row.deleted}
                                            onchange={(event) => update(at, {regex: event.currentTarget.checked})}
                                        />
                                        regular expression
                                    </label>
                                </div>
                            {/if}

                            {#if !row.deleted}
                                {@const problems = validateForm({...form, rows: [row]})}
                                {#if problems.length > 0}
                                    <ul class="text-error list-inside list-disc text-xs">
                                        {#each problems as problem (problem)}
                                            <li>{problem.replace(/^Alias 1: /, "")}</li>
                                        {/each}
                                    </ul>
                                {/if}
                            {/if}

                            <div class="flex justify-end">
                                <button type="button" class="btn btn-ghost btn-xs" {disabled} onclick={() => update(at, {deleted: !row.deleted})}>
                                    {row.deleted ? "Keep" : "Delete"}
                                </button>
                            </div>
                        </div>
                    {/each}

                    {#if form.rows.length === 0}
                        <p class="text-base-content/60 text-sm">This journal declares no aliases yet.</p>
                    {/if}
                </div>

                <div class="flex flex-wrap items-center gap-2">
                    <button type="button" class="btn btn-sm" {disabled} onclick={addRow} data-testid="imports-alias-add">Add an alias</button>
                    <button type="button" class="btn btn-primary btn-sm" disabled={disabled || !dirty} onclick={save} data-testid="imports-alias-save">
                        {#if aliasStore.saving}<span class="loading loading-spinner loading-xs"></span>{/if}
                        Save
                    </button>
                    {#if dirty}<span class="badge badge-warning badge-sm" data-testid="imports-alias-dirty">unsaved</span>{/if}
                    {#if savedAt !== null && !dirty}<span class="badge badge-success badge-sm" data-testid="imports-alias-saved">saved</span>{/if}
                    <span class="text-base-content/50 text-xs">A new alias is added at the end of {form.label}.</span>
                </div>

                {#if clientErrors.length > 0}
                    <ul class="alert alert-error rounded-box list-inside list-disc py-2 text-sm" data-testid="imports-alias-client-errors">
                        {#each clientErrors as problem (problem)}
                            <li>{problem}</li>
                        {/each}
                    </ul>
                {/if}
                {#if serverError !== null}
                    <div class="alert alert-error rounded-box items-start py-2 text-sm" role="alert" data-testid="imports-alias-server-error">
                        <span>{serverError}</span>
                    </div>
                {/if}
            {/if}
        </div>
    {/snippet}
</AsyncSection>
