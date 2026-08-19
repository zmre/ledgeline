<script lang="ts">
    // The Edit Rules tab: a master list of `*.rules` files beside the journal,
    // and a structured editor for the selected one.
    //
    // This was the whole Imports route until the WP-11 subnav arrived; it moved
    // here unchanged so `routes/imports/+page.svelte` could become a tab host and
    // nothing about the editor had to be re-reasoned to get there.
    //
    // This is deliberately NOT a text editor. Anyone who wants to edit the raw
    // text has a terminal and is better served by it; what a GUI can add is the
    // part that is hard to get right by hand — seeing what `%d/%m/%Y` does to a
    // real date, seeing which CSV column column 3 actually is, and reordering
    // rules in a list that says out loud that later matches win.
    //
    // The panel owns three pieces of state and nothing else: which file is
    // selected, the loaded document (`baseDoc`), and the live edit (`form`).
    // Everything structural — what is dirty, what to send, where a new setting
    // goes, what a reorder means — is a pure function in `$lib/imports/model`,
    // because components here are not unit-tested and that is exactly why the
    // logic does not live in them.
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {fieldNames, isDirty, settingText, toForm, toSaveRequest, validateForm, type FormItem, type RulesForm} from "$lib/imports/model";
    import {openRules, rulesIndex, rulesStore} from "$lib/imports/rulesStore.svelte";
    import type {RulesDocument, RulesPreview} from "$lib/imports/types";
    import AccountsPanel from "$lib/imports/ui/AccountsPanel.svelte";
    import PreferencesPanel from "$lib/imports/ui/PreferencesPanel.svelte";
    import RowMappingPanel from "$lib/imports/ui/RowMappingPanel.svelte";
    import RulesFileList from "$lib/imports/ui/RulesFileList.svelte";
    import RulesList from "$lib/imports/ui/RulesList.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {holdUnsavedEdits} from "$lib/stores/refreshAll";
    import {settings} from "$lib/stores/settings.svelte";

    type Tab = "prefs" | "mapping" | "accounts";
    const TABS: {id: Tab; label: string}[] = [
        {id: "prefs", label: "Preferences"},
        {id: "mapping", label: "Row mapping"},
        {id: "accounts", label: "Accounts"},
    ];

    let selectedId = $state<string | null>(null);
    /** The file the user asked for while the open one has unsaved edits. */
    let pendingId = $state<string | null>(null);
    let tab = $state<Tab>("prefs");
    let clientErrors = $state<string[]>([]);
    let serverError = $state<string | null>(null);
    let savedAt = $state<number | null>(null);

    /** The document as it is on disk — the baseline every diff and every Revert is against. */
    let baseDoc = $state<RulesDocument | null>(null);
    /** The live edit. Deeply reactive, so the cards can bind straight into it. */
    let form = $state<RulesForm | null>(null);
    /**
     * Bumped whenever the form is REPLACED wholesale (a new file, or Revert),
     * and never on a save. It keys the editor subtree, so "Revert" also discards
     * the local state the components own — a half-typed account field that has
     * not been committed yet, an open "delete this rule?" confirm — instead of
     * resetting the model and leaving the screen disagreeing with it.
     */
    let formEpoch = $state(0);
    /**
     * A CSV preview re-read after a save, replacing the one the document was
     * opened with — and the fact that one is in flight.
     *
     * THREE states, because two cannot tell the interesting ones apart:
     * `null` is "no save has refetched yet, so use what was opened",
     * `{value: null}` is "the refetch FAILED", which the mapping panel already
     * reports as such, and `previewPending` withholds the old preview entirely
     * while a new one is on its way. Showing the pre-save columns under the
     * post-save settings is the bug; showing them during the refetch would just
     * be the same bug for a shorter time.
     */
    let previewOverride = $state<{value: RulesPreview | null} | null>(null);
    let previewPending = $state(false);
    /**
     * Monotonic, so a slow refetch cannot land on top of a newer one — the
     * stale-response discipline `resource.svelte.ts` documents as invariant 1.
     * Two quick saves put two of these in flight, and without the token the
     * FIRST response can arrive last and win, which is precisely the stale
     * preview this refetch exists to remove, only harder to see.
     */
    let previewSeq = 0;

    // Select a file as soon as the listing arrives, and re-select when a
    // reconnect brings a different listing. Latched on the index OBJECT so
    // writing `selectedId` (which this effect reads) cannot re-trigger it.
    let selectedFor: unknown = null;
    $effect(() => {
        const index = rulesIndex.value;
        if (index === null || index === selectedFor) return;
        selectedFor = index;
        if (selectedId === null || !index.files.some((file) => file.id === selectedId)) {
            selectedId = index.files[0]?.id ?? null;
        }
    });

    // Open whatever is selected. Keyed on the nonce as well as the URL, because
    // a reconnect usually leaves the URL identical (FE-5d).
    let openedKey: string | null = null;
    $effect(() => {
        const url = settings.serverUrl;
        const id = selectedId;
        const key = `${settings.serverNonce}|${url}|${id}`;
        if (url === null || id === null || key === openedKey) return;
        openedKey = key;
        void rulesStore.open(url, id);
    });

    // Seed the form ONCE per loaded document — the `wasOpen` latch from
    // `TransactionModal`, keyed on document identity instead of a boolean. A
    // naive effect here would overwrite what the user is typing on every
    // unrelated reactive tick.
    //
    // Keyed on `id#revision` and no longer on the document OBJECT, which is what
    // `AliasPanel` already does. Object identity made every re-read a re-seed,
    // even of bytes that had not changed — so the header's Refresh button, which
    // now re-opens this document, would have thrown the user back to the
    // Preferences tab and rebuilt every card each time it was pressed. A
    // revision is the engine's own answer to "are these the same bytes".
    let seededFrom: string | null = null;
    $effect(() => {
        const open = openRules.value;
        if (open === null) return;
        const key = `${open.doc.id}#${open.doc.revision}`;
        if (key === seededFrom) return;
        seededFrom = key;
        baseDoc = open.doc;
        form = toForm(open.doc);
        formEpoch += 1;
        // This load carries its OWN preview, so any save-refetched one belonged
        // to the document being replaced. Bumping the token as well as clearing
        // the override is what stops a refetch still in flight for the previous
        // file from landing on this one.
        previewSeq += 1;
        previewOverride = null;
        previewPending = false;
        clientErrors = [];
        serverError = null;
        savedAt = null;
        tab = "prefs";
    });

    const index = $derived(rulesIndex.value);
    /**
     * The CSV preview to show — the refetched one once a save has produced it,
     * the opened one until then, and NOTHING while a refetch is in flight.
     *
     * Not `previewOverride?.value ?? openRules…`: a failed refetch is a real
     * `{value: null}`, and `??` would quietly fall back through it to the
     * pre-save preview — reinstating the stale data precisely when we have just
     * learned we cannot vouch for it.
     */
    const preview = $derived.by(() => {
        if (previewPending) return null;
        return previewOverride === null ? (openRules.value?.preview ?? null) : previewOverride.value;
    });
    /**
     * The loaded document's parse warnings — from `baseDoc`, NOT from
     * `openRules`.
     *
     * A save does not refetch the resource (see `save` below), so `openRules`
     * still holds the parse of the bytes as they were when the file was opened.
     * Reading the warnings off it left a file that had just been fixed still
     * showing the complaint it was fixed for, until something unrelated
     * re-opened it. `baseDoc` is re-seeded from what the engine WROTE and
     * re-parsed, so it is the only one of the two that moves when a save lands.
     *
     * This was never specific to one warning: every diagnostic `doc.warnings()`
     * produces came through the same stale reference.
     */
    const warnings = $derived(baseDoc?.warnings ?? []);
    const baseline = $derived(baseDoc === null ? null : toForm(baseDoc));
    const dirty = $derived(baseline !== null && form !== null && isDirty(baseline, form));
    /** Read-only unless BOTH the server allows writes and this document does. */
    const canEdit = $derived(index?.editable === true && form?.editable === true);
    const disabled = $derived(!canEdit || rulesStore.saving);

    // The header's Refresh button re-opens this document. Say so while there is
    // an unsaved edit in the form, because re-reading the file underneath one
    // replaces the user's typing with the bytes on disk and reports nothing.
    // Withdrawn on unmount as well as on every clean state, so leaving the tab
    // cannot leave a claim behind that blocks refreshes forever.
    $effect(() => {
        holdUnsavedEdits("openRules", dirty);
        return () => holdUnsavedEdits("openRules", false);
    });

    const csvFields = $derived(form === null ? [] : (fieldNames(form.items) ?? []));
    const fallbackAccount = $derived(form === null ? "" : (settingText(form.items, "account2") ?? ""));

    function updateItems(items: FormItem[]): void {
        if (form === null) return;
        form.items = items;
        savedAt = null;
    }

    function requestSelect(id: string): void {
        if (id === selectedId) {
            pendingId = null;
            return;
        }
        if (dirty) {
            pendingId = id;
            return;
        }
        pendingId = null;
        selectedId = id;
    }

    function confirmSwitch(): void {
        const id = pendingId;
        pendingId = null;
        if (id !== null) selectedId = id;
    }

    function revert(): void {
        if (baseDoc === null) return;
        form = toForm(baseDoc);
        formEpoch += 1;
        clientErrors = [];
        serverError = null;
    }

    async function save(): Promise<void> {
        if (form === null || baseline === null) return;
        clientErrors = validateForm(form);
        if (clientErrors.length > 0) return;
        serverError = null;
        const result = await rulesStore.save(form.id, toSaveRequest(baseline, form));
        if (!result.ok) {
            serverError = result.failure.message;
            return;
        }
        // Re-seed from what the engine WROTE, not from what we sent: item ids
        // are a parse's indices and are explicitly not stable across saves, so
        // keeping the old ones would make the next save address items that no
        // longer exist.
        baseDoc = result.doc;
        // The latch moves with it. A save does not refetch `openRules`, so this
        // is the only place that knows the revision advanced — without it the
        // next global refresh would read those same bytes back and re-seed the
        // form for nothing.
        seededFrom = `${result.doc.id}#${result.doc.revision}`;
        form = toForm(result.doc);
        savedAt = Date.now();
        // Not awaited: the save has LANDED, and a data file that cannot be
        // re-read must not turn a successful write into a failure on screen.
        // `refreshPreview` reports its own outcome into the mapping panel.
        void refreshPreview(result.doc.id);
    }

    /**
     * Re-read the CSV preview for the document a save just wrote.
     *
     * Never throws and never reports through `serverError`: the write already
     * succeeded, and this is a decoration on top of it. A failure becomes a
     * `{value: null}` override, which the mapping panel renders as "the preview
     * request failed" — the user is told the columns are unverified rather than
     * shown the pre-save ones as if they still described the file.
     *
     * A superseded response returns without touching `previewPending`: the
     * newer request owns that flag and is still going to clear it.
     */
    async function refreshPreview(id: string): Promise<void> {
        const token = ++previewSeq;
        previewPending = true;
        const url = settings.serverUrl;
        // No server IS a failed refetch, and is reported as one rather than
        // leaving the pre-save preview on screen looking current.
        const value = url === null ? null : await rulesStore.reloadPreview(url, id);
        if (token !== previewSeq) return;
        previewOverride = {value};
        previewPending = false;
    }

    /** Discard the local edit and re-read the file — the only safe answer to a 409. */
    function reloadDiscarding(): void {
        const url = settings.serverUrl;
        if (url === null || selectedId === null) return;
        rulesStore.clearConflict();
        void rulesStore.open(url, selectedId);
    }

    function retryIndex(): void {
        const url = settings.serverUrl;
        if (url !== null) void rulesStore.reloadIndex(url);
    }
