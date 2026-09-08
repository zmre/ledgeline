<!-- The Settings → Accounts tab: detect (or create) the file(s) that declare
     the chart of accounts, and add, remove, or edit each account's
     type/tags/note.

     Every account name the journal knows about is listed — declared or not
     (from postings). Selecting one opens its editor; saving an undeclared
     account sends a `declare` edit, and the account's home is whichever file
     is offered as the default target (see `model.ts::defaultTargetFile`).
     Typing a name nothing else knows about into "Add account" opens the same
     editor for it.

     Every "special" tag a report actually reads for meaning — `type`,
     `issection`, `bsterm`, `holdings`, `valuation` (closed vocabularies) and
     `isgroup`/`bsgroup` (free labels) — gets its own field pulled out of the
     general tag list, with inline help. See `model.ts::SPECIAL_TAGS`.

     Every decision here is a call into `model.ts`. This file is markup, the
     same split `AliasPanel.svelte` states for its own screen. -->
<script lang="ts">
    import AccountLabel from "$lib/components/AccountLabel.svelte";
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {dataView} from "$lib/stores/loadState";
    import {holdUnsavedEdits} from "$lib/stores/refreshAll";
    import {onServerReady} from "$lib/stores/serverWatch.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import {accountListing, accountsStore} from "../accountsStore.svelte";
    import {blankTag, defaultTargetFile, deleteSaveRequest, isDirty, mergedRows, SPECIAL_TAGS, toForm, toSaveRequest, validateForm} from "../model";
    import type {AccountForm} from "../model";
    import type {AccountEntry} from "../types";

    let {accountNames}: {accountNames: string[]} = $props();

    onServerReady((url) => {
        void accountsStore.ensureListing(url, settings.serverNonce);
    });

    const listing = $derived(accountListing.value);
    const view = $derived(dataView(accountListing.status, listing !== null));

    let filter = $state("");
    let newAccountName = $state("");
    let selectedName = $state<string | null>(null);
    let baseEntry = $state<AccountEntry | null>(null);
    let form = $state<AccountForm | null>(null);
    let removalStaged = $state(false);
    let clientErrors = $state<string[]>([]);
    let serverError = $state<string | null>(null);
    let savedAt = $state<number | null>(null);
    let createError = $state<string | null>(null);

    const rows = $derived(listing === null ? [] : mergedRows(accountNames, listing.files));
    const filteredRows = $derived.by(() => {
        const needle = filter.trim().toLowerCase();
        return needle === "" ? rows : rows.filter((row) => row.name.toLowerCase().includes(needle));
    });
    const targetFile = $derived(listing === null ? null : defaultTargetFile(listing.files));
    const canEdit = $derived(listing?.editable ?? false);
    const disabled = $derived(!canEdit || accountsStore.saving);

    // Same latch discipline `AliasPanel` uses, and for the same reason: a
    // PLAIN `let`, not `$state`, so re-seeding cannot depend on itself.
    let seededFor: string | null = null;

    $effect(() => {
        if (selectedName === null) {
            form = null;
            seededFor = null;
            removalStaged = false;
            return;
        }
        const key = `${selectedName}#${baseEntry?.journalId ?? ""}#${baseEntry?.index ?? "new"}`;
        if (key === seededFor) return;
        seededFor = key;
        form = toForm(selectedName, baseEntry);
        clientErrors = [];
        serverError = null;
        savedAt = null;
        removalStaged = false;
    });

    const dirty = $derived(removalStaged || (form === null ? false : isDirty(baseEntry, form)));

    $effect(() => {
        holdUnsavedEdits("accounts", dirty);
        return () => holdUnsavedEdits("accounts", false);
    });

    function select(name: string): void {
        const row = rows.find((r) => r.name === name) ?? null;
        selectedName = name;
        baseEntry = row?.entry ?? null;
    }

    /** "Add account": open the editor for a name nothing else knows about yet — the same seeding path a click on an existing row uses. */
    function addAccount(): void {
        const name = newAccountName.trim();
        if (name === "") return;
        select(name);
        newAccountName = "";
    }

    function addTag(): void {
        if (form === null) return;
        form = {...form, tags: [...form.tags, blankTag()]};
        savedAt = null;
    }

    function updateTag(at: number, patch: {name?: string; value?: string}): void {
        if (form === null) return;
        form = {...form, tags: form.tags.map((tag, i) => (i === at ? {...tag, ...patch} : tag))};
        savedAt = null;
    }

    function removeTag(at: number): void {
        if (form === null) return;
        form = {...form, tags: form.tags.filter((_, i) => i !== at)};
        savedAt = null;
    }

    async function save(): Promise<void> {
        if (form === null) return;
        serverError = null;

        if (removalStaged) {
            const entry = baseEntry;
            if (entry === null) return;
            const file = listing?.files.find((f) => f.journalId === entry.journalId);
            if (file === undefined) {
                serverError = "No journal file is available to write to.";
                return;
            }
            const result = await accountsStore.save(file.journalId, deleteSaveRequest(file.revision, entry.index));
            if (!result.ok) {
                serverError = result.failure.message;
                return;
            }
            selectedName = null;
            baseEntry = null;
            form = null;
            seededFor = null;
            removalStaged = false;
            return;
        }

        clientErrors = validateForm(form);
        if (clientErrors.length > 0) return;

        const entry = baseEntry;
        const file = entry === null ? targetFile : (listing?.files.find((f) => f.journalId === entry.journalId) ?? null);
        if (file === null || file === undefined) {
            serverError = "No journal file is available to write to.";
            return;
        }
        const result = await accountsStore.save(file.journalId, toSaveRequest(file.revision, entry, form));
        if (!result.ok) {
            serverError = result.failure.message;
            return;
        }
        // Re-seed from what the engine WROTE, exactly as `AliasPanel` does: an
        // index is a parse ordinal and `declare` can hand this account a new
        // one at any moment.
        baseEntry = result.file.accounts.find((account) => account.name === form?.name) ?? null;
        seededFor = `${form.name}#${baseEntry?.journalId ?? ""}#${baseEntry?.index ?? "new"}`;
        form = toForm(form.name, baseEntry);
        savedAt = Date.now();
    }

    async function createFile(): Promise<void> {
        createError = null;
        const result = await accountsStore.createFile();
        if (!result.ok) createError = result.failure.message;
    }

    function retry(): void {
        const url = settings.serverUrl;
        if (url !== null) void accountsStore.reload(url);
    }
