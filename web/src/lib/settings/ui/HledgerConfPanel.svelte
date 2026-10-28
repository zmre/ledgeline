<!-- Settings › hledger config (plan 30, Decision 10): the `hledger.conf` in
     force — found the way hledger finds it — and one control per option
     Ledgeline honours from it: general-section `--auto`, and `--depth` (plain
     and per-account) in the general section and the four report sections.
     Nothing for the options it ignores; those stay exactly as written.

     Every decision is a call into `hledgerConfModel.ts`; this file is markup,
     the split `AccountsPanel.svelte` states for its own screen. -->
<script lang="ts">
    import AsyncSection from "#lib/components/AsyncSection.svelte";
    import {dataView} from "#lib/stores/loadState.js";
    import {holdUnsavedEdits} from "#lib/stores/refreshAll.js";
    import {onServerReady} from "#lib/stores/serverWatch.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {hledgerConf, hledgerConfStore} from "../hledgerConfStore.svelte";
    import {
        blankOverride,
        isDirty,
        SECTION_EFFECT,
        SECTION_LABELS,
        toForm,
        toSaveBody,
        validateForm,
        type ConfForm,
        type SectionForm,
    } from "../hledgerConfModel";
    import {CONF_SECTIONS, type ConfSection, type HledgerConf} from "../hledgerConfTypes";

    onServerReady((url) => {
        void hledgerConfStore.ensure(url, String(settings.serverNonce));
    });

    const conf = $derived(hledgerConf.value);
    const view = $derived(dataView(hledgerConf.status, conf !== null));

    let form = $state<ConfForm | null>(null);
    let clientErrors = $state<string[]>([]);
    let serverError = $state<string | null>(null);
    let savedAt = $state<number | null>(null);

    // The latch is a PLAIN `let`, as in AccountsPanel: re-seeding must not
    // depend on itself. A new revision (or file) re-seeds; a re-read of the
    // same one leaves the user's edits alone.
    let seededFor: string | null = null;
    const keyOf = (c: HledgerConf): string => `${c.handle ?? ""}#${c.revision}`;

    $effect(() => {
        if (conf === null) return;
        const key = keyOf(conf);
        if (key === seededFor) return;
        seededFor = key;
        form = toForm(conf);
        clientErrors = [];
        serverError = null;
    });

    const dirty = $derived(conf !== null && form !== null && isDirty(conf, form));
    const disabled = $derived(conf === null || !conf.editable || hledgerConfStore.saving);

    $effect(() => {
        holdUnsavedEdits("hledgerConf", dirty);
        return () => holdUnsavedEdits("hledgerConf", false);
    });

    function updateSection(section: ConfSection, patch: Partial<SectionForm>): void {
        if (form === null) return;
        form = {...form, sections: {...form.sections, [section]: {...form.sections[section], ...patch}}};
        savedAt = null;
    }

    function updateOverride(section: ConfSection, at: number, patch: {regex?: string; depth?: string}): void {
        if (form === null) return;
        updateSection(section, {overrides: form.sections[section].overrides.map((o, i) => (i === at ? {...o, ...patch} : o))});
    }

    async function save(): Promise<void> {
        if (conf === null || form === null) return;
        clientErrors = validateForm(form);
        serverError = null;
        if (clientErrors.length > 0) return;
        const outcome = await hledgerConfStore.save(toSaveBody(conf, form));
        if (outcome.ok) {
            seededFor = keyOf(outcome.conf);
            form = toForm(outcome.conf);
            savedAt = Date.now();
        } else {
            serverError = outcome.failure.message;
        }
    }

    function retry(): void {
        const url = settings.serverUrl;
        if (url !== null) void hledgerConfStore.reload(url);
    }
</script>

<AsyncSection
    {view}
    value={conf}
    error={hledgerConf.error}
    testid="settings-hledger-conf-error"
    label="your hledger config"
    loadingLabel="Reading your hledger config"
    onRetry={retry}