</script>

<AsyncSection
    view={rulesIndex.view}
    value={index}
    error={rulesIndex.error}
    testid="imports-error"
    label="import rules"
    loadingLabel="Loading import rules"
    onRetry={retryIndex}
>
    {#snippet children(index)}
        {#if index.files.length === 0}
            <div class="card bg-base-200" data-testid="imports-empty">
                <div class="card-body items-center py-16 text-center">
                    <h2 class="card-title">No CSV import rules yet</h2>
                    <!-- By LABEL, never a path: the engine deliberately
                         never sends one, so there is none to show. -->
                    <p class="text-base-content/60 max-w-lg">
                        Ledgeline looked through <code>{index.rootLabel}</code>, the folder your journal is in, and found no
                        <code>*.rules</code> files. hledger keeps a CSV's import rules in a file beside it — create
                        <code>statement.csv.rules</code> next to <code>statement.csv</code> and it will show up here.
                    </p>
                    {#each index.warnings as warning (warning)}
                        <p class="text-base-content/50 max-w-lg text-xs">{warning}</p>
                    {/each}
                </div>
            </div>
        {:else}
            <div class="grid grid-cols-1 gap-3 lg:grid-cols-[16rem_minmax(0,1fr)]">
                <aside class="flex flex-col gap-2">
                    <h2 class="px-1 text-sm font-semibold tracking-tight">Rules files</h2>
                    <RulesFileList
                        files={index.files}
                        {selectedId}
                        {pendingId}
                        onSelect={requestSelect}
                        onConfirmSwitch={confirmSwitch}
                        onCancelSwitch={() => (pendingId = null)}
                    />
                    <p class="text-base-content/50 px-1 text-xs">Found in <code>{index.rootLabel}</code>, the folder your journal is in.</p>
                    {#if index.truncated}
                        <p class="text-warning px-1 text-xs">There are more rules files than Ledgeline will list; this is a subset.</p>
                    {/if}
                    {#each index.warnings as warning (warning)}
                        <p class="text-base-content/50 px-1 text-xs">{warning}</p>
                    {/each}
                </aside>

                <AsyncSection
                    view={openRules.view}
                    value={form}
                    error={openRules.error}
                    testid="imports-doc-error"
                    label="this rules file"
                    loadingLabel="Loading rules file"
                    onRetry={reloadDiscarding}
                >
                    {#snippet children(form)}
                        <div class="flex flex-col gap-3">
                            <div class="border-base-content/10 bg-base-200 rounded-box flex flex-wrap items-center gap-2 border px-3 py-2">
                                <h2 class="grow truncate text-sm font-semibold tracking-tight" data-testid="imports-open-file">{form.label}</h2>
                                {#if dirty}
                                    <span class="badge badge-warning badge-sm" data-testid="imports-dirty">unsaved</span>
                                {:else if savedAt !== null}
                                    <span class="badge badge-success badge-sm" data-testid="imports-saved">saved</span>
                                {/if}
                                <button type="button" class="btn btn-ghost btn-sm" disabled={!dirty || rulesStore.saving} onclick={revert}>Revert</button>
                                <button type="button" class="btn btn-primary btn-sm" disabled={!dirty || disabled} onclick={save}>
                                    {#if rulesStore.saving}<span class="loading loading-spinner loading-xs"></span>{/if}
                                    Save
                                </button>
                            </div>

                            {#if !canEdit}
                                <div class="alert alert-info rounded-box py-2 text-sm" role="status" data-testid="imports-read-only">
                                    <span>
                                        This file is read-only here. Ledgeline only writes rules files when it was started with a journal file bound to an
                                        editor — everything below still shows exactly what the file says.
                                    </span>
                                </div>
                            {/if}

                            {#if rulesStore.conflict}
                                <!-- `flex` before `flex-col`: daisyUI's `.alert` is
                                     `display:grid; grid-auto-flow:column`, so `flex-col` alone
                                     leaves the button beside the text, not under it. -->
                                <div
                                    class="alert alert-warning rounded-box flex flex-col items-start gap-2 py-2 text-sm"
                                    role="alert"
                                    data-testid="imports-conflict"
                                >
                                    <span>
                                        <code>{form.label}</code> changed on disk since you opened it, so nothing was written. Reload it and re-apply your edit —
                                        saving over it would discard whatever the other change was.
                                    </span>
                                    <button type="button" class="btn btn-sm" onclick={reloadDiscarding}>Reload and discard my changes</button>
                                </div>
                            {/if}

                            {#if warnings.length}
                                <div class="alert alert-warning rounded-box items-start px-3 py-2 text-sm" role="alert" data-testid="imports-warnings">
                                    <ul class="list-inside list-disc">
                                        {#each warnings as warning (warning.line + warning.message)}
                                            <li>{warning.line > 0 ? `Line ${warning.line}: ` : ""}{warning.message}</li>
                                        {/each}
                                    </ul>
                                </div>
                            {/if}

                            <!-- Keyed on the FILE, so switching files rebuilds the panels and the
                                 cards rather than handing them a new document under the old
                                 component's local state — a half-typed account mirror, an open
                                 "delete this rule?" confirm, a custom-date-format field that is
                                 custom for the file you just left. Deliberately not keyed on the
                                 revision: a save must not reset what the user is looking at. -->
                            {#key `${form.id}#${formEpoch}`}
                                <div class="border-base-content/10 rounded-box border">
                                    <div role="tablist" class="tabs tabs-border px-2 pt-1" aria-label="Rules file settings">
                                        {#each TABS as entry (entry.id)}
                                            <button
                                                type="button"
                                                role="tab"
                                                class="tab whitespace-nowrap {entry.id === tab ? 'tab-active' : ''}"
                                                aria-selected={entry.id === tab}
                                                onclick={() => (tab = entry.id)}
                                            >
                                                {entry.label}
                                            </button>
                                        {/each}
                                    </div>
                                    <div class="p-3">
                                        {#if tab === "prefs"}
                                            <PreferencesPanel items={form.items} source={baseDoc?.settings.source ?? null} onChange={updateItems} {disabled} />
                                        {:else if tab === "mapping"}
                                            <RowMappingPanel items={form.items} {preview} pending={previewPending} onChange={updateItems} {disabled} />
                                        {:else}
                                            <AccountsPanel items={form.items} accountNames={journal.accountNames} onChange={updateItems} {disabled} />
                                        {/if}
                                    </div>
                                </div>

                                <RulesList
                                    items={form.items}
                                    accountNames={journal.accountNames}
                                    {csvFields}
                                    {fallbackAccount}
                                    onChange={updateItems}
                                    {disabled}
                                />
                            {/key}

                            {#if clientErrors.length > 0}
                                <ul class="text-error list-inside list-disc text-sm" role="alert" data-testid="imports-client-errors">
                                    {#each clientErrors as message (message)}
                                        <li>{message}</li>
                                    {/each}
                                </ul>
                            {/if}
                            {#if serverError !== null}
                                <div class="alert alert-error py-2 text-sm" role="alert" data-testid="imports-server-error">
                                    <span class="break-words">{serverError}</span>
                                </div>
                            {/if}
                        </div>
                    {/snippet}
                </AsyncSection>
            </div>
        {/if}
    {/snippet}
</AsyncSection>