</script>

{#snippet help(text: string, label: string)}
    <span class="tooltip tooltip-right align-middle before:max-w-64 before:whitespace-normal" data-tip={text}>
        <button type="button" class="cursor-help align-middle text-base-content/50 hover:text-base-content" aria-label={`About ${label}`}>
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="currentColor" class="inline h-3.5 w-3.5" aria-hidden="true">
                <path
                    fill-rule="evenodd"
                    d="M8 15A7 7 0 1 0 8 1a7 7 0 0 0 0 14Zm.75-10.25a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0ZM7 7a.75.75 0 0 0 0 1.5h.25v3a.75.75 0 0 0 1.5 0v-3.75A.75.75 0 0 0 8 7H7Z"
                    clip-rule="evenodd"
                />
            </svg>
        </button>
    </span>
{/snippet}

<AsyncSection
    {view}
    value={listing}
    error={accountListing.error}
    testid="settings-accounts-error"
    label="your chart of accounts"
    loadingLabel="Reading your accounts"
    onRetry={retry}
>
    {#snippet children(loaded)}
        <div class="flex flex-col gap-4 lg:h-[calc(100dvh-11rem)]" data-testid="settings-accounts">
            {#if !loaded.editable}
                <div class="alert items-start rounded-box py-2 text-sm alert-info" role="status" data-testid="settings-accounts-read-only">
                    <span>This server has no journal open for editing, so these are shown but cannot be changed.</span>
                </div>
            {/if}

            {#if loaded.canCreateFile}
                <div class="alert flex flex-col items-start gap-2 rounded-box py-2 text-sm alert-info" data-testid="settings-accounts-empty-state">
                    <span>This journal declares no accounts yet. Create {loaded.createFileName} to give them a home of their own.</span>
                    <button type="button" class="btn btn-sm" disabled={!canEdit || accountsStore.creating} onclick={createFile}>
                        {#if accountsStore.creating}<span class="loading loading-xs loading-spinner"></span>{/if}
                        Create {loaded.createFileName}
                    </button>
                    {#if createError !== null}<span class="text-error">{createError}</span>{/if}
                </div>
            {/if}

            {#if accountsStore.conflict}
                <div class="alert flex flex-col items-start gap-2 rounded-box py-2 text-sm alert-warning" role="alert" data-testid="settings-accounts-conflict">
                    <span>This journal changed on disk since you opened it, so nothing was written. Reload it and re-apply your edit.</span>
                    <button
                        type="button"
                        class="btn btn-sm"
                        onclick={() => {
                            accountsStore.clearConflict();
                            const url = settings.serverUrl;
                            if (url !== null) void accountsStore.reload(url);
                        }}
                    >
                        Reload and discard my changes
                    </button>
                </div>
            {/if}

            <div class="flex flex-wrap items-center gap-2">
                <label class="input w-full max-w-xs input-sm">
                    <input type="text" placeholder="Filter accounts…" bind:value={filter} data-testid="settings-accounts-filter" />
                </label>
                <form
                    class="flex items-center gap-2"
                    onsubmit={(event) => {
                        event.preventDefault();
                        addAccount();
                    }}
                >
                    <input
                        type="text"
                        placeholder="New account name (e.g. assets:bank:new)"
                        class="input-bordered input w-full max-w-xs font-mono input-sm"
                        {disabled}
                        bind:value={newAccountName}
                        data-testid="settings-accounts-new-name"
                    />
                    <button type="submit" class="btn btn-sm" {disabled} data-testid="settings-accounts-add">Add account</button>
                </form>
            </div>

            <div class="grid min-h-0 grow grid-cols-1 gap-3 lg:grid-cols-[20rem_minmax(0,1fr)]">
                <ul
                    class="menu max-h-64 w-full flex-col flex-nowrap overflow-y-auto rounded-box bg-base-200 lg:max-h-none"
                    data-testid="settings-accounts-list"
                >
                    {#each filteredRows as row (row.name)}
                        <li>
                            <button
                                type="button"
                                class={row.name === selectedName ? "menu-active" : ""}
                                onclick={() => select(row.name)}
                                data-testid="settings-account-row"
                            >
                                <AccountLabel name={row.name} budget={44} />
                                {#if row.entry === null}
                                    <span class="badge badge-ghost badge-xs">undeclared</span>
                                {/if}
                            </button>
                        </li>
                    {/each}
                    {#if filteredRows.length === 0}
                        <li class="p-2 text-xs text-base-content/60">No accounts match.</li>
                    {/if}
                </ul>

                {#if form !== null}
                    <div class="flex flex-col gap-3 overflow-y-auto rounded-box border border-base-content/10 p-3" data-testid="settings-account-editor">
                        <div class="flex items-center gap-2">
                            <code class="text-sm break-all">{form.name}</code>
                            {#if baseEntry === null}<span class="badge badge-ghost badge-sm">not yet declared</span>{/if}
                            {#if baseEntry !== null}
                                <button
                                    type="button"
                                    class="btn ml-4 btn-outline btn-xs {removalStaged ? '' : 'btn-error'}"
                                    {disabled}
                                    onclick={() => (removalStaged = !removalStaged)}
                                    data-testid="settings-account-remove-toggle"
                                >
                                    {removalStaged ? "Keep account" : "Remove account"}
                                </button>
                            {/if}
                        </div>

                        {#if removalStaged}
                            <div class="alert py-2 text-sm alert-warning" data-testid="settings-account-removal-warning">
                                <span>Saving removes this declaration. Any transactions that still use "{form.name}" are untouched.</span>
                            </div>
                        {/if}

                        <div class="flex flex-col gap-3" class:opacity-50={removalStaged}>
                            <div class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
                                {#each SPECIAL_TAGS as spec (spec.key)}
                                    <div class="flex flex-col gap-1">
                                        <span class="flex items-center gap-1">
                                            <label class="form-control grow" for={`settings-account-${spec.key}`}>
                                                <span class="label-text text-xs">{spec.label}</span>
                                                {#if spec.kind === "closed"}
                                                    <select
                                                        id={`settings-account-${spec.key}`}
                                                        class="select-bordered select w-full select-sm"
                                                        bind:value={form.special[spec.key]}
                                                        disabled={disabled || removalStaged}
                                                    >
                                                        <option value="">(none)</option>
                                                        {#each spec.options ?? [] as option (option.value)}
                                                            <option value={option.value}>{option.label}</option>
                                                        {/each}
                                                    </select>
                                                {:else}
                                                    <input
                                                        id={`settings-account-${spec.key}`}
                                                        type="text"
                                                        class="input-bordered input w-full input-sm"
                                                        list={spec.suggestions ? `settings-account-${spec.key}-suggestions` : undefined}
                                                        disabled={disabled || removalStaged}
                                                        bind:value={form.special[spec.key]}
                                                    />
                                                    {#if spec.suggestions}
                                                        <datalist id={`settings-account-${spec.key}-suggestions`}>
                                                            {#each spec.suggestions as suggestion (suggestion)}
                                                                <option value={suggestion}></option>
                                                            {/each}
                                                        </datalist>
                                                    {/if}
                                                {/if}
                                            </label>
                                            {@render help(spec.help, spec.label)}
                                        </span>
                                    </div>
                                {/each}
                            </div>

                            <div class="flex flex-col gap-2">
                                <span class="label-text text-xs">Other tags</span>
                                {#each form.tags as tag, at (at)}
                                    <div class="flex items-center gap-2">
                                        <input
                                            type="text"
                                            class="input-bordered input w-32 font-mono input-sm"
                                            placeholder="name"
                                            value={tag.name}
                                            disabled={disabled || removalStaged}
                                            oninput={(event) => updateTag(at, {name: event.currentTarget.value})}
                                        />
                                        <input
                                            type="text"
                                            class="input-bordered input grow font-mono input-sm"
                                            placeholder="value"
                                            value={tag.value}
                                            disabled={disabled || removalStaged}
                                            oninput={(event) => updateTag(at, {value: event.currentTarget.value})}
                                        />
                                        <button type="button" class="btn btn-ghost btn-xs" disabled={disabled || removalStaged} onclick={() => removeTag(at)}>
                                            Remove
                                        </button>
                                    </div>
                                {/each}
                                <button
                                    type="button"
                                    class="btn w-fit btn-sm"
                                    disabled={disabled || removalStaged}
                                    onclick={addTag}
                                    data-testid="settings-account-add-tag"
                                >
                                    Add a tag
                                </button>
                            </div>

                            <label class="form-control w-full">
                                <span class="label-text text-xs">Note</span>
                                <textarea
                                    class="textarea-bordered textarea textarea-sm"
                                    rows="2"
                                    disabled={disabled || removalStaged}
                                    value={form.note}
                                    oninput={(event) => {
                                        if (form !== null) form = {...form, note: event.currentTarget.value};
                                        savedAt = null;
                                    }}></textarea>
                            </label>
                        </div>

                        <div class="flex flex-wrap items-center gap-2">
                            <button
                                type="button"
                                class="btn btn-primary btn-sm"
                                disabled={disabled || !dirty}
                                onclick={save}
                                data-testid="settings-account-save"
                            >
                                {#if accountsStore.saving}<span class="loading loading-xs loading-spinner"></span>{/if}
                                Save
                            </button>
                            {#if dirty}<span class="badge badge-sm badge-warning" data-testid="settings-account-dirty">unsaved</span>{/if}
                            {#if savedAt !== null && !dirty}<span class="badge badge-sm badge-success" data-testid="settings-account-saved">saved</span>{/if}
                        </div>

                        {#if clientErrors.length > 0}
                            <ul class="alert list-inside list-disc rounded-box py-2 text-sm alert-error" data-testid="settings-account-client-errors">
                                {#each clientErrors as problem (problem)}
                                    <li>{problem}</li>
                                {/each}
                            </ul>
                        {/if}
                        {#if serverError !== null}
                            <div class="alert items-start rounded-box py-2 text-sm alert-error" role="alert" data-testid="settings-account-server-error">
                                <span>{serverError}</span>
                            </div>
                        {/if}
                    </div>
                {:else}
                    <p class="text-sm text-base-content/60">Select an account, or add a new one, to edit its type, tags and note.</p>
                {/if}
            </div>
        </div>
    {/snippet}
</AsyncSection>