>
    {#snippet children(loaded)}
        <div class="flex max-w-3xl flex-col gap-4" data-testid="settings-hledger-conf">
            <p class="text-sm text-base-content/70">
                hledger reads one config file for every command you run: the nearest <code>hledger.conf</code> at or above your journal's folder, else
                <code>~/.hledger.conf</code>, else <code>~/.config/hledger/hledger.conf</code>. Ledgeline reads the options below from it and writes only the
                lines you change here; everything else in the file is left as it is.
            </p>

            <div class="flex flex-wrap items-baseline gap-2 text-sm" data-testid="settings-hledger-conf-file">
                <span class="font-semibold">Config file:</span>
                {#if loaded.handle !== null}
                    <code data-testid="settings-hledger-conf-handle">{loaded.handle}</code>
                {:else if loaded.createHandle !== null}
                    <span data-testid="settings-hledger-conf-handle">none — saving will create <code>{loaded.createHandle}</code> beside your main journal</span
                    >
                {:else}
                    <span data-testid="settings-hledger-conf-handle">none</span>
                {/if}
            </div>

            {#if loaded.hijacked !== null}
                <div class="alert items-start rounded-box py-2 text-sm alert-error" role="alert" data-testid="settings-hledger-conf-hijacked">
                    <span>{loaded.hijacked}</span>
                </div>
            {:else if !loaded.editable && loaded.readOnlyReason !== null}
                <div class="alert items-start rounded-box py-2 text-sm alert-info" role="status" data-testid="settings-hledger-conf-read-only">
                    <span>Shown but not editable here: {loaded.readOnlyReason}</span>
                </div>
            {/if}

            {#if loaded.warnings.length > 0}
                <ul
                    class="alert flex list-inside list-disc flex-col items-start rounded-box py-2 text-sm alert-warning"
                    data-testid="settings-hledger-conf-warnings"
                >
                    {#each loaded.warnings as warning, i (i)}
                        <li>{warning.message}</li>
                    {/each}
                </ul>
            {/if}

            {#if hledgerConfStore.conflict}
                <div
                    class="alert flex flex-col items-start gap-2 rounded-box py-2 text-sm alert-warning"
                    role="alert"
                    data-testid="settings-hledger-conf-conflict"
                >
                    <span>The config file changed on disk since you opened it, so nothing was written. Reload it and re-apply your change.</span>
                    <button type="button" class="btn btn-sm" onclick={retry}>Reload and discard my changes</button>
                </div>
            {/if}

            {#if form !== null}
                <label class="flex items-start gap-2 text-sm">
                    <input
                        type="checkbox"
                        class="checkbox mt-0.5 checkbox-sm"
                        checked={form.auto}
                        {disabled}
                        data-testid="settings-hledger-conf-auto"
                        onchange={(event) => {
                            if (form !== null) form = {...form, auto: event.currentTarget.checked};
                            savedAt = null;
                        }}
                    />
                    <span>
                        Apply automatic postings (<code>--auto</code>)
                        <span class="block text-xs text-base-content/60">
                            The journal's <code>= QUERY</code> rules add postings to matching transactions, for every hledger command and in Ledgeline.
                        </span>
                    </span>
                </label>

                <div class="flex flex-col gap-3">
                    <h3 class="text-sm font-semibold">Report depth (<code>--depth</code>)</h3>
                    {#each CONF_SECTIONS as section (section)}
                        {@const sectionForm = form.sections[section]}
                        <fieldset class="rounded-box border border-base-300 p-3" data-testid={`settings-hledger-conf-depth-${section}`}>
                            <legend class="px-1 text-sm font-medium">
                                {SECTION_LABELS[section]}
                                {#if section !== "general"}<code class="text-xs text-base-content/60">[{section}]</code>{/if}
                            </legend>
                            <p class="mb-2 text-xs text-base-content/60">{SECTION_EFFECT[section]}</p>
                            <label class="flex items-center gap-2 text-sm">
                                <span class="w-20">Depth</span>
                                <input
                                    class="input w-20 input-sm"
                                    inputmode="numeric"
                                    placeholder="not set"
                                    aria-label={`${SECTION_LABELS[section]} depth`}
                                    value={sectionForm.all}
                                    {disabled}
                                    oninput={(event) => updateSection(section, {all: event.currentTarget.value})}
                                />
                                {#if section !== "general" && loaded.effectiveDepth[section].all !== null}
                                    <span class="text-xs text-base-content/60">in force: {loaded.effectiveDepth[section].all}</span>
                                {/if}
                            </label>
                            {#each sectionForm.overrides as override, at (at)}
                                <div class="mt-2 flex flex-wrap items-center gap-2 text-sm" data-testid="settings-hledger-conf-override">
                                    <input
                                        class="input w-56 font-mono input-sm"
                                        placeholder="account regex, e.g. ^assets:inv"
                                        aria-label="Account pattern"
                                        value={override.regex}
                                        {disabled}
                                        oninput={(event) => updateOverride(section, at, {regex: event.currentTarget.value})}
                                    />
                                    <span>=</span>
                                    <input
                                        class="input w-16 input-sm"
                                        inputmode="numeric"
                                        aria-label="Depth for this pattern"
                                        value={override.depth}
                                        {disabled}
                                        oninput={(event) => updateOverride(section, at, {depth: event.currentTarget.value})}
                                    />
                                    <button
                                        type="button"
                                        class="btn btn-ghost btn-xs"
                                        {disabled}
                                        onclick={() => updateSection(section, {overrides: sectionForm.overrides.filter((_, i) => i !== at)})}
                                    >
                                        Remove
                                    </button>
                                </div>
                            {/each}
                            <button
                                type="button"
                                class="btn mt-2 btn-ghost btn-xs"
                                {disabled}
                                data-testid={`settings-hledger-conf-add-override-${section}`}
                                onclick={() => updateSection(section, {overrides: [...sectionForm.overrides, blankOverride()]})}
                            >
                                Add a per-account depth
                            </button>
                        </fieldset>
                    {/each}
                </div>

                <div class="flex flex-wrap items-center gap-2">
                    <button type="button" class="btn btn-primary btn-sm" disabled={disabled || !dirty} onclick={save} data-testid="settings-hledger-conf-save">
                        {#if hledgerConfStore.saving}<span class="loading loading-xs loading-spinner"></span>{/if}
                        Save
                    </button>
                    {#if dirty}<span class="badge badge-sm badge-warning" data-testid="settings-hledger-conf-dirty">unsaved</span>{/if}
                    {#if savedAt !== null && !dirty}<span class="badge badge-sm badge-success" data-testid="settings-hledger-conf-saved">saved</span>{/if}
                </div>

                {#if clientErrors.length > 0}
                    <ul class="alert list-inside list-disc rounded-box py-2 text-sm alert-error" data-testid="settings-hledger-conf-client-errors">
                        {#each clientErrors as problem (problem)}
                            <li>{problem}</li>
                        {/each}
                    </ul>
                {/if}
                {#if serverError !== null}
                    <div class="alert items-start rounded-box py-2 text-sm alert-error" role="alert" data-testid="settings-hledger-conf-server-error">
                        <span>{serverError}</span>
                    </div>
                {/if}
            {/if}
        </div>
    {/snippet}
</AsyncSection>
